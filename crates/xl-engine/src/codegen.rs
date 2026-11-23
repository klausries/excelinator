//! A workbook as a Polars program in Python: its regions as dataframes and its formulas
//! as Polars expressions, in the order of their dependence, with a check of every
//! result against the workbook at the end.
//!
//! Two ways to get a calculated column or cell:
//!
//! * **polars**: the formula written with Polars' own operations (`pl.col("qty") *
//!   pl.col("price")`). Readable and free of Excel, but with Polars' behaviour where
//!   Excel's differs (text in arithmetic, error values, comparing numbers to 15 digits).
//!   Only what the translator knows is written this way; the program's check tells
//!   whether the values are those of the workbook.
//! * **engine**: the values as the formula engine of this crate calculates them from
//!   the program's inputs (`wb.evaluate`), exactly as Excel does.
//!
//! [`Accuracy`] says which to prefer; single columns and cells can be given to the
//! engine by name (what a caller does for those whose Polars form does not reproduce
//! the workbook).

use crate::ir::Executor;
use crate::lift::ROW;
use crate::plans::{ColumnSource, Input, Mapper, RangeSource, Scalar, V};
use crate::regions::{regions, Region, RegionKind};
use std::collections::{HashMap, HashSet};
use xl_cells::Workbook;
use xl_formula::{BinaryOp, Expr, RefKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accuracy {
    /// Everything calculated by the engine.
    Exact,
    /// Polars expressions wherever the translator has one.
    Plain,
}

/// How one calculated column or cell is written in the program.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub region: String,
    /// The column or cell.
    pub name: String,
    pub formula: String,
    /// `polars` (written with Polars' own operations), `plan` (the engine's translation,
    /// written out) or `engine` (its values asked for).
    pub how: &'static str,
    /// Why the engine: no translation (and what stood in the way), or asked for.
    pub reason: Option<String>,
    /// The line of the program (1-based) where its statements begin.
    pub line: usize,
    /// The statement that takes the value from the engine instead (`exact[...]`).
    pub fallback: String,
    /// A Python expression for the value as a frame, to verify against the region.
    pub check: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub source: String,
    pub items: Vec<Item>,
    /// The line (1-based) where the check at the end begins: the last item ends before it.
    pub check_line: usize,
    /// The statement that asks the engine for its values (`exact = wb.evaluate(...)`),
    /// whether the program contains it or not.
    pub exact_statement: String,
}

/// A value while translating: what kind of Python expression the text is.
#[derive(Debug, Clone)]
enum Val {
    /// A Polars expression over the rows of the table at hand.
    Row(String),
    /// A Python value.
    Scalar(String),
    /// Whole columns of a table (region index, column names).
    Cols(usize, Vec<String>),
    /// The single values of a range of cells, as Python values.
    List(Vec<String>),
}

/// The text of a string constant of a formula (given with its quotes, `""` for a quote).
fn unquoted(s: &str) -> String {
    s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).map(|x| x.replace("\"\"", "\"")).unwrap_or_else(|| s.to_string())
}

fn py_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const KEYWORDS: [&str; 40] = [
    "false", "none", "true", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "pl", "xo", "wb", "regions",
    "exact",
];

struct Idents(HashSet<String>);

impl Idents {
    fn take(&mut self, want: &str) -> String {
        let mut base = String::new();
        for ch in want.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                base.push(ch.to_ascii_lowercase());
            } else if !base.ends_with('_') && !base.is_empty() {
                base.push('_');
            }
        }
        let mut base = base.trim_end_matches('_').to_string();
        if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
            base.insert(0, '_');
        }
        // Names the program itself uses.
        const USED: [&str; 20] = ["results", "problems", "math", "datetime", "sum", "min", "max", "abs", "len", "str", "round", "float", "int", "bool", "print", "list", "dict", "name", "frame", "isinstance"];
        if KEYWORDS.contains(&base.as_str()) || USED.contains(&base.as_str()) {
            base.push('_');
        }
        let mut name = base.clone();
        let mut n = 2;
        while !self.0.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

use crate::plans::Key;

struct Gen<'a> {
    wb: &'a Workbook,
    regions: &'a [Region],
    /// The Python variable of each region's frame.
    frames: Vec<String>,
    /// The Python variable of each named cell: (region, cell name) → variable.
    scalars: HashMap<Key, String>,
    cell_at: HashMap<(u32, u32, u32), Key>,
    /// Every cell of the workbook by place.
    at: HashMap<(u32, u32, u32), usize>,
    /// The last row (1-based) of each sheet that holds a cell.
    last_rows: Vec<u32>,
    calculated: HashSet<Key>,
    uses_round: bool,
    uses_numbers: bool,
    /// What translates formulas for plans; without it everything the Polars form lacks is the engine's.
    ex: Option<&'a dyn Executor>,
}

struct Ctx<'a> {
    /// The table whose rows the expression runs over, and the row the formula is of.
    table: Option<(usize, u32)>,
    sheet: u32,
    deps: &'a mut Vec<Key>,
    /// Statements to run before the expression.
    pre: &'a mut Vec<String>,
    temp: &'a str,
}

impl Gen<'_> {
    /// The value of a cell of the workbook as a Python literal (an empty cell or an
    /// error: None).
    fn literal(&self, sheet: u32, row: u32, col: u32) -> String {
        let Some(&i) = self.at.get(&(sheet, row, col)) else { return "None".into() };
        let c = &self.wb.cells;
        match c.kind[i] {
            xl_cells::Kind::Number if c.num[i].fract() == 0.0 && c.num[i].abs() < 1e15 => format!("{}", c.num[i] as i64),
            xl_cells::Kind::Number => format!("{:?}", c.num[i]),
            xl_cells::Kind::Bool => (if c.num[i] != 0.0 { "True" } else { "False" }).into(),
            xl_cells::Kind::Text => py_str(c.text[i].as_deref().unwrap_or("")),
            xl_cells::Kind::Blank | xl_cells::Kind::Error => "None".into(),
        }
    }

    fn sheet_of(&self, prefix: &Option<String>, home: u32) -> Result<u32, String> {
        match prefix {
            None => Ok(home),
            Some(p) => {
                let n = p.trim_end_matches('!').trim_matches('\'').replace("''", "'");
                self.wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(&n)).map(|i| i as u32).ok_or(format!("reference to {p} (another workbook or an unknown sheet)"))
            }
        }
    }

    fn dep(&self, key: Key, ctx: &mut Ctx) {
        if self.calculated.contains(&key) && !ctx.deps.contains(&key) {
            ctx.deps.push(key);
        }
    }

    fn cell(&self, s: u32, r: xl_formula::CellRef, text: &str, ctx: &mut Ctx) -> Result<Val, String> {
        let (row, col) = (r.row - 1, r.col - 1);
        if let Some((t, own_row)) = ctx.table {
            let table = &self.regions[t];
            if s == table.area.sheet && row == own_row && !r.abs_row {
                if let Some(c) = table.columns.iter().find(|c| c.col == col) {
                    self.dep((t, c.name.clone()), ctx);
                    return Ok(Val::Row(format!("pl.col({})", py_str(&c.name))));
                }
            }
        }
        if let Some(key) = self.cell_at.get(&(s, row, col)) {
            self.dep(key.clone(), ctx);
            return Ok(Val::Scalar(self.scalars[key].clone()));
        }
        // A label is its text; an empty cell counts as 0.
        let cells = &self.wb.cells;
        match self.at.get(&(s, row, col)).copied() {
            None => return Ok(Val::Scalar("0".into())),
            Some(i) if cells.formula[i].is_none() && cells.kind[i] == xl_cells::Kind::Blank => return Ok(Val::Scalar("0".into())),
            Some(i) if cells.formula[i].is_none() && cells.kind[i] == xl_cells::Kind::Text => return Ok(Val::Scalar(py_str(cells.text[i].as_deref().unwrap_or("")))),
            _ => {}
        }
        // A cell at a fixed place of a table or block: by its position in the frame.
        let fixed = ctx.table.is_none() || r.abs_row;
        for (t, region) in self.regions.iter().enumerate().filter(|(_, t)| t.kind != RegionKind::Cells && t.area.sheet == s) {
            let Some((first, last)) = region.data_rows() else { continue };
            if !(first..=last).contains(&row) {
                continue;
            }
            if let Some(c) = region.columns.iter().find(|c| c.col == col) {
                if !fixed || ctx.table.map(|x| x.0) == Some(t) {
                    return Err(format!("{text} is a cell of another row of a table"));
                }
                self.dep((t, c.name.clone()), ctx);
                return Ok(Val::Scalar(format!("{}.item({}, {})", self.frames[t], row - first, py_str(&c.name))));
            }
        }
        Err(format!("{text} is not a cell of a region"))
    }

    fn range(&self, s: u32, a: xl_formula::CellRef, z: xl_formula::CellRef, ctx: &mut Ctx) -> Result<Val, String> {
        for (t, table) in self.regions.iter().enumerate().filter(|(_, t)| t.kind == RegionKind::Table && t.area.sheet == s) {
            let Some((first, last)) = table.data_rows() else { continue };
            if a.row - 1 == first && z.row - 1 == last {
                let names: Vec<String> = table.columns.iter().filter(|c| (a.col - 1..=z.col - 1).contains(&c.col)).map(|c| c.name.clone()).collect();
                if names.len() == (z.col - a.col + 1) as usize {
                    for n in &names {
                        self.dep((t, n.clone()), ctx);
                    }
                    return Ok(Val::Cols(t, names));
                }
            }
        }
        // A range of single cells: their values, one by one (labels and empty cells left out).
        let count = (z.row - a.row + 1) as u64 * (z.col - a.col + 1) as u64;
        if a.row <= z.row && a.col <= z.col && count <= 400 {
            let mut list = Vec::new();
            for row in a.row - 1..z.row {
                for col in a.col - 1..z.col {
                    if let Some(key) = self.cell_at.get(&(s, row, col)) {
                        self.dep(key.clone(), ctx);
                        list.push(self.scalars[key].clone());
                    } else if self.at.get(&(s, row, col)).is_some_and(|&i| self.wb.cells.formula[i].is_some() || matches!(self.wb.cells.kind[i], xl_cells::Kind::Number | xl_cells::Kind::Bool | xl_cells::Kind::Error)) {
                        return Err("a range over cells of a table or block that is not its whole columns".into());
                    }
                }
            }
            return Ok(Val::List(list));
        }
        Err("a range that is not whole columns of a table".into())
    }

    /// A structured reference: `[@qty]`, `[@[unit price]]`, `Orders[amount]`, `Orders[[a]:[b]]`, `Orders`.
    fn structured(&self, table: &Option<String>, spec: &str, ctx: &mut Ctx) -> Result<Val, String> {
        let t = match table {
            Some(name) => self.regions.iter().position(|r| r.kind == RegionKind::Table && r.name.eq_ignore_ascii_case(name)).ok_or(format!("no table {name}"))?,
            None => ctx.table.map(|x| x.0).ok_or("a structured reference outside a table")?,
        };
        let unescape = |s: &str| s.replace("''", "\u{1}").replace('\'', "").replace('\u{1}', "'");
        let inner = spec.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(spec).trim();
        let this_row = inner.strip_prefix('@').map(str::trim).or_else(|| inner.strip_prefix("[#This Row],").map(str::trim));
        let strip = |s: &str| unescape(s.trim().strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(s.trim()));
        let column = |name: &str| self.regions[t].columns.iter().find(|c| c.name.eq_ignore_ascii_case(name)).map(|c| c.name.clone()).ok_or(format!("no column {name} in {}", self.regions[t].name));
        if let Some(name) = this_row {
            if ctx.table.map(|x| x.0) != Some(t) {
                return Err("this row of another table".into());
            }
            let name = column(&strip(name))?;
            self.dep((t, name.clone()), ctx);
            return Ok(Val::Row(format!("pl.col({})", py_str(&name))));
        }
        let names: Vec<String> = if inner.is_empty() || inner == "#Data" {
            self.regions[t].columns.iter().map(|c| c.name.clone()).collect()
        } else if inner.starts_with("[#") {
            return Err(format!("the structured reference {spec}"));
        } else if let Some((a, z)) = inner.split_once("]:[") {
            let (a, z) = (column(&strip(a.trim_start_matches('[')))?, column(&strip(z.trim_end_matches(']')))?);
            let cols = &self.regions[t].columns;
            let (i, j) = (cols.iter().position(|c| c.name == a).unwrap(), cols.iter().position(|c| c.name == z).unwrap());
            cols[i.min(j)..=i.max(j)].iter().map(|c| c.name.clone()).collect()
        } else {
            vec![column(&strip(inner))?]
        };
        for n in &names {
            self.dep((t, n.clone()), ctx);
        }
        Ok(Val::Cols(t, names))
    }

    /// A column as a Python expression: the Series of the table's frame.
    fn series(&self, v: &Val) -> Result<String, String> {
        match v {
            Val::Cols(t, names) if names.len() == 1 => Ok(format!("{}[{}]", self.frames[*t], py_str(&names[0]))),
            Val::Cols(..) => Err("several columns where one is expected".into()),
            _ => Err("a value where a column of a table is expected".into()),
        }
    }

    /// A value of one row or a single value: (Python text, is a Polars expression).
    fn operand(&self, v: Val) -> Result<(String, bool), String> {
        match v {
            Val::Row(s) => Ok((s, true)),
            Val::Scalar(s) => Ok((s, false)),
            Val::Cols(..) => Err("whole columns where a single value is expected".into()),
            Val::List(_) => Err("a range of cells where a single value is expected".into()),
        }
    }

    fn tr(&mut self, e: &Expr, ctx: &mut Ctx) -> Result<Val, String> {
        match e {
            Expr::Number(n) => Ok(Val::Scalar(n.parse::<f64>().map(|_| n.clone()).map_err(|_| format!("the number {n}"))?)),
            Expr::Str(s) => Ok(Val::Scalar(py_str(&unquoted(s)))),
            Expr::Bool(b) => Ok(Val::Scalar(if b.eq_ignore_ascii_case("true") { "True".into() } else { "False".into() })),
            Expr::Error(x) => Err(format!("the error value {x}")),
            Expr::Missing => Err("an empty argument".into()),
            Expr::Paren(x) | Expr::Spaced { expr: x, .. } => self.tr(x, ctx),
            Expr::Ref { prefix, kind: RefKind::Cell, text } => {
                let s = self.sheet_of(prefix, ctx.sheet)?;
                let r = xl_formula::parse_cell_ref(text).ok_or(format!("the reference {text}"))?;
                self.cell(s, r, text, ctx)
            }
            Expr::Ref { text, .. } => Err(format!("the whole rows or columns {text}")),
            Expr::Binary { op: BinaryOp::Range, lhs, rhs, .. } => match (lhs.unspaced(), rhs.unspaced()) {
                (Expr::Ref { prefix, kind: RefKind::Cell, text: a }, Expr::Ref { prefix: None, kind: RefKind::Cell, text: z }) => {
                    let s = self.sheet_of(prefix, ctx.sheet)?;
                    let (a, z) = (xl_formula::parse_cell_ref(a).ok_or("a range")?, xl_formula::parse_cell_ref(z).ok_or("a range")?);
                    self.range(s, a, z, ctx)
                }
                _ => Err("a range between other things than cells".into()),
            },
            Expr::Structured { table, spec } => self.structured(table, spec, ctx),
            Expr::Name { prefix: None, text } => {
                let d = self.wb.defined_names.iter().find(|d| d.name.eq_ignore_ascii_case(text)).ok_or(format!("the name {text}"))?;
                let target = xl_formula::parse(d.value.trim_start_matches('=')).map_err(|_| format!("the name {text}"))?;
                match target.unspaced() {
                    Expr::Ref { .. } | Expr::Binary { op: BinaryOp::Range, .. } => self.tr(&target, ctx),
                    _ => Err(format!("the name {text} (not a reference)")),
                }
            }
            Expr::Name { text, .. } => Err(format!("the name {text} of another workbook")),
            Expr::Unary { op, expr } => {
                let (x, row) = { let v = self.tr(expr, ctx)?; self.operand(v)? };
                let text = if *op == '-' { format!("(-{x})") } else { x };
                Ok(if row { Val::Row(text) } else { Val::Scalar(text) })
            }
            Expr::Postfix { expr, .. } => {
                let (x, row) = { let v = self.tr(expr, ctx)?; self.operand(v)? };
                let text = format!("({x} / 100)");
                Ok(if row { Val::Row(text) } else { Val::Scalar(text) })
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let (a, ra) = { let v = self.tr(lhs, ctx)?; self.operand(v)? };
                let (b, rb) = { let v = self.tr(rhs, ctx)?; self.operand(v)? };
                let row = ra || rb;
                let sign = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Pow => "**",
                    BinaryOp::Eq => "==",
                    BinaryOp::Ne => "!=",
                    BinaryOp::Lt => "<",
                    BinaryOp::Gt => ">",
                    BinaryOp::Le => "<=",
                    BinaryOp::Ge => ">=",
                    BinaryOp::Concat => {
                        let part = |x: &str, is_row: bool| if is_row { format!("{x}.cast(pl.String)") } else { format!("pl.lit({x}).cast(pl.String)") };
                        return Ok(if row { Val::Row(format!("pl.concat_str([{}, {}])", part(&a, ra), part(&b, rb))) } else { Val::Scalar(format!("(str({a}) + str({b}))")) });
                    }
                    BinaryOp::Range | BinaryOp::Intersect | BinaryOp::Union => return Err("an intersection or union of ranges".into()),
                };
                // A Python value on the left of a Polars expression: Polars takes it from the right.
                let text = if !ra && rb && matches!(op, BinaryOp::Pow) { format!("(pl.lit({a}) {sign} {b})") } else { format!("({a} {sign} {b})") };
                Ok(if row { Val::Row(text) } else { Val::Scalar(text) })
            }
            Expr::Call { name, args } => self.call(&name.to_ascii_uppercase(), args, ctx),
            Expr::Apply { .. } | Expr::Array(_) => Err("an array or a call of a result".into()),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], ctx: &mut Ctx) -> Result<Val, String> {
        let wrap = |text: String, row: bool| if row { Val::Row(text) } else { Val::Scalar(text) };
        let mut vals: Vec<Val> = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.tr(a, ctx)?);
        }
        let n = vals.len();
        let want = |ok: bool| if ok { Ok(()) } else { Err(format!("{name} with {n} arguments")) };
        let all_cols = n > 0 && vals.iter().all(|v| matches!(v, Val::Cols(..)));
        match name {
            // Aggregates over whole columns give one value.
            "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" | "COUNTA" if all_cols => {
                want(n == 1)?;
                let method = match name {
                    "SUM" => "sum",
                    "AVERAGE" => "mean",
                    "MIN" => "min",
                    "MAX" => "max",
                    _ => "count",
                };
                Ok(Val::Scalar(format!("{}.{method}()", self.series(&vals[0])?)))
            }
            // Aggregates over single values and ranges of single cells.
            "SUM" | "MIN" | "MAX" | "AVERAGE" | "COUNT" if vals.iter().any(|v| matches!(v, Val::List(_))) => {
                let mut all: Vec<String> = Vec::new();
                for v in vals {
                    match v {
                        Val::List(items) => all.extend(items),
                        Val::Scalar(x) => all.push(x),
                        _ => return Err(format!("{name} over a range of cells together with a value per row")),
                    }
                }
                // Of a range Excel takes the numbers only.
                self.uses_numbers = true;
                let list = format!("_numbers([{}])", all.join(", "));
                Ok(Val::Scalar(match name {
                    "SUM" => format!("sum({list})"),
                    "MIN" => format!("min({list}, default=0)"),
                    "MAX" => format!("max({list}, default=0)"),
                    "AVERAGE" => format!("(sum({list}) / len({list}))"),
                    _ => format!("len({list})"),
                }))
            }
            "SUM" | "MIN" | "MAX" => {
                want(n >= 1)?;
                let parts: Vec<(String, bool)> = vals.into_iter().map(|v| self.operand(v)).collect::<Result<_, _>>()?;
                let row = parts.iter().any(|p| p.1);
                let list = parts.iter().map(|p| p.0.clone()).collect::<Vec<_>>().join(", ");
                Ok(match (name, row) {
                    ("SUM", _) => wrap(format!("({})", parts.iter().map(|p| p.0.clone()).collect::<Vec<_>>().join(" + ")), row),
                    ("MIN", true) => Val::Row(format!("pl.min_horizontal({list})")),
                    ("MAX", true) => Val::Row(format!("pl.max_horizontal({list})")),
                    ("MIN", false) => Val::Scalar(format!("min({list})")),
                    (_, false) => Val::Scalar(format!("max({list})")),
                    _ => unreachable!(),
                })
            }
            "IF" => {
                want(n == 2 || n == 3)?;
                let mut it = vals.into_iter();
                let (c, rc) = self.operand(it.next().unwrap())?;
                let (a, ra) = self.operand(it.next().unwrap())?;
                let (b, rb) = match it.next() {
                    Some(v) => self.operand(v)?,
                    None => ("False".to_string(), false),
                };
                Ok(if rc || ra || rb {
                    let lit = |x: String, row: bool| if row { x } else { format!("pl.lit({x})") };
                    Val::Row(format!("pl.when({c}).then({}).otherwise({})", lit(a, ra), lit(b, rb)))
                } else {
                    Val::Scalar(format!("({a} if {c} else {b})"))
                })
            }
            "AND" | "OR" => {
                want(n >= 1)?;
                let parts: Vec<(String, bool)> = vals.into_iter().map(|v| self.operand(v)).collect::<Result<_, _>>()?;
                let row = parts.iter().any(|p| p.1);
                let sign = match (name, row) {
                    ("AND", true) => " & ",
                    ("AND", false) => " and ",
                    (_, true) => " | ",
                    (_, false) => " or ",
                };
                Ok(wrap(format!("({})", parts.iter().map(|p| if row && !p.1 { format!("pl.lit({})", p.0) } else { p.0.clone() }).collect::<Vec<_>>().join(sign)), row))
            }
            "NOT" => {
                want(n == 1)?;
                let (x, row) = self.operand(vals.remove(0))?;
                Ok(wrap(if row { format!("(~{x})") } else { format!("(not {x})") }, row))
            }
            "ABS" | "SQRT" | "INT" | "EXP" | "LN" | "LOG10" | "UPPER" | "LOWER" | "TRIM" | "LEN" | "ISBLANK" => {
                want(n == 1)?;
                let (x, row) = self.operand(vals.remove(0))?;
                let text = match (name, row) {
                    ("ABS", true) => format!("{x}.abs()"),
                    ("ABS", false) => format!("abs({x})"),
                    ("SQRT", true) => format!("{x}.sqrt()"),
                    ("SQRT", false) => format!("({x} ** 0.5)"),
                    ("INT", true) => format!("{x}.floor()"),
                    ("INT", false) => format!("({x} // 1)"),
                    ("EXP", true) => format!("{x}.exp()"),
                    ("LN", true) => format!("{x}.log()"),
                    ("LOG10", true) => format!("{x}.log10()"),
                    ("UPPER", true) => format!("{x}.str.to_uppercase()"),
                    ("UPPER", false) => format!("{x}.upper()"),
                    ("LOWER", true) => format!("{x}.str.to_lowercase()"),
                    ("LOWER", false) => format!("{x}.lower()"),
                    ("TRIM", true) => format!("{x}.str.strip_chars(\" \")"),
                    ("TRIM", false) => format!("{x}.strip(\" \")"),
                    ("LEN", true) => format!("{x}.str.len_chars()"),
                    ("LEN", false) => format!("len({x})"),
                    ("ISBLANK", true) => format!("{x}.is_null()"),
                    ("ISBLANK", false) => format!("({x} is None)"),
                    _ => return Err(format!("{name} of a single value")),
                };
                Ok(wrap(text, row))
            }
            "ROUND" => {
                want(n == 2)?;
                let digits = match args[1].unspaced() {
                    Expr::Number(d) => d.parse::<u32>().map_err(|_| "ROUND to places that are not a whole number from 0")?,
                    _ => return Err("ROUND to places that are not a fixed number".into()),
                };
                let (x, row) = self.operand(vals.remove(0))?;
                if !row {
                    self.uses_round = true;
                }
                Ok(wrap(if row { format!("{x}.round_sig_figs(15).round({digits}, mode=\"half_away_from_zero\")") } else { format!("_round({x}, {digits})") }, row))
            }
            "MOD" => {
                want(n == 2)?;
                let (b, rb) = self.operand(vals.remove(1))?;
                let (a, ra) = self.operand(vals.remove(0))?;
                Ok(wrap(format!("({a} % {b})"), ra || rb))
            }
            "LEFT" | "RIGHT" => {
                want(n == 1 || n == 2)?;
                let count = match vals.get(1) {
                    Some(Val::Scalar(c)) => c.clone(),
                    None => "1".into(),
                    _ => return Err(format!("{name} with a length that varies by row")),
                };
                let (x, row) = self.operand(vals.remove(0))?;
                Ok(wrap(
                    match (name, row) {
                        ("LEFT", true) => format!("{x}.str.head({count})"),
                        ("LEFT", false) => format!("{x}[:{count}]"),
                        (_, true) => format!("{x}.str.tail({count})"),
                        (_, false) => format!("{x}[-{count}:]"),
                    },
                    row,
                ))
            }
            "MID" => {
                want(n == 3)?;
                let (Val::Scalar(start), Val::Scalar(count)) = (vals[1].clone(), vals[2].clone()) else { return Err("MID with a start or length that varies by row".into()) };
                let (x, row) = self.operand(vals.remove(0))?;
                Ok(wrap(if row { format!("{x}.str.slice({start} - 1, {count})") } else { format!("{x}[{start} - 1:{start} - 1 + {count}]") }, row))
            }
            // A value looked up by its key in the columns of a table.
            "VLOOKUP" => {
                want(n == 4)?;
                if !matches!(args[3].unspaced(), Expr::Bool(b) if b.eq_ignore_ascii_case("false")) && !matches!(args[3].unspaced(), Expr::Number(z) if z == "0") {
                    return Err("VLOOKUP with an approximate match".into());
                }
                let Val::Cols(t, names) = vals[1].clone() else { return Err("VLOOKUP in something that is not columns of a table".into()) };
                let index = match args[2].unspaced() {
                    Expr::Number(k) => k.parse::<usize>().ok().filter(|k| (1..=names.len()).contains(k)).ok_or("VLOOKUP with a column number outside the table")?,
                    _ => return Err("VLOOKUP with a column number that is not fixed".into()),
                };
                self.lookup(vals.remove(0), t, &names[0], &names[index - 1])
            }
            "INDEX" => {
                want(n == 2)?;
                let (Val::Cols(t, values), Expr::Call { name: m, args: margs }) = (vals[0].clone(), args[1].unspaced()) else { return Err("INDEX without a MATCH as its row".into()) };
                if !m.eq_ignore_ascii_case("MATCH") || margs.len() != 3 || !matches!(margs[2].unspaced(), Expr::Number(z) if z == "0") || values.len() != 1 {
                    return Err("INDEX with something other than an exact MATCH over one column".into());
                }
                let key = self.tr(&margs[0], ctx)?;
                let Val::Cols(t2, keys) = self.tr(&margs[1], ctx)? else { return Err("MATCH in something that is not a column of a table".into()) };
                if t2 != t || keys.len() != 1 {
                    return Err("INDEX and MATCH over different tables".into());
                }
                self.lookup(key, t, &keys[0], &values[0])
            }
            "MATCH" => Err("MATCH outside INDEX".into()),
            "SUMIF" | "COUNTIF" | "AVERAGEIF" => {
                want(if name == "COUNTIF" { n == 2 } else { n == 2 || n == 3 })?;
                let Val::Cols(t, keys) = vals[0].clone() else { return Err(format!("{name} over something that is not a column of a table")) };
                let values = match vals.get(2) {
                    Some(Val::Cols(t2, v)) if *t2 == t && v.len() == 1 => v[0].clone(),
                    Some(_) => return Err(format!("{name} with values that are not a column of the same table")),
                    None => keys[0].clone(),
                };
                if keys.len() != 1 {
                    return Err(format!("{name} over several columns"));
                }
                if let Expr::Str(c) = args[1].unspaced() {
                    let c = unquoted(c);
                    if c.starts_with(['<', '>', '=']) || c.contains(['*', '?']) {
                        return Err(format!("{name} with a criterion that is a comparison or a pattern"));
                    }
                }
                let (crit, row) = self.operand(vals.remove(1))?;
                let frame = self.frames[t].clone();
                let (agg, method, empty) = match name {
                    "SUMIF" => (format!("pl.col({}).sum()", py_str(&values)), "sum()".to_string(), "0"),
                    "COUNTIF" => ("pl.len()".to_string(), "len()".to_string(), "0"),
                    _ => (format!("pl.col({}).mean()", py_str(&values)), "mean()".to_string(), "None"),
                };
                if row {
                    // Once per key, then looked up for each row.
                    let temp = format!("{}_by_{}", ctx.temp, Idents(HashSet::new()).take(&keys[0]));
                    ctx.pre.push(format!("{temp} = {frame}.group_by({}).agg({agg}.alias(\"_value\"))", py_str(&keys[0])));
                    Ok(Val::Row(format!("{crit}.replace_strict({temp}[{}], {temp}[\"_value\"], default={empty})", py_str(&keys[0]))))
                } else {
                    let picked = format!("{frame}.filter(pl.col({}) == {crit})", py_str(&keys[0]));
                    Ok(Val::Scalar(if name == "COUNTIF" { format!("{picked}.height") } else { format!("{picked}[{}].{method}", py_str(&values)) }))
                }
            }
            other => Err(format!("the function {other}")),
        }
    }

    fn lookup(&mut self, key: Val, t: usize, keys: &str, values: &str) -> Result<Val, String> {
        let (key, row) = self.operand(key)?;
        let frame = &self.frames[t];
        Ok(if row {
            Val::Row(format!("{key}.replace_strict({frame}[{}], {frame}[{}], default=None)", py_str(keys), py_str(values)))
        } else {
            Val::Scalar(format!("{frame}.filter(pl.col({}) == {key})[{}].first()", py_str(keys), py_str(values)))
        })
    }
}

/// A calculated column or cell written as the engine's plan: statements, and what it reads.
struct Planned {
    lines: Vec<String>,
    deps: Vec<Key>,
    /// What the translation took for granted about its inputs (their kinds), as conditions:
    /// if one does not hold, the engine's values are taken instead.
    guard: Vec<String>,
}

impl Gen<'_> {
    fn mapper(&self) -> Mapper<'_> {
        Mapper { wb: self.wb, regions: self.regions, at: &self.at, last_rows: &self.last_rows, cell_at: &self.cell_at, calculated: &self.calculated, ex: self.ex, mixed: false }
    }

    /// A cell's value as a Python value.
    fn scalar_code(&self, s: &Scalar) -> String {
        match s {
            Scalar::Named(key) => self.scalars[key].clone(),
            Scalar::TableCell { region, column, row, .. } => format!("{}.item({row}, {})", self.frames[*region], py_str(column)),
            Scalar::Sheet { sheet, row, col } => format!("_cell(wb, {}, {row}, {col})", py_str(&self.wb.sheets[*sheet as usize].name)),
            Scalar::Const(v) => match v {
                V::Null => "None".into(),
                V::Bool(b) => if *b { "True".into() } else { "False".into() },
                V::Num(x) => format!("{x:?}"),
                V::Text(t) => py_str(t),
            },
        }
    }

    /// The values of a fixed range as a Python list.
    fn range_code(&self, source: &RangeSource) -> String {
        match source {
            RangeSource::Columns(t, names) => format!("[{}]", names.iter().map(|c| format!("{}[{}]", self.frames[*t], py_str(c))).collect::<Vec<_>>().join(", ")),
            RangeSource::Sheet(sheet, r1, r2, c1, c2) => format!("_range(wb, {}, {r1}, {r2}, {c1}, {c2})", py_str(&self.wb.sheets[*sheet as usize].name)),
            RangeSource::Cells(list) => format!("[{}]", list.iter().map(|s| self.scalar_code(s)).collect::<Vec<_>>().join(", ")),
            RangeSource::Summary(_) => "[]".into(),
        }
    }

    /// The statements of the engine's plan for a calculated column or cell of a region.
    fn plan_step(&self, k: usize, name: &str) -> Result<Planned, String> {
        let m = self.mapper().map(k, name)?;
        let r = &self.regions[k];
        let n = (m.rows.1 - m.rows.0 + 1) as usize;
        let mut guard: Vec<String> = Vec::new();
        let mut before: Vec<String> = Vec::new();
        let mut columns: Vec<String> = Vec::new();
        for input in &m.inputs {
            match input {
                Input::Column { name, source } => {
                    let frame = |(t, c): &Key| format!("{}[{}]", self.frames[*t], py_str(c));
                    let code = match source {
                        ColumnSource::Numbers(c) => {
                            guard.push(format!("_numeric({})", frame(c)));
                            format!("{}.cast(pl.Float64)", frame(c))
                        }
                        ColumnSource::NoNumbers { column, text } => {
                            guard.push(if *text { format!("{}.dtype == pl.String", frame(column)) } else { format!("_numeric({})", frame(column)) });
                            format!("pl.Series(\"\", [None] * {n}, dtype=pl.Float64)")
                        }
                        ColumnSource::Text(c) => {
                            guard.push(format!("{}.dtype == pl.String", frame(c)));
                            frame(c)
                        }
                    };
                    columns.push(format!("    {code}.alias({}),", py_str(name)));
                }
                Input::Own { name, cell, text } => {
                    let v = self.scalar_code(cell);
                    let code = if *text { format!("pl.Series(\"\", [_text({v})], dtype=pl.String)") } else { format!("pl.Series(\"\", [_num({v})], dtype=pl.Float64)") };
                    columns.push(format!("    {code}.alias({}),", py_str(name)));
                }
                Input::Cell { name, cell, text } => {
                    let v = self.scalar_code(cell);
                    guard.push(if *text { format!("_is_text({v})") } else { format!("_num({v}) is not None") });
                    columns.push(if *text { format!("    pl.lit(_text({v}) or \"\", dtype=pl.String).alias({}),", py_str(name)) } else { format!("    pl.lit(_num({v}), dtype=pl.Float64).alias({}),", py_str(name)) });
                }
                Input::Range { index, source, parts } => {
                    before.push(format!("_a{index} = _summary({})", self.range_code(source)));
                    for (i, name) in parts {
                        columns.push(format!("    pl.lit(_a{index}[{i}], dtype=pl.Float64).alias({}),", py_str(name)));
                    }
                }
            }
        }
        let code = crate::pycode::plan_python(&m.plan, "_in", "_out", "_x")?;
        let mut lines = before;
        lines.push(format!("_in = pl.DataFrame({{{}: range({}, {})}}){}", py_str(ROW), m.rows.0 + 1, m.rows.1 + 2, if columns.is_empty() { String::new() } else { format!(".with_columns(\n{}\n)", columns.join("\n")) }));
        lines.extend(code);
        let is_bool = if m.is_bool { "True" } else { "False" };
        match r.kind {
            RegionKind::Cells => lines.push(format!("{} = _scalar(_out, {is_bool})", self.scalars[&(k, name.to_string())])),
            _ => {
                // A column the workbook shows as dates or times is one here, not serial numbers.
                let dtype = r.columns.iter().find(|c| c.name == name).map(|c| c.dtype.as_str()).filter(|d| matches!(*d, "Date" | "Datetime" | "Time" | "Duration"));
                let typed = match dtype {
                    Some(d) => format!(", {}{}", py_str(d), if self.wb.date1904 { ", True" } else { "" }),
                    None => String::new(),
                };
                lines.push(format!("{0} = {0}.with_columns(_result(_out, {is_bool}{typed}).alias({1}))", self.frames[k], py_str(name)));
            }
        }
        guard.sort();
        guard.dedup();
        Ok(Planned { lines, deps: m.deps, guard })
    }
}

/// One statement block of the program for a calculated column or cell.
struct Step {
    key: Key,
    formula: String,
    logic: Option<String>,
    /// The Polars form: statements before, and the expression.
    polars: Result<(Vec<String>, Val), String>,
    /// The engine's plan, written out: where the Polars form is missing; or why it cannot be.
    plan: Option<Result<Planned, String>>,
    deps: Vec<Key>,
}

/// The program for a workbook. `file` is the path the program loads the workbook from;
/// `engine` names columns and cells (region, name) to be left to the engine. With `ex`,
/// what has no Polars form is written as the engine's own translation (`plan`) where
/// that can be mapped on the program's data; without it, or where it cannot, the program
/// asks the engine for the values.
pub fn polars_program(wb: &Workbook, file: &str, accuracy: Accuracy, engine: &[(String, String)], ex: Option<&dyn Executor>) -> Program {
    polars_program_with(wb, file, accuracy, engine, &[], ex)
}

/// As [`polars_program`]; `plan_only` names columns and cells (region, name) whose
/// Polars form is not to be written (it did not give the workbook's values): the
/// engine's translation where there is one, else its values.
pub fn polars_program_with(wb: &Workbook, file: &str, accuracy: Accuracy, engine: &[(String, String)], plan_only: &[(String, String)], ex: Option<&dyn Executor>) -> Program {
    let found = regions(wb);
    let mut idents = Idents(HashSet::new());
    let frames: Vec<String> = found.iter().map(|r| idents.take(&r.name)).collect();
    let at = (0..wb.cells.len()).map(|i| ((wb.cells.sheet[i], wb.cells.row[i], wb.cells.col[i]), i)).collect();
    let mut gen = Gen { wb, regions: &found, frames, scalars: HashMap::new(), cell_at: HashMap::new(), at, last_rows: crate::lift::last_rows_of(wb), calculated: HashSet::new(), uses_round: false, uses_numbers: false, ex };
    for (k, r) in found.iter().enumerate() {
        for cell in &r.cells {
            let key = (k, cell.name.clone());
            gen.scalars.insert(key.clone(), idents.take(&cell.name));
            gen.cell_at.insert((r.area.sheet, cell.row, cell.col), key.clone());
            if cell.formula.is_some() {
                gen.calculated.insert(key);
            }
        }
        for c in r.columns.iter().filter(|c| c.is_calculated()) {
            gen.calculated.insert((k, c.name.clone()));
        }
    }
    // Translate everything; the order comes from what reads what.
    let mut steps: Vec<Step> = Vec::new();
    for (k, r) in found.iter().enumerate() {
        let first = r.data_rows().map(|x| x.0);
        // (name, formula, logic, the table and first row of a column, rows that are
        // exceptions with a formula of their own, rows that hold a plain value instead of
        // the column's formula: (row of the sheet, the value as Python))
        type Entry = (String, String, Option<String>, Option<(usize, u32)>, usize, Vec<(u32, String)>);
        let entries: Vec<Entry> = match r.kind {
            RegionKind::Cells => r.cells.iter().filter_map(|c| c.formula.clone().map(|f| (c.name.clone(), f, c.logic.clone(), None, 0, Vec::new()))).collect(),
            _ => r
                .columns
                .iter()
                .filter(|c| c.is_calculated())
                .map(|c| {
                    let formula = c.formula.clone().unwrap_or_else(|| format!("(values; {} rows have a formula of their own)", c.exceptions.len()));
                    // Rows with a plain value are written as that value where the column has a formula.
                    let (values, own) = if c.formula.is_some() {
                        let values: Vec<(u32, String)> = c.exceptions.iter().filter(|e| e.formula.is_none()).map(|e| (e.row, gen.literal(r.area.sheet, e.row, c.col))).collect();
                        let own = c.exceptions.len() - values.len();
                        (values, own)
                    } else {
                        (Vec::new(), c.exceptions.len())
                    };
                    (c.name.clone(), formula, c.logic.clone(), first.map(|row| (k, row)), own, values)
                })
                .collect(),
        };
        for (name, formula, logic, table, exceptions, values) in entries {
            let mut deps = Vec::new();
            let mut pre = Vec::new();
            let temp = format!("_{}", Idents(HashSet::new()).take(&name));
            let asked = engine.iter().any(|(reg, n)| reg == &r.name && n == &name);
            let polars = if accuracy == Accuracy::Exact {
                Err("exact values asked for".to_string())
            } else if asked {
                Err("left to the engine".to_string())
            } else if plan_only.iter().any(|(reg, n)| reg == &r.name && n == &name) {
                Err("its Polars form did not give the workbook's values".to_string())
            } else if exceptions > 0 {
                Err(format!("{exceptions} rows of the column are exceptions (a formula or value of their own)"))
            } else {
                match xl_formula::parse(&formula) {
                    Ok(e) => {
                        let mut ctx = Ctx { table, sheet: r.area.sheet, deps: &mut deps, pre: &mut pre, temp: &temp };
                        gen.tr(&e, &mut ctx).and_then(|v| match (&v, table) {
                            (Val::Cols(..), _) => Err("whole columns as a result".to_string()),
                            (Val::List(_), _) => Err("a range of cells as a result".to_string()),
                            (Val::Row(_), None) => Err("a value per row in a single cell".to_string()),
                            _ => Ok(v),
                        })
                    }
                    Err(_) => Err("the formula is not understood".to_string()),
                }
            };
            // The rows with a plain value: that value, the column's formula in the others.
            let polars = match (polars, table) {
                // Only a formula translated row by row: one that comes out as one value
                // (a reference to the row above taken as a fixed cell) is not that column.
                (Ok(Val::Scalar(_) | Val::Cols(..) | Val::List(_)), Some(_)) if !values.is_empty() => Err(format!("{} rows hold a value of their own, and the column's formula is not one per row", values.len())),
                (Ok(Val::Row(base)), Some((_, first))) if !values.is_empty() => {
                    let index = "pl.int_range(pl.len())";
                    let cases: String = values.iter().enumerate().map(|(n, (row, value))| format!("{}({index} == {}).then(pl.lit({value}))", if n == 0 { "pl.when" } else { ".when" }, row - first)).collect();
                    Ok(Val::Row(format!("{cases}.otherwise({base})")))
                }
                (other, _) => other,
            };
            let polars = polars.map(|v| (pre, v));
            let mut plan = None;
            if polars.is_err() && !asked && exceptions == 0 && values.is_empty() && gen.ex.is_some() {
                let p = gen.plan_step(k, &name);
                if let Ok(p) = &p {
                    deps = p.deps.clone();
                }
                plan = Some(p);
            }
            steps.push(Step { key: (k, name), formula, logic, polars, plan, deps });
        }
    }
    // Dependence order (what cannot be ordered, a circle, keeps its place at the end).
    let mut done: HashSet<Key> = HashSet::new();
    let mut order: Vec<usize> = Vec::new();
    let mut left: Vec<usize> = (0..steps.len()).collect();
    while !left.is_empty() {
        let ready: Vec<usize> = left.iter().copied().filter(|&i| steps[i].deps.iter().all(|d| done.contains(d) || *d == steps[i].key)).collect();
        if ready.is_empty() {
            for &i in &left {
                steps[i].polars = Err("on a circular reference".to_string());
                steps[i].plan = Some(Err("on a circular reference".to_string()));
            }
            order.extend(left.drain(..));
            break;
        }
        for i in ready {
            done.insert(steps[i].key.clone());
            order.push(i);
            left.retain(|&x| x != i);
        }
    }
    let uses_engine = steps.iter().any(|s| s.polars.is_err() && !matches!(&s.plan, Some(Ok(p)) if p.guard.is_empty()));
    let uses_plan = steps.iter().any(|s| s.polars.is_err() && matches!(s.plan, Some(Ok(_))));
    let mut src = String::new();
    let mut items = Vec::new();
    src.push_str(&format!("\"\"\"{} as a Polars program (written by xlcells).\n\nThe regions of the workbook are loaded as dataframes, every calculated column and cell\nis worked out again, and the results are compared with the workbook at the end.\n\"\"\"\n", file.replace('"', "'")));
    src.push_str("import polars as pl\n\nimport xlcells.openpyxl as xo\n\n");
    let mut uses_value = false;
    let mut body = String::new();
    body.push_str(&format!("wb = xo.load_workbook({})\nregions = wb.regions\n\n# --- data: what is not calculated ---\n", py_str(file)));
    for (k, r) in found.iter().enumerate() {
        let var = &gen.frames[k];
        let sheet = &wb.sheets[r.area.sheet as usize].name;
        let calculated: Vec<String> = r.columns.iter().filter(|c| c.formula.is_some()).map(|c| py_str(&c.name)).collect();
        // strict=False: data without the calculated columns (a ported project's inputs) is fine too.
        let drop = if calculated.is_empty() { String::new() } else { format!(".drop({}, strict=False)", calculated.join(", ")) };
        body.push_str(&format!("{var} = regions[{}].to_polars(){drop}  # {} {sheet}!{}\n", py_str(&r.name), r.kind.as_str(), r.range()));
        for cell in r.cells.iter().filter(|c| c.formula.is_none()) {
            body.push_str(&format!("{} = _value({var}[{}][0])\n", gen.scalars[&(k, cell.name.clone())], py_str(&cell.name)));
            uses_value = true;
        }
    }
    let inputs: Vec<String> = found.iter().enumerate().map(|(k, r)| format!("{}: {}", py_str(&r.name), gen.frames[k])).collect();
    let exact_statement = format!("exact = wb.evaluate({{{}}})", inputs.join(", "));
    if uses_engine {
        body.push_str(&format!("\n# The formula engine's values for these inputs: exactly Excel's.\n{exact_statement}\n"));
    }
    body.push_str("\n# --- logic: calculated columns and cells, in the order of their dependence ---\n");
    for &i in &order {
        let step = &steps[i];
        let (k, name) = &step.key;
        let r = &found[*k];
        let var = &gen.frames[*k];
        let is_cell = r.kind == RegionKind::Cells;
        let title = if is_cell { format!("{}: {}", r.name, name) } else { format!("{}[{}]", r.name, name) };
        // A name with a line break would end the comment.
        let title = title.replace(['\n', '\r'], " ");
        let line = body.lines().count() + 1;
        let (fallback, check) = if is_cell {
            let scalar = &gen.scalars[&step.key];
            (format!("{scalar} = _value(exact[{}][{}][0])", py_str(&r.name), py_str(name)), format!("pl.DataFrame({{{}: [{scalar}]}}, strict=False)", py_str(name)))
        } else {
            (format!("{var} = {var}.with_columns(exact[{}][{}])", py_str(&r.name), py_str(name)), format!("{var}.select({})", py_str(name)))
        };
        body.push_str(&format!("# {title}  ={}\n", step.logic.as_deref().unwrap_or(&step.formula).replace(['\n', '\r'], " ")));
        match &step.polars {
            Ok((pre, value)) => {
                for line in pre {
                    body.push_str(line);
                    body.push('\n');
                }
                let strip = |s: &str| if s.starts_with('(') && s.ends_with(')') && balanced(&s[1..s.len() - 1]) { s[1..s.len() - 1].to_string() } else { s.to_string() };
                match (value, is_cell) {
                    (Val::Scalar(s), true) => body.push_str(&format!("{} = {}\n", gen.scalars[&step.key], strip(s))),
                    (Val::Scalar(s), false) => body.push_str(&format!("{var} = {var}.with_columns(pl.lit({}).alias({}))\n", strip(s), py_str(name))),
                    (Val::Row(s), _) => body.push_str(&format!("{var} = {var}.with_columns({}.alias({}))\n", wrap_for_alias(&strip(s)), py_str(name))),
                    (Val::Cols(..) | Val::List(_), _) => unreachable!(),
                }
                items.push(Item { region: r.name.clone(), name: name.clone(), formula: step.formula.clone(), how: "polars", reason: None, line, fallback, check });
            }
            Err(why) if matches!(step.plan, Some(Ok(_))) => {
                body.push_str(&format!("#   by the engine's translation: {why}\n"));
                if let Some(Ok(p)) = &step.plan {
                    // What the translation took for granted is checked: if it does not hold
                    // any more, the engine's values are taken.
                    let indent = if p.guard.is_empty() { "" } else { "    " };
                    if !p.guard.is_empty() {
                        body.push_str(&format!("if {}:  # what the translation was made for\n", p.guard.join(" and ")));
                    }
                    for line in &p.lines {
                        for part in line.split('\n') {
                            body.push_str(indent);
                            body.push_str(part);
                            body.push('\n');
                        }
                    }
                    if !p.guard.is_empty() {
                        body.push_str(&format!("else:\n    {fallback}\n"));
                    }
                }
                items.push(Item { region: r.name.clone(), name: name.clone(), formula: step.formula.clone(), how: "plan", reason: Some(why.clone()), line, fallback, check });
            }
            Err(why) => {
                let why = match &step.plan {
                    Some(Err(no_plan)) => format!("{why}; no translation by the engine to write out: {no_plan}"),
                    _ => why.clone(),
                };
                body.push_str(&format!("#   by the engine: {why}\n"));
                body.push_str(&fallback);
                body.push('\n');
                uses_value |= is_cell;
                items.push(Item { region: r.name.clone(), name: name.clone(), formula: step.formula.clone(), how: "engine", reason: Some(why.clone()), line, fallback, check });
            }
        }
    }
    let check_line = body.lines().count() + 2;
    body.push_str("\n# --- check: every value against the workbook ---\nresults = {\n");
    for (k, r) in found.iter().enumerate() {
        if r.kind == RegionKind::Cells {
            let fields: Vec<String> = r.cells.iter().map(|c| format!("{}: [{}]", py_str(&c.name), gen.scalars[&(k, c.name.clone())])).collect();
            body.push_str(&format!("    {}: pl.DataFrame({{{}}}, strict=False),\n", py_str(&r.name), fields.join(", ")));
        } else {
            body.push_str(&format!("    {}: {},\n", py_str(&r.name), gen.frames[k]));
        }
    }
    body.push_str("}\nproblems = pl.concat([regions[name].verify(frame) for name, frame in results.items()])\nprint(problems if problems.height else \"every value as in the workbook\")\n");
    if gen.uses_round {
        src.push_str("\ndef _round(x, digits=0):\n    \"\"\"Round halves away from zero, as Excel does.\"\"\"\n    q = 10 ** digits\n    return (float(f\"{abs(x) * q:.15g}\") + 0.5) // 1 / q * (1 if x >= 0 else -1)\n\n\n");
    }
    // The engine's values of single cells go through `_value`: it is always there.
    let uses_value = uses_value || !steps.is_empty();
    if uses_value {
        src.insert_str(src.find("import polars").unwrap_or(0), "import datetime\n\n");
        src.push_str("\ndef _value(x):\n    \"\"\"A cell's value for a calculation, as Excel takes it: an empty cell counts as 0,\n    a date or time as its serial number (days since 1899-12-30).\"\"\"\n    if x is None:\n        return 0\n    if isinstance(x, datetime.datetime):\n        days = (x - datetime.datetime(1899, 12, 30)) / datetime.timedelta(days=1)\n        return days - 1 if days < 61 else days  # Excel counts a 29 February 1900\n    if isinstance(x, datetime.date):\n        days = (x - datetime.date(1899, 12, 30)).days\n        return days - 1 if days < 61 else days\n    if isinstance(x, datetime.time):\n        return (x.hour * 3600 + x.minute * 60 + x.second + x.microsecond / 1e6) / 86400\n    if isinstance(x, datetime.timedelta):\n        return x / datetime.timedelta(days=1)\n    return x\n\n");
    }
    if uses_plan {
        src.push_str(PLAN_HELPERS);
    }
    if gen.uses_numbers {
        src.push_str("\ndef _numbers(values):\n    \"\"\"The numbers among values: what SUM, MIN, MAX, AVERAGE and COUNT take from a range.\"\"\"\n    return [v for v in values if isinstance(v, (int, float)) and not isinstance(v, bool)]\n\n");
    }
    // The lines of the items count from the top of the whole program.
    let offset = src.lines().count() + 1;
    for item in items.iter_mut() {
        item.line += offset;
    }
    src.push('\n');
    src.push_str(&body);
    Program { source: src, items, check_line: check_line + offset, exact_statement }
}

/// What the program needs to run the engine's plans: values of cells and ranges in the
/// form the plans take them, and the plans' results as columns and values.
const PLAN_HELPERS: &str = r#"
def _num(x):
    """A cell's value as the engine takes a number: booleans as 1 and 0, dates and times
    as serial numbers; text and empty cells are no number."""
    if isinstance(x, bool):
        return 1.0 if x else 0.0
    if isinstance(x, (int, float)):
        return float(x)
    if isinstance(x, (datetime.datetime, datetime.date, datetime.time, datetime.timedelta)):
        return float(_value(x))
    return None


def _text(x):
    """A cell's value as the engine takes text."""
    return x if isinstance(x, str) else None


def _is_text(x):
    """Whether a cell still holds what a text input was made for: text, or nothing (an
    empty cell is 0 in the program)."""
    return isinstance(x, str) or not x


def _numeric(column):
    """Whether a column of a frame holds numbers (or booleans, or nothing)."""
    return column.dtype.is_numeric() or column.dtype in (pl.Boolean, pl.Null)


def _cell(wb, sheet, row, col):
    """A cell of the workbook that is in no table (0-based row and column)."""
    value = wb[sheet].cell(row=row + 1, column=col + 1).value
    return None if isinstance(value, str) and value.startswith("=") else value


def _range(wb, sheet, r1, r2, c1, c2):
    """The values of a range of the workbook (1-based), row by row."""
    return [v for row in wb[sheet].iter_rows(min_row=r1, max_row=r2, min_col=c1, max_col=c2, values_only=True) for v in row]


def _summary(values):
    """The sum, count, smallest, largest and sum of squares of the numbers among values
    (cells and columns): what SUM, COUNT, MIN, MAX, AVERAGE and STDEV take from a range.
    Booleans and text do not count."""
    numbers = []
    for v in values:
        if isinstance(v, pl.Series):
            numbers.extend(x for x in v.drop_nulls().to_list() if isinstance(x, (int, float)) and not isinstance(x, bool))
        elif not isinstance(v, (bool, str)) and v is not None:
            n = _num(v)
            if n is not None:
                numbers.append(n)
    total = squares = 0.0
    for n in numbers:
        total += n
        squares += n * n
    return (total, float(len(numbers)), min(numbers, default=None), max(numbers, default=None), squares)


def _typed(number, dtype, date1904):
    """Serial numbers as the dates, times or durations a workbook column shows (a Date
    column whose values have a time of day becomes a Datetime)."""
    if dtype == "Time":
        return (number * 86_400e9).round().cast(pl.Int64).cast(pl.Time)
    if dtype == "Duration":
        return (number * 86_400_000).round().cast(pl.Int64).cast(pl.Duration("ms"))
    days = number.floor()
    # The 1900 system counts a 29 February 1900: before March 1900 a serial is one ahead.
    epoch = 24_107 if date1904 else 25_569
    since_1970 = days - epoch if date1904 else pl.select(pl.when(days < 61).then(days - (epoch - 1)).otherwise(days - epoch)).to_series()
    fraction = number - days
    if dtype == "Date" and not (fraction.drop_nulls() != 0).any():
        return since_1970.cast(pl.Int32).cast(pl.Date)
    return (since_1970 * 86_400_000 + (fraction * 86_400_000).round()).cast(pl.Int64).cast(pl.Datetime("ms"))


def _result(out, is_bool=False, dtype=None, date1904=False):
    """The plan's values as a column: booleans, text where the formula gives text, else
    numbers (whole numbers as integers; an error is null), or dates, times and durations
    where `dtype` says the workbook shows the column so."""
    number, text = out["_r"], out["_rt"]
    number = pl.select(pl.when(number.is_finite()).then(number)).to_series()
    # An empty text is no value in a table (the cell looks empty).
    text = pl.select(pl.when(text != "").then(text)).to_series()
    if is_bool:
        return number.cast(pl.Boolean)
    if dtype in ("Date", "Datetime", "Time", "Duration") and text.null_count() == text.len():
        return _typed(number, dtype, date1904)
    if text.null_count() < text.len():
        shown = [None if n is None else (str(int(n)) if n == int(n) else repr(n)) for n in number.to_list()]
        return pl.Series([t if t is not None else s for t, s in zip(text.to_list(), shown)], dtype=pl.String)
    present = number.drop_nulls()
    if present.len() and (present == present.floor()).all() and present.abs().max() < 9e15:
        return number.cast(pl.Int64)
    return number


def _scalar(out, is_bool=False):
    """The value of a plan that works out one cell."""
    return _result(out, is_bool).to_list()[0]

"#;

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut quote = false;
    let mut prev = ' ';
    for ch in s.chars() {
        match ch {
            '"' if prev != '\\' => quote = !quote,
            '(' if !quote => depth += 1,
            ')' if !quote => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        prev = ch;
    }
    depth == 0
}

/// An expression that `.alias` can follow: a sum or comparison needs its brackets.
fn wrap_for_alias(s: &str) -> String {
    let mut depth = 0i32;
    let mut quote = false;
    let mut bare = false;
    let mut prev = ' ';
    for ch in s.chars() {
        match ch {
            '"' if prev != '\\' => quote = !quote,
            '(' | '[' if !quote => depth += 1,
            ')' | ']' if !quote => depth -= 1,
            ' ' if !quote && depth == 0 => bare = true,
            _ => {}
        }
        prev = ch;
    }
    if bare || s.starts_with('-') || s.starts_with('~') {
        format!("({s})")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xl_cells::{Sheet, Value};

    fn sample() -> Workbook {
        let mut wb = Workbook::default();
        wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
        wb.sheets.push(Sheet { name: "Model".into(), ..Default::default() });
        let c = &mut wb.cells;
        c.push(0, 0, 0, Value::Text("Orders".into()), None, 0);
        for (k, h) in ["item", "qty", "price", "amount", "big"].iter().enumerate() {
            c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
        }
        for r in 0..3u32 {
            c.push(0, 2 + r, 0, Value::Text(format!("item{r}")), None, 0);
            c.push(0, 2 + r, 1, Value::Number((r + 1) as f64), None, 0);
            c.push(0, 2 + r, 2, Value::Number(2.5), None, 0);
            c.push(0, 2 + r, 3, Value::Number(0.0), Some(format!("ROUND(B{0}*C{0}*(1+Model!$B$1),2)", r + 3)), 0);
            c.push(0, 2 + r, 4, Value::Text("".into()), Some(format!("IF(D{0}>Model!$B$3/2,\"yes\",TEXT(D{0},\"0.0\"))", r + 3)), 0);
        }
        c.push(1, 0, 0, Value::Text("Tax rate".into()), None, 0);
        c.push(1, 0, 1, Value::Number(0.19), None, 0);
        c.push(1, 1, 0, Value::Text("Orders".into()), None, 0);
        c.push(1, 1, 1, Value::Number(3.0), Some("COUNTA(Sales!A3:A5)".into()), 0);
        c.push(1, 2, 0, Value::Text("Gross".into()), None, 0);
        c.push(1, 2, 1, Value::Number(0.0), Some("SUM(Sales!D3:D5)".into()), 0);
        wb
    }

    #[test]
    fn formulas_become_polars_in_the_order_of_dependence() {
        let p = polars_program(&sample(), "book.xlsx", Accuracy::Plain, &[], None);
        let how: Vec<(&str, &str)> = p.items.iter().map(|i| (i.name.as_str(), i.how)).collect();
        // The amount first, then what reads it; TEXT has no translation.
        assert_eq!(how, vec![("amount", "polars"), ("Orders", "polars"), ("Gross", "polars"), ("big", "engine")]);
        assert!(p.source.contains("orders = orders.with_columns(((pl.col(\"qty\") * pl.col(\"price\")) * (1 + tax_rate)).round_sig_figs(15).round(2, mode=\"half_away_from_zero\").alias(\"amount\"))"), "{}", p.source);
        assert!(p.source.contains("gross = orders[\"amount\"].sum()"), "{}", p.source);
        // Each item knows the line its statements begin at.
        let lines: Vec<&str> = p.source.lines().collect();
        assert!(lines[p.items[2].line - 1].starts_with("# Model: Gross"), "{:?}", lines[p.items[2].line - 1]);
        assert!(p.source.contains("orders_2 = orders[\"item\"].count()") || p.source.contains("= orders[\"item\"].count()"), "{}", p.source);
        assert!(p.source.contains("exact = wb.evaluate("), "{}", p.source);
        assert!(p.source.contains("orders = orders.with_columns(exact[\"Orders\"][\"big\"])"), "{}", p.source);
        assert_eq!(p.items[3].reason.as_deref(), Some("the function TEXT"));
    }

    #[test]
    fn the_engine_can_be_asked_for() {
        let exact = polars_program(&sample(), "book.xlsx", Accuracy::Exact, &[], None);
        assert!(exact.items.iter().all(|i| i.how == "engine"));
        let one = polars_program(&sample(), "book.xlsx", Accuracy::Plain, &[("Model".into(), "Gross".into())], None);
        assert_eq!(one.items.iter().find(|i| i.name == "Gross").unwrap().how, "engine");
        assert!(one.source.contains("gross = _value(exact[\"Model\"][\"Gross\"][0])"));
    }
}
