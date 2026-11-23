//! A cell-level API in the shape of pyExcelerator, on top of [`xl_cells`].
//!
//! pyExcelerator's model is: a workbook has sheets, `write(row, col, label, style)`
//! puts one value into one cell, `save` writes the file, and `parse_xls` returns per
//! sheet a map from `(row, col)` to the value. This crate keeps that model and its
//! names, with rows and columns 0-based as there. It is an upward-compatible
//! re-creation of the interface (no code is shared with pyExcelerator); the file
//! format is xlsx.
//!
//! The same calls are exposed to Python, JavaScript/TypeScript and Dart by the
//! bindings; this crate is the single implementation behind all of them.

use std::collections::BTreeMap;
use xl_cells::{Column, Row, Sheet, Style};
pub use xl_cells as cells;
pub use xl_cells::{Error, Kind, Result, Value};

/// Font attributes of a style (pyExcelerator `Font`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Font {
    pub bold: bool,
    pub italic: bool,
}

/// A cell style (pyExcelerator `XFStyle`).
#[derive(Debug, Clone, PartialEq)]
pub struct XFStyle {
    pub num_format_str: String,
    pub font: Font,
}

impl Default for XFStyle {
    fn default() -> Self {
        XFStyle { num_format_str: "General".into(), font: Font::default() }
    }
}

/// A formula (pyExcelerator `Formula`). The text has no leading `=`. Arguments may be
/// separated by `;` as in pyExcelerator or by `,`.
#[derive(Debug, Clone, PartialEq)]
pub struct Formula(pub String);

impl Formula {
    pub fn new(text: &str) -> Self {
        Formula(text.to_string())
    }

    /// The text as stored in the file: `;` between arguments becomes `,`. Inside
    /// string literals, quoted sheet names and array constants nothing changes.
    pub fn text(&self) -> String {
        let s = self.0.strip_prefix('=').unwrap_or(&self.0);
        let mut out = String::with_capacity(s.len());
        let mut quote: Option<char> = None;
        let mut braces = 0u32;
        for c in s.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"') | (None, '\'') => quote = Some(c),
                (None, '{') => braces += 1,
                (None, '}') => braces = braces.saturating_sub(1),
                (None, ';') if braces == 0 => {
                    out.push(',');
                    continue;
                }
                _ => {}
            }
            out.push(c);
        }
        out
    }
}

/// What `write` accepts.
#[derive(Debug, Clone, PartialEq)]
pub enum Label {
    /// An empty string writes a blank (styled) cell, as in pyExcelerator.
    Text(String),
    Number(f64),
    Bool(bool),
    Formula(Formula),
    /// A formula together with the value it currently has.
    FormulaWithValue(Formula, Value),
    Error(String),
    Blank,
}

impl From<&str> for Label {
    fn from(s: &str) -> Self {
        Label::Text(s.to_string())
    }
}
impl From<String> for Label {
    fn from(s: String) -> Self {
        Label::Text(s)
    }
}
impl From<f64> for Label {
    fn from(n: f64) -> Self {
        Label::Number(n)
    }
}
impl From<i32> for Label {
    fn from(n: i32) -> Self {
        Label::Number(n as f64)
    }
}
impl From<i64> for Label {
    fn from(n: i64) -> Self {
        Label::Number(n as f64)
    }
}
impl From<bool> for Label {
    fn from(b: bool) -> Self {
        Label::Bool(b)
    }
}
impl From<Formula> for Label {
    fn from(f: Formula) -> Self {
        Label::Formula(f)
    }
}

/// Serial number of a date and time in the 1900 date system (what pyExcelerator
/// stores for `datetime`). `None` for dates Excel cannot represent.
pub fn date_serial(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32, microsecond: u32) -> Option<f64> {
    let days = xl_serial::serial_from_ymd(year, month, day, xl_serial::DateSystem::V1900)?;
    Some(days as f64 + time_serial(hour, minute, second, microsecond))
}

/// Fraction of a day (what pyExcelerator stores for `time`).
pub fn time_serial(hour: u32, minute: u32, second: u32, microsecond: u32) -> f64 {
    (hour as f64 * 3600.0 + minute as f64 * 60.0 + second as f64 + microsecond as f64 / 1e6) / 86400.0
}

/// A workbook being built (pyExcelerator `Workbook`). Sheets are addressed by the
/// index `add_sheet` returns.
#[derive(Debug, Clone, Default)]
pub struct Workbook {
    wb: xl_cells::Workbook,
}

impl Workbook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a worksheet and return its index.
    pub fn add_sheet(&mut self, name: &str) -> usize {
        self.wb.sheets.push(Sheet { name: name.to_string(), ..Sheet::default() });
        self.wb.sheets.len() - 1
    }

    pub fn sheet_count(&self) -> usize {
        self.wb.sheets.len()
    }

    /// Register a style and return its index for [`Workbook::write_styled`].
    pub fn add_style(&mut self, style: &XFStyle) -> u32 {
        self.wb.intern_style(&Style { num_format: style.num_format_str.clone(), bold: style.font.bold, italic: style.font.italic, ..Style::default() })
    }

    /// `ws.write(r, c, label)` with the default style.
    pub fn write(&mut self, sheet: usize, r: u32, c: u32, label: impl Into<Label>) {
        self.write_styled(sheet, r, c, label, 0)
    }

    /// `ws.write(r, c, label, style)`; `style` comes from [`Workbook::add_style`].
    pub fn write_styled(&mut self, sheet: usize, r: u32, c: u32, label: impl Into<Label>, style: u32) {
        let (value, formula) = match label.into() {
            Label::Text(s) if s.is_empty() => (Value::Blank, None),
            Label::Text(s) => (Value::Text(s), None),
            Label::Number(n) => (Value::Number(n), None),
            Label::Bool(b) => (Value::Bool(b), None),
            Label::Error(e) => (Value::Error(e), None),
            Label::Blank => (Value::Blank, None),
            Label::Formula(f) => (Value::Blank, Some(f.text())),
            Label::FormulaWithValue(f, v) => (v, Some(f.text())),
        };
        self.wb.cells.push(sheet as u32, r, c, value, formula, style);
    }

    /// `ws.merge(r1, r2, c1, c2)`: note the pyExcelerator argument order.
    pub fn merge(&mut self, sheet: usize, r1: u32, r2: u32, c1: u32, c2: u32) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            let m = (r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2));
            // Merging the same range again is harmless and happens in existing scripts.
            if !s.merges.contains(&m) {
                s.merges.push(m);
            }
        }
    }

    /// `ws.write_merge(r1, r2, c1, c2, label, style)`.
    pub fn write_merge(&mut self, sheet: usize, r1: u32, r2: u32, c1: u32, c2: u32, label: impl Into<Label>, style: u32) {
        self.merge(sheet, r1, r2, c1, c2);
        self.write_styled(sheet, r1.min(r2), c1.min(c2), label, style);
    }

    /// `ws.col(c).width = width`: the width in 1/256 of a character, as in pyExcelerator.
    pub fn set_col_width(&mut self, sheet: usize, c: u32, width: u32) {
        self.set_col_width_chars(sheet, c, c, width as f64 / 256.0);
    }

    /// Set the width of the columns `first..=last` in characters.
    pub fn set_col_width_chars(&mut self, sheet: usize, first: u32, last: u32, width: f64) {
        self.set_columns(sheet, Column::width(first, last, width));
    }

    /// Set the attributes of the columns `c.first..=c.last`, replacing what they had.
    pub fn set_columns(&mut self, sheet: usize, c: Column) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            // Ranges reaching into the new one keep their parts outside it.
            let mut out = Vec::with_capacity(s.columns.len() + 2);
            for old in s.columns.drain(..) {
                if old.last < c.first || old.first > c.last {
                    out.push(old);
                    continue;
                }
                if old.first < c.first {
                    out.push(Column { last: c.first - 1, ..old.clone() });
                }
                if old.last > c.last {
                    out.push(Column { first: c.last + 1, ..old });
                }
            }
            out.push(c);
            out.sort_by_key(|w| w.first);
            s.columns = out;
        }
    }

    /// Make the cell written last an array formula over `range` (such as `A1:B3`),
    /// and keep the spelling of a whole number as a float (`1.0`).
    pub fn mark_last(&mut self, array_ref: Option<&str>, float_text: bool) {
        if let Some(a) = self.wb.cells.array_ref.last_mut() {
            *a = array_ref.map(str::to_string);
        }
        if let Some(f) = self.wb.cells.float_text.last_mut() {
            *f = float_text;
        }
    }

    /// Give the text cell written last formatted runs: the content of its string item as
    /// XML (`<r><rPr><b/></rPr><t>bold</t></r>…`). Its runs' text must be the cell's text.
    pub fn mark_last_rich(&mut self, xml: &str) -> Result<()> {
        let i = self.wb.cells.len().checked_sub(1).ok_or_else(|| Error::Invalid("no cell written yet".into()))?;
        if self.wb.cells.kind[i] != Kind::Text {
            return Err(Error::Invalid("formatted runs belong to a text cell".into()));
        }
        let plain = xl_cells::rich_text(xml)?;
        if Some(plain.as_str()) != self.wb.cells.text[i].as_deref() {
            return Err(Error::Invalid(format!("the runs read {plain:?}, the cell holds {:?}", self.wb.cells.text[i].as_deref().unwrap_or(""))));
        }
        self.wb.cells.rich[i] = Some(xml.into());
        Ok(())
    }

    /// Write the number cell written last as an ISO 8601 date cell (`t="d"`): 1 date,
    /// 2 date and time, 3 time.
    pub fn mark_last_iso(&mut self, form: u8) -> Result<()> {
        let i = self.wb.cells.len().checked_sub(1).ok_or_else(|| Error::Invalid("no cell written yet".into()))?;
        self.wb.cells.iso[i] = match form {
            0 => xl_cells::IsoDate::No,
            1 => xl_cells::IsoDate::Date,
            2 => xl_cells::IsoDate::DateTime,
            3 => xl_cells::IsoDate::Time,
            f => return Err(Error::Invalid(format!("unknown ISO date form {f}"))),
        };
        Ok(())
    }

    /// Register a style given in the `key=value` form of [`xl_cells::Style::to_spec`].
    pub fn add_style_spec(&mut self, spec: &str) -> u32 {
        self.wb.intern_style(&Style::from_spec(spec))
    }

    /// Replace the style list, index for index (used when a loaded workbook is saved,
    /// so that cells keep the style numbers of the original file).
    pub fn set_styles(&mut self, specs: &[String]) {
        self.wb.styles = specs.iter().map(|s| Style::from_spec(s)).collect();
        if self.wb.styles.is_empty() {
            self.wb.styles.push(Style::default());
        }
    }

    /// Mark a sheet as not being a worksheet (1 chart sheet, 2 other). Such sheets
    /// can only be saved onto the original file, which holds their content.
    pub fn set_sheet_kind(&mut self, sheet: usize, kind: u8) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            s.kind = match kind {
                0 => xl_cells::SheetKind::Worksheet,
                1 => xl_cells::SheetKind::Chartsheet,
                _ => xl_cells::SheetKind::Other,
            };
        }
    }

    /// The cell table itself, for what has no method here (defined names,
    /// hyperlinks, auto filters, data validations).
    pub fn cells_mut(&mut self) -> &mut xl_cells::Workbook {
        &mut self.wb
    }

    /// Tell which sheet of the original file this sheet is (its position there).
    pub fn set_sheet_origin(&mut self, sheet: usize, origin: u32) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            s.origin = Some(origin);
        }
    }

    /// Row height in points.
    pub fn set_row_height(&mut self, sheet: usize, row: u32, height: f64) {
        let mut r = self.wb.sheets.get(sheet).and_then(|s| s.rows.iter().find(|r| r.row == row).cloned()).unwrap_or(Row { row, ..Row::default() });
        r.height = Some(height);
        r.custom_height = true;
        self.set_row(sheet, r);
    }

    /// Set the attributes of a row, replacing what it had.
    pub fn set_row(&mut self, sheet: usize, r: Row) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            s.rows.retain(|x| x.row != r.row);
            s.rows.push(r);
            s.rows.sort_by_key(|r| r.row);
        }
    }

    /// Keep the first `rows` rows and `cols` columns in place when scrolling.
    pub fn set_freeze(&mut self, sheet: usize, rows: u32, cols: u32) {
        if let Some(s) = self.wb.sheets.get_mut(sheet) {
            s.freeze = Some((rows, cols)).filter(|f| *f != (0, 0));
        }
    }

    /// The file, written onto `original` (the file this workbook was loaded from):
    /// everything not modelled here is kept. The flag is false when a fresh file had
    /// to be written instead (sheets added, removed or renamed).
    pub fn to_bytes_onto(&self, original: Vec<u8>) -> Result<(Vec<u8>, bool)> {
        xl_cells::update_bytes_report(original, &self.wb)
    }

    /// Use the 1904 date system.
    pub fn set_date1904(&mut self, on: bool) {
        self.wb.date1904 = on;
    }

    /// The xlsx file as bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        xl_cells::write_bytes(&self.wb)
    }

    /// `w.save(filename)`.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        std::fs::write(path, self.to_bytes()?).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// The underlying cell table.
    pub fn cells(&self) -> &xl_cells::Workbook {
        &self.wb
    }
}

/// The values of one sheet by `(row, col)`.
pub type SheetValues = BTreeMap<(u32, u32), Value>;

/// `parse_xls(filename)`: per sheet its name and the values of its non-empty cells.
/// Dates arrive as numbers, errors as their text, as in pyExcelerator.
pub fn parse_xls(bytes: Vec<u8>) -> Result<Vec<(String, SheetValues)>> {
    Ok(values(&read_any(bytes)?))
}

/// Whether the bytes are an Excel 97-2003 file (a compound file) rather than an .xlsx.
pub fn is_xls(bytes: &[u8]) -> bool {
    xl_xls::is_cfb(bytes)
}

/// A workbook from .xlsx or .xls bytes, told apart by their content.
pub fn read_any(bytes: Vec<u8>) -> Result<xl_cells::Workbook> {
    if xl_xls::is_cfb(&bytes) {
        xl_xls::read_bytes(&bytes).map_err(|e| Error::Invalid(e.to_string()))
    } else {
        xl_cells::read_bytes(bytes)
    }
}

/// `parse_xls` for an already loaded cell table.
pub fn values(wb: &xl_cells::Workbook) -> Vec<(String, SheetValues)> {
    let mut out: Vec<(String, SheetValues)> = wb.sheets.iter().map(|s| (s.name.clone(), SheetValues::new())).collect();
    let c = &wb.cells;
    for i in 0..c.len() {
        if c.kind[i] != Kind::Blank {
            out[c.sheet[i] as usize].1.insert((c.row[i], c.col[i]), c.value(i));
        }
    }
    out
}

/// The cell table in the column layout the language bindings hand out: one entry per
/// cell in every vector, strings as offsets into one UTF-8 buffer (the Arrow layout).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub sheet_names: Vec<String>,
    pub date1904: bool,
    pub sheet: Vec<u32>,
    pub row: Vec<u32>,
    pub col: Vec<u32>,
    /// [`Kind`] as its number: 0 blank, 1 number, 2 text, 3 bool, 4 error.
    pub kind: Vec<u8>,
    pub num: Vec<f64>,
    /// 1 where the cell's number format is a date or time format.
    pub is_date: Vec<u8>,
    /// 1 where a whole number is spelled with a decimal point or exponent in the file.
    pub float_text: Vec<u8>,
    /// `len + 1` offsets into `text_data`; cells without text have an empty slice.
    pub text_offsets: Vec<u32>,
    pub text_data: Vec<u8>,
    pub formula_offsets: Vec<u32>,
    pub formula_data: Vec<u8>,
    pub num_format: Vec<String>,
    /// Bold and italic per style (same index as `num_format`).
    pub bold: Vec<u8>,
    pub italic: Vec<u8>,
    /// Index into `num_format` per cell.
    pub style: Vec<u32>,
    /// Defined names as (name, sheet it is local to or -1, value, hidden).
    pub defined_names: Vec<(String, i64, String, bool)>,
    /// Hyperlinks as (sheet, cell, target, location, tooltip, display).
    pub hyperlinks: Vec<(u32, String, Option<String>, Option<String>, Option<String>, Option<String>)>,
    /// Auto filters as (sheet, range).
    pub auto_filter: Vec<(u32, String)>,
    /// Data validations as (sheet, attributes, formula1, formula2).
    pub validations: Vec<(u32, Vec<(String, String)>, Option<String>, Option<String>)>,
    /// Pictures and charts as (sheet, drawing).
    pub drawings: Vec<(u32, xl_cells::Drawing)>,
    /// Comments as (sheet, cell, author, text).
    pub comments: Vec<(u32, String, String, String)>,
    /// (sheet, table part XML).
    pub tables: Vec<(u32, String)>,
    /// (sheet, first row, first column, last row, last column) of the used range as stated by the file.
    pub dimensions: Vec<(u32, u32, u32, u32, u32)>,
    /// Children of `<workbook>` kept as XML: (element name, XML).
    pub workbook_extra: Vec<(String, String)>,
    /// Document property parts: (kind, XML).
    pub properties: Vec<(String, String)>,
    /// Conditional formats as (sheet, ranges, rules); a rule is (attributes, content
    /// XML, format XML).
    #[allow(clippy::type_complexity)]
    pub cond_formats: Vec<(u32, String, Vec<(Vec<(String, String)>, String, Option<String>)>)>,
    /// Range of an array formula per cell, empty for other cells.
    pub array_ref: Vec<String>,
    /// (cell index, string item XML) for text cells with formatted runs.
    pub rich: Vec<(u32, String)>,
    /// (cell index, form) for ISO 8601 date cells: 1 date, 2 date and time, 3 time.
    pub iso: Vec<(u32, u8)>,
    /// Per sheet: 0 worksheet, 1 chart sheet, 2 other.
    pub sheet_kind: Vec<u8>,
    /// Every style in the `key=value` form of [`xl_cells::Style::to_spec`].
    pub style_spec: Vec<String>,
    /// (name, format as a style spec, builtin id or -1, hidden, custom builtin, outline level or -1) of the named styles.
    pub named_styles: Vec<(String, String, i64, bool, bool, i64)>,
    /// Row heights as (sheet, row, points).
    /// (sheet, row attributes).
    pub rows: Vec<(u32, xl_cells::Row)>,
    /// Frozen panes as (sheet, rows, columns).
    pub freeze: Vec<(u32, u32, u32)>,
    /// (sheet, element name, XML) of the worksheet children kept as XML.
    pub extra: Vec<(u32, String, String)>,
    /// Merged ranges as (sheet, first row, first col, last row, last col).
    pub merges: Vec<(u32, u32, u32, u32, u32)>,
    /// Column widths as (sheet, first col, last col, width in characters).
    /// (sheet, column attributes).
    pub columns: Vec<(u32, xl_cells::Column)>,
}

impl Table {
    pub fn len(&self) -> usize {
        self.row.len()
    }

    pub fn is_empty(&self) -> bool {
        self.row.is_empty()
    }

    pub fn text(&self, i: usize) -> &str {
        std::str::from_utf8(&self.text_data[self.text_offsets[i] as usize..self.text_offsets[i + 1] as usize]).unwrap_or("")
    }

    pub fn formula(&self, i: usize) -> &str {
        std::str::from_utf8(&self.formula_data[self.formula_offsets[i] as usize..self.formula_offsets[i + 1] as usize]).unwrap_or("")
    }
}

/// Largest text buffer a [`Table`] can address.
const MAX_TEXT: usize = u32::MAX as usize;

/// Read a workbook into a [`Table`].
pub fn read_table(bytes: Vec<u8>) -> Result<Table> {
    let wb = read_any(bytes)?;
    let c = &wb.cells;
    let n = c.len();
    let mut t = Table {
        sheet_names: wb.sheets.iter().map(|s| s.name.clone()).collect(),
        date1904: wb.date1904,
        sheet: c.sheet.clone(),
        row: c.row.clone(),
        col: c.col.clone(),
        kind: c.kind.iter().map(|k| *k as u8).collect(),
        num: c.num.clone(),
        is_date: Vec::with_capacity(n),
        float_text: c.float_text.iter().map(|&b| b as u8).collect(),
        text_offsets: Vec::with_capacity(n + 1),
        text_data: Vec::new(),
        formula_offsets: Vec::with_capacity(n + 1),
        formula_data: Vec::new(),
        num_format: wb.styles.iter().map(|s| s.num_format.clone()).collect(),
        bold: wb.styles.iter().map(|s| s.bold as u8).collect(),
        italic: wb.styles.iter().map(|s| s.italic as u8).collect(),
        style: c.style.clone(),
        defined_names: wb.defined_names.iter().map(|n| (n.name.clone(), n.local_sheet.map(|l| l as i64).unwrap_or(-1), n.value.clone(), n.hidden)).collect(),
        hyperlinks: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.hyperlinks.iter().map(move |h| (i as u32, h.cell.clone(), h.target.clone(), h.location.clone(), h.tooltip.clone(), h.display.clone()))).collect(),
        auto_filter: wb.sheets.iter().enumerate().filter_map(|(i, s)| s.auto_filter.clone().map(|r| (i as u32, r))).collect(),
        validations: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.validations.iter().map(move |v| (i as u32, v.attrs.clone(), v.formula1.clone(), v.formula2.clone()))).collect(),
        drawings: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.drawings.iter().map(move |d| (i as u32, d.clone()))).collect(),
        comments: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.comments.iter().map(move |c| (i as u32, c.cell.clone(), c.author.clone(), c.text.clone()))).collect(),
        tables: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.tables.iter().map(move |t| (i as u32, t.clone()))).collect(),
        dimensions: wb.sheets.iter().enumerate().filter_map(|(i, s)| s.dimension.map(|d| (i as u32, d.0, d.1, d.2, d.3))).collect(),
        workbook_extra: wb.extra.clone(),
        properties: wb.properties.clone(),
        cond_formats: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.cond_formats.iter().map(move |c| (i as u32, c.sqref.clone(), c.rules.iter().map(|r| (r.attrs.clone(), r.inner.clone(), r.dxf.clone())).collect()))).collect(),
        array_ref: c.array_ref.iter().map(|a| a.clone().unwrap_or_default()).collect(),
        rich: c.rich.iter().enumerate().filter_map(|(i, r)| r.as_deref().map(|r| (i as u32, r.to_string()))).collect(),
        iso: c.iso.iter().enumerate().filter(|(_, f)| **f != xl_cells::IsoDate::No).map(|(i, f)| (i as u32, *f as u8)).collect(),
        sheet_kind: wb.sheets.iter().map(|s| match s.kind {
            xl_cells::SheetKind::Worksheet => 0,
            xl_cells::SheetKind::Chartsheet => 1,
            xl_cells::SheetKind::Other => 2,
        }).collect(),
        style_spec: wb.styles.iter().map(Style::to_spec).collect(),
        named_styles: wb
            .named_styles
            .iter()
            .map(|n| (n.name.clone(), n.style.to_spec(), n.builtin_id.map(i64::from).unwrap_or(-1), n.hidden, n.custom_builtin, n.i_level.map(i64::from).unwrap_or(-1)))
            .collect(),
        rows: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.rows.iter().map(move |r| (i as u32, r.clone()))).collect(),
        freeze: wb.sheets.iter().enumerate().filter_map(|(i, s)| s.freeze.map(|f| (i as u32, f.0, f.1))).collect(),
        extra: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.extra.iter().map(move |(n, x)| (i as u32, n.clone(), x.clone()))).collect(),
        merges: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.merges.iter().map(move |m| (i as u32, m.0, m.1, m.2, m.3))).collect(),
        columns: wb.sheets.iter().enumerate().flat_map(|(i, s)| s.columns.iter().map(move |c| (i as u32, c.clone()))).collect(),
    };
    let dates: Vec<bool> = wb.styles.iter().map(Style::is_date).collect();
    // Sum first: one shared string referenced by many cells can stand for far more text than
    // the file holds (POI's poc-shared-strings.xlsx: 12,000 cells x 1 MB). Refuse before allocating.
    let text_len: usize = c.text.iter().map(|s| s.as_deref().map_or(0, str::len)).sum();
    let formula_len: usize = c.formula.iter().map(|s| s.as_deref().map_or(0, str::len)).sum();
    if text_len > MAX_TEXT || formula_len > MAX_TEXT {
        return Err(Error::Invalid("the workbook's text does not fit into 4 GiB".into()));
    }
    t.text_data.reserve_exact(text_len);
    t.formula_data.reserve_exact(formula_len);
    t.text_offsets.push(0);
    t.formula_offsets.push(0);
    for i in 0..n {
        t.is_date.push((c.kind[i] == Kind::Number && dates[c.style[i] as usize]) as u8);
        if let Some(s) = c.text[i].as_deref() {
            t.text_data.extend_from_slice(s.as_bytes());
        }
        if let Some(f) = c.formula[i].as_deref() {
            t.formula_data.extend_from_slice(f.as_bytes());
        }
        t.text_offsets.push(t.text_data.len() as u32);
        t.formula_offsets.push(t.formula_data.len() as u32);
    }
    Ok(t)
}
