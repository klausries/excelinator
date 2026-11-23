//! The data model: a workbook's cells as one long, column-oriented table.
//!
//! Every column is a plain vector of the same length, so the table maps one to one
//! onto a dataframe (Polars, Arrow) without this crate depending on either.

use std::sync::Arc;

/// What a cell holds. The numeric values are part of the public layout.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// No value (a styled or formula cell without a cached value).
    Blank = 0,
    /// A number in `num`. Dates are numbers with a date number format.
    Number = 1,
    /// A string in `text`.
    Text = 2,
    /// A boolean; `num` is 0.0 or 1.0.
    Bool = 3,
    /// An error; `text` is its literal such as `#DIV/0!`.
    Error = 4,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Kind> {
        [Kind::Blank, Kind::Number, Kind::Text, Kind::Bool, Kind::Error].get(v as usize).copied()
    }
}

/// One cell value, for convenient access by row.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
}

/// All cells of a workbook. Rows and columns are 0-based. All vectors have equal length.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cells {
    pub sheet: Vec<u32>,
    pub row: Vec<u32>,
    pub col: Vec<u32>,
    pub kind: Vec<Kind>,
    pub num: Vec<f64>,
    /// Shared: cells holding the same shared string point to one allocation.
    pub text: Vec<Option<Arc<str>>>,
    /// Formula text without the leading `=`, in the file's (English, comma) notation.
    pub formula: Vec<Option<String>>,
    /// Index into `Workbook::styles`.
    pub style: Vec<u32>,
    /// A whole number below 1e16 that the file spells with a decimal point or exponent
    /// (`1.0`, `1E2`). Kept so that the spelling survives a round trip; libraries such
    /// as openpyxl read `1` as an integer and `1.0` as a float.
    pub float_text: Vec<bool>,
    /// For an array formula: the range it fills, such as `N2` or `A1:B3`.
    pub array_ref: Vec<Option<String>>,
    /// For a text cell whose string says more than its plain text (formatted runs,
    /// phonetic guides): the content of its `<si>` or `<is>` element as XML, without the
    /// element itself. The concatenated `<t>` of its runs is `text`. Shared like `text`.
    pub rich: Vec<Option<Arc<str>>>,
    /// A number the file gives as an ISO 8601 date cell (`t="d"`): how it was written,
    /// and is written again. The number is the serial date.
    pub iso: Vec<IsoDate>,
}

/// The form of an ISO 8601 date cell (`t="d"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsoDate {
    /// Not an ISO date cell.
    #[default]
    No,
    /// `2021-01-01`
    Date,
    /// `2021-01-01T10:10:10`
    DateTime,
    /// `10:10:10`
    Time,
}

impl IsoDate {
    /// The serial number of an ISO 8601 date, date and time, or time, with its form.
    pub fn parse(text: &str, date1904: bool) -> Option<(f64, IsoDate)> {
        let system = if date1904 { xl_serial::DateSystem::V1904 } else { xl_serial::DateSystem::V1900 };
        let t = text.trim();
        let t = t.strip_suffix('Z').unwrap_or(t);
        let time = |s: &str| -> Option<f64> {
            let mut parts = s.split(':');
            let h: u32 = parts.next()?.parse().ok()?;
            let m: u32 = parts.next()?.parse().ok()?;
            let sec: f64 = parts.next().unwrap_or("0").parse().ok()?;
            if parts.next().is_some() || h > 24 || (h == 24 && (m > 0 || sec > 0.0)) || m > 59 || !(0.0..60.0).contains(&sec) {
                return None;
            }
            Some((h as f64 * 3600.0 + m as f64 * 60.0 + sec) / 86400.0)
        };
        let date = |s: &str| -> Option<f64> {
            let mut parts = s.splitn(3, '-');
            let y: i32 = parts.next()?.parse().ok()?;
            let mo: u32 = parts.next()?.parse().ok()?;
            let d: u32 = parts.next()?.parse().ok()?;
            Some(xl_serial::serial_from_ymd(y, mo, d, system)? as f64)
        };
        if let Some((d, tm)) = t.split_once('T') {
            return Some((date(d)? + time(tm)?, IsoDate::DateTime));
        }
        if t.contains(':') {
            return Some((time(t)?, IsoDate::Time));
        }
        Some((date(t)?, IsoDate::Date))
    }

    /// The ISO 8601 text of a serial number in this form (milliseconds where there are any).
    pub fn format(self, serial: f64, date1904: bool) -> Option<String> {
        let system = if date1904 { xl_serial::DateSystem::V1904 } else { xl_serial::DateSystem::V1900 };
        if !serial.is_finite() || serial < 0.0 {
            return None;
        }
        let ms_total = (serial * 86_400_000.0).round() as i64;
        let (day, ms) = (ms_total.div_euclid(86_400_000), ms_total.rem_euclid(86_400_000));
        let clock = || {
            let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
            if f == 0 { format!("{h:02}:{m:02}:{s:02}") } else { format!("{h:02}:{m:02}:{s:02}.{f:03}") }
        };
        let ymd = || xl_serial::ymd_from_serial(day, system).map(|(y, mo, d)| format!("{y:04}-{mo:02}-{d:02}"));
        match self {
            IsoDate::No => None,
            IsoDate::Date if ms == 0 => ymd(),
            IsoDate::Date | IsoDate::DateTime => Some(format!("{}T{}", ymd()?, clock())),
            IsoDate::Time if day == 0 => Some(clock()),
            IsoDate::Time => Some(format!("{}T{}", ymd()?, clock())),
        }
    }
}

impl Cells {
    pub fn len(&self) -> usize {
        self.row.len()
    }

    pub fn is_empty(&self) -> bool {
        self.row.is_empty()
    }

    pub fn push(&mut self, sheet: u32, row: u32, col: u32, value: Value, formula: Option<String>, style: u32) {
        let (kind, num, text) = match value {
            Value::Blank => (Kind::Blank, 0.0, None),
            Value::Number(n) => (Kind::Number, n, None),
            Value::Text(s) => (Kind::Text, 0.0, Some(Arc::from(s))),
            Value::Bool(b) => (Kind::Bool, b as u8 as f64, None),
            Value::Error(s) => (Kind::Error, 0.0, Some(Arc::from(s))),
        };
        self.push_raw(sheet, row, col, kind, num, text, formula, style);
    }

    /// Append a cell from its column values. `text` may be shared between cells.
    #[allow(clippy::too_many_arguments)]
    pub fn push_raw(&mut self, sheet: u32, row: u32, col: u32, kind: Kind, num: f64, text: Option<Arc<str>>, formula: Option<String>, style: u32) {
        self.sheet.push(sheet);
        self.row.push(row);
        self.col.push(col);
        self.kind.push(kind);
        self.num.push(num);
        self.text.push(text);
        self.formula.push(formula);
        self.style.push(style);
        self.float_text.push(false);
        self.array_ref.push(None);
        self.rich.push(None);
        self.iso.push(IsoDate::No);
    }

    pub fn value(&self, i: usize) -> Value {
        match self.kind[i] {
            Kind::Blank => Value::Blank,
            Kind::Number => Value::Number(self.num[i]),
            Kind::Text => Value::Text(self.text[i].as_deref().unwrap_or_default().to_string()),
            Kind::Bool => Value::Bool(self.num[i] != 0.0),
            Kind::Error => Value::Error(self.text[i].as_deref().unwrap_or_default().to_string()),
        }
    }

    /// Keep the cells `keep` says yes to (one answer per cell), in their order.
    pub fn retain(&mut self, keep: &[bool]) {
        macro_rules! each {
            ($($field:ident),*) => {$({
                let mut k = 0;
                self.$field.retain(|_| {
                    k += 1;
                    keep[k - 1]
                });
            })*};
        }
        each!(sheet, row, col, kind, num, text, formula, style, float_text, array_ref, rich, iso);
    }

    /// Indices in (sheet, row, col) order. Of cells written to the same place the last one wins.
    pub fn sorted_unique(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.len()).collect();
        idx.sort_by_key(|&i| (self.sheet[i], self.row[i], self.col[i], i));
        let mut out: Vec<usize> = Vec::with_capacity(idx.len());
        for i in idx {
            match out.last_mut() {
                Some(last) if (self.sheet[*last], self.row[*last], self.col[*last]) == (self.sheet[i], self.row[i], self.col[i]) => *last = i,
                _ => out.push(i),
            }
        }
        out
    }
}

/// One side of a cell border.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Border {
    /// `thin`, `medium`, `thick`, `dashed`, `dotted`, `double`, `hair`, ...
    pub style: String,
    /// ARGB such as `FF000000`.
    pub color: Option<String>,
}

/// Cell formatting. Index 0 of `Workbook::styles` is the default style.
/// Colours are ARGB hex strings (`FFFF0000`); theme and indexed colours are not modelled.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// Number format code, `General` by default.
    pub num_format: String,
    pub bold: bool,
    pub italic: bool,
    /// `single`, `double`, `singleAccounting`, `doubleAccounting`.
    pub underline: Option<String>,
    pub strike: bool,
    /// `None` is the default font (Calibri).
    pub font_name: Option<String>,
    /// In points; `None` is the default size (11).
    pub font_size: Option<f64>,
    pub font_color: Option<String>,
    /// Solid fill colour.
    pub fill: Option<String>,
    /// Left, right, top, bottom.
    pub borders: [Option<Border>; 4],
    /// `left`, `center`, `right`, `fill`, `justify`, `centerContinuous`, `distributed`, `general`.
    pub h_align: Option<String>,
    /// `top`, `center`, `bottom`, `justify`, `distributed`.
    pub v_align: Option<String>,
    pub wrap: bool,
    pub rotation: u32,
    pub indent: u32,
    /// The font, fill and border as XML (`<font>…</font>` and so on) when they hold more
    /// than the fields above say: theme and indexed colours, tints, gradients, other
    /// patterns, diagonal borders, font families and schemes. Written in place of the
    /// fields; the fields still give what they can.
    pub font_xml: Option<String>,
    pub fill_xml: Option<String>,
    pub border_xml: Option<String>,
    /// `<alignment …/>` of the cell format when it holds more than the alignment fields
    /// (shrink to fit, reading order, …); written in place of them.
    pub alignment_xml: Option<String>,
    /// `<protection …/>` of the cell format (locked, hidden).
    pub protection_xml: Option<String>,
    /// Further attributes of the cell format (`quotePrefix`, `pivotButton`).
    pub xf_attrs: Vec<(String, String)>,
    /// The named cell style the format is based on (`Good`, a style of the file);
    /// `None` is `Normal`.
    pub named: Option<String>,
}

/// A named cell style (`Normal`, `Good`, `Heading 1`, one of the file's own): its name,
/// its format and how Excel lists it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NamedStyle {
    pub name: String,
    pub style: Style,
    /// Number of a built-in style (0 is Normal).
    pub builtin_id: Option<u32>,
    pub hidden: bool,
    pub custom_builtin: bool,
    pub i_level: Option<u32>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            num_format: "General".into(),
            bold: false,
            italic: false,
            underline: None,
            strike: false,
            font_name: None,
            font_size: None,
            font_color: None,
            fill: None,
            borders: [None, None, None, None],
            h_align: None,
            v_align: None,
            wrap: false,
            rotation: 0,
            indent: 0,
            font_xml: None,
            fill_xml: None,
            border_xml: None,
            alignment_xml: None,
            protection_xml: None,
            xf_attrs: Vec::new(),
            named: None,
        }
    }
}

const SEP: char = '\u{1f}';
const SIDES: [&str; 4] = ["bl", "br", "bt", "bb"];

impl Style {
    /// Whether the number format shows a date or time (so a number is a date serial).
    pub fn is_date(&self) -> bool {
        is_date_format(&self.num_format)
    }

    /// The style as one string of `key=value` fields separated by U+001F; the form in
    /// which styles cross the language bindings. Keys: fmt, b, i, u, strike, font, sz,
    /// color, fill, bl/br/bt/bb (`style` or `style:ARGB`), h, v, wrap, rot, indent;
    /// fontxml, fillxml, borderxml, alignxml, protxml, xf.<attribute>, named.
    pub fn to_spec(&self) -> String {
        let mut f: Vec<String> = vec![format!("fmt={}", self.num_format)];
        let mut put = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                f.push(format!("{k}={v}"));
            }
        };
        put("b", self.bold.then(|| "1".into()));
        put("i", self.italic.then(|| "1".into()));
        put("u", self.underline.clone());
        put("strike", self.strike.then(|| "1".into()));
        put("font", self.font_name.clone());
        put("sz", self.font_size.map(|v| v.to_string()));
        put("color", self.font_color.clone());
        put("fill", self.fill.clone());
        for (k, b) in SIDES.iter().zip(&self.borders) {
            put(k, b.as_ref().map(|b| match &b.color {
                Some(c) => format!("{}:{c}", b.style),
                None => b.style.clone(),
            }));
        }
        put("h", self.h_align.clone());
        put("v", self.v_align.clone());
        put("wrap", self.wrap.then(|| "1".into()));
        put("rot", (self.rotation != 0).then(|| self.rotation.to_string()));
        put("indent", (self.indent != 0).then(|| self.indent.to_string()));
        put("fontxml", self.font_xml.clone());
        put("fillxml", self.fill_xml.clone());
        put("borderxml", self.border_xml.clone());
        put("alignxml", self.alignment_xml.clone());
        put("protxml", self.protection_xml.clone());
        for (k, v) in &self.xf_attrs {
            put(&format!("xf.{k}"), Some(v.clone()));
        }
        put("named", self.named.clone());
        f.join(&SEP.to_string())
    }

    /// Inverse of [`Style::to_spec`]. Unknown keys are ignored.
    pub fn from_spec(spec: &str) -> Style {
        let mut s = Style::default();
        for field in spec.split(SEP) {
            let Some((k, v)) = field.split_once('=') else { continue };
            let text = || Some(v.to_string());
            match k {
                "fmt" => s.num_format = v.to_string(),
                "b" => s.bold = v == "1",
                "i" => s.italic = v == "1",
                "u" => s.underline = text(),
                "strike" => s.strike = v == "1",
                "font" => s.font_name = text(),
                "sz" => s.font_size = v.parse().ok(),
                "color" => s.font_color = text(),
                "fill" => s.fill = text(),
                "h" => s.h_align = text(),
                "v" => s.v_align = text(),
                "wrap" => s.wrap = v == "1",
                "rot" => s.rotation = v.parse().unwrap_or(0),
                "indent" => s.indent = v.parse().unwrap_or(0),
                "fontxml" => s.font_xml = text(),
                "fillxml" => s.fill_xml = text(),
                "borderxml" => s.border_xml = text(),
                "alignxml" => s.alignment_xml = text(),
                "protxml" => s.protection_xml = text(),
                "named" => s.named = text(),
                attr if attr.starts_with("xf.") => s.xf_attrs.push((attr[3..].to_string(), v.to_string())),
                side => {
                    if let Some(i) = SIDES.iter().position(|x| *x == side) {
                        let (style, color) = match v.split_once(':') {
                            Some((a, c)) => (a, Some(c.to_string())),
                            None => (v, None),
                        };
                        s.borders[i] = Some(Border { style: style.to_string(), color });
                    }
                }
            }
        }
        s
    }
}

/// Whether a number format code shows a date or time. Quoted text, escaped characters
/// and bracketed colour/condition/locale sections are ignored; `[h]`, `[m]`, `[s]`
/// (elapsed time) count.
pub fn is_date_format(code: &str) -> bool {
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            '[' => {
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d.to_ascii_lowercase());
                }
                if !inner.is_empty() && (inner.chars().all(|x| x == 'h') || inner.chars().all(|x| x == 'm') || inner.chars().all(|x| x == 's')) {
                    return true;
                }
            }
            'd' | 'D' | 'm' | 'M' | 'y' | 'Y' | 'h' | 'H' | 's' | 'S' => return true,
            _ => {}
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetKind {
    #[default]
    Worksheet,
    Chartsheet,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

/// The attributes of a range of columns (`<col>`). Columns are 0-based, inclusive.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Column {
    pub first: u32,
    pub last: u32,
    /// Width in characters of the default font's digits.
    pub width: Option<f64>,
    /// The width was set by hand (`customWidth`).
    pub custom_width: bool,
    pub hidden: bool,
    /// The width fits the content (`bestFit`).
    pub best_fit: bool,
    /// Outline (grouping) level, 0 to 7.
    pub outline_level: u8,
    /// The outline group these columns close is collapsed.
    pub collapsed: bool,
    /// Index into `Workbook::styles` for empty cells of these columns.
    pub style: Option<u32>,
    /// Show phonetic guides (`phonetic`).
    pub phonetic: bool,
}

impl Column {
    /// A column range that only has a width.
    pub fn width(first: u32, last: u32, width: f64) -> Self {
        Column { first, last, width: Some(width), custom_width: true, ..Column::default() }
    }
}

/// The attributes of a row (`<row>`), 0-based.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub row: u32,
    /// Height in points.
    pub height: Option<f64>,
    /// The height was set by hand (`customHeight`).
    pub custom_height: bool,
    pub hidden: bool,
    /// Outline (grouping) level, 0 to 7.
    pub outline_level: u8,
    /// The outline group this row closes is collapsed.
    pub collapsed: bool,
    /// Index into `Workbook::styles` (`s`); applies to empty cells when `custom_format`.
    pub style: Option<u32>,
    pub custom_format: bool,
    /// Thick top or bottom border on cells of the row (`thickTop`, `thickBot`).
    pub thick_top: bool,
    pub thick_bot: bool,
    /// Show phonetic guides (`ph`).
    pub phonetic: bool,
}

impl Row {
    /// A row that only has a height.
    pub fn height(row: u32, height: f64) -> Self {
        Row { row, height: Some(height), custom_height: true, ..Row::default() }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub kind: SheetKind,
    pub visibility: Visibility,
    /// Merged ranges as (first row, first col, last row, last col), 0-based, inclusive.
    pub merges: Vec<(u32, u32, u32, u32)>,
    /// Column ranges with width or other attributes (`<col>`), ascending, not overlapping.
    pub columns: Vec<Column>,
    /// Rows with a height or other attributes (`<row>`), ascending by row.
    pub rows: Vec<Row>,
    /// Frozen panes: number of rows and of columns that stay in place.
    pub freeze: Option<(u32, u32)>,
    /// Range of the auto filter, such as `A1:D20`.
    pub auto_filter: Option<String>,
    pub validations: Vec<Validation>,
    pub hyperlinks: Vec<Hyperlink>,
    pub cond_formats: Vec<CondFormat>,
    pub comments: Vec<Comment>,
    /// Pictures and charts placed on the sheet. Empty if the sheet's drawing holds
    /// anything else (shapes, groups); such a drawing is only kept when saving onto
    /// the original file.
    pub drawings: Vec<Drawing>,
    /// Tables (list objects), each as the XML of its table part (root `<table>`). Ids are
    /// kept when every table has one and no two share it, else all are numbered anew;
    /// display names must be unique in the workbook, ignoring case.
    pub tables: Vec<String>,
    /// The used range as the file states it (`<dimension>`): first row, first column,
    /// last row, last column, 0-based. Read only: writing computes it from the cells.
    pub dimension: Option<(u32, u32, u32, u32)>,
    /// Children of `<worksheet>` this crate does not model, by element name, each as XML
    /// that stands on its own (`sheetPr`, `sheetViews`, `sheetFormatPr`, `sheetProtection`,
    /// `printOptions`, `pageMargins`, `pageSetup`, `headerFooter`, `rowBreaks`, `colBreaks`,
    /// `autoFilter`, …). Written in schema order; `sheetViews` here takes the place of
    /// `freeze`, `autoFilter` the place of `auto_filter`.
    pub extra: Vec<(String, String)>,
    /// Position of this sheet in the file it was read from; `None` for a new sheet.
    /// Saving onto the original uses it to tell a renamed or moved sheet from a new one.
    pub origin: Option<u32>,
}

/// A data validation rule. The attributes are kept as written (`type`, `operator`,
/// `allowBlank`, `showErrorMessage`, `errorTitle`, `error`, `promptTitle`, `prompt`,
/// `sqref`, ...), so nothing of a rule is lost on the way through.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Validation {
    pub attrs: Vec<(String, String)>,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
}

/// Where a picture or chart sits: its top-left cell with offsets (in EMU, 914400 per
/// inch), and either a bottom-right cell or a size.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Anchor {
    /// (column, column offset, row, row offset), 0-based.
    pub from: (u32, i64, u32, i64),
    pub to: Option<(u32, i64, u32, i64)>,
    /// (width, height) in EMU.
    pub ext: Option<(i64, i64)>,
}

/// A picture (file content and extension such as `png`) or a chart (its part as XML).
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    pub anchor: Anchor,
    pub image: Option<(Arc<[u8]>, String)>,
    pub chart: Option<Arc<str>>,
}

/// A comment (note) on a cell. The text is plain; formatting inside a comment is not kept.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub cell: String,
    pub author: String,
    pub text: String,
}

/// Conditional formatting of a range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CondFormat {
    /// The ranges, such as `A1:A10 C1:C10`.
    pub sqref: String,
    pub rules: Vec<CondRule>,
}

/// One rule. Its attributes (`type`, `operator`, `priority`, `stopIfTrue`, ...) and
/// its content (formulas, colour scale, data bar, icon set) are kept as written, so
/// every kind of rule passes through; `dxf` is the format the rule applies.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CondRule {
    pub attrs: Vec<(String, String)>,
    /// Child elements of the rule as XML in the spreadsheet namespace.
    pub inner: String,
    /// Content of the differential format (`<font>`, `<fill>`, `<border>`, ...) as XML.
    pub dxf: Option<String>,
}

/// A hyperlink on a cell or range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hyperlink {
    /// Cell or range, such as `A1`.
    pub cell: String,
    /// Address outside the workbook (URL, file, mail).
    pub target: Option<String>,
    /// Place inside a workbook, such as `Sheet2!A1`.
    pub location: Option<String>,
    pub tooltip: Option<String>,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DefinedName {
    /// Not shown in the name manager (as the names Excel keeps for filters).
    pub hidden: bool,
    pub name: String,
    /// Index of the sheet the name is local to; `None` for workbook scope.
    pub local_sheet: Option<u32>,
    pub value: String,
}

/// A workbook this one refers to (`[1]Sheet1!A1`): what the file kept of its cells
/// (xl/externalLinks). The order is the one the formulas count by.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct External {
    /// Where the file said the workbook is.
    pub target: Option<String>,
    pub sheets: Vec<String>,
    /// Names defined in that workbook: name and what it refers to (as written there).
    pub names: Vec<(String, String)>,
    /// Sheet index into `sheets`, 0-based row and column, and the cached value.
    pub cells: Vec<(u32, u32, u32, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    /// Dates count from 1904 instead of 1900.
    pub date1904: bool,
    pub defined_names: Vec<DefinedName>,
    pub styles: Vec<Style>,
    pub cells: Cells,
    /// Workbooks referred to, as the file kept them. Read only: writing a new file does
    /// not emit them; saving onto the original leaves them as they were.
    pub externals: Vec<External>,
    /// The named cell styles, in the order of the file. Empty means `Normal` alone.
    pub named_styles: Vec<NamedStyle>,
    /// Children of `<workbook>` this crate does not model, by element name, each as XML
    /// that stands on its own (`fileVersion`, `fileSharing`, `workbookPr`,
    /// `workbookProtection`, `bookViews`, `calcPr`, ...). `workbookPr` takes the place of
    /// the one written for `date1904` (its `date1904` is made to agree), `bookViews` that
    /// of the default view; `calcPr` gets `fullCalcOnLoad` when formulas lack values.
    pub extra: Vec<(String, String)>,
    /// Document properties as package parts by kind, each as XML: `core` (title, author,
    /// dates; `docProps/core.xml`), `app` (application; `docProps/app.xml`), `custom`.
    pub properties: Vec<(String, String)>,
    /// The workbook's theme (`xl/theme/theme1.xml`) as XML: its colours and fonts are what
    /// theme colours (`<color theme="4" tint="0.4"/>`) and theme fonts stand for. `None`:
    /// the Office default.
    pub theme: Option<Arc<str>>,
    /// The workbook's own colours for indexed colours 0 to 63 (`<indexedColors>`), ARGB;
    /// empty: the standard palette.
    pub palette: Vec<String>,
    /// The workbook's own table styles (`<tableStyles>`), and the names of the default
    /// table and pivot table styles.
    pub table_styles: Vec<TableStyle>,
    pub default_table_style: Option<String>,
    pub default_pivot_style: Option<String>,
}

/// A table style of the workbook: which parts of a table (`wholeTable`, `headerRow`,
/// `firstRowStripe`, ...) get which differential format.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableStyle {
    pub name: String,
    /// Usable for tables, for pivot tables (both when neither is said).
    pub table: bool,
    pub pivot: bool,
    /// Element type, stripe size (`size`, for stripes), and the format as XML (the
    /// content of a `<dxf>`: `<font>`, `<fill>`, `<border>`).
    pub elements: Vec<(String, Option<u32>, String)>,
}

impl Default for Workbook {
    fn default() -> Self {
        Workbook {
            sheets: Vec::new(),
            date1904: false,
            defined_names: Vec::new(),
            styles: vec![Style::default()],
            cells: Cells::default(),
            externals: Vec::new(),
            named_styles: Vec::new(),
            extra: Vec::new(),
            properties: Vec::new(),
            theme: None,
            palette: Vec::new(),
            table_styles: Vec::new(),
            default_table_style: None,
            default_pivot_style: None,
        }
    }
}

impl Workbook {
    /// Index of an equal style, adding it if new.
    pub fn intern_style(&mut self, style: &Style) -> u32 {
        match self.styles.iter().position(|s| s == style) {
            Some(i) => i as u32,
            None => {
                self.styles.push(style.clone());
                (self.styles.len() - 1) as u32
            }
        }
    }
}

/// Built-in number formats by id (ECMA-376 part 1, 18.8.30, en-US).
pub fn builtin_num_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => r#"_(* #,##0_);_(* \(#,##0\);_(* "-"_);_(@_)"#,
        42 => r#"_($* #,##0_);_($* \(#,##0\);_($* "-"_);_(@_)"#,
        43 => r#"_(* #,##0.00_);_(* \(#,##0.00\);_(* "-"??_);_(@_)"#,
        44 => r#"_($* #,##0.00_);_($* \(#,##0.00\);_($* "-"??_);_(@_)"#,
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

pub fn builtin_num_format_id(code: &str) -> Option<u32> {
    (0..50).find(|&id| builtin_num_format(id) == Some(code))
}
