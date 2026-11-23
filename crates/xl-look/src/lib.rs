//! xl-look: the formatting a cell of an Excel workbook shows, attribute by attribute,
//! and where each attribute comes from.
//!
//! Excel builds what a cell looks like from layers, lowest first:
//!
//! 1. the workbook default (the `Normal` cell style);
//! 2. for a cell without an entry of its own, the format of its row, else of its column;
//! 3. a table style, where the cell lies in a table (only for attributes the cell's own
//!    format leaves at the default);
//! 4. the cell's own format, which Excel stores complete, together with the named cell
//!    style it is based on: an attribute that equals the style's comes from the style;
//! 5. conditional formats, by priority, as far as their rules hold.
//!
//! [`Look`] gives every attribute once, resolved (colours as RGB, theme colours and tints
//! worked out), with its [`Source`], and says when a value is not certain (a conditional
//! format whose rule could not be decided).

pub mod color;
pub mod cond;
pub mod parts;
pub mod pivot;
pub mod table;
mod tree;

use color::{Colors, Spec};
use cond::{Hit, Mark, Rule};
use parts::{Partial, Raw, KEYS};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use table::{Element, Table};
use xl_cells::{Value, Workbook};

/// A resolved attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Text(String),
    Number(f64),
    Flag(bool),
    /// `RRGGBB`.
    Rgb(String),
    /// No colour (no fill, no border).
    None,
}

/// Where an attribute's value comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// The workbook default: the `Normal` cell style.
    Default,
    /// A named cell style other than `Normal` (`Good`, `Heading 1`, one of the file).
    Style(String),
    /// The cell's own format.
    Cell,
    /// The format of the row (0-based), for a cell without an entry.
    Row(u32),
    /// The format of the column (0-based), for a cell without an entry.
    Column(u32),
    /// An element (`headerRow`, `firstRowStripe`, …) of the style of a table.
    Table { table: String, style: String, element: String },
    /// A conditional format: its range and the priority of the rule.
    Conditional { range: String, priority: i64, kind: String },
}

impl Source {
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::Style(_) => "style",
            Source::Cell => "cell",
            Source::Row(_) => "row",
            Source::Column(_) => "column",
            Source::Table { .. } => "table",
            Source::Conditional { .. } => "conditional",
        }
    }

    /// A short description: `style Good`, `table Table1 (TableStyleMedium9, headerRow)`.
    pub fn describe(&self) -> String {
        match self {
            Source::Default => "default".into(),
            Source::Style(n) => format!("style {n}"),
            Source::Cell => "cell".into(),
            Source::Row(r) => format!("row {}", r + 1),
            Source::Column(c) => format!("column {}", xl_formula::col_name(c + 1)),
            Source::Table { table, style, element } => format!("table {table} ({style}, {element})"),
            Source::Conditional { range, priority, kind } => format!("conditional {range} ({kind}, priority {priority})"),
        }
    }
}

/// One attribute of a cell's look.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub value: Val,
    pub source: Source,
    /// A conditional format whose rule could not be decided might set it.
    pub certain: bool,
    /// For colours: how the file states it (`theme 4 tint 0.6`, `rgb FF0000`, `indexed
    /// 10`, `auto`).
    pub spec: Option<String>,
}

/// A data bar a conditional format draws in the cell.
#[derive(Debug, Clone, PartialEq)]
pub struct DataBar {
    pub color: Option<String>,
    /// Where the value lies between the bar's minimum and maximum, 0 to 1.
    pub fraction: f64,
    pub source: Source,
}

/// An icon a conditional format shows in the cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    /// `3TrafficLights1`, `5Arrows`, …
    pub set: String,
    /// 0 is the icon of the lowest values.
    pub index: usize,
    pub source: Source,
}

/// What a cell looks like.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Look {
    /// Every attribute of [`parts::KEYS`], in that order.
    pub attrs: Vec<(&'static str, Attr)>,
    pub data_bar: Option<DataBar>,
    pub icon: Option<Icon>,
    /// Conditional formats covering the cell whose rule could not be decided, and why.
    pub undecided: Vec<(String, String)>,
}

impl Look {
    pub fn get(&self, key: &str) -> Option<&Attr> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, a)| a)
    }

    pub fn value(&self, key: &str) -> Option<&Val> {
        self.get(key).map(|a| &a.value)
    }
}

fn spec_text(s: &Spec) -> String {
    let t = |t: f64| if t == 0.0 { String::new() } else { format!(" tint {t}") };
    match s {
        Spec::Rgb(rgb, tint) => format!("rgb {rgb}{}", t(*tint)),
        Spec::Theme(i, tint) => format!("theme {i}{}", t(*tint)),
        Spec::Indexed(i, tint) => format!("indexed {i}{}", t(*tint)),
        Spec::Auto => "auto".into(),
    }
}

/// Works out the look of cells of one workbook. Formats, table styles and conditional
/// formats are read once; formula rules are decided for the cells asked for, in one
/// calculation per call of [`Looker::looks`], and remembered.
pub struct Looker<'a> {
    wb: Cow<'a, Workbook>,
    pub colors: Colors,
    /// Apply table styles (default true).
    pub with_tables: bool,
    /// Apply conditional formats (default true).
    pub with_conditional: bool,
    /// The pivot tables of each sheet ([`pivot::pivot_areas`] of the file): their styles
    /// are not applied, and what they could set is marked as not certain.
    pub pivots: Vec<Vec<pivot::PivotArea>>,
    /// The moment for `TODAY()` and `NOW()` in rules, as a serial date of the workbook's
    /// date system; by default the current local time, as in Excel. What was decided
    /// for another moment is decided again.
    pub now: f64,
    normal: Partial,
    styles: Vec<Partial>,
    named: HashMap<String, Partial>,
    tables: Vec<Vec<(Table, Option<(String, Vec<Element>)>)>>,
    rules: Vec<Vec<Rule>>,
    index: HashMap<(u32, u32, u32), usize>,
    /// The style of the rows with a format of their own, by (sheet, row).
    row_styles: HashMap<(u32, u32), u32>,
    decided: HashMap<(u32, usize, u32, u32), Hit>,
    /// The moment (`now`, its bits) `decided` holds for.
    decided_now: u64,
    ranges: HashMap<(u32, usize), (Vec<f64>, Vec<Value>)>,
    engine: Engine,
}

/// Makes what runs the formula engine's expressions, for the formulas of rules.
pub type Engine = fn() -> Box<dyn xl_engine::ir::Executor>;

#[cfg(feature = "polars")]
fn builtin() -> Box<dyn xl_engine::ir::Executor> {
    Box::new(xl_polars::builtin::PolarsExecutor::default())
}

impl<'a> Looker<'a> {
    /// A looker whose rule formulas are calculated by the built-in Polars.
    #[cfg(feature = "polars")]
    pub fn new(wb: &'a Workbook) -> Self {
        Self::build(Cow::Borrowed(wb), builtin)
    }

    /// As [`Looker::new`], keeping the workbook itself: to hold on to beside nothing else.
    #[cfg(feature = "polars")]
    pub fn owning(wb: Workbook) -> Looker<'static> {
        Looker::build(Cow::Owned(wb), builtin)
    }

    /// A looker whose rule formulas are calculated with the executors `engine` makes.
    pub fn with_engine(wb: &'a Workbook, engine: Engine) -> Self {
        Self::build(Cow::Borrowed(wb), engine)
    }

    /// As [`Looker::with_engine`], keeping the workbook itself.
    pub fn owning_with_engine(wb: Workbook, engine: Engine) -> Looker<'static> {
        Looker::build(Cow::Owned(wb), engine)
    }

    fn build(book: Cow<'a, Workbook>, engine: Engine) -> Self {
        let wb: &Workbook = &book;
        let colors = Colors::new(wb.theme.as_deref(), &wb.palette);
        let normal_style = wb.named_styles.iter().find(|n| n.builtin_id == Some(0)).or_else(|| wb.named_styles.iter().find(|n| n.name == "Normal")).map(|n| n.style.clone()).unwrap_or_default();
        let normal = parts::of_style(&normal_style);
        let styles = wb.styles.iter().map(parts::of_style).collect();
        let named = wb.named_styles.iter().map(|n| (n.name.clone(), parts::of_style(&n.style))).collect();
        let own: HashMap<String, Vec<Element>> = wb.table_styles.iter().filter(|t| t.table).map(|t| (t.name.clone(), table::own(t))).collect();
        let tables = wb
            .sheets
            .iter()
            .map(|s| {
                s.tables
                    .iter()
                    .filter_map(|x| Table::of(x))
                    .map(|t| {
                        let style = t.style.clone().and_then(|n| own.get(&n).cloned().or_else(|| table::presets().get(&n).cloned()).map(|e| (n, e)));
                        (t, style)
                    })
                    .collect()
            })
            .collect();
        let c = &wb.cells;
        let index = (0..c.len()).map(|i| ((c.sheet[i], c.row[i], c.col[i]), i)).collect();
        // A row with its own format (`customFormat`) has style 0 when it names none.
        let mut row_styles = HashMap::new();
        for (s, sheet) in wb.sheets.iter().enumerate() {
            for row in sheet.rows.iter().filter(|x| x.custom_format) {
                row_styles.entry((s as u32, row.row)).or_insert(row.style.unwrap_or(0));
            }
        }
        let now = now_serial(wb.date1904);
        let rules = cond::rules(wb);
        Looker { wb: book, colors, with_tables: true, with_conditional: true, pivots: Vec::new(), now, normal, styles, named, tables, rules, index, row_styles, decided: HashMap::new(), decided_now: now.to_bits(), ranges: HashMap::new(), engine }
    }

    /// The look of one cell (sheet index, 0-based row and column).
    pub fn look(&mut self, sheet: u32, row: u32, col: u32) -> Look {
        self.looks(&[(sheet, row, col)]).pop().unwrap_or_default()
    }

    /// The looks of many cells; the formula rules for all of them are decided together.
    pub fn looks(&mut self, cells: &[(u32, u32, u32)]) -> Vec<Look> {
        if self.decided_now != self.now.to_bits() {
            self.decided.clear();
            self.decided_now = self.now.to_bits();
        }
        let mut wanted: Vec<(u32, usize, u32, u32)> = Vec::new();
        let mut seen = HashSet::new();
        for &(s, r, c) in cells {
            let Some(rules) = self.rules.get(s as usize) else { continue };
            for (k, rule) in rules.iter().enumerate() {
                let key = (s, k, r, c);
                if rule.by_formula() && rule.covers(r, c) && !self.decided.contains_key(&key) && seen.insert(key) {
                    // Rules Excel decides by their type, not by their formula.
                    let value = self.index.get(&(s, r, c)).map(|&i| self.wb.cells.value(i)).unwrap_or(Value::Blank);
                    match cond::by_type(rule, &value, self.now, self.wb.date1904) {
                        Some(hit) => {
                            self.decided.insert(key, hit);
                        }
                        None => wanted.push(key),
                    }
                }
            }
        }
        if !wanted.is_empty() && self.with_conditional {
            let hits = cond::decide(&self.wb, &self.rules, &wanted, self.now, &*(self.engine)());
            self.decided.extend(wanted.into_iter().zip(hits));
        }
        cells.iter().map(|&(s, r, c)| self.resolve(s, r, c)).collect()
    }

    fn resolve(&mut self, s: u32, r: u32, c: u32) -> Look {
        // The parts apart, so that what is remembered (`ranges`) can grow beside the rest.
        let Looker { wb, colors, with_tables, with_conditional, pivots, normal, styles, named, tables, rules, index, row_styles, decided, ranges, .. } = self;
        let (wb, colors): (&Workbook, &Colors) = (wb, colors);
        // The format layer: the cell's own, or its row's or column's.
        let (style_index, own_source) = match index.get(&(s, r, c)) {
            Some(&i) => (Some(wb.cells.style[i] as usize), Source::Cell),
            None => {
                let sheet = wb.sheets.get(s as usize);
                let row = row_styles.get(&(s, r)).copied();
                let col = sheet.and_then(|sh| sh.columns.iter().find(|x| x.first <= c && c <= x.last)).and_then(|x| x.style);
                match (row, col) {
                    (Some(st), _) => (Some(st as usize), Source::Row(r)),
                    (_, Some(st)) => (Some(st as usize), Source::Column(c)),
                    _ => (None, Source::Default),
                }
            }
        };
        let mut layer: Vec<(Raw, Source)> = Vec::with_capacity(KEYS.len());
        let own = style_index.and_then(|i| styles.get(i));
        let named_name = style_index.and_then(|i| wb.styles.get(i)).and_then(|st| st.named.clone()).filter(|n| n != "Normal");
        let named = named_name.as_ref().and_then(|n| named.get(n));
        for key in KEYS {
            let normal = normal.get(key).cloned().unwrap_or(Raw::Nothing);
            let entry = match own.and_then(|p| p.get(key)) {
                None => (normal, Source::Default),
                Some(v) if *v == normal && named.and_then(|n| n.get(key)).is_none_or(|nv| nv == v) => (v.clone(), Source::Default),
                Some(v) if named.and_then(|n| n.get(key)) == Some(v) => (v.clone(), Source::Style(named_name.clone().unwrap_or_default())),
                Some(v) => (v.clone(), own_source.clone()),
            };
            layer.push(entry);
        }
        // Table styles, under the cell's own format.
        if let Some(tables) = tables.get(s as usize).filter(|_| *with_tables) {
            for (t, style) in tables.iter().filter(|(t, _)| t.contains(r, c)) {
                let Some((style_name, elements)) = style else { continue };
                let size_of = |kind: &str| elements.iter().find(|e| e.kind == kind).map(|e| e.size).unwrap_or(1);
                for kind in table::ORDER {
                    let Some(e) = elements.iter().find(|e| e.kind == kind) else { continue };
                    let Some(region) = t.region(kind, &size_of, r, c) else { continue };
                    let p = table::element_for_cell(&e.look, region, r, c);
                    for (k, key) in KEYS.iter().enumerate() {
                        if let Some(v) = p.get(key) {
                            if matches!(layer[k].1, Source::Default | Source::Table { .. }) {
                                layer[k] = (v.clone(), Source::Table { table: t.name.clone(), style: style_name.clone(), element: kind.to_string() });
                            }
                        }
                    }
                }
            }
        }
        // Conditional formats, highest priority first.
        let mut certain = vec![true; KEYS.len()];
        let mut set_by_cf = vec![false; KEYS.len()];
        let mut look = Look::default();
        let mut all_uncertain = false;
        let value = index.get(&(s, r, c)).map(|&i| wb.cells.value(i)).unwrap_or(Value::Blank);
        let sheet_rules: &[Rule] = rules.get(s as usize).filter(|_| *with_conditional).map(Vec::as_slice).unwrap_or(&[]);
        for (k, rule) in sheet_rules.iter().enumerate() {
            if !rule.covers(r, c) {
                continue;
            }
            let source = Source::Conditional { range: rule.sqref.clone(), priority: rule.priority, kind: rule.kind.clone() };
            let mark = if rule.by_formula() {
                Mark::Hit(decided.get(&(s, k, r, c)).cloned().unwrap_or(Hit::Unknown("not decided".into())))
            } else {
                let (sorted, all) = ranges.entry((s, k)).or_insert_with(|| {
                    let mut nums = cond::numbers(wb, rule);
                    nums.sort_by(|a, b| a.total_cmp(b));
                    let c = &wb.cells;
                    let all = (0..c.len()).filter(|&i| c.sheet[i] == rule.sheet && rule.covers(c.row[i], c.col[i])).map(|i| c.value(i)).collect();
                    (nums, all)
                });
                let cell = |text: &str| -> Option<f64> {
                    let (sheet, at) = match text.rsplit_once('!') {
                        Some((p, a)) => {
                            let name = p.trim_matches('\'').replace("''", "'");
                            (wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(&name))? as u32, a)
                        }
                        None => (s, text),
                    };
                    let r = xl_formula::parse_cell_ref(&at.replace('$', ""))?;
                    let i = *index.get(&(sheet, r.row - 1, r.col - 1))?;
                    match wb.cells.value(i) {
                        Value::Number(n) => Some(n),
                        _ => None,
                    }
                };
                cond::range_rule(rule, &value, sorted, all, colors, &cell)
            };
            let mut apply = |p: &Partial, layer: &mut Vec<(Raw, Source)>, sure: bool| {
                for (i, key) in KEYS.iter().enumerate() {
                    if let Some(v) = p.get(key) {
                        if !set_by_cf[i] {
                            if sure {
                                layer[i] = (v.clone(), source.clone());
                                set_by_cf[i] = true;
                                // A rule above that was not decided might have set it.
                                certain[i] = certain[i] && !all_uncertain;
                            } else {
                                certain[i] = false;
                            }
                        }
                    }
                }
            };
            match mark {
                Mark::Hit(Hit::Yes) => {
                    apply(&rule.look, &mut layer, true);
                    if rule.stop {
                        break;
                    }
                }
                Mark::Hit(Hit::No) => {}
                Mark::Hit(Hit::Unknown(why)) => {
                    look.undecided.push((rule.sqref.clone(), why));
                    apply(&rule.look, &mut layer, false);
                    if rule.stop {
                        all_uncertain = true;
                    }
                }
                Mark::Fill(spec) => {
                    let mut p = Partial::new();
                    p.insert("fill_pattern", Raw::Text("solid".into()));
                    p.insert("fill_color", Raw::Color(spec));
                    apply(&p, &mut layer, true);
                }
                Mark::Bar { color, fraction } => {
                    if look.data_bar.is_none() {
                        look.data_bar = Some(DataBar { color: color.and_then(|c| colors.rgb(&c, Some("000000"))), fraction, source });
                    }
                }
                Mark::Icon { set, index } => {
                    if look.icon.is_none() {
                        look.icon = Some(Icon { set, index, source });
                    }
                }
            }
        }
        // A pivot table's style is not worked out: what it could set is not certain.
        if let Some(p) = pivots.get(s as usize).and_then(|ps| ps.iter().find(|p| (p.area.0..=p.area.2).contains(&r) && (p.area.1..=p.area.3).contains(&c))) {
            look.undecided.push((p.name.clone(), format!("pivot table (style {} not applied)", p.style.as_deref().unwrap_or("none"))));
            for (i, key) in KEYS.iter().enumerate() {
                if !set_by_cf[i] && (key.starts_with("font_") || key.starts_with("fill_") || key.starts_with("border_") || matches!(*key, "bold" | "italic" | "underline" | "strike")) {
                    certain[i] = false;
                }
            }
        }
        for (i, key) in KEYS.iter().enumerate() {
            let (raw, source) = layer[i].clone();
            let (value, spec) = match raw {
                Raw::Text(t) => (Val::Text(t), None),
                Raw::Number(n) => (Val::Number(n), None),
                Raw::Flag(b) => (Val::Flag(b), None),
                Raw::Nothing => (Val::None, None),
                Raw::Color(sp) => {
                    let auto = if *key == "fill_background" { "FFFFFF" } else { "000000" };
                    (colors.rgb(&sp, Some(auto)).map(Val::Rgb).unwrap_or(Val::None), Some(spec_text(&sp)))
                }
            };
            look.attrs.push((key, Attr { value, source, certain: certain[i], spec }));
        }
        // No fill: no fill colours, whatever a pattern's colours were.
        if look.value("fill_pattern") == Some(&Val::Text("none".into())) {
            for (k, a) in look.attrs.iter_mut() {
                if matches!(*k, "fill_color" | "fill_background") {
                    a.value = Val::None;
                }
            }
        }
        look
    }
}

/// The current local time (the clock a user of Excel sees) as a serial date (1900 or
/// 1904 system).
pub fn now_serial(date1904: bool) -> f64 {
    let secs = chrono::Local::now().naive_local().and_utc().timestamp_micros() as f64 / 1e6;
    // 1970-01-01 is serial 25569 in the 1900 system, 24107 in the 1904 system.
    let epoch = if date1904 { 24107.0 } else { 25569.0 };
    epoch + secs / 86400.0
}

/// The look of one cell.
#[cfg(feature = "polars")]
pub fn look(wb: &Workbook, sheet: u32, row: u32, col: u32) -> Look {
    Looker::new(wb).look(sheet, row, col)
}
