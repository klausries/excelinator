//! The engine's translation of a calculated column or cell, mapped on the data of the
//! regions.
//!
//! A translation ([`crate::lift::translate_program`]) is a plan over inputs named after
//! places of the workbook. [`Mapper`] says what each of those inputs is in the frames of
//! the regions ([`Mapped`]): a column of a table, a named cell, the numbers of a range.
//! That mapping is used three ways:
//!
//! * the Polars program writes it out as Python ([`crate::codegen`]);
//! * it is kept with the data as a note ([`Mapped::json`]), so that the logic travels
//!   with the Parquet files of a store;
//! * [`recalculate`] runs the plans of such notes on the frames again, without the
//!   workbook: the calculation of the workbook, in the database.

use crate::data::{Column, Data, Frame};
use crate::ir::{self, Executor, Plan, Table};
use crate::lift::{cell_input_name, column_name, range_input_name, text_column_name, translate_program_in, Run, ROW};
use crate::regions::{Region, RegionKind};
use serde_json::{json, Value as J};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use xl_cells::{Kind, Workbook};

/// A column or cell of a region: (index of the region, name).
pub type Key = (usize, String);

/// A single value as a calculation takes it: dates and times are their serial numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Null,
    Bool(bool),
    Num(f64),
    Text(String),
}

impl V {
    /// As a number: booleans as 1 and 0; text and nothing are no number.
    pub fn num(&self) -> Option<f64> {
        match self {
            V::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            V::Num(x) => Some(*x),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            V::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Whether it is still what a text input was made for: text, or nothing.
    fn is_text(&self) -> bool {
        match self {
            V::Text(_) | V::Null => true,
            V::Bool(b) => !*b,
            V::Num(x) => *x == 0.0,
        }
    }

    fn json(&self) -> J {
        match self {
            V::Null => J::Null,
            V::Bool(b) => json!(b),
            V::Num(x) => json!(x),
            V::Text(t) => json!(t),
        }
    }

    fn of_json(v: &J) -> V {
        match v {
            J::Bool(b) => V::Bool(*b),
            J::Number(n) => V::Num(n.as_f64().unwrap_or(f64::NAN)),
            J::String(s) => V::Text(s.clone()),
            _ => V::Null,
        }
    }
}

/// Where a single value comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// A cell of a region of single cells.
    Named(Key),
    /// A cell of a column of a table; the row counts from the first row of its data.
    /// `parse`: the cell is a number in a column of mixed kinds, which is text in the
    /// data: the number is read from its text.
    TableCell { region: usize, column: String, row: u32, parse: bool },
    /// A cell of the sheet that is in no region (0-based): not data of the regions.
    Sheet { sheet: u32, row: u32, col: u32 },
    /// A value that is not data (what a note keeps of a cell in no region).
    Const(V),
}

/// A column of a table as the input of a plan.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnSource {
    /// Its numbers (booleans as 1 and 0).
    Numbers(Key),
    /// No numbers: the column held none when the translation was made, nothing at all
    /// or (`text`) text only.
    NoNumbers { column: Key, text: bool },
    Text(Key),
}

/// The cells of a fixed range.
#[derive(Debug, Clone, PartialEq)]
pub enum RangeSource {
    /// Whole columns of a table.
    Columns(usize, Vec<String>),
    /// Cells of the sheet in no region (1-based, `sheet, r1, r2, c1, c2`).
    Sheet(u32, u32, u32, u32, u32),
    Cells(Vec<Scalar>),
    /// The summary itself (what a note keeps of cells in no region).
    Summary([Option<f64>; 5]),
}

/// One input of a plan, or several (the parts of a range's summary).
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Column { name: String, source: ColumnSource },
    /// A cell read by a single cell's formula where a column's formula reads its row.
    Own { name: String, cell: Scalar, text: bool },
    /// A fixed cell: it keeps its kind (number or text), or the plan does not hold.
    Cell { name: String, cell: Scalar, text: bool },
    /// The sum, count, smallest, largest and sum of squares of a range's numbers:
    /// `parts` names the inputs for those the plan reads (index, name).
    Range { index: usize, source: RangeSource, parts: Vec<(usize, String)> },
}

/// A translation and where its inputs are.
#[derive(Debug, Clone)]
pub struct Mapped {
    pub plan: Plan,
    pub inputs: Vec<Input>,
    /// The calculated columns and cells it reads.
    pub deps: Vec<Key>,
    /// The rows of the sheet it is for (0-based, first and last).
    pub rows: (u32, u32),
    pub is_bool: bool,
}

/// The cells of a workbook by place, and which of them the regions name or calculate.
pub struct Context {
    pub at: HashMap<(u32, u32, u32), usize>,
    /// The last row (1-based) of each sheet that holds a cell.
    pub last_rows: Vec<u32>,
    pub cell_at: HashMap<(u32, u32, u32), Key>,
    pub calculated: HashSet<Key>,
}

impl Context {
    pub fn new(wb: &Workbook, regions: &[Region]) -> Context {
        let at = (0..wb.cells.len()).map(|i| ((wb.cells.sheet[i], wb.cells.row[i], wb.cells.col[i]), i)).collect();
        let mut cell_at = HashMap::new();
        let mut calculated = HashSet::new();
        for (k, r) in regions.iter().enumerate() {
            for cell in &r.cells {
                cell_at.insert((r.area.sheet, cell.row, cell.col), (k, cell.name.clone()));
                if cell.formula.is_some() {
                    calculated.insert((k, cell.name.clone()));
                }
            }
            for c in r.columns.iter().filter(|c| c.is_calculated()) {
                calculated.insert((k, c.name.clone()));
            }
        }
        Context { at, last_rows: crate::lift::last_rows_of(wb), cell_at, calculated }
    }
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", xl_formula::col_name(col + 1), row + 1)
}

/// Whether a formula may read cells of a column (1-based) of a sheet: a cell or range
/// that covers it, whole rows, or a reference that does not say (a structured one).
fn reads_column(e: &xl_formula::Expr, col: u32) -> bool {
    use xl_formula::{BinaryOp, Expr, RefKind};
    let column = |x: &Expr| match x.unspaced() {
        Expr::Ref { kind: RefKind::Cell, text, .. } => xl_formula::parse_cell_ref(text).map(|r| r.col),
        _ => None,
    };
    let go = |x: &Expr| reads_column(x, col);
    match e {
        Expr::Ref { kind: RefKind::Cell, .. } => column(e) == Some(col),
        Expr::Ref { kind: RefKind::Rows, .. } | Expr::Structured { .. } => true,
        Expr::Ref { kind: RefKind::Columns, text, .. } => {
            let ends: Vec<Option<u32>> = text.split(':').map(|t| xl_formula::parse_cell_ref(&format!("{}1", t.replace('$', ""))).map(|r| r.col)).collect();
            match ends.as_slice() {
                [Some(a), Some(z)] => (*a.min(z)..=*a.max(z)).contains(&col),
                [Some(a)] => *a == col,
                _ => true,
            }
        }
        Expr::Binary { op: BinaryOp::Range, lhs, rhs, .. } => match (column(lhs), column(rhs)) {
            (Some(a), Some(z)) => (a.min(z)..=a.max(z)).contains(&col),
            _ => true,
        },
        Expr::Binary { lhs, rhs, .. } => go(lhs) || go(rhs),
        Expr::Call { args, .. } => args.iter().any(go),
        Expr::Apply { callee, args } => go(callee) || args.iter().any(go),
        Expr::Unary { expr, .. } | Expr::Postfix { expr, .. } | Expr::Paren(expr) | Expr::Spaced { expr, .. } => go(expr),
        Expr::Array(rows) => rows.iter().flatten().any(go),
        _ => false,
    }
}

/// Maps translations of a workbook on its regions.
pub struct Mapper<'a> {
    pub wb: &'a Workbook,
    pub regions: &'a [Region],
    pub at: &'a HashMap<(u32, u32, u32), usize>,
    pub last_rows: &'a [u32],
    pub cell_at: &'a HashMap<(u32, u32, u32), Key>,
    pub calculated: &'a HashSet<Key>,
    pub ex: Option<&'a dyn Executor>,
    /// Numbers in columns of mixed kinds are read from their text in the data (for
    /// notes); else from the workbook (for a program, which has it).
    pub mixed: bool,
}

impl<'a> Mapper<'a> {
    pub fn new(wb: &'a Workbook, regions: &'a [Region], context: &'a Context, ex: Option<&'a dyn Executor>) -> Mapper<'a> {
        Mapper { wb, regions, at: &context.at, last_rows: &context.last_rows, cell_at: &context.cell_at, calculated: &context.calculated, ex, mixed: false }
    }

    fn calc_dep(&self, key: &Key, deps: &mut Vec<Key>) {
        if self.calculated.contains(key) && !deps.contains(key) {
            deps.push(key.clone());
        }
    }

    /// Where a cell's value is, and the calculated column or cell it is.
    pub fn scalar_of(&self, sheet: u32, row: u32, col: u32, deps: &mut Vec<Key>) -> Scalar {
        if let Some(key) = self.cell_at.get(&(sheet, row, col)) {
            self.calc_dep(key, deps);
            return Scalar::Named(key.clone());
        }
        for (t, region) in self.regions.iter().enumerate().filter(|(_, t)| t.kind != RegionKind::Cells && t.area.sheet == sheet) {
            let Some((first, last)) = region.data_rows() else { continue };
            if (first..=last).contains(&row) {
                if let Some(c) = region.columns.iter().find(|c| c.col == col) {
                    // A number in a column of mixed kinds is text in the frame: it is read
                    // from that text where that gives the number back (a number or a date,
                    // not a boolean), else from the workbook.
                    let found = self.at.get(&(sheet, row, col)).copied();
                    let number = found.is_some_and(|i| matches!(self.wb.cells.kind[i], Kind::Number | Kind::Bool));
                    let mixed = c.dtype == "String" && number;
                    let plain = found.is_some_and(|i| self.wb.cells.kind[i] == Kind::Number);
                    if mixed && !(self.mixed && plain) {
                        break;
                    }
                    self.calc_dep(&(t, c.name.clone()), deps);
                    return Scalar::TableCell { region: t, column: c.name.clone(), row: row - first, parse: mixed };
                }
            }
        }
        Scalar::Sheet { sheet, row, col }
    }

    /// The numbers (or `text`) of a column of the sheet (`col`, 0-based) in rows
    /// `first..=last`, as a column of the region that holds them.
    pub fn column_of(&self, sheet: u32, col: u32, rows: (u32, u32), text: bool, deps: &mut Vec<Key>) -> Result<ColumnSource, String> {
        let found = self.regions.iter().enumerate().find(|(_, t)| t.kind != RegionKind::Cells && t.area.sheet == sheet && t.data_rows() == Some(rows) && t.columns.iter().any(|c| c.col == col));
        let Some((t, region)) = found else {
            return Err(format!("{} is not a column of a table over these rows", a1(rows.0, col).trim_end_matches(|c: char| c.is_ascii_digit())));
        };
        let c = region.columns.iter().find(|c| c.col == col).expect("found");
        let key = (t, c.name.clone());
        self.calc_dep(&key, deps);
        let kinds = |f: &dyn Fn(Kind) -> bool| (rows.0..=rows.1).any(|r| self.at.get(&(sheet, r, col)).is_some_and(|&i| f(self.wb.cells.kind[i])));
        match (c.dtype.as_str(), text) {
            ("Int64" | "Float64" | "Boolean", false) => Ok(ColumnSource::Numbers(key)),
            ("Null", false) => Ok(ColumnSource::NoNumbers { column: key, text: false }),
            ("String", false) if !kinds(&|k| matches!(k, Kind::Number | Kind::Bool)) => Ok(ColumnSource::NoNumbers { column: key, text: true }),
            ("String", false) => Err(format!("the column {} holds numbers and text", c.name)),
            ("String", true) => Ok(ColumnSource::Text(key)),
            (_, true) => Err(format!("the column {} holds text but is not text in its frame", c.name)),
            (d, false) => Err(format!("the column {} is of the type {d}", c.name)),
        }
    }

    /// The cells of a fixed range (1-based, `sheet, r1, r2, c1, c2`).
    pub fn range_of(&self, (sheet, r1, r2, c1, c2): (u32, u32, u32, u32, u32), deps: &mut Vec<Key>) -> Result<RangeSource, String> {
        let (a, z) = (r1 - 1, r2 - 1);
        if let Some((t, region)) = self.regions.iter().enumerate().find(|(_, t)| t.kind != RegionKind::Cells && t.area.sheet == sheet && t.data_rows() == Some((a, z))) {
            let names: Vec<&crate::regions::RegionColumn> = region.columns.iter().filter(|c| (c1 - 1..=c2 - 1).contains(&c.col)).collect();
            if names.len() == (c2 - c1 + 1) as usize {
                for c in &names {
                    if !matches!(c.dtype.as_str(), "Int64" | "Float64" | "Boolean" | "String" | "Null") {
                        return Err(format!("the column {} is of the type {}", c.name, c.dtype));
                    }
                    // Numbers in a column of mixed kinds are text in its frame.
                    if c.dtype == "String" && (a..=z).any(|r| self.at.get(&(sheet, r, c.col)).is_some_and(|&i| matches!(self.wb.cells.kind[i], Kind::Number | Kind::Bool))) {
                        return Err(format!("the column {} holds numbers and text", c.name));
                    }
                    self.calc_dep(&(t, c.name.clone()), deps);
                }
                return Ok(RangeSource::Columns(t, names.iter().map(|c| c.name.clone()).collect()));
            }
        }
        let in_table = |row: u32, col: u32| self.regions.iter().any(|t| t.kind != RegionKind::Cells && t.area.sheet == sheet && t.data_rows().is_some_and(|(f, l)| (f..=l).contains(&row)) && t.columns.iter().any(|c| c.col == col));
        let count = (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64;
        let named = (a..=z).any(|r| (c1 - 1..=c2 - 1).any(|c| self.cell_at.contains_key(&(sheet, r, c))));
        // Part of a table: its cells one by one, by their place in the table.
        if count > 400 && (a..=z).any(|r| (c1 - 1..=c2 - 1).any(|c| in_table(r, c))) {
            return Err("a large range over part of a table".into());
        }
        if count > 400 {
            if named {
                return Err("a large range that holds named cells".into());
            }
            return Ok(RangeSource::Sheet(sheet, r1, r2, c1, c2));
        }
        let mut list = Vec::new();
        for r in a..=z {
            for c in c1 - 1..=c2 - 1 {
                list.push(self.scalar_of(sheet, r, c, deps));
            }
        }
        Ok(RangeSource::Cells(list))
    }

    /// The engine's translation of a calculated column or cell of a region, on the data of
    /// the regions. `Err` says why there is none.
    pub fn map(&self, k: usize, name: &str) -> Result<Mapped, String> {
        let r = &self.regions[k];
        match r.kind {
            RegionKind::Cells => {
                let c = r.cells.iter().find(|c| c.name == name).ok_or("no such cell")?;
                self.map_run(Run { sheet: r.area.sheet, col: c.col, row: c.row, len: 1, formula: c.formula.clone().ok_or("no formula")? }, (c.row, c.row), true)
            }
            _ => {
                let c = r.columns.iter().find(|c| c.name == name).ok_or("no such column")?;
                let (first, last) = r.data_rows().ok_or("no rows")?;
                let formula = c.formula.clone().ok_or("no formula")?;
                // The translation takes the formula to be in every row: what it makes of
                // the column's own rows does not hold where some of them are not so.
                if !c.exceptions.is_empty() && xl_formula::parse(&formula).map_or(true, |e| reads_column(&e, c.col + 1)) {
                    return Err("it reads its own column, which has rows of their own".into());
                }
                self.map_run(Run { sheet: r.area.sheet, col: c.col, row: first, len: last - first + 1, formula }, (first, last), false)
            }
        }
    }

    /// The translation of one row of a column (`row`: of the sheet, 0-based), as that of
    /// a single cell: of a row with a formula of its own, or of any row of a column that
    /// has no translation as a whole. `each`: the column is worked out row by row, top to
    /// bottom; else its rows of their own are, after the column.
    pub fn map_row(&self, k: usize, name: &str, row: u32, each: bool) -> Result<Mapped, String> {
        let r = &self.regions[k];
        let c = r.columns.iter().find(|c| c.name == name).ok_or("no such column")?;
        let first = r.data_rows().ok_or("no rows")?.0;
        let formula = c.formula_at(row, first).ok_or("no formula")?;
        let m = self.map_run(Run { sheet: r.area.sheet, col: c.col, row, len: 1, formula }, (row, row), true)?;
        let mut cells: Vec<&Scalar> = Vec::new();
        for i in &m.inputs {
            match i {
                Input::Own { cell, .. } | Input::Cell { cell, .. } => cells.push(cell),
                Input::Range { source: RangeSource::Cells(list), .. } => cells.extend(list),
                Input::Range { source: RangeSource::Columns(t, names), .. } if *t == k && names.iter().any(|n| n == name) => return Err("it reads its own column".into()),
                _ => {}
            }
        }
        let later = |s: &&Scalar| matches!(s, Scalar::TableCell { region, column, row: at, .. } if *region == k && column == name && first + at >= row && (each || c.exception(first + at).is_some()));
        if cells.iter().any(later) {
            return Err("it reads a row of its column that is worked out after it".into());
        }
        Ok(m)
    }

    fn map_run(&self, run: Run, rows: (u32, u32), single: bool) -> Result<Mapped, String> {
        let ex = self.ex.ok_or("no engine to translate with")?;
        let t = translate_program_in(ex, self.wb, self.at, self.last_rows, &run)?;
        let l = &t.lifted;
        if l.own.is_some() {
            return Err("it reads its own column at other rows".into());
        }
        if let Some(why) = t.guards.first() {
            return Err(format!("rows the translation does not hold for: {why}"));
        }
        if l.reach != (0, 0) {
            return Err("it reads rows above or below its own".into());
        }
        if !l.parsed_columns.is_empty() {
            return Err("it reads text as numbers".into());
        }
        let a = &l.assumptions;
        if !a.extents.is_empty() {
            return Err("it depends on how far the sheet reaches".into());
        }
        if a.columns.iter().any(|c| c.error) {
            return Err("a column it reads holds error values".into());
        }
        // What the translation took as it was must not be data of the program.
        for p in &a.pinned {
            for row in p.first..=p.last {
                let held = self.at.get(&(p.sheet, row, p.col)).is_some_and(|&i| self.wb.cells.kind[i] != Kind::Blank || self.wb.cells.formula[i].is_some());
                if held {
                    return Err(format!("it depends on the value of {}", a1(row, p.col)));
                }
            }
        }
        let used: HashSet<String> = t.plan.steps.iter().flatten().map(|(_, e)| e).chain(t.plan.outputs.iter().map(|(_, e)| e)).flat_map(|e| e.columns()).collect();
        let mut deps = Vec::new();
        let mut inputs: Vec<Input> = Vec::new();
        for (text, columns) in [(false, &l.columns), (true, &l.text_columns)] {
            for &(sheet, col1) in columns {
                let name = if text { text_column_name(sheet, col1) } else { column_name(sheet, col1) };
                if used.contains(&name) {
                    inputs.push(if single { Input::Own { name, cell: self.scalar_of(sheet, rows.0, col1 - 1, &mut deps), text } } else { Input::Column { name, source: self.column_of(sheet, col1 - 1, rows, text, &mut deps)? } });
                }
            }
        }
        for &(sheet, row, col, text) in &a.cell_inputs {
            let name = cell_input_name(sheet, row, col, text);
            if used.contains(&name) {
                inputs.push(Input::Cell { name, cell: self.scalar_of(sheet, row, col, &mut deps), text });
            }
        }
        const PARTS: [char; 5] = ['s', 'n', 'l', 'h', 'q'];
        for (index, &(key, _)) in a.ranges.iter().enumerate() {
            let parts: Vec<(usize, String)> = PARTS.iter().enumerate().map(|(i, w)| (i, range_input_name(key, *w))).filter(|(_, n)| used.contains(n)).collect();
            if !parts.is_empty() {
                inputs.push(Input::Range { index, source: self.range_of(key, &mut deps)?, parts });
            }
        }
        let given: HashSet<&str> = inputs
            .iter()
            .flat_map(|i| match i {
                Input::Column { name, .. } | Input::Own { name, .. } | Input::Cell { name, .. } => vec![name.as_str()],
                Input::Range { parts, .. } => parts.iter().map(|(_, n)| n.as_str()).collect(),
            })
            .collect();
        if let Some(missing) = used.iter().filter(|u| *u != ROW && !u.starts_with("_t") && !given.contains(u.as_str())).min() {
            return Err(format!("the plan reads {missing}, which has no source in the program"));
        }
        let is_bool = l.is_bool;
        Ok(Mapped { plan: t.plan, inputs, deps, rows, is_bool })
    }

    /// The value of a cell of the workbook as a calculation takes it.
    fn sheet_value(&self, sheet: u32, row: u32, col: u32) -> V {
        let Some(&i) = self.at.get(&(sheet, row, col)) else { return V::Null };
        let c = &self.wb.cells;
        if c.formula[i].as_deref().is_some_and(|f| !f.is_empty()) {
            return V::Null;
        }
        match c.kind[i] {
            Kind::Number => V::Num(c.num[i]),
            Kind::Bool => V::Bool(c.num[i] != 0.0),
            Kind::Text => V::Text(c.text[i].as_deref().unwrap_or("").to_string()),
            _ => V::Null,
        }
    }

    /// A column or cell: its name within the region `home`, else `[region, name]`.
    fn key_json(&self, home: usize, (k, name): &Key) -> J {
        if *k == home {
            json!(name)
        } else {
            json!([self.regions[*k].name, name])
        }
    }

    fn scalar_json(&self, home: usize, s: &Scalar) -> J {
        match s {
            Scalar::Named(key) => self.key_json(home, key),
            Scalar::TableCell { region, column, row, parse: false } => json!({ "table": [self.regions[*region].name, column, row] }),
            Scalar::TableCell { region, column, row, parse: true } => json!({ "table": [self.regions[*region].name, column, row], "parse": true }),
            Scalar::Sheet { sheet, row, col } => json!({ "value": self.sheet_value(*sheet, *row, *col).json(), "at": format!("{}!{}", self.wb.sheets[*sheet as usize].name, a1(*row, *col)) }),
            Scalar::Const(v) => json!({ "value": v.json() }),
        }
    }

    /// A mapped translation as a note: the plan (see [`ir::wire`]) and its inputs by the
    /// names of regions, columns and cells. The inputs are named `i0`, `i1`, … in the plan,
    /// so that the same formula in another place is the same plan. Cells in no region are
    /// not data of the regions: their values are kept in the note. `home` is the region
    /// the note is of: its own columns and cells are written by their name alone.
    pub fn json(&self, home: usize, m: &Mapped) -> J {
        let mut short: HashMap<&str, String> = HashMap::new();
        for i in &m.inputs {
            let names: Vec<&str> = match i {
                Input::Column { name, .. } | Input::Own { name, .. } | Input::Cell { name, .. } => vec![name],
                Input::Range { parts, .. } => parts.iter().map(|(_, n)| n.as_str()).collect(),
            };
            for n in names {
                let k = short.len();
                short.entry(n).or_insert_with(|| format!("i{k}"));
            }
        }
        // What an input is taken as is said where it is not a number.
        let with = |mut o: J, how: &str| {
            if how != "number" {
                o["as"] = json!(how);
            }
            o
        };
        let kind = |text: bool| if text { "text" } else { "number" };
        let inputs: Vec<J> = m
            .inputs
            .iter()
            .map(|i| match i {
                Input::Column { name, source } => {
                    let (key, how) = match source {
                        ColumnSource::Numbers(k) => (k, "number"),
                        ColumnSource::NoNumbers { column, text: false } => (column, "empty"),
                        ColumnSource::NoNumbers { column, text: true } => (column, "no number"),
                        ColumnSource::Text(k) => (k, "text"),
                    };
                    with(json!({ "name": short[name.as_str()], "column": self.key_json(home, key) }), how)
                }
                Input::Own { name, cell, text } => with(json!({ "name": short[name.as_str()], "own": self.scalar_json(home, cell) }), kind(*text)),
                Input::Cell { name, cell, text } => with(json!({ "name": short[name.as_str()], "cell": self.scalar_json(home, cell) }), kind(*text)),
                Input::Range { source, parts, .. } => {
                    let range = match source {
                        RangeSource::Columns(t, names) => json!({ "columns": [self.regions[*t].name, names] }),
                        RangeSource::Cells(list) => json!({ "cells": list.iter().map(|s| self.scalar_json(home, s)).collect::<Vec<_>>() }),
                        RangeSource::Sheet(sheet, r1, r2, c1, c2) => {
                            let values: Vec<V> = (r1 - 1..=r2 - 1).flat_map(|r| (c1 - 1..=c2 - 1).map(move |c| (r, c))).map(|(r, c)| self.sheet_value(*sheet, r, c)).collect();
                            json!({ "summary": summary(values.iter().map(Part::Value)), "at": format!("{}!{}:{}", self.wb.sheets[*sheet as usize].name, a1(r1 - 1, c1 - 1), a1(r2 - 1, c2 - 1)) })
                        }
                        RangeSource::Summary(s) => json!({ "summary": s }),
                    };
                    json!({ "range": range, "parts": parts.iter().map(|(i, n)| json!([i, short[n.as_str()]])).collect::<Vec<_>>() })
                }
            })
            .collect();
        let mut plan: J = serde_json::from_str(&ir::wire::plan_json(&m.plan)).unwrap_or(J::Null);
        for node in plan["nodes"].as_array_mut().into_iter().flatten() {
            if node[0] == "col" {
                if let Some(to) = node[1].as_str().and_then(|n| short.get(n)) {
                    node[1] = json!(to);
                }
            }
        }
        let mut o = json!({ "plan": plan, "inputs": inputs, "first_row": m.rows.0 + 1, "rows": m.rows.1 - m.rows.0 + 1 });
        if m.is_bool {
            o["bool"] = json!(true);
        }
        o
    }
}

/// A part of a range: a column, or a single value.
pub enum Part<'a> {
    Column(&'a Data),
    Value(&'a V),
}

/// The sum, count, smallest, largest and sum of squares of the numbers among the parts
/// of a range: what SUM, COUNT, MIN, MAX, AVERAGE and STDEV take from it. Booleans and
/// text do not count; of a column, only numbers do.
pub fn summary<'a>(parts: impl Iterator<Item = Part<'a>>) -> [Option<f64>; 5] {
    let mut numbers: Vec<f64> = Vec::new();
    for p in parts {
        match p {
            Part::Column(Data::Int64(v)) => numbers.extend(v.iter().flatten().map(|&x| x as f64)),
            Part::Column(Data::Float64(v)) => numbers.extend(v.iter().flatten()),
            Part::Column(_) => {}
            Part::Value(V::Num(x)) => numbers.push(*x),
            Part::Value(_) => {}
        }
    }
    let (mut total, mut squares) = (0.0, 0.0);
    for n in &numbers {
        total += n;
        squares += n * n;
    }
    let least = numbers.iter().copied().reduce(f64::min);
    let most = numbers.iter().copied().reduce(f64::max);
    [Some(total), Some(numbers.len() as f64), least, most, Some(squares)]
}

/// The serial number of a day (days since 1970) as a workbook counts it.
fn serial(days: f64, date1904: bool) -> f64 {
    let s = days + if date1904 { 24_107.0 } else { 25_569.0 };
    // The 1900 system counts a 29 February 1900: days before March 1900 are one less.
    if !date1904 && s < 61.0 {
        s - 1.0
    } else {
        s
    }
}

/// A value of a column as a calculation takes it.
pub fn value_at(data: &Data, k: usize, date1904: bool) -> V {
    match data {
        Data::Null(_) => V::Null,
        Data::Bool(v) => v.get(k).copied().flatten().map_or(V::Null, V::Bool),
        Data::Int64(v) => v.get(k).copied().flatten().map_or(V::Null, |x| V::Num(x as f64)),
        Data::Float64(v) => v.get(k).copied().flatten().map_or(V::Null, V::Num),
        Data::String(v) => v.get(k).cloned().flatten().map_or(V::Null, V::Text),
        Data::Date(v) => v.get(k).copied().flatten().map_or(V::Null, |d| V::Num(serial(d as f64, date1904))),
        Data::Datetime(v) => v.get(k).copied().flatten().map_or(V::Null, |ms| V::Num(serial(ms.div_euclid(86_400_000) as f64, date1904) + ms.rem_euclid(86_400_000) as f64 / 86_400_000.0)),
        Data::Time(v) => v.get(k).copied().flatten().map_or(V::Null, |ns| V::Num(ns as f64 / 86_400e9)),
        Data::Duration(v) => v.get(k).copied().flatten().map_or(V::Null, |ms| V::Num(ms as f64 / 86_400_000.0)),
    }
}

/// A number as Python writes it (`repr`): the text of a number in a column of text.
fn number_text(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e300 {
        return format!("{x:.0}");
    }
    let e = format!("{x:e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..16).contains(&exp) {
        format!("{x}")
    } else {
        format!("{mantissa}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

/// Serial numbers as the dates, times or durations a workbook column shows (a Date
/// column whose values have a time of day becomes a Datetime).
fn typed(number: &[Option<f64>], dtype: &str, date1904: bool) -> Data {
    match dtype {
        "Time" => return Data::Time(number.iter().map(|n| n.map(|x| (x * 86_400e9).round_ties_even() as i64)).collect()),
        "Duration" => return Data::Duration(number.iter().map(|n| n.map(|x| (x * 86_400_000.0).round_ties_even() as i64)).collect()),
        _ => {}
    }
    let since_1970 = |x: f64| {
        let days = x.floor();
        if date1904 {
            days - 24_107.0
        } else if days < 61.0 {
            days - 25_568.0
        } else {
            days - 25_569.0
        }
    };
    if dtype == "Date" && number.iter().flatten().all(|x| *x == x.floor()) {
        return Data::Date(number.iter().map(|n| n.map(|x| since_1970(x) as i32)).collect());
    }
    Data::Datetime(number.iter().map(|n| n.map(|x| (since_1970(x) * 86_400_000.0 + ((x - x.floor()) * 86_400_000.0).round_ties_even()) as i64)).collect())
}

/// A plan's values (`_r`, the numbers, and `_rt`, the text) as a column: booleans, text
/// where the formula gives text, else numbers (whole numbers as integers; an error is
/// null), or dates, times and durations where `dtype` says the workbook shows them so.
pub fn result_data(out: &Table, is_bool: bool, dtype: Option<&str>, date1904: bool) -> Result<Data, String> {
    let find = |name: &str| out.columns.iter().find(|(n, _)| n == name).map(|(_, v)| v).ok_or_else(|| format!("the plan gave no {name}"));
    let (r, rt) = (find("_r")?, find("_rt")?);
    let n = out.height;
    let number: Vec<Option<f64>> = (0..n).map(|k| r.f64(k).filter(|x| x.is_finite())).collect();
    // An empty text is no value in a table (the cell looks empty).
    let text: Vec<Option<&str>> = (0..n).map(|k| rt.str(k).filter(|t| !t.is_empty())).collect();
    if is_bool {
        return Ok(Data::Bool(number.iter().map(|x| x.map(|x| x != 0.0)).collect()));
    }
    let no_text = text.iter().all(Option::is_none);
    if let Some(d) = dtype.filter(|d| matches!(*d, "Date" | "Datetime" | "Time" | "Duration") && no_text) {
        return Ok(typed(&number, d, date1904));
    }
    if !no_text {
        return Ok(Data::String(text.iter().zip(&number).map(|(t, x)| t.map(str::to_string).or_else(|| x.map(number_text))).collect()));
    }
    let mut present = number.iter().flatten().peekable();
    if present.peek().is_some() && present.all(|x| *x == x.floor() && x.abs() < 9e15) {
        return Ok(Data::Int64(number.iter().map(|x| x.map(|x| x as i64)).collect()));
    }
    Ok(Data::Float64(number))
}

/// One calculated column or cell of the notes.
struct Item {
    key: Key,
    formula: String,
    dtype: Option<String>,
    /// The mapped plan, or why there is none; `None`: a column of values (with exceptions).
    mapped: Option<Result<Noted, String>>,
    /// Of a column that is worked out row by row: the rows it had when the note was made.
    rows: Option<usize>,
    /// The rows of a column that are worked out one by one, or keep their value: those not
    /// as the column is, or all of them. Top to bottom.
    exceptions: Vec<Exc>,
}

/// A row of a column with a formula of its own, or a plain value (`mapped` is `None`).
struct Exc {
    /// Of the data, 0-based.
    row: usize,
    formula: String,
    /// What the row's value is where the column is text of mixed kinds (a date).
    dtype: Option<String>,
    mapped: Option<Result<Noted, String>>,
}

impl Item {
    /// Every plan of the item: its own and those of its exceptions.
    fn plans(&self) -> impl Iterator<Item = &Noted> {
        self.mapped.iter().chain(self.exceptions.iter().filter_map(|e| e.mapped.as_ref())).filter_map(|m| m.as_ref().ok())
    }
}

/// Put the value `from[j]` into `data[k]`. Where the kinds differ the column becomes what
/// holds both: numbers with fractions, else text (as a column of mixed kinds is in a frame).
fn put(data: &mut Data, k: usize, from: &Data, j: usize) {
    if k >= data.len() || j >= from.len() {
        return;
    }
    // A column without any value has no kind yet (it is stored as text).
    let empty = match &*data {
        Data::Null(n) => Some(*n),
        Data::String(v) if !matches!(from, Data::String(_)) && v.iter().all(Option::is_none) => Some(v.len()),
        _ => None,
    };
    if let Some(n) = empty {
        *data = from.nulls_like(n);
    }
    match (&mut *data, from) {
        (Data::Null(_), _) => return,
        (Data::Bool(a), Data::Bool(b)) => return a[k] = b[j],
        (Data::Int64(a), Data::Int64(b)) => return a[k] = b[j],
        (Data::Float64(a), Data::Float64(b)) => return a[k] = b[j],
        (Data::Float64(a), Data::Int64(b)) => return a[k] = b[j].map(|x| x as f64),
        (Data::String(a), Data::String(b)) => return a[k] = b[j].clone(),
        (Data::Date(a), Data::Date(b)) => return a[k] = b[j],
        (Data::Datetime(a), Data::Datetime(b)) => return a[k] = b[j],
        (Data::Time(a), Data::Time(b)) => return a[k] = b[j],
        (Data::Duration(a), Data::Duration(b)) => return a[k] = b[j],
        _ => {}
    }
    let Some(text) = crate::frame::mixed_text(from, j) else {
        // Nothing: in whatever the column holds.
        let none = data.nulls_like(1);
        return put(data, k, &none, 0);
    };
    if let (Data::Int64(a), Data::Float64(b)) = (&*data, from) {
        let mut wider: Vec<Option<f64>> = a.iter().map(|x| x.map(|x| x as f64)).collect();
        wider[k] = b[j];
        *data = Data::Float64(wider);
        return;
    }
    let mut texts: Vec<Option<String>> = match &*data {
        Data::String(a) => a.clone(),
        other => (0..other.len()).map(|i| crate::frame::mixed_text(other, i)).collect(),
    };
    texts[k] = Some(text);
    *data = Data::String(texts);
}

/// Run plans on a table each. Those with the same plan and the same columns go through
/// it together, one below the other, where the plan works row by row.
fn run_each(ex: &dyn Executor, jobs: &[(&Arc<Plan>, Table)], rowwise: &mut HashMap<usize, bool>) -> Vec<Result<Table, String>> {
    let mut out: Vec<Option<Result<Table, String>>> = jobs.iter().map(|_| None).collect();
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut batch_at: HashMap<(usize, String), usize> = HashMap::new();
    for (i, (plan, t)) in jobs.iter().enumerate() {
        let id = Arc::as_ptr(plan) as usize;
        if t.height == 1 && *rowwise.entry(id).or_insert_with(|| row_by_row(plan)) {
            let at = *batch_at.entry((id, signature(t))).or_insert_with(|| {
                batches.push(Vec::new());
                batches.len() - 1
            });
            batches[at].push(i);
        } else {
            out[i] = Some(ex.run(plan, t));
        }
    }
    for batch in batches {
        let plan = jobs[batch[0]].0;
        let together = if batch.len() > 1 { stacked(batch.iter().map(|&i| &jobs[i].1)).and_then(|all| ex.run(plan, &all).ok()).filter(|o| o.height == batch.len()) } else { None };
        for (k, &i) in batch.iter().enumerate() {
            out[i] = Some(match &together {
                Some(all) => Ok(row_of(all, k)),
                None => ex.run(plan, &jobs[i].1),
            });
        }
    }
    out.into_iter().map(|o| o.unwrap_or_else(|| Err("not calculated".into()))).collect()
}

struct Noted {
    plan: Arc<Plan>,
    inputs: Vec<Input>,
    deps: Vec<Key>,
    first_row: u32,
    rows: usize,
    is_bool: bool,
}

/// What [`recalculate`] did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recalculation {
    /// Calculated again: (region, column or cell). A row of a column with a formula of
    /// its own is named `column (row n)`, n counted from 1 in the data.
    pub done: Vec<(String, String)>,
    /// Left with the values they had: (region, column or cell, formula, why).
    pub kept: Vec<(String, String, String, String)>,
    /// Calculated again, but from something that was left as it was (directly, or through
    /// what it reads): (region, column or cell, what was left). Such a value is that of
    /// the workbook only if what was left would not have changed.
    pub from_kept: Vec<(String, String, String)>,
}

/// The data of the regions while their notes are run.
struct World<'a> {
    names: Vec<String>,
    single: Vec<bool>,
    frames: &'a mut [Frame],
    /// The values of single cells calculated in this run, before they are typed.
    fresh: HashMap<Key, V>,
    date1904: bool,
}

impl World<'_> {
    fn column(&self, (k, name): &Key) -> Result<&Data, String> {
        self.frames[*k].column(name).map(|c| &c.data).ok_or_else(|| format!("{} has no column {name:?} any more", self.names[*k]))
    }

    fn scalar(&self, s: &Scalar) -> Result<V, String> {
        match s {
            Scalar::Named(key) => {
                if let Some(v) = self.fresh.get(key) {
                    return Ok(v.clone());
                }
                // An empty cell counts as 0.
                Ok(match value_at(self.column(key)?, 0, self.date1904) {
                    V::Null => V::Num(0.0),
                    v => v,
                })
            }
            Scalar::TableCell { region, column, row, parse } => {
                let data = self.column(&(*region, column.clone()))?;
                if *row as usize >= data.len() {
                    return Err(format!("{}[{column}] has no row {} any more", self.names[*region], row + 1));
                }
                Ok(match value_at(data, *row as usize, self.date1904) {
                    // The text of a number, or of a date as a frame writes it.
                    V::Text(t) if *parse => match t.parse::<f64>().ok().or_else(|| crate::store::parse_datetime(&t).map(|ms| serial(ms.div_euclid(86_400_000) as f64, self.date1904) + ms.rem_euclid(86_400_000) as f64 / 86_400_000.0)) {
                        Some(x) => V::Num(x),
                        None => V::Text(t),
                    },
                    v => v,
                })
            }
            Scalar::Const(v) => Ok(v.clone()),
            Scalar::Sheet { .. } => Err("a cell of the workbook that is not in the data".into()),
        }
    }

    /// The input table of an item, or why its plan does not hold for the data.
    fn table(&self, key: &Key, m: &Noted) -> Result<Table, String> {
        self.table_of(if self.single[key.0] { 1 } else { self.frames[key.0].height }, m)
    }

    /// The input table of a plan for `n` rows.
    fn table_of(&self, n: usize, m: &Noted) -> Result<Table, String> {
        if n != m.rows && has_series(&m.plan) {
            return Err(format!("it was translated for {} rows, there are {n}", m.rows));
        }
        let mut t = Table::new(n);
        t.push(ROW, (0..n as i64).map(|k| m.first_row as i64 + k).collect::<Vec<i64>>());
        let what = |key: &Key| format!("{}[{}]", self.names[key.0], key.1);
        for input in &m.inputs {
            match input {
                Input::Column { name, source } => match source {
                    ColumnSource::Numbers(c) | ColumnSource::NoNumbers { column: c, text: false } => {
                        let data = self.column(c)?;
                        let numbers: Vec<Option<f64>> = match data {
                            Data::Int64(v) => v.iter().map(|x| x.map(|x| x as f64)).collect(),
                            Data::Float64(v) => v.clone(),
                            Data::Bool(v) => v.iter().map(|x| x.map(|b| if b { 1.0 } else { 0.0 })).collect(),
                            Data::Null(k) => vec![None; *k],
                            other => return Err(format!("{} held numbers and holds {} now", what(c), other.type_name())),
                        };
                        t.push(name.clone(), if matches!(source, ColumnSource::Numbers(_)) { numbers } else { vec![None; n] });
                    }
                    ColumnSource::NoNumbers { column: c, text: true } => {
                        if !matches!(self.column(c)?, Data::String(_)) {
                            return Err(format!("{} held text and holds {} now", what(c), self.column(c)?.type_name()));
                        }
                        t.push(name.clone(), vec![None::<f64>; n]);
                    }
                    ColumnSource::Text(c) => match self.column(c)? {
                        Data::String(v) => t.push(name.clone(), v.clone()),
                        other => return Err(format!("{} held text and holds {} now", what(c), other.type_name())),
                    },
                },
                Input::Own { name, cell, text } => {
                    let v = self.scalar(cell)?;
                    if *text {
                        t.push(name.clone(), vec![v.text().map(str::to_string); n]);
                    } else {
                        t.push(name.clone(), vec![v.num(); n]);
                    }
                }
                Input::Cell { name, cell, text } => {
                    let v = self.scalar(cell)?;
                    if *text {
                        if !v.is_text() {
                            return Err("a cell it reads held text and holds a number now".into());
                        }
                        // A text cell without a value holds the empty text (a formula's "").
                        let value = v.text().unwrap_or("");
                        if !crate::lift::text_reads_plainly(value, self.date1904) {
                            return Err(format!("a text it reads ({value:?}) is a number to Excel, which the plan does not see"));
                        }
                        t.push(name.clone(), vec![Some(value.to_string()); n]);
                    } else {
                        let Some(x) = v.num() else { return Err("a cell it reads held a number and holds none now".into()) };
                        t.push(name.clone(), vec![Some(x); n]);
                    }
                }
                Input::Range { source, parts, .. } => {
                    let values: Vec<V>;
                    let s = match source {
                        RangeSource::Columns(k, names) => {
                            let columns = names.iter().map(|c| self.column(&(*k, c.clone()))).collect::<Result<Vec<_>, _>>()?;
                            summary(columns.into_iter().map(Part::Column))
                        }
                        RangeSource::Cells(list) => {
                            values = list.iter().map(|s| self.scalar(s)).collect::<Result<_, _>>()?;
                            // A boolean is no number of a range.
                            summary(values.iter().map(Part::Value))
                        }
                        RangeSource::Summary(s) => *s,
                        RangeSource::Sheet(..) => return Err("a range of the workbook that is not in the data".into()),
                    };
                    for (i, name) in parts {
                        t.push(name.clone(), vec![s[*i]; n]);
                    }
                }
            }
        }
        Ok(t)
    }
}

fn has_series(plan: &Plan) -> bool {
    let mut seen = HashSet::new();
    let mut stack: Vec<&ir::Expr> = plan.steps.iter().flatten().map(|(_, e)| e).chain(plan.outputs.iter().map(|(_, e)| e)).collect();
    while let Some(e) = stack.pop() {
        if !seen.insert(e.id()) {
            continue;
        }
        match e.node() {
            ir::Node::Literal(ir::Lit::Series(_)) => return true,
            ir::Node::Column(_) | ir::Node::Literal(_) => {}
            ir::Node::Binary(a, _, b) => stack.extend([a, b]),
            ir::Node::Function(_, args) => stack.extend(args),
            ir::Node::Ternary(c, a, b) => stack.extend([c, a, b]),
            ir::Node::Cast(a, _) => stack.push(a),
        }
    }
    false
}

/// The mapped plan of a column or cell of a note; `plan` is its plan.
fn noted(v: &J, plan: Arc<Plan>, home: usize, index: &HashMap<&str, usize>) -> Result<Noted, String> {
    let key = |x: &J| -> Result<Key, String> {
        if let Some(name) = x.as_str() {
            return Ok((home, name.to_string()));
        }
        let (r, n) = (x[0].as_str().ok_or("a column or cell is a name or [region, name]")?, x[1].as_str().ok_or("a column or cell is a name or [region, name]")?);
        Ok((*index.get(r).ok_or_else(|| format!("the region {r:?} is not among the data"))?, n.to_string()))
    };
    let scalar = |x: &J| -> Result<Scalar, String> {
        if x.is_string() || x.is_array() {
            Ok(Scalar::Named(key(x)?))
        } else if let Some(t) = x["table"].as_array() {
            let (region, column) = key(&x["table"])?;
            Ok(Scalar::TableCell { region, column, row: t.get(2).and_then(J::as_u64).ok_or("a cell of a table is [region, column, row]")? as u32, parse: x["parse"] == true })
        } else {
            Ok(Scalar::Const(V::of_json(&x["value"])))
        }
    };
    let mut inputs = Vec::new();
    for i in v["inputs"].as_array().ok_or("a note has inputs")? {
        let name = || i["name"].as_str().map(str::to_string).ok_or("an input has a name");
        let how = i["as"].as_str().unwrap_or("number");
        if !i["column"].is_null() {
            let c = key(&i["column"])?;
            let source = match how {
                "number" => ColumnSource::Numbers(c),
                "empty" => ColumnSource::NoNumbers { column: c, text: false },
                "no number" => ColumnSource::NoNumbers { column: c, text: true },
                "text" => ColumnSource::Text(c),
                other => return Err(format!("a column as {other:?} is not understood")),
            };
            inputs.push(Input::Column { name: name()?, source });
        } else if !i["own"].is_null() {
            inputs.push(Input::Own { name: name()?, cell: scalar(&i["own"])?, text: how == "text" });
        } else if !i["cell"].is_null() {
            inputs.push(Input::Cell { name: name()?, cell: scalar(&i["cell"])?, text: how == "text" });
        } else if !i["range"].is_null() {
            let r = &i["range"];
            let source = if let Some(c) = r["columns"].as_array() {
                let region = c.first().and_then(J::as_str).ok_or("columns are [region, [names]]")?;
                let names = c.get(1).and_then(J::as_array).ok_or("columns are [region, [names]]")?.iter().filter_map(|n| n.as_str().map(str::to_string)).collect();
                RangeSource::Columns(*index.get(region).ok_or_else(|| format!("the region {region:?} is not among the data"))?, names)
            } else if let Some(list) = r["cells"].as_array() {
                RangeSource::Cells(list.iter().map(scalar).collect::<Result<_, _>>()?)
            } else {
                let s = r["summary"].as_array().filter(|s| s.len() == 5).ok_or("a range is columns, cells or a summary")?;
                RangeSource::Summary(std::array::from_fn(|k| s[k].as_f64()))
            };
            let parts = i["parts"].as_array().ok_or("a range has parts")?.iter().filter_map(|p| Some((p[0].as_u64()? as usize, p[1].as_str()?.to_string()))).filter(|(k, _)| *k < 5).collect();
            inputs.push(Input::Range { index: 0, source, parts });
        } else {
            return Err(format!("an input of a note is not understood: {i}"));
        }
    }
    // What it reads: the columns and cells of its inputs.
    fn of_scalar(s: &Scalar, deps: &mut Vec<Key>) {
        match s {
            Scalar::Named(key) => deps.push(key.clone()),
            Scalar::TableCell { region, column, .. } => deps.push((*region, column.clone())),
            Scalar::Sheet { .. } | Scalar::Const(_) => {}
        }
    }
    let mut deps: Vec<Key> = Vec::new();
    for i in &inputs {
        match i {
            Input::Column { source: ColumnSource::Numbers(c) | ColumnSource::Text(c) | ColumnSource::NoNumbers { column: c, .. }, .. } => deps.push(c.clone()),
            Input::Own { cell, .. } | Input::Cell { cell, .. } => of_scalar(cell, &mut deps),
            Input::Range { source: RangeSource::Columns(k, names), .. } => deps.extend(names.iter().map(|n| (*k, n.clone()))),
            Input::Range { source: RangeSource::Cells(list), .. } => list.iter().for_each(|s| of_scalar(s, &mut deps)),
            Input::Range { .. } => {}
        }
    }
    Ok(Noted {
        plan,
        inputs,
        deps,
        first_row: v["first_row"].as_u64().unwrap_or(1) as u32,
        rows: v["rows"].as_u64().unwrap_or(1) as usize,
        is_bool: v["bool"].as_bool().unwrap_or(false),
    })
}

/// What was left as it was, or calculated from that: whole columns and cells, and single
/// rows of columns; each with what was left.
#[derive(Default)]
struct Stale {
    whole: HashMap<Key, String>,
    rows: HashMap<Key, HashMap<usize, String>>,
}

impl Stale {
    fn any(&self, key: &Key) -> Option<&String> {
        self.whole.get(key).or_else(|| self.rows.get(key).and_then(|r| r.values().next()))
    }

    fn scalar(&self, s: &Scalar) -> Option<&String> {
        match s {
            Scalar::Named(key) => self.any(key),
            Scalar::TableCell { region, column, row, .. } => {
                let key = (*region, column.clone());
                self.whole.get(&key).or_else(|| self.rows.get(&key).and_then(|r| r.get(&(*row as usize))))
            }
            _ => None,
        }
    }

    /// What a plan reads that is stale.
    fn read_by(&self, m: &Noted) -> Option<String> {
        m.inputs
            .iter()
            .find_map(|i| match i {
                Input::Column { source: ColumnSource::Numbers(c) | ColumnSource::Text(c) | ColumnSource::NoNumbers { column: c, .. }, .. } => self.any(c),
                Input::Own { cell, .. } | Input::Cell { cell, .. } => self.scalar(cell),
                Input::Range { source: RangeSource::Columns(k, names), .. } => names.iter().find_map(|n| self.any(&(*k, n.clone()))),
                Input::Range { source: RangeSource::Cells(list), .. } => list.iter().find_map(|s| self.scalar(s)),
                Input::Range { .. } => None,
            })
            .cloned()
    }

    fn row(&mut self, key: &Key, row: usize, what: String) {
        self.rows.entry(key.clone()).or_default().entry(row).or_insert(what);
    }
}

/// Calculate the data of regions again from their notes (see [`crate::store`] for the
/// form of a note): every column and cell that has a plan, in the order of what reads
/// what. `frames` are the regions' data in the order of `notes`; they are changed in
/// place. What has no plan, or whose plan does not hold for the data any more, keeps the
/// values it has.
pub fn recalculate(ex: &dyn Executor, notes: &[J], frames: &mut [Frame]) -> Result<Recalculation, String> {
    if notes.len() != frames.len() {
        return Err("one note for every frame".into());
    }
    let names: Vec<String> = notes.iter().map(|n| n["region"].as_str().unwrap_or("").to_string()).collect();
    let index: HashMap<&str, usize> = names.iter().enumerate().map(|(k, n)| (n.as_str(), k)).collect();
    let single: Vec<bool> = notes.iter().map(|n| n["kind"] == "cells").collect();
    let mut items: Vec<Item> = Vec::new();
    let mut shared: HashMap<(usize, usize), Arc<Plan>> = HashMap::new();
    for (k, note) in notes.iter().enumerate() {
        // The mapped plan of a column, a cell or an exception: the plan is in it, or one
        // of the note's `plans` by its number.
        let mut mapped = |c: &J| -> Result<Noted, String> {
            let plan = match &c["plan"] {
                J::Null => Err(c["why"].as_str().unwrap_or("no plan").to_string()),
                J::Number(n) => {
                    let n = n.as_u64().unwrap_or(u64::MAX) as usize;
                    match shared.get(&(k, n)) {
                        Some(p) => Ok(p.clone()),
                        None => note["plans"].get(n).ok_or_else(|| format!("the note has no plan {n}")).and_then(ir::wire::plan_from_value).map(|p| {
                            let p = Arc::new(p);
                            shared.insert((k, n), p.clone());
                            p
                        }),
                    }
                }
                own => ir::wire::plan_from_value(own).map(Arc::new),
            };
            plan.and_then(|p| noted(c, p, k, &index))
        };
        for c in note["columns"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let Some(name) = c["name"].as_str() else { continue };
            let formula = c["formula"].as_str();
            let mut exceptions: Vec<Exc> = Vec::new();
            for e in c["exceptions"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                let Some(row) = e["row"].as_u64() else { continue };
                let own = e["formula"].as_str();
                exceptions.push(Exc { row: row as usize, formula: own.unwrap_or("").to_string(), dtype: e["dtype"].as_str().map(str::to_string), mapped: own.map(|_| mapped(e)) });
            }
            // A column worked out row by row: its rows are as rows of their own.
            let each = c["each"].as_array();
            for e in each.map(|a| a.as_slice()).unwrap_or(&[]) {
                let Some(row) = e["row"].as_u64() else { continue };
                exceptions.push(Exc { row: row as usize, formula: formula.unwrap_or("").to_string(), dtype: e["dtype"].as_str().map(str::to_string), mapped: Some(mapped(e)) });
            }
            exceptions.sort_by_key(|e| e.row);
            if formula.is_none() && exceptions.iter().all(|e| e.mapped.is_none()) {
                continue;
            }
            let main = if each.is_some() { None } else { formula.map(|_| mapped(c)) };
            let rows = each.map(|_| c["rows"].as_u64().unwrap_or(0) as usize);
            items.push(Item { key: (k, name.to_string()), formula: formula.unwrap_or("").to_string(), dtype: c["dtype"].as_str().map(str::to_string), mapped: main, rows, exceptions });
        }
    }
    let mut world = World { names: names.clone(), single, frames, fresh: HashMap::new(), date1904: notes.iter().any(|n| n["date1904"] == true) };
    let mut report = Recalculation::default();
    // In the order of dependence, round by round: what reads nothing that is still to
    // come. What is left over at the end reads itself in a circle.
    let place: HashMap<&Key, usize> = items.iter().enumerate().map(|(i, item)| (&item.key, i)).collect();
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    let mut waiting: Vec<usize> = vec![0; items.len()];
    for (i, item) in items.iter().enumerate() {
        let mut reads: Vec<usize> = item.plans().flat_map(|m| m.deps.iter()).filter_map(|d| place.get(d).copied()).filter(|&j| j != i).collect();
        reads.sort_unstable();
        reads.dedup();
        waiting[i] = reads.len();
        for j in reads {
            readers[j].push(i);
        }
    }
    // What was left as it was, and what was calculated from it: each with what was left.
    let mut kept = Stale::default();
    let mut rowwise: HashMap<usize, bool> = HashMap::new();
    let mut ready: Vec<usize> = (0..items.len()).filter(|&i| waiting[i] == 0).collect();
    while !ready.is_empty() {
        // The plans' results: single cells with the same plan go through it together, one
        // row each (where the plan works row by row).
        let mut outs: HashMap<usize, Result<Table, String>> = HashMap::new();
        let mut batches: Vec<Vec<(usize, Table)>> = Vec::new();
        let mut batch_at: HashMap<(usize, String), usize> = HashMap::new();
        for &i in &ready {
            let item = &items[i];
            let m = match &item.mapped {
                Some(Ok(m)) => m,
                Some(Err(why)) => {
                    outs.insert(i, Err(why.clone()));
                    continue;
                }
                None => continue,
            };
            match world.table(&item.key, m) {
                Err(why) => {
                    outs.insert(i, Err(why));
                }
                Ok(t) => {
                    let id = Arc::as_ptr(&m.plan) as usize;
                    if world.single[item.key.0] && *rowwise.entry(id).or_insert_with(|| row_by_row(&m.plan)) {
                        let at = *batch_at.entry((id, signature(&t))).or_insert_with(|| {
                            batches.push(Vec::new());
                            batches.len() - 1
                        });
                        batches[at].push((i, t));
                    } else {
                        outs.insert(i, ex.run(&m.plan, &t));
                    }
                }
            }
        }
        for batch in batches {
            let plan = &items[batch[0].0].plans().next().expect("has a plan").plan;
            let together = if batch.len() > 1 { stacked(batch.iter().map(|(_, t)| t)).and_then(|all| ex.run(plan, &all).ok()).filter(|out| out.height == batch.len()) } else { None };
            match together {
                Some(out) => {
                    for (k, (i, _)) in batch.iter().enumerate() {
                        outs.insert(*i, Ok(row_of(&out, k)));
                    }
                }
                None => {
                    for (i, t) in &batch {
                        outs.insert(*i, ex.run(plan, t));
                    }
                }
            }
        }
        for &i in &ready {
            let item = &items[i];
            let (region, name) = (names[item.key.0].clone(), item.key.1.clone());
            let is_single = world.single[item.key.0];
            let whole = if is_single { format!("{region}: {name}") } else { format!("{region}[{name}]") };
            // What the column held: for the rows that keep their value.
            let old: Option<Data> = if item.exceptions.is_empty() { None } else { world.frames[item.key.0].column(&name).map(|c| c.data.clone()) };
            let mut main_done = false;
            if let Some(mapped) = &item.mapped {
                let outcome = outs.remove(&i).unwrap_or_else(|| Err("not calculated".into())).and_then(|out| {
                    let m = mapped.as_ref().map_err(Clone::clone)?;
                    let data = result_data(&out, m.is_bool, item.dtype.as_deref().filter(|_| !is_single), world.date1904)?;
                    if is_single {
                        world.fresh.insert(item.key.clone(), value_at(&data, 0, world.date1904));
                    }
                    // A cell the workbook shows as a date is one in the data.
                    let data = if is_single { result_data(&out, m.is_bool, item.dtype.as_deref(), world.date1904)? } else { data };
                    let frame = &mut world.frames[item.key.0];
                    if data.len() != frame.height {
                        return Err(format!("the plan gave {} values for {} rows", data.len(), frame.height));
                    }
                    match frame.columns.iter_mut().find(|c| c.name == name) {
                        Some(c) => c.data = data,
                        None => frame.columns.push(Column::new(name.clone(), data)),
                    }
                    Ok(())
                });
                match outcome {
                    Ok(()) => {
                        main_done = true;
                        if let Ok(m) = mapped {
                            if let Some(what) = kept.read_by(m) {
                                report.from_kept.push((region.clone(), name.clone(), what.clone()));
                                kept.whole.insert(item.key.clone(), what);
                            }
                        }
                        report.done.push((region.clone(), name.clone()));
                    }
                    Err(why) => {
                        kept.whole.insert(item.key.clone(), whole.clone());
                        report.kept.push((region.clone(), name.clone(), item.formula.clone(), why));
                    }
                }
            }
            if item.exceptions.is_empty() {
                continue;
            }
            // The exceptions of the column, into the column as it is now: a plain value
            // is the one the row had; a formula of its own is worked out, first those that
            // do not read the column itself (together), then the others top to bottom.
            let height = world.frames[item.key.0].height;
            if let Some(rows) = item.rows.filter(|rows| *rows < height) {
                for row in rows..height {
                    kept.row(&item.key, row, format!("{whole}, row {}", row + 1));
                }
                report.kept.push((region.clone(), format!("{name} (rows {} to {height})", rows + 1), item.formula.clone(), "the column is worked out row by row, and these rows are new".into()));
            }
            let into = |world: &mut World, row: usize, from: &Data, j: usize| {
                if let Some(c) = world.frames[item.key.0].columns.iter_mut().find(|c| c.name == name) {
                    put(&mut c.data, row, from, j);
                }
            };
            if let (true, Some(old)) = (main_done, &old) {
                for e in item.exceptions.iter().filter(|e| e.mapped.is_none() && e.row < height) {
                    into(&mut world, e.row, old, e.row);
                }
            }
            let reads_itself = |e: &Exc| matches!(&e.mapped, Some(Ok(m)) if m.deps.contains(&item.key));
            let calculated: Vec<&Exc> = item.exceptions.iter().filter(|e| e.mapped.is_some() && e.row < height).collect();
            let (after, free): (Vec<&Exc>, Vec<&Exc>) = calculated.into_iter().partition(|e| reads_itself(e));
            let mut results: Vec<(&Exc, Result<Table, String>)> = Vec::new();
            let mut jobs: Vec<(&Arc<Plan>, Table)> = Vec::new();
            let mut job_of: Vec<&Exc> = Vec::new();
            for e in free {
                match e.mapped.as_ref().expect("has a formula").as_ref().map_err(Clone::clone).and_then(|m| world.table_of(1, m).map(|t| (&m.plan, t))) {
                    Ok(job) => {
                        jobs.push(job);
                        job_of.push(e);
                    }
                    Err(why) => results.push((e, Err(why))),
                }
            }
            results.extend(job_of.into_iter().zip(run_each(ex, &jobs, &mut rowwise)));
            results.sort_by_key(|(e, _)| e.row);
            let mut pending = after.into_iter();
            loop {
                for (e, out) in results.drain(..) {
                    let at = format!("{name} (row {})", e.row + 1);
                    let m = e.mapped.as_ref().and_then(|m| m.as_ref().ok());
                    let outcome = out.and_then(|out| result_data(&out, m.is_some_and(|m| m.is_bool), e.dtype.as_deref().or(item.dtype.as_deref()), world.date1904));
                    match outcome {
                        Ok(one) => {
                            into(&mut world, e.row, &one, 0);
                            if let Some(what) = m.and_then(|m| kept.read_by(m)) {
                                report.from_kept.push((region.clone(), at.clone(), what.clone()));
                                kept.row(&item.key, e.row, what);
                            }
                            report.done.push((region.clone(), at));
                        }
                        Err(why) => {
                            // The row keeps the value it had.
                            if let (true, Some(old)) = (main_done, &old) {
                                into(&mut world, e.row, old, e.row);
                            }
                            kept.row(&item.key, e.row, format!("{whole}, row {}", e.row + 1));
                            report.kept.push((region.clone(), at, e.formula.clone(), why));
                        }
                    }
                }
                let Some(e) = pending.next() else { break };
                let out = e.mapped.as_ref().expect("has a formula").as_ref().map_err(Clone::clone).and_then(|m| world.table_of(1, m).and_then(|t| ex.run(&m.plan, &t)));
                results.push((e, out));
            }
            // Rows of another kind made the column text: where they are all booleans now
            // (the column held numbers for them before), it is a column of booleans.
            if let Some(c) = world.frames[item.key.0].columns.iter_mut().find(|c| c.name == name) {
                if let (Data::String(v), false) = (&c.data, matches!(old, Some(Data::String(_)))) {
                    if v.iter().flatten().all(|t| t == "true" || t == "false") && v.iter().any(Option::is_some) {
                        c.data = Data::Bool(v.iter().map(|t| t.as_deref().map(|t| t == "true")).collect());
                    }
                }
            }
        }
        let mut next: Vec<usize> = Vec::new();
        for &i in &ready {
            for &r in &readers[i] {
                waiting[r] -= 1;
                if waiting[r] == 0 {
                    next.push(r);
                }
            }
        }
        next.sort_unstable();
        ready = next;
    }
    for (_, item) in items.iter().enumerate().filter(|(i, _)| waiting[*i] > 0) {
        report.kept.push((names[item.key.0].clone(), item.key.1.clone(), item.formula.clone(), "on a circular reference".into()));
    }
    Ok(report)
}

/// Whether a plan works out every row from that row alone, so that rows of different
/// cells can go through it together: no function over the whole column, no list of values.
fn row_by_row(plan: &Plan) -> bool {
    use ir::F::*;
    let mut seen = HashSet::new();
    let mut stack: Vec<&ir::Expr> = plan.steps.iter().flatten().map(|(_, e)| e).chain(plan.outputs.iter().map(|(_, e)| e)).collect();
    while let Some(e) = stack.pop() {
        if !seen.insert(e.id()) {
            continue;
        }
        match e.node() {
            ir::Node::Literal(ir::Lit::Series(_)) => return false,
            ir::Node::Column(_) | ir::Node::Literal(_) => {}
            ir::Node::Binary(a, _, b) => stack.extend([a, b]),
            ir::Node::Ternary(c, a, b) => stack.extend([c, a, b]),
            ir::Node::Cast(a, _) => stack.push(a),
            ir::Node::Function(f, args) => {
                let each_row = matches!(
                    f,
                    Not | IsNull | IsNotNull | IsNan | IsFinite | FillNull | Abs | Floor | Ceil | Sqrt | Exp | Log | Pow | StrToLowercase | StrToUppercase | StrLenChars | StrHead | StrTail | StrSlice | StrContains(_) | StrReplaceAll(_) | StrStripChars | StrSplit | StrFindLiteral | StrExtract(_) | ListFirst | MinHorizontal | MaxHorizontal
                );
                if !each_row {
                    return false;
                }
                stack.extend(args);
            }
        }
    }
    true
}

/// The columns of a table and their kinds: tables with the same can be stacked.
fn signature(t: &Table) -> String {
    t.columns
        .iter()
        .map(|(n, v)| {
            let kind = match v {
                ir::Values::F64(_) => 'f',
                ir::Values::OptF64(_) => 'F',
                ir::Values::I64(_) => 'i',
                ir::Values::Str(_) => 's',
                ir::Values::OptStr(_) => 'S',
                ir::Values::OptBool(_) => 'b',
            };
            format!("{n}\u{1}{kind}\u{1}")
        })
        .collect()
}

/// Tables of the same columns, one below the other.
fn stacked<'a>(mut tables: impl Iterator<Item = &'a Table>) -> Option<Table> {
    use ir::Values::*;
    let mut all = tables.next()?.clone();
    for t in tables {
        all.height += t.height;
        for ((_, into), (_, from)) in all.columns.iter_mut().zip(&t.columns) {
            match (into, from) {
                (F64(a), F64(b)) => a.extend(b),
                (OptF64(a), OptF64(b)) => a.extend(b),
                (I64(a), I64(b)) => a.extend(b),
                (Str(a), Str(b)) => a.extend(b.iter().cloned()),
                (OptStr(a), OptStr(b)) => a.extend(b.iter().cloned()),
                (OptBool(a), OptBool(b)) => a.extend(b),
                _ => return None,
            }
        }
    }
    Some(all)
}

/// One row of a table as a table.
fn row_of(t: &Table, k: usize) -> Table {
    use ir::Values::*;
    let columns = t
        .columns
        .iter()
        .map(|(n, v)| {
            let one = match v {
                F64(a) => F64(vec![a[k]]),
                OptF64(a) => OptF64(vec![a[k]]),
                I64(a) => I64(vec![a[k]]),
                Str(a) => Str(vec![a[k].clone()]),
                OptStr(a) => OptStr(vec![a[k].clone()]),
                OptBool(a) => OptBool(vec![a[k]]),
            };
            (n.clone(), one)
        })
        .collect();
    Table { height: 1, columns }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_as_python_writes_them() {
        assert_eq!(number_text(3.0), "3");
        assert_eq!(number_text(-0.5), "-0.5");
        assert_eq!(number_text(1.5e-7), "1.5e-07");
        assert_eq!(number_text(1.25e20), "125000000000000000000");
        assert_eq!(number_text(1.5e16 + 0.0), "15000000000000000");
        assert_eq!(number_text(0.0001), "0.0001");
    }

    #[test]
    fn a_range_is_summed_up_by_its_numbers() {
        let column = Data::Float64(vec![Some(1.5), None, Some(2.5)]);
        let text = Data::String(vec![Some("7".into())]);
        let (a, b, c) = (V::Num(4.0), V::Bool(true), V::Text("x".into()));
        let s = summary([Part::Column(&column), Part::Column(&text), Part::Value(&a), Part::Value(&b), Part::Value(&c)].into_iter());
        assert_eq!(s, [Some(8.0), Some(3.0), Some(1.5), Some(4.0), Some(1.5 * 1.5 + 2.5 * 2.5 + 16.0)]);
        assert_eq!(summary(std::iter::empty()), [Some(0.0), Some(0.0), None, None, Some(0.0)]);
    }

    #[test]
    fn serial_numbers_become_dates_where_the_workbook_shows_them() {
        // 2026-01-31 is day 46053 of the 1900 system and day 20484 since 1970.
        assert_eq!(typed(&[Some(46_053.0), None], "Date", false), Data::Date(vec![Some(20_484), None]));
        assert_eq!(typed(&[Some(46_053.5)], "Date", false), Data::Datetime(vec![Some(20_484 * 86_400_000 + 43_200_000)]));
        assert_eq!(typed(&[Some(0.5)], "Time", false), Data::Time(vec![Some(43_200_000_000_000)]));
        assert_eq!(value_at(&Data::Date(vec![Some(20_484)]), 0, false), V::Num(46_053.0));
        // Before March 1900 a serial is one ahead of the days.
        assert_eq!(value_at(&typed(&[Some(59.0)], "Date", false), 0, false), V::Num(59.0));
    }
}
