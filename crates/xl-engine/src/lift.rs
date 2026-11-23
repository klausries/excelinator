//! Column formulas → Polars expressions, verified against the stored values.
//!
//! A *run* is a stretch of at least [`MIN_RUN`] cells in one column whose formulas
//! are the same once references are read relative to the cell (the result of filling
//! a formula down). A run is one formula over a table. If it only looks at cells of
//! its own row and at fixed cells, it becomes one Polars expression over the columns
//! it reads. The expression is evaluated for the run's rows and compared with the
//! values stored in the file; only then does the run count as lifted.

use crate::ir::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use xl_cells::{Kind, Workbook};
use xl_formula::{parse, parse_cell_ref, BinaryOp, CellRef, Expr as Fx, RefKind};

mod array;
mod deps;
mod more;

pub const MIN_RUN: usize = 3;

/// A formula filled down a column.
#[derive(Debug, Clone)]
pub struct Run {
    pub sheet: u32,
    pub col: u32,
    /// First row and number of rows.
    pub row: u32,
    pub len: u32,
    /// The formula of the first cell.
    pub formula: String,
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub runs: u64,
    pub run_cells: u64,
    /// Runs and cells whose Polars expression reproduces every stored value.
    pub lifted_runs: u64,
    pub lifted_cells: u64,
    /// Translated, but at least one value differs.
    pub mismatch_runs: u64,
    pub mismatch_cells: u64,
    /// Not translated, by reason, in cells.
    pub unsupported: BTreeMap<String, u64>,
    pub examples: Vec<String>,
    /// The cells of translated runs by how workbook values went into the translation
    /// (`BAKED_*`, or-ed; 0: the expression reads only its row's columns).
    pub baked: BTreeMap<u32, u64>,
}

impl Report {
    pub fn add(&mut self, other: &Report) {
        self.runs += other.runs;
        self.run_cells += other.run_cells;
        self.lifted_runs += other.lifted_runs;
        self.lifted_cells += other.lifted_cells;
        self.mismatch_runs += other.mismatch_runs;
        self.mismatch_cells += other.mismatch_cells;
        for (k, v) in &other.unsupported {
            *self.unsupported.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.baked {
            *self.baked.entry(*k).or_default() += v;
        }
    }
}

/// The runs of a workbook, in sheet, column, row order.
pub fn runs(wb: &Workbook) -> Vec<Run> {
    runs_from(wb, MIN_RUN)
}

/// Runs of at least `min_run` cells; with 1 every formula cell is in one.
pub fn runs_from(wb: &Workbook, min_run: usize) -> Vec<Run> {
    let c = &wb.cells;
    let mut cells: Vec<usize> = (0..c.len()).filter(|&i| c.formula[i].is_some()).collect();
    cells.sort_by_key(|&i| (c.sheet[i], c.col[i], c.row[i]));
    let mut out: Vec<Run> = Vec::new();
    let mut current: Option<(Run, String)> = None;
    let flush = |current: &mut Option<(Run, String)>, out: &mut Vec<Run>| {
        if let Some((run, _)) = current.take() {
            if run.len as usize >= min_run {
                out.push(run);
            }
        }
    };
    for i in cells {
        let text = c.formula[i].as_deref().unwrap_or("");
        let relative = match parse(text) {
            Ok(e) => e.relative_form(c.row[i] + 1, c.col[i] + 1),
            Err(_) => format!("unparsed:{}:{}", c.row[i], text),
        };
        match &mut current {
            Some((run, form)) if run.sheet == c.sheet[i] && run.col == c.col[i] && run.row + run.len == c.row[i] && *form == relative => run.len += 1,
            _ => {
                flush(&mut current, &mut out);
                current = Some((Run { sheet: c.sheet[i], col: c.col[i], row: c.row[i], len: 1, formula: text.to_string() }, relative));
            }
        }
    }
    flush(&mut current, &mut out);
    out
}

/// Intermediate columns: expressions used more than once are computed once, under
/// a name, in the order given.
type Lets = Vec<(String, Expr)>;

/// Give an expression a name so that using it several times does not repeat it.
fn bind(lets: &mut Lets, e: Expr) -> Expr {
    if e.is_column() || e.is_literal() {
        return e;
    }
    let e = balance(e);
    // An expression of literals alone may come out with one row: give it the frame's.
    let e = if e.reads_columns() { e } else { more::full(e) };
    let name = format!("_t{}", lets.len());
    lets.push((name.clone(), e));
    col(name)
}

/// A long chain `a + b + c + …` (or of `*`, and, or) rebuilt as a balanced tree, so
/// that sums over thousands of cells do not make expressions thousands deep.
fn balance(e: Expr) -> Expr {
    let Node::Binary(_, op, _) = e.node() else { return e };
    let op = *op;
    if !matches!(op, Op::Plus | Op::Multiply | Op::And | Op::Or) {
        return e;
    }
    fn leaves(e: &Expr, op: Op, out: &mut Vec<Expr>) {
        match e.node() {
            Node::Binary(left, o, right) if *o == op => {
                leaves(left, op, out);
                leaves(right, op, out);
            }
            _ => out.push(e.clone()),
        }
    }
    let mut parts = Vec::new();
    leaves(&e, op, &mut parts);
    if parts.len() < 64 {
        return e;
    }
    fn build(parts: &[Expr], op: Op) -> Expr {
        if parts.len() == 1 {
            return parts[0].clone();
        }
        let mid = parts.len() / 2;
        Expr::new(Node::Binary(build(&parts[..mid], op), op, build(&parts[mid..], op)))
    }
    build(&parts, op)
}

fn null_num() -> Expr {
    lit(NULL).cast(DataType::Float64)
}

fn null_text() -> Expr {
    lit(NULL).cast(DataType::String)
}

fn null_bool() -> Expr {
    lit(NULL).cast(DataType::Boolean)
}

/// A value whose kind is only known per row: what a cell holds, or what an `IF`
/// with a number in one branch and text in the other gives.
#[derive(Clone)]
struct Dyn {
    /// The number; null where the value is not a number.
    num: Expr,
    /// The text; null where the value is not text. `None`: never text.
    text: Option<Expr>,
    /// Both null means an empty cell (otherwise: an error).
    blank: bool,
    /// NaN in `num` stands for an error value in the cell.
    nan: bool,
    /// What the text is as a number (a number or a date written out); null where it is none.
    parsed: Option<Expr>,
}

impl Dyn {
    /// The number with an empty cell as 0 and an error as null; null where text.
    fn number_part(&self) -> Expr {
        let mut n = self.num.clone();
        if self.blank {
            n = match &self.text {
                Some(t) => when(t.clone().is_null()).then(n.clone().fill_null(lit(0.0))).otherwise(null_num()),
                None => n.fill_null(lit(0.0)),
            };
        }
        if self.nan {
            n = when(n.clone().is_nan()).then(null_num()).otherwise(n);
        }
        n
    }

    /// True where the cell is empty.
    fn is_blank(&self) -> Expr {
        if !self.blank {
            return lit(false);
        }
        match &self.text {
            Some(t) => self.num.clone().is_null().and(t.clone().is_null()),
            None => self.num.clone().is_null(),
        }
    }

    fn is_error(&self) -> Expr {
        if self.blank {
            return if self.nan { self.num.clone().is_nan().fill_null(lit(false)) } else { lit(false) };
        }
        let none = match &self.text {
            Some(t) => self.num.clone().is_null().and(t.clone().is_null()),
            None => self.num.clone().is_null(),
        };
        none.or(self.num.clone().is_finite().not().fill_null(lit(false)))
    }
}

/// A translated value. Null stands for an error value throughout.
#[derive(Clone)]
enum T {
    Num(Expr),
    Bool(Expr),
    Text(Expr),
    Dyn(Dyn),
}

/// A number as Excel writes it into text: rounded to 15 significant digits (9 for
/// 9.000000000000002), whole numbers without a fraction.
fn number_text(lets: &mut Lets, e: Expr) -> Expr {
    let e = bind(lets, e);
    let magnitude = e.clone().abs().log(lit(10.0)).floor();
    let scale = bind(lets, lit(10.0).pow(lit(14.0) - magnitude));
    let rounded = (e.clone().abs() * scale.clone() + lit(0.5)).floor() / scale.clone();
    let rounded = when(e.clone().lt(lit(0.0))).then(lit(0.0) - rounded.clone()).otherwise(rounded);
    let usable = e.clone().is_finite().and(e.clone().neq(lit(0.0))).and(scale.is_finite());
    let e = bind(lets, when(usable).then(rounded).otherwise(e));
    let whole = e.clone().eq(e.clone().floor()).and(e.clone().abs().lt(lit(1e15)));
    when(whole).then(e.clone().cast(DataType::Int64).cast(DataType::String)).otherwise(e.cast(DataType::String))
}

impl T {
    fn can_be_text(&self) -> bool {
        matches!(self, T::Text(_)) || matches!(self, T::Dyn(d) if d.text.is_some())
    }

    /// The value in arithmetic: text that reads as a number counts as that number,
    /// other text is an error.
    fn num(self) -> Expr {
        match self {
            T::Num(e) => e,
            T::Bool(e) => e.cast(DataType::Float64),
            T::Text(t) => t.cast(DataType::Float64),
            T::Dyn(d) => match &d.text {
                None => d.number_part(),
                Some(t) => {
                    let as_number = d.parsed.clone().unwrap_or_else(|| t.clone().cast(DataType::Float64));
                    when(t.clone().is_not_null()).then(as_number).otherwise(d.number_part())
                }
            },
        }
    }

    fn truth(self) -> Lifted<Expr> {
        Ok(match self {
            T::Num(e) => e.neq(lit(0.0)),
            T::Bool(e) => e,
            T::Text(_) => return Err("text as a condition".into()),
            T::Dyn(d) => match &d.text {
                None => d.number_part().neq(lit(0.0)),
                Some(t) => when(t.clone().is_not_null()).then(null_bool()).otherwise(d.number_part().neq(lit(0.0))),
            },
        })
    }

    /// The value as text, as `&` sees it: an empty cell is the empty text.
    fn text(self, lets: &mut Lets) -> Expr {
        match self {
            T::Num(e) => number_text(lets, e),
            T::Bool(e) => when(bind(lets, e.clone()).is_null()).then(null_text()).otherwise(when(e).then(lit("TRUE")).otherwise(lit("FALSE"))),
            T::Text(t) => t,
            T::Dyn(d) => {
                let n = if d.nan { when(d.num.clone().is_nan()).then(null_num()).otherwise(d.num.clone()) } else { d.num.clone() };
                let mut s = number_text(lets, n);
                if let Some(t) = &d.text {
                    s = when(t.clone().is_not_null()).then(t.clone()).otherwise(s);
                }
                if d.blank {
                    s = when(d.is_blank()).then(lit("")).otherwise(s);
                }
                s
            }
        }
    }

    /// Number part and text part, with an empty cell settled as 0.
    fn parts(self) -> Lifted<(Expr, Expr)> {
        Ok(match self {
            T::Num(e) => (e, null_text()),
            T::Text(t) => (null_num(), t),
            T::Bool(_) => return Err("truth value mixed with text".into()),
            T::Dyn(d) => (d.number_part(), d.text.clone().unwrap_or_else(null_text)),
        })
    }

    /// The same value with its parts under names.
    fn bound(self, lets: &mut Lets) -> T {
        match self {
            T::Num(e) => T::Num(bind(lets, e)),
            T::Bool(e) => T::Bool(bind(lets, e)),
            T::Text(e) => T::Text(bind(lets, e)),
            T::Dyn(d) => T::Dyn(Dyn { num: bind(lets, d.num), text: d.text.map(|t| bind(lets, t)), blank: d.blank, nan: d.nan, parsed: d.parsed.map(|p| bind(lets, p)) }),
        }
    }

    fn is_error(&self) -> Expr {
        match self {
            T::Num(e) => e.clone().is_null().or(e.clone().is_finite().not().fill_null(lit(false))),
            T::Bool(e) | T::Text(e) => e.clone().is_null(),
            T::Dyn(d) => d.is_error(),
        }
    }
}

/// `IF`: the condition picks a branch; an error in the condition is the result.
fn choose(lets: &mut Lets, condition: Expr, yes: T, no: T) -> Lifted<T> {
    let condition = bind(lets, condition);
    fn pick(c: &Expr, a: Expr, b: Expr, null: Expr) -> Expr {
        when(c.clone().is_null()).then(null).otherwise(when(c.clone()).then(a).otherwise(b))
    }
    let c = &condition;
    Ok(match (yes, no) {
        (T::Bool(a), T::Bool(b)) => T::Bool(pick(c, a, b, null_bool())),
        (T::Text(a), T::Text(b)) => T::Text(pick(c, a, b, null_text())),
        (a, b) if !a.can_be_text() && !b.can_be_text() => T::Num(pick(c, a.num(), b.num(), null_num())),
        (a, b) => {
            let ((an, at), (bn, bt)) = (a.parts()?, b.parts()?);
            let (num, text) = (bind(lets, pick(c, an, bn, null_num())), bind(lets, pick(c, at, bt, null_text())));
            T::Dyn(Dyn { num, text: Some(text), blank: false, nan: false, parsed: None })
        }
    })
}

fn compare_op(op: &BinaryOp, a: Expr, b: Expr) -> Expr {
    match op {
        BinaryOp::Eq => a.eq(b),
        BinaryOp::Ne => a.neq(b),
        BinaryOp::Lt => a.lt(b),
        BinaryOp::Gt => a.gt(b),
        BinaryOp::Le => a.lt_eq(b),
        BinaryOp::Ge => a.gt_eq(b),
        _ => unreachable!("comparison"),
    }
}

/// Numbers compared as Excel and LibreOffice do: two numbers that agree to about 15
/// significant digits (2^-48 relative) are equal, so `0.1+0.2=0.3` holds and a counter
/// built by adding 1 again and again equals its target.
fn compare_numbers(lets: &mut Lets, op: &BinaryOp, a: Expr, b: Expr) -> Expr {
    let (a, b) = (bind(lets, a), bind(lets, b));
    let scale = when(a.clone().abs().gt(b.clone().abs())).then(a.clone().abs()).otherwise(b.clone().abs());
    let close = bind(lets, (a.clone() - b.clone()).abs().lt_eq(scale * lit(3.552713678800501e-15)));
    match op {
        BinaryOp::Eq => a.eq(b).or(close),
        BinaryOp::Ne => a.neq(b).and(close.not()),
        BinaryOp::Lt => a.lt(b).and(close.not()),
        BinaryOp::Gt => a.gt(b).and(close.not()),
        BinaryOp::Le => a.lt_eq(b).or(close),
        BinaryOp::Ge => a.gt_eq(b).or(close),
        _ => unreachable!("comparison"),
    }
}

/// What a comparison gives when the left side sorts before (`less`) or after the right.
fn compare_fixed(op: &BinaryOp, less: bool) -> Expr {
    lit(match op {
        BinaryOp::Eq => false,
        BinaryOp::Ne => true,
        BinaryOp::Lt | BinaryOp::Le => less,
        BinaryOp::Gt | BinaryOp::Ge => !less,
        _ => unreachable!("comparison"),
    })
}

/// One side of a comparison: its number (null where text or error), its text in
/// lower case, and where it is an empty cell.
struct Side {
    num: Expr,
    text: Option<Expr>,
    blank: Option<Expr>,
}

fn side(lets: &mut Lets, t: T) -> Side {
    match t {
        T::Text(e) => Side { num: null_num(), text: Some(bind(lets, e.str().to_lowercase())), blank: None },
        T::Dyn(d) => Side {
            num: bind(lets, d.number_part()),
            text: d.text.clone().map(|t| bind(lets, t.str().to_lowercase())),
            blank: d.blank.then(|| bind(lets, d.is_blank())),
        },
        other => Side { num: bind(lets, other.num()), text: None, blank: None },
    }
}

/// Excel's comparison: numbers by value, text without regard to case, any number
/// before any text; an empty cell is 0 against a number and "" against text.
fn compare(lets: &mut Lets, op: &BinaryOp, a: T, b: T) -> Expr {
    let (a, b) = (side(lets, a), side(lets, b));
    let mut result = compare_numbers(lets, op, a.num.clone(), b.num.clone());
    // One side text, the other not: the other is a number, an empty cell or an error.
    let against = |text: &Expr, other: &Side, text_is_left: bool| {
        let fixed = compare_fixed(op, !text_is_left);
        let decided = match &other.blank {
            Some(blank) => {
                let empty = if text_is_left { compare_op(op, text.clone(), lit("")) } else { compare_op(op, lit(""), text.clone()) };
                when(blank.clone()).then(empty).otherwise(fixed)
            }
            None => fixed,
        };
        when(other.num.clone().is_null()).then(null_bool()).otherwise(decided)
    };
    if let Some(tb) = &b.text {
        result = when(tb.clone().is_not_null()).then(against(tb, &a, false)).otherwise(result);
    }
    if let Some(ta) = &a.text {
        result = when(ta.clone().is_not_null()).then(against(ta, &b, true)).otherwise(result);
    }
    if let (Some(ta), Some(tb)) = (&a.text, &b.text) {
        result = when(ta.clone().is_not_null().and(tb.clone().is_not_null())).then(compare_op(op, ta.clone(), tb.clone())).otherwise(result);
    }
    result
}

/// The numbers of a fixed range in the form the aggregates need.
#[derive(Debug, Clone, Copy)]
struct Summary {
    sum: f64,
    count: f64,
    min: f64,
    max: f64,
    sum_squares: f64,
    /// The range holds an error value: so does every aggregate over it but COUNT.
    error: bool,
}

/// The aggregates of a fixed range as expressions: constants, or the inputs of the plan.
#[derive(Clone)]
struct Agg {
    sum: Expr,
    count: Expr,
    /// Null where the range holds no number.
    min: Expr,
    max: Expr,
    sum_squares: Expr,
    /// Whether the range holds no number, where that is known (not for inputs).
    empty: Option<bool>,
}

impl Agg {
    fn baked(s: &Summary) -> Agg {
        Agg { sum: lit(s.sum), count: lit(s.count), min: lit(s.min), max: lit(s.max), sum_squares: lit(s.sum_squares), empty: Some(s.count == 0.0) }
    }
}

/// An argument of an aggregate.
enum Item {
    /// A value per row; null where the cell is empty or holds text.
    Rows(Expr),
    /// A fixed range.
    Fixed(Agg),
    /// A range that grows with the row: its sum and its count of numbers so far.
    Growing { sum: Expr, count: Expr, max: Expr, min: Expr },
}

/// The arguments of an aggregate, and where one of them holds an error.
struct Spread {
    items: Vec<Item>,
    error: Option<Expr>,
}

impl Spread {
    fn poison(&mut self, e: Expr) {
        self.error = Some(match self.error.take() {
            Some(known) => known.or(e),
            None => e,
        });
    }

    fn guard(&self, result: Expr) -> Expr {
        match &self.error {
            Some(e) => when(e.clone()).then(null_num()).otherwise(result),
            None => result,
        }
    }
}

type RangeCache = std::cell::RefCell<HashMap<(u32, u32, u32, u32, u32), Lifted<Summary>>>;

/// What a translation reads: columns of its own row, and the cells behind constants.
struct Context<'a> {
    wb: &'a Workbook,
    at: &'a HashMap<(u32, u32, u32), usize>,
    ranges: &'a RangeCache,
    sheet: u32,
    /// Row of the first formula of the run (1-based), the run's column and length.
    row: u32,
    run_col: u32,
    len: u32,
    /// (sheet, column) pairs read, columns 1-based; and those read as text too.
    columns: BTreeSet<(u32, u32)>,
    text_columns: BTreeSet<(u32, u32)>,
    parsed_columns: BTreeSet<(u32, u32)>,
    /// Smallest and largest row offset read (0 = own row).
    reach: (i64, i64),
    /// Inside a defined name: only fixed references mean the same in every cell.
    in_name: u32,
    lets: Lets,
    /// Rows the translation does not hold for, by reason (see `guard`).
    guards: Vec<(String, Expr)>,
    /// Reading the run's own column at other rows is allowed: the result is then
    /// found by iteration (see `LiftedRun::own`). Smallest and largest offset read.
    own: Option<(i64, i64)>,
    own_dir: i8,
    /// Set by the last lookup: where its key is an error, and where its result is #N/A.
    last_na: Option<(Expr, Expr)>,
    recursive: bool,
    /// The formula is an array formula: ranges in scalar places are taken cell by cell.
    array: bool,
    /// Inside SUM, where FALSE from an IF without else branch counts as nothing.
    summing: bool,
    /// Sheets from this index on are what the file kept of other workbooks.
    external_from: u32,
    /// The last row holding a cell, per sheet (1-based; 0 for an empty sheet).
    last_rows: &'a [u32],
    /// What works out fixed parts of a formula while it is translated.
    ex: &'a dyn Executor,
    /// How values of the workbook went into the translation (see [`LiftedRun::baked`]).
    baked: std::cell::Cell<u32>,
    /// Fixed cells become inputs of the plan (see [`Options::param`]); off while a fixed
    /// expression is worked out at translation, whose value then decides the translation.
    param: std::cell::Cell<bool>,
    /// The cells whose value or kind shaped the translation, beyond the inputs.
    reads: std::cell::RefCell<HashSet<(u32, u32, u32)>>,
    /// The sheets whose extent (last row) shaped the translation.
    extents: std::cell::RefCell<BTreeSet<u32>>,
    /// Fixed cells taken as inputs (sheet, row, column; 0-based): true if the cell holds text.
    cell_inputs: std::cell::RefCell<BTreeMap<(u32, u32, u32), bool>>,
    /// Fixed ranges taken as inputs (sheet, rows, columns; 1-based), with whether they hold an error.
    range_inputs: std::cell::RefCell<BTreeMap<(u32, u32, u32, u32, u32), bool>>,
    /// Fixed ranges whose values went into the translation as they are.
    pinned_ranges: std::cell::RefCell<BTreeSet<(u32, u32, u32, u32, u32)>>,
    /// The columns read as inputs, by (sheet, column): whether they hold text, whether errors.
    column_kinds: BTreeMap<(u32, u32), ColumnKinds>,
}

/// How a run is translated.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Fixed cells (absolute row, `$B$1`, `Tax_rate`) become inputs of the plan instead of
    /// the constants they hold. The expressions then hold for other values of those cells
    /// (and for other data, as far as [`Assumptions`] hold). Without it (the default) the
    /// value of every fixed cell is part of the expression.
    pub param: bool,
}

/// A cell range of one column, rows `first..=last`, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rows {
    pub sheet: u32,
    pub col: u32,
    pub first: u32,
    pub last: u32,
}

/// What a translation took from the workbook beyond the columns it reads as inputs: the
/// expression holds as long as these hold.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Assumptions {
    /// Cells whose value (or kind) shaped the expression: these are the same.
    pub pinned: Vec<Rows>,
    /// Sheets whose extent (the last row holding a cell) is the same.
    pub extents: Vec<u32>,
    /// Fixed cells taken as inputs (sheet, row, column; 0-based, text or number): they keep
    /// their kind.
    pub cell_inputs: Vec<(u32, u32, u32, bool)>,
    /// Columns read as inputs: in these rows they hold no other kinds than these.
    pub columns: Vec<ColumnKinds>,
    /// Fixed ranges whose numbers are inputs (sheet, first row, last row, first column,
    /// last column; 1-based, and whether they hold an error): the error stays as it is.
    pub ranges: Vec<((u32, u32, u32, u32, u32), bool)>,
}

/// What a column read as input holds, in the rows read (0-based; rows above the sheet are skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnKinds {
    pub sheet: u32,
    /// 1-based, as in the translation.
    pub col: u32,
    pub first: i64,
    pub last: i64,
    pub text: bool,
    pub error: bool,
}

/// A fixed cell's value is in the expression as a constant.
pub const BAKED_CELL: u32 = 1;
/// An aggregate over a fixed range (its sum, count, extremes) is in the expression as a constant.
pub const BAKED_RANGE: u32 = 2;
/// A value decided how the formula was translated (which column a lookup returns,
/// where an `OFFSET` points, a criterion).
pub const BAKED_DECISION: u32 = 4;
/// The values of a block of cells are in the expression (a lookup table, an array).
pub const BAKED_BLOCK: u32 = 8;

/// A fixed block of cells; rows and columns 1-based, inclusive.
#[derive(Debug, Clone, Copy)]
struct Block {
    sheet: u32,
    r1: u32,
    r2: u32,
    c1: u32,
    c2: u32,
}

/// A key of a lookup table.
#[derive(PartialEq)]
enum Key {
    Blank,
    Num(f64),
    /// In lower case: lookups do not regard case.
    Text(String),
    Other,
}

/// The kinds of error value an expression can give, as far as the translator can tell.
#[derive(Debug, Clone, Copy, Default)]
struct Info {
    /// Bit 0: `#N/A`; bit 1: any other error value.
    err: u8,
    /// The expression can be text (arithmetic on it may then give `#VALUE!`).
    text: bool,
}

const NA: u8 = 1;
const OTHER: u8 = 2;

impl Info {
    fn join(self, o: Info) -> Info {
        Info { err: self.err | o.err, text: self.text || o.text }
    }

    /// The result of an operation on these operands that converts them to numbers.
    fn arithmetic(self) -> Info {
        Info { err: self.err | if self.text { OTHER } else { 0 }, text: false }
    }
}

/// Where an `OFFSET` points.
enum Target {
    /// A block that is the same for every cell of the run.
    Fixed(Block),
    /// A block that moves with the row like a relative reference: its left column,
    /// the row offset of its top row from the formula's row, height and width.
    Shifted { sheet: u32, col: u32, offset: i64, height: u32, width: u32 },
    /// One cell per row, picked from a fixed column (`vertical`) or a fixed row by a
    /// position computed per row (1-based sheet row or column).
    Gather { sheet: u32, vertical: bool, line: u32, index: Expr },
    /// Outside the sheet: `#REF!`.
    Nowhere,
}

const SHEET_ROWS: i64 = 1_048_576;
const SHEET_COLS: i64 = 16_384;

fn column_number(letters: &str) -> Option<u32> {
    let letters = letters.trim_start_matches('$');
    if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(letters.bytes().fold(0u32, |n, b| n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32))
}

/// `A1:B1:C1` (cells of one sheet, all fixed or moving alike) as the range around them.
fn range_chain(e: &Fx) -> Option<Fx> {
    fn cells<'a>(e: &'a Fx, out: &mut Vec<(&'a Option<String>, CellRef)>) -> bool {
        match core(e) {
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => cells(lhs, out) && cells(rhs, out),
            Fx::Ref { prefix, kind: RefKind::Cell, text } => match parse_cell_ref(text) {
                Some(r) => {
                    out.push((prefix, r));
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
    let mut found = Vec::new();
    if !cells(e, &mut found) || found.len() < 3 {
        return None;
    }
    let first = found[0];
    if found.iter().any(|(p, r)| (p.is_some() && *p != first.0) || r.abs_row != first.1.abs_row || r.abs_col != first.1.abs_col) {
        return None;
    }
    let (r1, r2) = (found.iter().map(|x| x.1.row).min()?, found.iter().map(|x| x.1.row).max()?);
    let (c1, c2) = (found.iter().map(|x| x.1.col).min()?, found.iter().map(|x| x.1.col).max()?);
    let text = |r: u32, c: u32| format!("{}{}{}{}", if first.1.abs_col { "$" } else { "" }, xl_formula::col_name(c), if first.1.abs_row { "$" } else { "" }, r);
    Some(Fx::Binary {
        op: BinaryOp::Range,
        op_text: ":".into(),
        lhs: Box::new(Fx::Ref { prefix: first.0.clone(), kind: RefKind::Cell, text: text(r1, c1) }),
        rhs: Box::new(Fx::Ref { prefix: None, kind: RefKind::Cell, text: text(r2, c2) }),
    })
}

/// Functions that take ranges as ranges: a range among their arguments is not taken
/// cell by cell in an array formula.
const RANGE_FUNCTIONS: &[&str] = &[
    "SUM", "SUMPRODUCT", "COUNT", "COUNTA", "AVERAGE", "MIN", "MAX", "MINA", "MAXA", "STDEV", "STDEVP", "VAR", "VARP", "VLOOKUP", "HLOOKUP", "LOOKUP",
    "MATCH", "INDEX", "OFFSET", "SUMIF", "SUMIFS", "COUNTIF", "COUNTIFS", "AVERAGEIF", "AVERAGEIFS", "RANK", "ROWS", "COLUMNS", "ROW", "COLUMN", "NPV",
    "XNPV", "XIRR", "CORREL", "PEARSON", "AND", "OR", "WORKDAY", "NETWORKDAYS",
];

fn if_without_else(e: &Fx) -> bool {
    match e {
        Fx::Spaced { expr, .. } | Fx::Paren(expr) | Fx::Unary { expr, .. } | Fx::Postfix { expr, .. } => if_without_else(expr),
        Fx::Binary { lhs, rhs, .. } => if_without_else(lhs) || if_without_else(rhs),
        Fx::Call { name, args } => (name.eq_ignore_ascii_case("IF") && args.len() == 2) || args.iter().any(if_without_else),
        _ => false,
    }
}

/// The first error value among the operands of a chain of unions and intersections.
fn first_error(e: &Fx) -> Option<String> {
    match core(e) {
        Fx::Error(t) => Some(t.clone()),
        Fx::Binary { op: BinaryOp::Union | BinaryOp::Intersect, lhs, rhs, .. } => first_error(lhs).or_else(|| first_error(rhs)),
        _ => None,
    }
}

/// Every cell reference made fixed, everywhere in a formula.
fn fixed_refs(e: &Fx) -> Fx {
    let go = |x: &Fx| Box::new(fixed_refs(x));
    match e {
        Fx::Ref { kind: RefKind::Cell, .. } => absolute(e),
        Fx::Spaced { before, expr, after } => Fx::Spaced { before: before.clone(), expr: go(expr), after: after.clone() },
        Fx::Paren(x) => Fx::Paren(go(x)),
        Fx::Unary { op, expr } => Fx::Unary { op: *op, expr: go(expr) },
        Fx::Postfix { expr, op } => Fx::Postfix { expr: go(expr), op: *op },
        Fx::Binary { op, op_text, lhs, rhs } => Fx::Binary { op: op.clone(), op_text: op_text.clone(), lhs: go(lhs), rhs: go(rhs) },
        Fx::Call { name, args } => Fx::Call { name: name.clone(), args: args.iter().map(fixed_refs).collect() },
        other => other.clone(),
    }
}

/// A reference with every cell fixed (`A1` becomes `$A$1`): what INDIRECT gives.
fn absolute(e: &Fx) -> Fx {
    match e {
        Fx::Ref { prefix, kind: RefKind::Cell, text } => match parse_cell_ref(text) {
            Some(r) => Fx::Ref { prefix: prefix.clone(), kind: RefKind::Cell, text: format!("${}${}", xl_formula::col_name(r.col), r.row) },
            None => e.clone(),
        },
        Fx::Binary { op, op_text, lhs, rhs } => Fx::Binary { op: op.clone(), op_text: op_text.clone(), lhs: Box::new(absolute(lhs)), rhs: Box::new(absolute(rhs)) },
        Fx::Paren(x) => Fx::Paren(Box::new(absolute(x))),
        Fx::Spaced { expr, .. } => absolute(expr),
        other => other.clone(),
    }
}

impl Context<'_> {
    /// INDIRECT with a fixed text is the reference it spells; CHOOSE with a fixed index
    /// among references is the reference chosen. Both are put in before translating.
    fn resolve(&mut self, e: &Fx) -> Lifted<Fx> {
        let go = |ctx: &mut Self, x: &Fx| ctx.resolve(x).map(Box::new);
        Ok(match e {
            Fx::Call { name, args } if name.eq_ignore_ascii_case("INDIRECT") && !args.is_empty() && self.is_fixed(&args[0], 0) => {
                if args.len() > 1 && !self.fixed_number(&args[1])?.is_some_and(|v| v != 0.0) {
                    return Err("INDIRECT in R1C1 form".into());
                }
                let Some(text) = self.fixed_text_of(&args[0])? else { return Ok(Fx::Error("#REF!".into())) };
                match parse(text.trim()) {
                    Ok(r) if matches!(core(&r), Fx::Ref { .. } | Fx::Binary { op: BinaryOp::Range, .. } | Fx::Name { .. }) => absolute(&r),
                    _ => Fx::Error("#REF!".into()),
                }
            }
            Fx::Call { name, args } if name.eq_ignore_ascii_case("CHOOSE") && args.len() > 1 && self.is_fixed(&args[0], 0) && args[1..].iter().all(|a| matches!(core(a), Fx::Name { .. } | Fx::Ref { .. } | Fx::Binary { op: BinaryOp::Range, .. })) => {
                match self.fixed_number(&args[0])? {
                    Some(v) if v >= 1.0 && (v as usize) < args.len() => self.resolve(&args[v as usize])?,
                    _ => Fx::Error("#VALUE!".into()),
                }
            }
            // OFFSET with fixed arguments is the block it points at.
            Fx::Call { name, args } if name.eq_ignore_ascii_case("OFFSET") && args.iter().all(|a| self.is_fixed(a, 0)) => match {
                let resolved = args.iter().map(|a| self.resolve(a)).collect::<Lifted<Vec<_>>>()?;
                self.offset(&resolved)
            } {
                Ok(Target::Fixed(b)) => self.block_fx(b),
                Ok(Target::Nowhere) => Fx::Error("#REF!".into()),
                _ => Fx::Call { name: name.clone(), args: args.iter().map(|a| self.resolve(a)).collect::<Lifted<Vec<_>>>()? },
            },
            Fx::Call { name, args } => Fx::Call { name: name.clone(), args: args.iter().map(|a| self.resolve(a)).collect::<Lifted<Vec<_>>>()? },
            Fx::Spaced { before, expr, after } => Fx::Spaced { before: before.clone(), expr: go(self, expr)?, after: after.clone() },
            Fx::Paren(x) => Fx::Paren(go(self, x)?),
            Fx::Unary { op, expr } => Fx::Unary { op: *op, expr: go(self, expr)? },
            Fx::Postfix { expr, op } => Fx::Postfix { expr: go(self, expr)?, op: *op },
            // Sheet!A1:Sheet!A9 is Sheet!A1:A9.
            Fx::Binary { op: BinaryOp::Range, op_text, lhs, rhs } if matches!((core(lhs), core(rhs)), (Fx::Ref { prefix: Some(a), kind: RefKind::Cell, .. }, Fx::Ref { prefix: Some(b), kind: RefKind::Cell, .. }) if a.eq_ignore_ascii_case(b)) => {
                let Fx::Ref { text, kind, .. } = core(rhs) else { unreachable!() };
                Fx::Binary { op: BinaryOp::Range, op_text: op_text.clone(), lhs: Box::new(core(lhs).clone()), rhs: Box::new(Fx::Ref { prefix: None, kind: kind.clone(), text: text.clone() }) }
            }
            // A range whose ends are names or OFFSET: the fixed block it spans.
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if !matches!((core(lhs), core(rhs)), (Fx::Ref { .. }, Fx::Ref { prefix: None, .. })) && self.is_fixed(e, 0) => match self.table(e) {
                Ok(Some(b)) => self.block_fx(b),
                Ok(None) => Fx::Error("#REF!".into()),
                Err(_) => Fx::Binary { op: BinaryOp::Range, op_text: ":".into(), lhs: go(self, lhs)?, rhs: go(self, rhs)? },
            },
            // A name for a range of another form (A1:OFFSET(…)): the block.
            Fx::Name { prefix, text } => match self.name(&qualified(prefix, text)) {
                Ok(f) if !matches!(core(&f), Fx::Ref { .. } | Fx::Number(_) | Fx::Str(_) | Fx::Bool(_) | Fx::Error(_)) && !matches!(core(&f), Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if matches!((core(lhs), core(rhs)), (Fx::Ref { .. }, Fx::Ref { prefix: None, .. }))) && self.is_fixed(&f, 0) => match self.table(e) {
                    Ok(Some(b)) => self.block_fx(b),
                    _ => e.clone(),
                },
                _ => e.clone(),
            },
            Fx::Binary { op, op_text, lhs, rhs } => Fx::Binary { op: op.clone(), op_text: op_text.clone(), lhs: go(self, lhs)?, rhs: go(self, rhs)? },
            other => other.clone(),
        })
    }

    /// A fixed block written as a reference.
    fn block_fx(&self, b: Block) -> Fx {
        let sheet = Some(format!("'{}'!", self.wb.sheets[b.sheet as usize].name.replace('\'', "''")));
        let cell = |r: u32, c: u32| format!("${}${}", xl_formula::col_name(c), r);
        if b.r1 == b.r2 && b.c1 == b.c2 {
            Fx::Ref { prefix: sheet, kind: RefKind::Cell, text: cell(b.r1, b.c1) }
        } else {
            Fx::Binary {
                op: BinaryOp::Range,
                op_text: ":".into(),
                lhs: Box::new(Fx::Ref { prefix: sheet, kind: RefKind::Cell, text: cell(b.r1, b.c1) }),
                rhs: Box::new(Fx::Ref { prefix: None, kind: RefKind::Cell, text: cell(b.r2, b.c2) }),
            }
        }
    }

    /// The shape (rows, columns) of the ranges an expression takes cell by cell, if any.
    fn shape(&self, e: &Fx) -> Lifted<Option<(u32, u32)>> {
        let mut found: Option<(u32, u32)> = None;
        let mut merge = |s: Option<(u32, u32)>| -> Lifted<()> {
            if let Some(s) = s {
                // Arrays of different sizes: the larger size; a single row or column is
                // repeated, cells beyond a smaller array are #N/A.
                found = Some(match found {
                    None => s,
                    Some(f) => (f.0.max(s.0), f.1.max(s.1)),
                });
            }
            Ok(())
        };
        match e {
            Fx::Spaced { expr, .. } | Fx::Paren(expr) | Fx::Unary { expr, .. } | Fx::Postfix { expr, .. } => merge(self.shape(expr)?)?,
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { kind: RefKind::Cell, text: a, .. }, Fx::Ref { kind: RefKind::Cell, text: b, .. }) = (core(lhs), core(rhs)) else {
                    return Err("array formula over a range of another form".into());
                };
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                merge(Some((p.row.abs_diff(q.row) + 1, p.col.abs_diff(q.col) + 1)))?;
            }
            Fx::Binary { lhs, rhs, .. } => {
                merge(self.shape(lhs)?)?;
                merge(self.shape(rhs)?)?;
            }
            Fx::Name { prefix, text } => {
                if let Ok(f) = self.name(&qualified(prefix, text)) {
                    if matches!(core(&f), Fx::Binary { op: BinaryOp::Range, .. }) {
                        merge(self.shape(&f)?)?;
                    }
                }
            }
            Fx::Call { name, args } if name.eq_ignore_ascii_case("TRANSPOSE") && args.len() == 1 => {
                merge(self.shape(&args[0])?.map(|(h, w)| (w, h)))?;
            }
            Fx::Call { name, args } if !RANGE_FUNCTIONS.contains(&name.to_ascii_uppercase().as_str()) => {
                for a in args {
                    merge(self.shape(a)?)?;
                }
            }
            _ => {}
        }
        Ok(found)
    }

    /// The expression once for every cell of its ranges, row by row.
    fn elements(&self, e: &Fx) -> Lifted<Vec<Fx>> {
        let Some((h, w)) = self.shape(e)? else { return Ok(vec![e.clone()]) };
        let (cells, steps) = (400, 20_000);
        if h as u64 * w as u64 > cells || self.lets.len() > steps {
            return Err("large array formula: taken cell by cell it would be too large".into());
        }
        let mut out = Vec::new();
        for i in 0..h {
            for j in 0..w {
                out.push(self.element(e, i, j)?);
            }
        }
        Ok(out)
    }

    fn element(&self, e: &Fx, i: u32, j: u32) -> Lifted<Fx> {
        let at = |x: &Fx| self.element(x, i, j).map(Box::new);
        Ok(match e {
            Fx::Spaced { before, expr, after } => Fx::Spaced { before: before.clone(), expr: at(expr)?, after: after.clone() },
            Fx::Paren(expr) => Fx::Paren(at(expr)?),
            Fx::Unary { op, expr } => Fx::Unary { op: *op, expr: at(expr)? },
            Fx::Postfix { expr, op } => Fx::Postfix { expr: at(expr)?, op: *op },
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, text: a, .. }, Fx::Ref { text: b, .. }) = (core(lhs), core(rhs)) else { return Err("range form".into()) };
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                let (h, w) = (p.row.abs_diff(q.row) + 1, p.col.abs_diff(q.col) + 1);
                let (i, j) = (if h == 1 { 0 } else { i }, if w == 1 { 0 } else { j });
                if i >= h || j >= w {
                    return Ok(Fx::Error("#N/A".into()));
                }
                let (row, column) = (p.row.min(q.row) + i, p.col.min(q.col) + j);
                let text = format!("{}{}{}{}", if p.abs_col { "$" } else { "" }, xl_formula::col_name(column), if p.abs_row { "$" } else { "" }, row);
                Fx::Ref { prefix: prefix.clone(), kind: RefKind::Cell, text }
            }
            Fx::Binary { op, op_text, lhs, rhs } => Fx::Binary { op: op.clone(), op_text: op_text.clone(), lhs: at(lhs)?, rhs: at(rhs)? },
            Fx::Name { prefix, text } => match self.name(&qualified(prefix, text)) {
                Ok(f) if matches!(core(&f), Fx::Binary { op: BinaryOp::Range, .. }) => self.element(&f, i, j)?,
                _ => e.clone(),
            },
            Fx::Call { name, args } if name.eq_ignore_ascii_case("TRANSPOSE") && args.len() == 1 => self.element(&args[0], j, i)?,
            Fx::Call { name, args } if !RANGE_FUNCTIONS.contains(&name.to_ascii_uppercase().as_str()) => {
                Fx::Call { name: name.clone(), args: args.iter().map(|a| self.element(a, i, j)).collect::<Lifted<Vec<_>>>()? }
            }
            other => other.clone(),
        })
    }
}

impl Context<'_> {
    /// A reference over several sheets as one reference per sheet.
    fn across_sheets(&self, e: &Fx) -> Lifted<Option<Vec<Fx>>> {
        let prefix = match core(e) {
            Fx::Ref { prefix: Some(p), .. } => p,
            Fx::Binary { op: BinaryOp::Range, lhs, .. } => match core(lhs) {
                Fx::Ref { prefix: Some(p), .. } => p,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let name = prefix.strip_suffix('!').unwrap_or(prefix);
        let name = name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')).unwrap_or(name);
        let Some((first, last)) = name.split_once(':') else { return Ok(None) };
        if first.starts_with('[') {
            return Err("reference to another workbook across several sheets".into());
        }
        let find = |n: &str| self.wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(n)).ok_or_else(|| "reference to an unknown sheet".to_string());
        let (a, b) = (find(first)?, find(last)?);
        let mut out = Vec::new();
        for k in a.min(b)..=a.max(b) {
            let p = Some(format!("'{}'!", self.wb.sheets[k].name.replace('\'', "''")));
            out.push(match core(e) {
                Fx::Ref { kind, text, .. } => Fx::Ref { prefix: p, kind: kind.clone(), text: text.clone() },
                Fx::Binary { op, op_text, lhs, rhs } => match core(lhs) {
                    Fx::Ref { kind, text, .. } => Fx::Binary { op: op.clone(), op_text: op_text.clone(), lhs: Box::new(Fx::Ref { prefix: p, kind: kind.clone(), text: text.clone() }), rhs: rhs.clone() },
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            });
        }
        Ok(Some(out))
    }
}

/// A name as formulas write it, with its sheet or workbook prefix.
fn qualified(prefix: &Option<String>, text: &str) -> String {
    match prefix {
        Some(p) => format!("{p}{text}"),
        None => text.to_string(),
    }
}

/// Whether the cells of a column (1-based) in the given sheet rows (0-based, inclusive) of
/// a workbook hold text, and whether they hold errors.
fn holds_in(wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, sheet: u32, col: u32, first: i64, last: i64) -> (bool, bool) {
    let (mut text, mut error) = (false, false);
    for row in first.max(0)..=last {
        if let Some(&i) = at.get(&(sheet, row as u32, col - 1)) {
            match wb.cells.kind[i] {
                Kind::Text => text = true,
                Kind::Blank if wb.cells.formula[i].is_some() => text = true,
                Kind::Error => error = true,
                _ => {}
            }
        }
        if text && error {
            break;
        }
    }
    (text, error)
}

/// The numbers of a range (sheet, first row, last row, first column, last column; 1-based).
fn summarize(wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, (sheet, r1, r2, c1, c2): (u32, u32, u32, u32, u32)) -> Summary {
    let mut s = Summary { sum: 0.0, count: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY, sum_squares: 0.0, error: false };
    for r in r1..=r2 {
        for c in c1..=c2 {
            if let Some(&i) = at.get(&(sheet, r - 1, c - 1)) {
                match wb.cells.kind[i] {
                    Kind::Number => {
                        let v = wb.cells.num[i];
                        s.sum += v;
                        s.count += 1.0;
                        s.min = s.min.min(v);
                        s.max = s.max.max(v);
                        s.sum_squares += v * v;
                    }
                    Kind::Error => s.error = true,
                    _ => {}
                }
            }
        }
    }
    s
}

/// The columns of the input table that hold the aggregates of a fixed range: `what` is
/// `s` (sum), `n` (count), `l` (smallest), `h` (largest), `q` (sum of squares).
pub(crate) fn range_input_name((sheet, r1, r2, c1, c2): (u32, u32, u32, u32, u32), what: char) -> String {
    format!("_a{sheet}_{r1}_{r2}_{c1}_{c2}{what}")
}

/// The column of the input table that holds a fixed cell (`_k<sheet>r<row>c<col>`, 0-based;
/// `t` after `_k` for a text).
pub(crate) fn cell_input_name(sheet: u32, row: u32, col: u32, text: bool) -> String {
    format!("_k{}{sheet}r{row}c{col}", if text { "t" } else { "" })
}

pub(crate) fn column_name(sheet: u32, col: u32) -> String {
    format!("s{sheet}c{col}")
}

pub(crate) fn parsed_column_name(sheet: u32, col: u32) -> String {
    format!("s{sheet}c{col}p")
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// Day count of a calendar date as Excel numbers it (1900 system, from 1 March 1900).
fn serial(year: i64, month: i64, day: i64, date1904: bool) -> Option<f64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    // The day must exist in the month.
    let (back_y, back_m, back_d) = {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + (m <= 2) as i64, m, d)
    };
    if (back_y, back_m, back_d) != (year, month, day) {
        return None;
    }
    let serial = if date1904 { days + 24_107 } else { days + 25_569 };
    // Before 1 March 1900 the count differs; not handled.
    (serial >= 61 || date1904 && serial >= 0).then_some(serial as f64)
}

fn two_digit_year(y: i64, digits: usize) -> i64 {
    if digits > 2 { y } else if y < 30 { 2000 + y } else { 1900 + y }
}

/// Whether a text is in arithmetic what a plain cast of it to a number gives: the same
/// number, or none. A plan that takes a text as an input reads it by such a cast.
pub fn text_reads_plainly(text: &str, date1904: bool) -> bool {
    match (text_as_number(text, date1904), text.parse::<f64>().ok()) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// What Excel reads a text as in arithmetic (US settings): a number, a percentage, or a
/// date written as `1/31/2001`, `31-JAN-2001`, `Jan 31, 2001` or `2001-01-31`.
fn text_as_number(text: &str, date1904: bool) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let plain = |t: &str| -> Option<f64> {
        let ok = t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'));
        if ok && t.bytes().any(|b| b.is_ascii_digit()) { t.parse::<f64>().ok().filter(|v| v.is_finite()) } else { None }
    };
    if let Some(v) = plain(t) {
        return Some(v);
    }
    if let Some(v) = t.strip_suffix('%').and_then(plain) {
        return Some(v / 100.0);
    }
    let lower = t.to_ascii_lowercase();
    let number = |s: &str| s.parse::<i64>().ok().map(|n| (n, s.len()));
    let month_of = |s: &str| MONTHS.iter().position(|m| s.len() >= 3 && s.starts_with(m)).map(|i| i as i64 + 1);
    // 1/31/2001, 1-31-2001, 2001-01-31
    let parts: Vec<&str> = lower.split(|c| c == '/' || c == '-').collect();
    if parts.len() == 3 {
        if let (Some((a, la)), Some((b, _)), Some((c, lc))) = (number(parts[0]), number(parts[1]), number(parts[2])) {
            if la == 4 {
                return serial(a, b, c, date1904);
            }
            return serial(two_digit_year(c, lc), a, b, date1904);
        }
        // 31-jan-2001
        if let (Some((d, _)), Some(m), Some((y, ly))) = (number(parts[0]), month_of(parts[1]), number(parts[2])) {
            return serial(two_digit_year(y, ly), m, d, date1904);
        }
    }
    // jan 31, 2001
    let words: Vec<&str> = lower.split(|c: char| c == ' ' || c == ',').filter(|w| !w.is_empty()).collect();
    if words.len() == 3 {
        if let (Some(m), Some((d, _)), Some((y, ly))) = (month_of(words[0]), number(words[1]), number(words[2])) {
            return serial(two_digit_year(y, ly), m, d, date1904);
        }
    }
    None
}

pub(crate) fn text_column_name(sheet: u32, col: u32) -> String {
    format!("s{sheet}c{col}t")
}

/// The sheet row of each frame row, 1-based.
pub(crate) const ROW: &str = "_row";
/// The run's own column, number and text parts, while it is computed by iteration.
const OWN_NUM: &str = "_own";
const OWN_TEXT: &str = "_ownt";

type Lifted<V> = Result<V, String>;

fn unquote(literal: &str) -> String {
    let inner = literal.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(literal);
    inner.replace("\"\"", "\"")
}

/// The terms of a chain `a + b - c …` with their signs (true: subtracted).
fn terms(e: &Fx) -> Option<Vec<(bool, &Fx)>> {
    let Fx::Binary { op: op @ (BinaryOp::Add | BinaryOp::Sub), lhs, rhs, .. } = e else { return None };
    let mut out: Vec<(bool, &Fx)> = vec![(matches!(op, BinaryOp::Sub), rhs.as_ref())];
    let mut left: &Fx = lhs.as_ref();
    while let Fx::Binary { op: o @ (BinaryOp::Add | BinaryOp::Sub), lhs: l, rhs: r, .. } = left.unspaced() {
        out.push((matches!(o, BinaryOp::Sub), r.as_ref()));
        left = l.as_ref();
    }
    out.push((false, left));
    out.reverse();
    Some(out)
}

/// Without spaces, parentheses and a leading plus.
fn core(e: &Fx) -> &Fx {
    match e {
        Fx::Spaced { expr, .. } | Fx::Paren(expr) => core(expr),
        Fx::Unary { op: '+', expr } => core(expr),
        other => other,
    }
}

fn floor_div(a: Expr, b: f64) -> Expr {
    (a / lit(b)).floor()
}

/// Year, month and day of a day count since 1970-01-01 (proleptic Gregorian).
fn civil(lets: &mut Lets, days: Expr) -> (Expr, Expr, Expr) {
    let z = bind(lets, days + lit(719_468.0));
    let era = bind(lets, floor_div(z.clone(), 146_097.0));
    let doe = bind(lets, z - era.clone() * lit(146_097.0));
    let yoe = bind(lets, floor_div(doe.clone() - floor_div(doe.clone(), 1460.0) + floor_div(doe.clone(), 36_524.0) - floor_div(doe.clone(), 146_096.0), 365.0));
    let doy = bind(lets, doe - (lit(365.0) * yoe.clone() + floor_div(yoe.clone(), 4.0) - floor_div(yoe.clone(), 100.0)));
    let mp = bind(lets, floor_div(lit(5.0) * doy.clone() + lit(2.0), 153.0));
    let day = doy - floor_div(lit(153.0) * mp.clone() + lit(2.0), 5.0) + lit(1.0);
    let month = bind(lets, when(mp.clone().lt(lit(10.0))).then(mp.clone() + lit(3.0)).otherwise(mp - lit(9.0)));
    let year = yoe + era * lit(400.0) + when(month.clone().lt_eq(lit(2.0))).then(lit(1.0)).otherwise(lit(0.0));
    (year, month, day)
}

/// Day count since 1970-01-01 of a year, month (any number, carried into the year)
/// and day (any number, counted on from the first of the month).
fn days_from_civil(lets: &mut Lets, year: Expr, month: Expr, day: Expr) -> Expr {
    let month = bind(lets, month);
    let carry = bind(lets, floor_div(month.clone() - lit(1.0), 12.0));
    let m = bind(lets, month - lit(1.0) - carry.clone() * lit(12.0) + lit(1.0));
    let y = bind(lets, year + carry - when(m.clone().lt_eq(lit(2.0))).then(lit(1.0)).otherwise(lit(0.0)));
    let era = bind(lets, floor_div(y.clone(), 400.0));
    let yoe = bind(lets, y - era.clone() * lit(400.0));
    let shifted = when(m.clone().gt(lit(2.0))).then(m.clone() - lit(3.0)).otherwise(m + lit(9.0));
    let doy = floor_div(lit(153.0) * shifted + lit(2.0), 5.0) + day - lit(1.0);
    let doe = yoe.clone() * lit(365.0) + floor_div(yoe.clone(), 4.0) - floor_div(yoe, 100.0) + doy;
    era * lit(146_097.0) + doe - lit(719_468.0)
}

impl Context<'_> {
    /// The sheet a reference names: its own without a prefix, another sheet of the
    /// workbook by name. Other workbooks and sheet ranges are not followed.
    fn sheet_of(&self, prefix: &Option<String>) -> Lifted<u32> {
        let Some(p) = prefix else { return Ok(self.sheet) };
        let name = p.strip_suffix('!').unwrap_or(p);
        let name = match name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')) {
            Some(quoted) => quoted.replace("''", "'"),
            None => name.to_string(),
        };
        match self.wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(&name)) {
            Some(i) => Ok(i as u32),
            None if name.starts_with('[') => Err("reference to another workbook that is not kept in the file".into()),
            None if name.contains(':') => Err("reference across several sheets".into()),
            None => Err("reference to an unknown sheet".into()),
        }
    }

    /// What was taken from the workbook beyond the inputs.
    fn assumptions(&self) -> Assumptions {
        let inputs = self.cell_inputs.borrow();
        let mut cells: Vec<(u32, u32, u32)> = self.reads.borrow().iter().copied().collect();
        cells.sort_unstable();
        let mut pinned: Vec<Rows> = Vec::new();
        for (sheet, row, col) in cells {
            match pinned.last_mut() {
                Some(r) if r.sheet == sheet && r.col == col && r.last + 1 == row => r.last = row,
                _ => pinned.push(Rows { sheet, col, first: row, last: row }),
            }
        }
        for &(sheet, r1, r2, c1, c2) in self.pinned_ranges.borrow().iter() {
            for c in c1..=c2 {
                pinned.push(Rows { sheet, col: c - 1, first: r1 - 1, last: r2 - 1 });
            }
        }
        Assumptions {
            ranges: self.range_inputs.borrow().iter().map(|(&k, &e)| (k, e)).collect(),
            pinned,
            extents: self.extents.borrow().iter().copied().collect(),
            cell_inputs: inputs.iter().map(|(&(s, r, c), &t)| (s, r, c, t)).collect(),
            columns: self.column_kinds.values().copied().collect(),
        }
    }

    /// A cell by place, noted as read: its value or kind shapes what is translated.
    fn peek(&self, key: &(u32, u32, u32)) -> Option<&usize> {
        self.reads.borrow_mut().insert(*key);
        self.at.get(key)
    }

    fn peek_has(&self, key: &(u32, u32, u32)) -> bool {
        self.peek(key).is_some()
    }

    /// The last row holding a cell of a sheet (1-based), noted as read.
    fn extent(&self, sheet: u32) -> u32 {
        self.extents.borrow_mut().insert(sheet);
        self.last_rows[sheet as usize]
    }

    /// A fixed cell.
    fn constant(&self, sheet: u32, row: u32, col: u32) -> Lifted<T> {
        self.baked.set(self.baked.get() | BAKED_CELL);
        if self.param.get() {
            // A number or a text is an input of the plan; what the cell holds then is for
            // the caller to supply. Another kind is taken as it is.
            let found = self.at.get(&(sheet, row - 1, col - 1)).copied();
            let kind = found.map(|i| self.wb.cells.kind[i]);
            let text = matches!(kind, Some(Kind::Text));
            // A text that Excel reads as a number in another way than a plain cast does (a
            // date, a percentage) is taken as it is: the plan would read it differently.
            let plain = !text || found.is_some_and(|i| text_reads_plainly(self.wb.cells.text[i].as_deref().unwrap_or(""), self.wb.date1904));
            if plain && matches!(kind, Some(Kind::Number | Kind::Bool | Kind::Text)) && !(sheet >= self.external_from) {
                self.cell_inputs.borrow_mut().insert((sheet, row - 1, col - 1), text);
                let name = cell_input_name(sheet, row - 1, col - 1, text);
                return Ok(if text { T::Text(col_expr(&name)) } else { T::Num(col_expr(&name)) });
            }
        }
        let empty = T::Dyn(Dyn { num: null_num(), text: None, blank: true, nan: false, parsed: None });
        Ok(match self.peek(&(sheet, row - 1, col - 1)) {
            // Another workbook keeps only some of its cells in this file: an absent one is
            // unknown, not empty.
            None if sheet >= self.external_from => return Err("cells of another workbook not kept in the file".into()),
            None => empty,
            Some(&i) => match self.wb.cells.kind[i] {
                Kind::Blank if self.wb.cells.formula[i].is_some() => T::Text(lit("")),
                Kind::Blank => empty,
                Kind::Number | Kind::Bool => T::Num(lit(self.wb.cells.num[i])),
                Kind::Text => T::Text(lit(self.wb.cells.text[i].as_deref().unwrap_or("").to_string())),
                Kind::Error => T::Num(null_num()),
            },
        })
    }

    /// Sum, count, minimum and maximum of the numbers in a fixed range (blanks and
    /// text do not take part, as in Excel's aggregates). Looked up once per workbook.
    fn constant_range(&self, sheet: u32, p: CellRef, q: CellRef) -> Lifted<Summary> {
        let (r1, r2, c1, c2) = (p.row.min(q.row), p.row.max(q.row), p.col.min(q.col), p.col.max(q.col));
        self.not_own_cells(Block { sheet, r1, r2, c1, c2 })?;
        if sheet >= self.external_from && !(r1..=r2).any(|r| (c1..=c2).any(|c| self.at.contains_key(&(sheet, r - 1, c - 1)))) {
            return Err("cells of another workbook not kept in the file".into());
        }
        let key = (sheet, r1, r2, c1, c2);
        if let Some(known) = self.ranges.borrow().get(&key) {
            return known.clone();
        }
        let result = if (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64 > 2_000_000 { Err("very large fixed range".to_string()) } else { Ok(summarize(self.wb, self.at, key)) };
        self.ranges.borrow_mut().insert(key, result.clone());
        result
    }

    /// The aggregates of a fixed range, and whether it holds an error. The values are
    /// constants of the expression, or (with `param`) inputs of the plan.
    fn range_agg(&self, sheet: u32, p: CellRef, q: CellRef) -> Lifted<(Agg, bool)> {
        let summary = self.constant_range(sheet, p, q)?;
        let key = (sheet, p.row.min(q.row), p.row.max(q.row), p.col.min(q.col), p.col.max(q.col));
        if !self.param.get() || sheet >= self.external_from {
            self.baked.set(self.baked.get() | BAKED_RANGE);
            self.pinned_ranges.borrow_mut().insert(key);
            return Ok((Agg::baked(&summary), summary.error));
        }
        self.range_inputs.borrow_mut().insert(key, summary.error);
        let name = |what: char| col_expr(&range_input_name(key, what));
        Ok((Agg { sum: name('s'), count: name('n'), min: name('l'), max: name('h'), sum_squares: name('q'), empty: None }, summary.error))
    }

    /// Whether the cells of a column in the given sheet rows (0-based, inclusive)
    /// hold text, and whether they hold errors.
    fn holds(&self, sheet: u32, col: u32, first: i64, last: i64) -> (bool, bool) {
        holds_in(self.wb, self.at, sheet, col, first, last)
    }

    /// A column of some sheet at a row offset from the formula's row: the numbers
    /// (null where empty or text, NaN where an error), and what else the rows hold.
    fn column(&mut self, sheet: u32, col: u32, offset: i64) -> Lifted<Dyn> {
        if self.in_name > 0 {
            return Err("relative reference in a defined name".into());
        }
        if sheet == self.sheet && col == self.run_col {
            if offset == 0 {
                return Err("circular reference: the formula reads its own cell".into());
            }
            if !self.recursive {
                return Err("reference to its own column".into());
            }
            let (lo, hi) = self.own.unwrap_or((offset, offset));
            let (lo, hi) = (lo.min(offset), hi.max(offset));
            // Only references that land inside the run can form a cycle.
            if offset.abs() < self.len as i64 {
                let dir = offset.signum() as i8;
                if self.own_dir != 0 && self.own_dir != dir {
                    return Err("circular reference: the column reads rows above and below".into());
                }
                self.own_dir = dir;
            }
            if false {
                return Err("circular reference: the column reads rows above and below".into());
            }
            if offset.abs() > 10_000 {
                return Err("reference to a far row".into());
            }
            self.own = Some((lo, hi));
            self.reach = (self.reach.0.min(offset), self.reach.1.max(offset));
            let at_offset = |e: Expr| e.shift(lit(-offset));
            return Ok(Dyn { num: at_offset(col_expr(OWN_NUM)), text: Some(at_offset(col_expr(OWN_TEXT))), blank: true, nan: true, parsed: None });
        }
        if offset.abs() > 10_000 {
            return Err("reference to a far row".into());
        }
        let first = self.row as i64 - 1 + offset;
        if sheet >= self.external_from && (first..first + self.len as i64).all(|r| r < 0 || !self.peek_has(&(sheet, r as u32, col - 1))) {
            return Err("cells of another workbook not kept in the file".into());
        }
        self.columns.insert((sheet, col));
        self.reach = (self.reach.0.min(offset), self.reach.1.max(offset));
        let (has_text, has_error) = self.holds(sheet, col, first, first + self.len as i64 - 1);
        let last = first + self.len as i64 - 1;
        let kinds = self.column_kinds.entry((sheet, col)).or_insert(ColumnKinds { sheet, col, first, last, text: false, error: false });
        *kinds = ColumnKinds { sheet, col, first: kinds.first.min(first), last: kinds.last.max(last), text: kinds.text || has_text, error: kinds.error || has_error };
        let at_offset = |e: Expr| if offset == 0 { e } else { e.shift(lit(-offset)) };
        let text = has_text.then(|| {
            self.text_columns.insert((sheet, col));
            at_offset(col_expr(&text_column_name(sheet, col)))
        });
        let parsed = text.is_some().then(|| {
            self.parsed_columns.insert((sheet, col));
            at_offset(col_expr(&parsed_column_name(sheet, col)))
        });
        Ok(Dyn { num: at_offset(col_expr(&column_name(sheet, col))), text, blank: true, nan: has_error, parsed })
    }

    /// A single-cell reference: a column at a row offset, or a constant.
    fn cell(&mut self, prefix: &Option<String>, text: &str) -> Lifted<T> {
        let sheet = self.sheet_of(prefix)?;
        let r = parse_cell_ref(text).ok_or("reference form")?;
        if r.abs_row {
            if self.in_name > 0 && !r.abs_col {
                return Err("relative reference in a defined name".into());
            }
            return self.constant(sheet, r.row, r.col);
        }
        Ok(T::Dyn(self.column(sheet, r.col, r.row as i64 - self.row as i64)?))
    }

    /// The formula behind a defined name of this sheet or of the workbook.
    fn name(&self, name: &str) -> Lifted<Fx> {
        let names = &self.wb.defined_names;
        // `[1]!Name`: a name of another workbook; `Sheet!Name`: a name of that sheet.
        let found = if name.starts_with('[') || name.starts_with("'[") {
            names.iter().find(|n| n.local_sheet.is_none() && n.name.eq_ignore_ascii_case(name.trim_start_matches('\'')))
        } else if let Some((sheet, local)) = name.rsplit_once('!') {
            let sheet = self.sheet_of(&Some(format!("{sheet}!")))?;
            names.iter().find(|n| n.local_sheet == Some(sheet) && n.name.eq_ignore_ascii_case(local))
        } else {
            names
                .iter()
                .find(|n| n.local_sheet == Some(self.sheet) && n.name.eq_ignore_ascii_case(name))
                .or_else(|| names.iter().find(|n| n.local_sheet.is_none() && n.name.eq_ignore_ascii_case(name)))
        };
        let found = match found {
            Some(f) => f,
            None if name.starts_with('[') || name.starts_with("'[") => return Err("name of another workbook not kept in the file".into()),
            None => return Err("unknown name".into()),
        };
        if self.in_name > 8 {
            return Err("names nested too deep".into());
        }
        parse(found.value.strip_prefix('=').unwrap_or(&found.value)).map_err(|_| "defined name not parsed".to_string())
    }

    /// Whether an expression means the same in every cell of the run: it reads no cell
    /// that moves with the row.
    fn is_fixed(&self, e: &Fx, depth: u32) -> bool {
        match e {
            Fx::Spaced { expr, .. } | Fx::Paren(expr) | Fx::Postfix { expr, .. } => self.is_fixed(expr, depth),
            Fx::Number(_) | Fx::Str(_) | Fx::Bool(_) | Fx::Error(_) | Fx::Missing => true,
            Fx::Ref { kind: RefKind::Cell, text, .. } => parse_cell_ref(text).is_some_and(|r| r.abs_row),
            Fx::Name { prefix, text } => depth < 8 && self.name(&qualified(prefix, text)).is_ok_and(|f| self.is_fixed(&f, depth + 1)),
            Fx::Unary { op, expr } => *op != '@' && self.is_fixed(expr, depth),
            Fx::Binary { lhs, rhs, .. } => self.is_fixed(lhs, depth) && self.is_fixed(rhs, depth),
            Fx::Call { name, args } => {
                !matches!(name.to_ascii_uppercase().as_str(), "ROW" | "COLUMN" | "RAND" | "RANDBETWEEN" | "NOW" | "TODAY" | "CELL" | "INDIRECT") && args.iter().all(|a| self.is_fixed(a, depth))
            }
            _ => false,
        }
    }

    /// The number a fixed expression gives, worked out now with the same rules as
    /// everything else; `None` if it gives an error value.
    fn fixed_number(&mut self, e: &Fx) -> Lifted<Option<f64>> {
        // Its value decides the translation: the cells it reads are constants, noted as read.
        let was = self.param.replace(false);
        let r = self.fixed_number_in(e);
        self.param.set(was);
        r
    }

    fn fixed_number_in(&mut self, e: &Fx) -> Lifted<Option<f64>> {
        self.baked.set(self.baked.get() | BAKED_DECISION);
        let read = self.columns.len();
        let saved = std::mem::take(&mut self.lets);
        let translated = self.operand(e).map(|t| t.num());
        let lets = std::mem::replace(&mut self.lets, saved);
        let expr = translated?;
        if self.columns.len() != read {
            return Err("fixed expression that reads the row".into());
        }
        let out = fixed_value(self.ex, self.row, lets, expr.cast(DataType::Float64)).map_err(|e| format!("polars: {}", e.lines().next().unwrap_or("")))?;
        let value = out.f64(0);
        Ok(value.filter(|v| v.is_finite()))
    }

    /// The reference an `OFFSET` starts from: a cell, a range, or a name for one.
    fn anchor(&mut self, e: &Fx) -> Lifted<(u32, CellRef, CellRef)> {
        match core(e) {
            Fx::Name { prefix, text } => {
                let formula = self.name(&qualified(prefix, text))?;
                self.in_name += 1;
                let result = self.anchor(&formula);
                self.in_name -= 1;
                result
            }
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                let r = parse_cell_ref(text).ok_or("reference form")?;
                if self.in_name > 0 && !(r.abs_row && r.abs_col) {
                    return Err("relative reference in a defined name".into());
                }
                Ok((self.sheet_of(prefix)?, r, r))
            }
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (lhs.unspaced(), rhs.unspaced()) else {
                    return Err("range form".into());
                };
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                if self.in_name > 0 && !(p.abs_row && p.abs_col && q.abs_row && q.abs_col) {
                    return Err("relative reference in a defined name".into());
                }
                if p.abs_row != q.abs_row {
                    return Err("OFFSET from a range fixed at one end".into());
                }
                Ok((self.sheet_of(prefix)?, p, q))
            }
            _ => Err("OFFSET from a computed reference".into()),
        }
    }

    /// Where `OFFSET(reference, rows, cols, [height], [width])` points. Offsets and
    /// sizes are cut to whole numbers as Excel does.
    fn offset(&mut self, args: &[Fx]) -> Lifted<Target> {
        if !(3..=5).contains(&args.len()) {
            return Err(format!("OFFSET with {} arguments", args.len()));
        }
        let (sheet, p, q) = self.anchor(&args[0])?;
        let (top, left) = (p.row.min(q.row) as i64, p.col.min(q.col) as i64);
        let mut size = [(p.row.max(q.row) as i64 - top + 1), (p.col.max(q.col) as i64 - left + 1)];
        for (k, arg) in args.iter().skip(3).enumerate() {
            if matches!(core(arg), Fx::Missing) {
                continue;
            }
            if !self.is_fixed(arg, 0) {
                return Err("OFFSET with a size that changes with the row".into());
            }
            let Some(v) = self.fixed_number(arg)? else { return Ok(Target::Nowhere) };
            if v.trunc() < 1.0 {
                return Err("OFFSET with a size below 1".into());
            }
            size[k] = v.trunc() as i64;
        }
        let single = size == [1, 1];
        // Each offset: a number known now, or an expression per row.
        let mut fixed = [Some(0i64), Some(0i64)];
        let mut moving: [Option<Expr>; 2] = [None, None];
        for k in 0..2 {
            let arg = &args[k + 1];
            if matches!(core(arg), Fx::Missing) {
                continue;
            }
            if self.is_fixed(arg, 0) {
                let Some(v) = self.fixed_number(arg)? else { return Ok(Target::Nowhere) };
                if v.abs() > 2e6 {
                    return Ok(Target::Nowhere);
                }
                fixed[k] = Some(v.trunc() as i64);
            } else {
                let x = self.number(arg)?;
                fixed[k] = None;
                moving[k] = Some(when(x.clone().lt(lit(0.0))).then(x.clone().ceil()).otherwise(x.floor()));
            }
        }
        let relative = !p.abs_row;
        match (fixed, relative) {
            ([Some(r), Some(c)], false) => {
                let (r1, c1) = (top + r, left + c);
                if r1 < 1 || c1 < 1 || r1 + size[0] - 1 > SHEET_ROWS || c1 + size[1] - 1 > SHEET_COLS {
                    return Ok(Target::Nowhere);
                }
                Ok(Target::Fixed(Block { sheet, r1: r1 as u32, r2: (r1 + size[0] - 1) as u32, c1: c1 as u32, c2: (c1 + size[1] - 1) as u32 }))
            }
            ([Some(r), Some(c)], true) => {
                let c1 = left + c;
                if c1 < 1 || c1 + size[1] - 1 > SHEET_COLS {
                    return Ok(Target::Nowhere);
                }
                Ok(Target::Shifted { sheet, col: c1 as u32, offset: top - self.row as i64 + r, height: size[0] as u32, width: size[1] as u32 })
            }
            ([None, Some(c)], _) if single => {
                let line = left + c;
                if !(1..=SHEET_COLS).contains(&line) {
                    return Ok(Target::Nowhere);
                }
                let rows = moving[0].take().expect("moving rows");
                let index = if relative { col_expr(ROW).cast(DataType::Float64) + lit((top - self.row as i64) as f64) + rows } else { lit(top as f64) + rows };
                Ok(Target::Gather { sheet, vertical: true, line: line as u32, index })
            }
            ([Some(r), None], false) if single => {
                let line = top + r;
                if !(1..=SHEET_ROWS).contains(&line) {
                    return Ok(Target::Nowhere);
                }
                Ok(Target::Gather { sheet, vertical: false, line: line as u32, index: lit(left as f64) + moving[1].take().expect("moving columns") })
            }
            ([None, None], _) => Err("OFFSET moving in both directions".into()),
            ([Some(_), None], true) => Err("OFFSET moving sideways from a moving cell".into()),
            _ => Err("OFFSET range that moves with a computed offset".into()),
        }
    }

    /// The cell of a fixed column (or row) at a position computed per row: the cells
    /// of that line are known, so picking one is a replacement of positions by values.
    fn gather(&mut self, sheet: u32, vertical: bool, line: u32, index: Expr) -> Lifted<T> {
        if vertical && sheet == self.sheet && line == self.run_col {
            return Err("reference to its own column".into());
        }
        let c = &self.wb.cells;
        let length = if vertical { self.extent(sheet) } else { SHEET_COLS as u32 };
        let (mut keys, mut numbers, mut texts) = (Vec::new(), Vec::new(), Vec::new());
        for k in 1..=length {
            let (row, col) = if vertical { (k, line) } else { (line, k) };
            let Some(&i) = self.peek(&(sheet, row - 1, col - 1)) else { continue };
            let (number, text) = match c.kind[i] {
                Kind::Blank if c.formula[i].is_some() => (None, Some("")),
                Kind::Blank => continue,
                Kind::Number | Kind::Bool => (Some(c.num[i]), None),
                Kind::Text => (None, Some(c.text[i].as_deref().unwrap_or(""))),
                Kind::Error => (Some(f64::NAN), None),
            };
            if !vertical && sheet == self.sheet && col == self.run_col && row >= self.row && row < self.row + self.len {
                return Err("reference to its own column".into());
            }
            keys.push(k as f64);
            numbers.push(number);
            texts.push(text);
        }
        let any_text = texts.iter().any(|t| t.is_some());
        let index = bind(&mut self.lets, index);
        let limit = if vertical { SHEET_ROWS } else { SHEET_COLS };
        let valid = bind(&mut self.lets, index.clone().is_not_null().and(index.clone().gt_eq(lit(1.0))).and(index.clone().lt_eq(lit(limit as f64))));
        // A position without a cell is an empty cell; a position outside the sheet is #REF!.
        let (number, text) = if keys.is_empty() {
            (null_num(), null_text())
        } else {
            let old = Series::new("k".into(), keys);
            (
                index.clone().replace_strict(lit(old.clone()), lit(Series::new("n".into(), numbers)), Some(null_num()), Some(DataType::Float64)),
                index.replace_strict(lit(old), lit(Series::new("t".into(), texts)), Some(null_text()), Some(DataType::String)),
            )
        };
        let num = bind(&mut self.lets, when(valid.clone()).then(number).otherwise(lit(f64::NAN)));
        let text = any_text.then(|| bind(&mut self.lets, when(valid).then(text).otherwise(null_text())));
        Ok(T::Dyn(Dyn { num, text, blank: true, nan: true, parsed: None }))
    }

    /// The fixed block a lookup searches: a range fixed in its rows, whole columns
    /// (as far as the sheet holds cells), or a name for one of these.
    fn block(&mut self, e: &Fx) -> Lifted<Block> {
        match core(e) {
            Fx::Name { prefix, text: name } => {
                let formula = self.name(&qualified(prefix, name))?;
                self.in_name += 1;
                let result = self.block(&formula);
                self.in_name -= 1;
                result
            }
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (lhs.unspaced(), rhs.unspaced()) else {
                    return Err("range form".into());
                };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                if !(p.abs_row && q.abs_row) {
                    return Err("lookup table that moves with the row".into());
                }
                if self.in_name > 0 && !(p.abs_col && q.abs_col) {
                    return Err("relative reference in a defined name".into());
                }
                // As written: rows past the last cell are empty, but they are part of the block
                // (COUNTIF of empty cells counts them). Very tall blocks are cut at the last cell.
                // (Only then does the translation depend on how far the sheet reaches.)
                let r2 = p.row.max(q.row);
                let r2 = if r2 - p.row.min(q.row) > 100_000 { r2.min(self.extent(sheet).max(1)).max(p.row.min(q.row)) } else { r2 };
                Ok(Block { sheet, r1: p.row.min(q.row), r2, c1: p.col.min(q.col), c2: p.col.max(q.col) })
            }
            Fx::Ref { prefix, kind: RefKind::Columns, text } => {
                let sheet = self.sheet_of(prefix)?;
                let (a, b) = text.split_once(':').ok_or("column range form")?;
                let (c1, c2) = (column_number(a).ok_or("column range form")?, column_number(b).ok_or("column range form")?);
                Ok(Block { sheet, r1: 1, r2: self.extent(sheet).max(1), c1: c1.min(c2), c2: c1.max(c2) })
            }
            _ => Err("lookup table form".into()),
        }
    }

    /// One column of an aggregate's range at a row offset: errors poison the result.
    fn rows_item(&mut self, out: &mut Spread, sheet: u32, col: u32, offset: i64) -> Lifted<()> {
        let d = self.column(sheet, col, offset)?;
        if d.nan {
            out.poison(d.num.clone().is_nan().fill_null(lit(false)));
            out.items.push(Item::Rows(when(d.num.clone().is_nan()).then(null_num()).otherwise(d.num)));
        } else {
            out.items.push(Item::Rows(d.num));
        }
        Ok(())
    }

    /// The arguments of an aggregate.
    fn spread(&mut self, args: &[Fx]) -> Lifted<Spread> {
        let mut out = Spread { items: Vec::new(), error: None };
        for arg in args {
            self.spread_one(arg, &mut out)?;
        }
        if out.items.is_empty() {
            return Err("aggregate without arguments".into());
        }
        Ok(out)
    }

    fn spread_one(&mut self, arg: &Fx, out: &mut Spread) -> Lifted<()> {
        // Sheet1:Sheet3!A1 is A1 on each sheet from the first to the last.
        if let Some(each) = self.across_sheets(arg)? {
            for one in each {
                self.spread_one(&one, out)?;
            }
            return Ok(());
        }
        match arg.unspaced() {
            // Parentheses around a reference leave it a reference; a union gives each of its parts.
            Fx::Paren(inner) if matches!(core(inner), Fx::Name { .. } | Fx::Ref { .. } | Fx::Binary { op: BinaryOp::Range, .. } | Fx::Paren(_)) => self.spread_one(inner, out)?,
            _ if matches!(core(arg), Fx::Binary { op: BinaryOp::Union, .. }) => {
                let Fx::Binary { lhs, rhs, .. } = core(arg) else { unreachable!() };
                self.spread_one(lhs, out)?;
                self.spread_one(rhs, out)?;
            }
            Fx::Name { prefix, text: name } => {
                let formula = self.name(&qualified(prefix, name))?;
                self.in_name += 1;
                let result = self.spread_one(&formula, out);
                self.in_name -= 1;
                result?
            }
            Fx::Call { name, args } if name.eq_ignore_ascii_case("OFFSET") => match self.offset(args)? {
                Target::Nowhere => {
                    out.poison(lit(true));
                    out.items.push(Item::Rows(null_num()));
                }
                Target::Fixed(b) => {
                    let corner = |row, col| CellRef { row, col, abs_row: true, abs_col: true };
                    let (agg, error) = self.range_agg(b.sheet, corner(b.r1, b.c1), corner(b.r2, b.c2))?;
                    if error {
                        out.poison(lit(true));
                    }
                    out.items.push(Item::Fixed(agg));
                }
                Target::Shifted { sheet, col, offset, height, width } => {
                    if height as u64 * width as u64 > 400 {
                        return Err("large moving range".into());
                    }
                    if (self.row as i64 + offset) < 1 {
                        out.poison(lit(true));
                        out.items.push(Item::Rows(null_num()));
                    } else {
                        for r in 0..height as i64 {
                            for c in 0..width {
                                self.rows_item(out, sheet, col + c, offset + r)?;
                            }
                        }
                    }
                }
                Target::Gather { sheet, vertical, line, index } => {
                    // Text and empty cells do not take part, an error is the result.
                    let T::Dyn(d) = self.gather(sheet, vertical, line, index)? else { unreachable!("gather gives a cell") };
                    out.poison(d.num.clone().is_nan().fill_null(lit(false)));
                    out.items.push(Item::Rows(when(d.num.clone().is_nan()).then(null_num()).otherwise(d.num)));
                }
            },
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if !matches!((lhs.unspaced(), rhs.unspaced()), (Fx::Ref { kind: RefKind::Cell, .. }, Fx::Ref { prefix: None, kind: RefKind::Cell, .. })) => {
                // A chain A1:B1:C1 is the block around its cells; ends given by names or
                // OFFSET are fixed blocks.
                if let Some(chain) = range_chain(arg) {
                    return self.spread_one(&chain, out);
                }
                match self.table(arg)? {
                    None => {
                        out.poison(lit(true));
                        out.items.push(Item::Rows(null_num()));
                    }
                    Some(b) => {
                        let corner = |row, col| CellRef { row, col, abs_row: true, abs_col: true };
                        let (agg, error) = self.range_agg(b.sheet, corner(b.r1, b.c1), corner(b.r2, b.c2))?;
                        if error {
                            out.poison(lit(true));
                        }
                        out.items.push(Item::Fixed(agg));
                    }
                }
            }
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (lhs.unspaced(), rhs.unspaced()) else {
                    return Err("range form".into());
                };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                let (c1, c2) = (p.col.min(q.col), p.col.max(q.col));
                if p.abs_row && q.abs_row {
                    if self.in_name > 0 && !(p.abs_col && q.abs_col) {
                        return Err("relative reference in a defined name".into());
                    }
                    let (agg, error) = self.range_agg(sheet, p, q)?;
                    if error {
                        out.poison(lit(true));
                    }
                    out.items.push(Item::Fixed(agg));
                } else if !p.abs_row && !q.abs_row {
                    // A block that moves with the row: every cell of it is a column at an offset.
                    let (r1, r2) = (p.row.min(q.row), p.row.max(q.row));
                    if (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64 > 400 {
                        return Err("large moving range".into());
                    }
                    for r in r1..=r2 {
                        for c in c1..=c2 {
                            self.rows_item(out, sheet, c, r as i64 - self.row as i64)?;
                        }
                    }
                } else {
                    // Fixed at one end: the range grows with the row.
                    let (anchor, moving) = if p.abs_row { (p.row, q.row) } else { (q.row, p.row) };
                    if moving < anchor {
                        return Err("range that grows upwards".into());
                    }
                    for c in c1..=c2 {
                        self.growing(out, sheet, c, anchor, moving as i64 - self.row as i64)?;
                    }
                }
            }
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                let r = parse_cell_ref(text).ok_or("reference form")?;
                if r.abs_row {
                    // A fixed cell: text and empty cells do not take part.
                    let sheet = self.sheet_of(prefix)?;
                    if self.in_name > 0 && !r.abs_col {
                        return Err("relative reference in a defined name".into());
                    }
                    let (agg, error) = self.range_agg(sheet, r, r)?;
                    if error {
                        out.poison(lit(true));
                    }
                    out.items.push(Item::Fixed(agg));
                } else {
                    let sheet = self.sheet_of(prefix)?;
                    self.rows_item(out, sheet, r.col, r.row as i64 - self.row as i64)?;
                }
            }
            _ if self.array && self.shape(arg)?.is_some() => {
                // An array over fixed ranges: computed now, element by element.
                if self.is_fixed(arg, 0) {
                    if let Ok(summary) = self.array_summary(arg) {
                        if summary.error {
                            out.poison(lit(true));
                        }
                        out.items.push(Item::Fixed(Agg::baked(&summary)));
                        return Ok(());
                    }
                }
                // An array formula: the expression is taken cell by cell over its ranges.
                if if_without_else(arg) && !self.summing {
                    return Err("array of IF without an else branch outside SUM".into());
                }
                for element in self.elements(arg)? {
                    if self.lets.len() > 6_000 {
                        return Err("large array formula: taken cell by cell it would be too large".into());
                    }
                    match self.translate(&element)? {
                        T::Num(e) => {
                            let e = bind(&mut self.lets, e);
                            out.poison(e.clone().is_null());
                            out.items.push(Item::Rows(e));
                        }
                        // Truth values and text in an array take no part.
                        T::Bool(_) | T::Text(_) => {}
                        T::Dyn(d) => {
                            out.poison(d.is_error());
                            let number = match &d.text {
                                Some(t) => when(t.clone().is_null()).then(d.num.clone()).otherwise(null_num()),
                                None => d.num.clone(),
                            };
                            out.items.push(Item::Rows(when(number.clone().is_nan()).then(null_num()).otherwise(number)));
                        }
                    }
                }
            }
            _ => {
                let value = self.translate(arg)?.num();
                out.poison(value.clone().is_null());
                out.items.push(Item::Rows(value));
            }
        }
        Ok(())
    }

    /// A column from a fixed sheet row down to the formula's row plus `offset`.
    fn growing(&mut self, out: &mut Spread, sheet: u32, col: u32, anchor: u32, offset: i64) -> Lifted<()> {
        if self.in_name > 0 {
            return Err("relative reference in a defined name".into());
        }
        if sheet == self.sheet && col == self.run_col {
            if offset >= 0 {
                return Err("circular reference: the formula reads its own cell".into());
            }
            if !self.recursive {
                return Err("reference to its own column".into());
            }
            // The column's own rows above, as found so far by the iteration.
            if self.own_dir > 0 {
                return Err("circular reference: the column reads rows above and below".into());
            }
            self.own_dir = -1;
            let lowest = anchor as i64 - self.row as i64;
            let (lo, hi) = self.own.unwrap_or((lowest, offset));
            self.own = Some((lo.min(lowest), hi.max(offset)));
            self.reach = (self.reach.0.min(lowest), self.reach.1.max(offset));
            let raw = when(col_expr(ROW).gt_eq(lit(anchor as i64))).then(col_expr(OWN_NUM)).otherwise(null_num());
            let sum = raw.clone().fill_null(lit(0.0)).cum_sum(false).shift(lit(-offset));
            out.poison(sum.clone().is_nan().fill_null(lit(false)));
            let count = raw.clone().is_not_null().cast(DataType::Float64).cum_sum(false).shift(lit(-offset));
            let seen = raw.clone().is_not_null().cast(DataType::Float64).cum_sum(false).gt(lit(0.0));
            let max = when(seen.clone()).then(raw.clone().fill_null(lit(f64::NEG_INFINITY)).cum_max(false)).otherwise(null_num()).shift(lit(-offset));
            let min = when(seen).then(raw.fill_null(lit(f64::INFINITY)).cum_min(false)).otherwise(null_num()).shift(lit(-offset));
            out.items.push(Item::Growing { sum, count, max, min });
            return Ok(());
        }
        let last = self.row as i64 - 1 + offset + self.len as i64 - 1;
        if last - anchor as i64 > 1_000_000 {
            return Err("very large growing range".into());
        }
        self.columns.insert((sheet, col));
        self.reach = (self.reach.0.min(anchor as i64 - self.row as i64), self.reach.1.max(offset));
        let (_, has_error) = self.holds(sheet, col, anchor as i64 - 1, last);
        let raw = when(col_expr(ROW).gt_eq(lit(anchor as i64))).then(col_expr(&column_name(sheet, col))).otherwise(null_num());
        let at_offset = |e: Expr| if offset == 0 { e } else { e.shift(lit(-offset)) };
        let sum = at_offset(raw.clone().fill_null(lit(0.0)).cum_sum(false));
        if has_error {
            // An error stays in the range from its row on.
            out.poison(sum.clone().is_nan().fill_null(lit(false)));
        }
        let count = at_offset(raw.clone().is_not_null().cast(DataType::Float64).cum_sum(false));
        // Empty cells take no part: they count as -inf (+inf) until a number has come.
        let seen = raw.clone().is_not_null().cast(DataType::Float64).cum_sum(false).gt(lit(0.0));
        let max = at_offset(when(seen.clone()).then(raw.clone().fill_null(lit(f64::NEG_INFINITY)).cum_max(false)).otherwise(null_num()));
        let min = at_offset(when(seen).then(raw.fill_null(lit(f64::INFINITY)).cum_min(false)).otherwise(null_num()));
        out.items.push(Item::Growing { sum, count, max, min });
        Ok(())
    }

    /// A formula that continues the cell above it: `=C1+B2` filled down is a running
    /// total, `=C1*B2` a running product. `None` if the formula is not of that form.
    fn recurrence(&mut self, e: &Fx) -> Lifted<Option<T>> {
        let is_previous = |ctx: &Self, x: &Fx| match core(x) {
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                ctx.sheet_of(prefix).is_ok_and(|s| s == ctx.sheet) && parse_cell_ref(text).is_some_and(|r| !r.abs_row && r.col == ctx.run_col && r.row + 1 == ctx.row)
            }
            _ => false,
        };
        let e = core(e);
        // (the step, whether steps multiply)
        let (step, multiply): (Expr, bool) = if is_previous(self, e) {
            (lit(0.0), false)
        } else if let Some(all) = terms(e) {
            let previous: Vec<usize> = (0..all.len()).filter(|&i| !all[i].0 && is_previous(self, all[i].1)).collect();
            if previous.len() != 1 {
                return Ok(None);
            }
            let mut sum = lit(0.0);
            for (i, (negative, term)) in all.iter().enumerate() {
                if i != previous[0] {
                    let value = self.translate(term)?.num();
                    sum = if *negative { sum - value } else { sum + value };
                }
            }
            (sum, false)
        } else if let Fx::Binary { op: op @ (BinaryOp::Mul | BinaryOp::Div), lhs, rhs, .. } = e {
            if is_previous(self, lhs) {
                let other = self.translate(rhs)?.num();
                (if matches!(op, BinaryOp::Div) { divide(&mut self.lets, lit(1.0), other) } else { other }, true)
            } else if matches!(op, BinaryOp::Mul) && is_previous(self, rhs) {
                (self.translate(lhs)?.num(), true)
            } else {
                return Ok(None);
            }
        } else {
            return Ok(None);
        };
        let step = bind(&mut self.lets, step);
        // The cell above the run starts it off.
        let start = if self.row < 2 {
            0.0
        } else {
            match self.peek(&(self.sheet, self.row - 2, self.run_col - 1)) {
                None => 0.0,
                Some(&i) => match self.wb.cells.kind[i] {
                    Kind::Blank => 0.0,
                    Kind::Number | Kind::Bool => self.wb.cells.num[i],
                    Kind::Text => return Ok(None),
                    Kind::Error => return Ok(None),
                },
            }
        };
        let first = lit(self.row as i64);
        let inside = col_expr(ROW).gt_eq(first.clone());
        let neutral = if multiply { 1.0 } else { 0.0 };
        let steps = when(inside.clone()).then(step.clone().fill_null(lit(neutral))).otherwise(lit(neutral));
        let begin = when(col_expr(ROW).eq(first)).then(lit(start)).otherwise(lit(neutral));
        let running = if multiply { (begin * steps).cum_prod(false) } else { (begin + steps).cum_sum(false) };
        // An error in a step stays in every row after it.
        let broken = when(inside).then(step.is_null().cast(DataType::Float64)).otherwise(lit(0.0)).cum_sum(false).gt(lit(0.0));
        Ok(Some(T::Num(when(broken).then(null_num()).otherwise(running))))
    }

    /// Excel's serial day number as days since 1970-01-01; null where there is no
    /// such day. The second value marks serial 0 and the day 1900-02-29 that Excel
    /// counts although it never was.
    fn serial_days(&mut self, serial: Expr) -> (Expr, Expr, Expr) {
        let s = bind(&mut self.lets, serial.floor());
        if self.wb.date1904 {
            let days = when(s.clone().lt(lit(0.0))).then(null_num()).otherwise(s - lit(24_107.0));
            return (days, lit(false), lit(false));
        }
        let days = when(s.clone().lt(lit(0.0)))
            .then(null_num())
            .otherwise(when(s.clone().lt(lit(61.0))).then(s.clone() - lit(25_568.0)).otherwise(s.clone() - lit(25_569.0)));
        (days, s.clone().eq(lit(0.0)), s.eq(lit(60.0)))
    }

    /// Error kinds and text found in a block of cells (rows 0-based, inclusive).
    fn scan(&self, sheet: u32, c1: u32, c2: u32, r1: i64, r2: i64) -> Info {
        let (r1, r2) = (r1.max(0), r2.max(-1));
        if (r2 - r1 + 1).max(0) as u64 * (c2 - c1 + 1) as u64 > 2_000_000 {
            return Info { err: NA | OTHER, text: true };
        }
        let mut info = Info::default();
        for r in r1..=r2 {
            for c in c1..=c2 {
                if let Some(&i) = self.peek(&(sheet, r as u32, c - 1)) {
                    match self.wb.cells.kind[i] {
                        Kind::Error => info.err |= if self.wb.cells.text[i].as_deref().is_some_and(|t| t.eq_ignore_ascii_case("#N/A")) { NA } else { OTHER },
                        Kind::Text => info.text = true,
                        Kind::Blank if self.wb.cells.formula[i].is_some() => info.text = true,
                        _ => {}
                    }
                }
            }
        }
        info
    }

    /// The fixed block a reference or name stands for, if it is one (for `kinds`).
    fn block_of(&self, e: &Fx) -> Option<Block> {
        match core(e) {
            Fx::Name { prefix, text } => self.name(&qualified(prefix, text)).ok().and_then(|f| self.block_of(&f)),
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (core(lhs), core(rhs)) else { return None };
                let sheet = self.sheet_of(prefix).ok()?;
                let (p, q) = (parse_cell_ref(a)?, parse_cell_ref(b)?);
                (p.abs_row && q.abs_row).then(|| Block { sheet, r1: p.row.min(q.row), r2: p.row.max(q.row), c1: p.col.min(q.col), c2: p.col.max(q.col) })
            }
            _ => None,
        }
    }

    /// Which error values an expression can give. An over-estimate: `ISNA` and `ISERR`
    /// are only translated where it leaves no doubt which of the two an error is.
    fn kinds(&self, e: &Fx) -> Lifted<Info> {
        let len = self.len as i64 - 1;
        let any = Info { err: NA | OTHER, text: true };
        Ok(match e {
            Fx::Spaced { expr, .. } | Fx::Paren(expr) => self.kinds(expr)?,
            Fx::Number(_) | Fx::Bool(_) => Info::default(),
            Fx::Str(_) => Info { err: 0, text: true },
            Fx::Error(t) => Info { err: if t.eq_ignore_ascii_case("#N/A") { NA } else { OTHER }, text: false },
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                let sheet = self.sheet_of(prefix)?;
                let r = parse_cell_ref(text).ok_or("reference form")?;
                if r.abs_row {
                    self.scan(sheet, r.col, r.col, r.row as i64 - 1, r.row as i64 - 1)
                } else {
                    self.scan(sheet, r.col, r.col, r.row as i64 - 1, r.row as i64 - 1 + len)
                }
            }
            Fx::Ref { prefix, kind: RefKind::Columns, text } => {
                let sheet = self.sheet_of(prefix)?;
                let (a, b) = text.split_once(':').ok_or("column range form")?;
                let (c1, c2) = (column_number(a).ok_or("column range form")?, column_number(b).ok_or("column range form")?);
                self.scan(sheet, c1.min(c2), c1.max(c2), 0, self.extent(sheet) as i64 - 1)
            }
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (lhs.unspaced(), rhs.unspaced()) else { return Ok(any) };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                let (c1, c2) = (p.col.min(q.col), p.col.max(q.col));
                let (lo, hi) = (p.row.min(q.row) as i64 - 1, p.row.max(q.row) as i64 - 1);
                match (p.abs_row, q.abs_row) {
                    (true, true) => self.scan(sheet, c1, c2, lo, hi),
                    (false, false) => self.scan(sheet, c1, c2, lo, hi + len),
                    _ => {
                        let anchor = if p.abs_row { p.row } else { q.row } as i64 - 1;
                        let moving = if p.abs_row { q.row } else { p.row } as i64 - 1;
                        self.scan(sheet, c1, c2, anchor.min(moving), anchor.max(moving) + len)
                    }
                }
            }
            Fx::Name { prefix, text } => self.kinds(&self.name(&qualified(prefix, text))?)?,
            Fx::Unary { expr, .. } | Fx::Postfix { expr, .. } => self.kinds(expr)?.arithmetic(),
            Fx::Binary { op, lhs, rhs, .. } => {
                let (a, b) = (self.kinds(lhs)?, self.kinds(rhs)?);
                match op {
                    BinaryOp::Concat => Info { err: a.err | b.err, text: true },
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => Info { err: a.err | b.err, text: false },
                    BinaryOp::Div | BinaryOp::Pow => Info { err: a.arithmetic().err | b.arithmetic().err | OTHER, text: false },
                    _ => Info { err: a.arithmetic().err | b.arithmetic().err, text: false },
                }
            }
            Fx::Call { name, args } => {
                let name = name.to_ascii_uppercase();
                let mut all = Info::default();
                let mut each = Vec::with_capacity(args.len());
                for a in args {
                    let k = self.kinds(a)?;
                    all = all.join(k);
                    each.push(k);
                }
                match name.as_str() {
                    "NA" => Info { err: NA, text: false },
                    "ISERROR" | "ISERR" | "ISNA" | "ISNUMBER" | "ISTEXT" | "ISNONTEXT" | "ISBLANK" | "COUNT" | "TRUE" | "FALSE" | "PI" => Info::default(),
                    "IFERROR" => each.get(1).copied().unwrap_or(any),
                    "IF" => Info { err: all.err, text: each.iter().skip(1).any(|k| k.text) },
                    "SUM" | "MIN" | "MAX" => Info { err: all.err | if args.iter().any(|a| matches!(core(a), Fx::Str(_))) { OTHER } else { 0 }, text: false },
                    "AVERAGE" => Info { err: all.err | OTHER, text: false },
                    "AND" | "OR" | "NOT" => Info { err: all.err | if all.text { OTHER } else { 0 }, text: false },
                    "ABS" | "INT" | "SIGN" | "EXP" | "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "TRUNC" | "N" => all.arithmetic(),
                    "CHOOSE" => Info { err: all.err | OTHER, text: all.text },
                    "VLOOKUP" | "HLOOKUP" | "MATCH" | "LOOKUP" => {
                        let Some(table) = args.get(1) else { return Ok(any) };
                        // A result picked from a known column: only that column's errors.
                        let picked = match (name.as_str(), args.get(2).map(core)) {
                            ("VLOOKUP" | "HLOOKUP", Some(Fx::Number(n))) => n.parse::<u32>().ok(),
                            ("MATCH", _) => Some(0),
                            _ => None,
                        };
                        let t = match (picked, self.block_of(table)) {
                            (Some(0), _) => Info::default(),
                            (Some(k), Some(b)) => {
                                let line = if name == "VLOOKUP" { Block { c1: b.c1 + k - 1, c2: b.c1 + k - 1, ..b } } else { Block { r1: b.r1 + k - 1, r2: b.r1 + k - 1, ..b } };
                                let inside = if name == "VLOOKUP" { k <= b.c2 - b.c1 + 1 } else { k <= b.r2 - b.r1 + 1 };
                                if inside { self.scan(line.sheet, line.c1, line.c2, line.r1 as i64 - 1, line.r2 as i64 - 1) } else { Info { err: OTHER, text: false } }
                            }
                            _ => self.kinds(table)?,
                        };
                        let rest = each.iter().skip(2).fold(Info::default(), |a, b| a.join(*b));
                        Info { err: NA | each[0].err | t.err | rest.err | if picked.is_none() { OTHER } else { 0 }, text: t.text }
                    }
                    "CONCATENATE" => Info { err: all.err, text: true },
                    // The cell pointed at is not followed here.
                    "OFFSET" => any,
                    // Everything else may also raise #VALUE!, #NUM! and the like.
                    _ => Info { err: all.err | OTHER, text: matches!(name.as_str(), "LEFT" | "RIGHT" | "MID" | "UPPER" | "LOWER") },
                }
            }
            _ => any,
        })
    }

    fn translate(&mut self, e: &Fx) -> Lifted<T> {
        Ok(match e {
            Fx::Spaced { expr, .. } | Fx::Paren(expr) => self.translate(expr)?,
            Fx::Number(n) => T::Num(lit(n.parse::<f64>().map_err(|_| "number form")?)),
            Fx::Bool(b) => T::Bool(lit(b.eq_ignore_ascii_case("TRUE"))),
            Fx::Str(s) => T::Text(lit(unquote(s))),
            Fx::Error(_) => T::Num(null_num()),
            Fx::Ref { prefix, kind: RefKind::Cell, text } => self.cell(prefix, text)?,
            Fx::Ref { .. } => return Err("whole row or column".into()),
            Fx::Name { prefix, text: name } => {
                let formula = self.name(&qualified(prefix, name))?;
                self.in_name += 1;
                let result = self.translate(&formula);
                self.in_name -= 1;
                result?
            }
            Fx::Structured { .. } => return Err("table reference".into()),
            Fx::Unary { op, expr } => match op {
                '-' => T::Num(lit(0.0) - self.translate(expr)?.num()),
                '+' => self.translate(expr)?,
                _ => return Err("implicit intersection".into()),
            },
            Fx::Postfix { expr, .. } => T::Num(self.translate(expr)?.num() / lit(100.0)),
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if matches!((core(lhs), core(rhs)), (Fx::Ref { kind: RefKind::Cell, .. }, Fx::Ref { prefix: None, kind: RefKind::Cell, .. })) => {
                // A range where one value is wanted: the cell in the formula's own row (a
                // column of cells) or own column (a row of cells); otherwise #VALUE!.
                if self.array {
                    return Err("range in an array formula where one value is wanted".into());
                }
                let (Fx::Ref { prefix, text: a, .. }, Fx::Ref { text: b, .. }) = (core(lhs), core(rhs)) else { unreachable!() };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                let (c1, c2) = (p.col.min(q.col), p.col.max(q.col));
                if p == q {
                    // A1:A1 is the cell A1.
                    return self.translate(lhs);
                }
                if c1 == c2 {
                    if p.abs_row != q.abs_row {
                        return Err("range fixed at one end where one value is wanted".into());
                    }
                    let (r1, r2) = (p.row.min(q.row), p.row.max(q.row));
                    let first = if p.abs_row { r1 as i64 - self.row as i64 } else { r1 as i64 - self.row as i64 };
                    let last = if p.abs_row { r2 as i64 - (self.row + self.len - 1) as i64 } else { r2 as i64 - self.row as i64 };
                    if first <= 0 && last >= 0 {
                        // A single cell: the cell in its row, as a fixed reference (also inside a name).
                        if self.len == 1 {
                            let text = format!("${}${}", xl_formula::col_name(c1), self.row);
                            return self.translate(&Fx::Ref { prefix: prefix.clone(), kind: RefKind::Cell, text });
                        }
                        return Ok(T::Dyn(self.column(sheet, c1, 0)?));
                    }
                    if p.abs_row {
                        return Err("range where one value is wanted, partly beside the formula's rows".into());
                    }
                    return Ok(T::Num(null_num()));
                }
                if p.row == q.row {
                    if (c1..=c2).contains(&self.run_col) {
                        let r = CellRef { col: self.run_col, ..p };
                        let text = format!("{}{}{}{}", if r.abs_col { "$" } else { "" }, xl_formula::col_name(r.col), if r.abs_row { "$" } else { "" }, r.row);
                        return self.translate(&Fx::Ref { prefix: prefix.clone(), kind: RefKind::Cell, text });
                    }
                    return Ok(T::Num(null_num()));
                }
                T::Num(null_num())
            }
            Fx::Binary { op, lhs, rhs, .. } => {
                // An error value in a union or intersection is the result (the first one).
                if matches!(op, BinaryOp::Union | BinaryOp::Intersect) && first_error(e).is_some() {
                    return self.translate(&Fx::Error(first_error(e).unwrap_or_default()));
                }
                if matches!(op, BinaryOp::Range | BinaryOp::Union | BinaryOp::Intersect) {
                    return Err("range outside a function".into());
                }
                if matches!(op, BinaryOp::Concat) {
                    let (a, b) = (self.translate(lhs)?, self.translate(rhs)?);
                    return Ok(T::Text(a.text(&mut self.lets) + b.text(&mut self.lets)));
                }
                if let Some(all) = terms(e) {
                    // A chain a + b - c ... is taken as one sum, so that every term
                    // appears once in the sum and once in the scale it is judged against.
                    let mut sum: Option<Expr> = None;
                    let mut sizes = Vec::with_capacity(all.len());
                    for (negative, term) in all {
                        let value = self.operand(term)?.num();
                        let value = bind(&mut self.lets, value);
                        sizes.push(value.clone().abs());
                        sum = Some(match (sum, negative) {
                            (None, false) => value,
                            (None, true) => lit(0.0) - value,
                            (Some(s), false) => s + value,
                            (Some(s), true) => s - value,
                        });
                    }
                    return Ok(T::Num(near_zero(sum.expect("two terms"), sizes)));
                }
                let arithmetic = matches!(op, BinaryOp::Mul | BinaryOp::Div | BinaryOp::Pow);
                let (a, b) = if arithmetic { (self.operand(lhs)?, self.operand(rhs)?) } else { (self.translate(lhs)?, self.translate(rhs)?) };
                match op {
                    BinaryOp::Mul => T::Num(a.num() * b.num()),
                    BinaryOp::Div => T::Num(divide(&mut self.lets, a.num(), b.num())),
                    BinaryOp::Pow => T::Num(a.num().pow(b.num())),
                    _ => T::Bool(compare(&mut self.lets, op, a, b)),
                }
            }
            // A reference to deleted cells as an argument: Excel and LibreOffice disagree on
            // what functions make of it (COUNTA counts it or fails), so it is not computed.
            Fx::Call { args, .. } if args.iter().any(|a| matches!(core(a), Fx::Error(e) if e == "#REF!")) => {
                return Err("reference to deleted cells (#REF!) as an argument".into());
            }
            Fx::Call { name, args } => self.call(&name.to_ascii_uppercase(), args)?,
            Fx::Apply { .. } => return Err("lambda".into()),
            Fx::Array(_) => return Err("array constant".into()),
            // An argument left out reads as an empty cell: 0 in numbers, "" in text.
            Fx::Missing => T::Dyn(Dyn { num: null_num(), text: None, blank: true, nan: false, parsed: None }),
        })
    }

    /// An operand of arithmetic: text written out in the formula reads as the number
    /// or date it spells, or is an error.
    fn operand(&mut self, e: &Fx) -> Lifted<T> {
        if let Fx::Str(literal) = core(e) {
            return Ok(T::Num(match text_as_number(&unquote(literal), self.wb.date1904) {
                Some(v) => lit(v),
                None => null_num(),
            }));
        }
        self.translate(e)
    }

    /// An argument as a number, under a name if it is more than a column.
    fn number(&mut self, a: &Fx) -> Lifted<Expr> {
        let value = self.operand(a)?.num();
        Ok(bind(&mut self.lets, value))
    }

    fn string(&mut self, a: &Fx) -> Lifted<Expr> {
        let value = self.translate(a)?;
        Ok(value.text(&mut self.lets))
    }

    fn call(&mut self, name: &str, args: &[Fx]) -> Lifted<T> {
        let arity = |low: usize, high: usize| if (low..=high).contains(&args.len()) { Ok(()) } else { Err(format!("{name} with {} arguments", args.len())) };
        let constant = |a: &Fx, what: &str| match core(a) {
            Fx::Missing => Ok(0.0),
            Fx::Number(n) => n.parse::<f64>().map_err(|_| format!("{name} {what}")),
            Fx::Unary { op: '-', expr } => match core(expr) {
                Fx::Number(n) => n.parse::<f64>().map(|v| -v).map_err(|_| format!("{name} {what}")),
                _ => Err(format!("{name} with computed {what}")),
            },
            _ => Err(format!("{name} with computed {what}")),
        };
        Ok(match name {
            "SUM" | "COUNT" | "AVERAGE" => {
                let outer = self.summing;
                self.summing = name == "SUM";
                let spread = self.spread(args);
                self.summing = outer;
                let spread = spread?;
                let mut sum = lit(0.0);
                let mut count = lit(0.0);
                for item in &spread.items {
                    match item {
                        Item::Rows(e) => {
                            sum = sum + e.clone().fill_null(lit(0.0));
                            count = count + e.clone().is_not_null().cast(DataType::Float64);
                        }
                        Item::Fixed(s) => {
                            sum = sum + s.sum.clone();
                            count = count + s.count.clone();
                        }
                        Item::Growing { sum: s, count: c, .. } => {
                            sum = sum + s.clone();
                            count = count + c.clone();
                        }
                    }
                }
                T::Num(match name {
                    "SUM" => spread.guard(sum),
                    // COUNT counts numbers and passes over errors.
                    "COUNT" => count,
                    _ => spread.guard(divide(&mut self.lets, sum, count)),
                })
            }
            "STDEV" | "STDEV.S" | "STDEVP" | "STDEV.P" | "VAR" | "VAR.S" | "VARP" | "VAR.P" => {
                let population = name.ends_with('P') || name.ends_with(".P");
                let root = name.starts_with("STDEV");
                let spread = self.spread(args)?;
                let (mut sum, mut count) = (lit(0.0), lit(0.0));
                for item in &spread.items {
                    match item {
                        Item::Rows(e) => {
                            sum = sum + e.clone().fill_null(lit(0.0));
                            count = count + e.clone().is_not_null().cast(DataType::Float64);
                        }
                        Item::Fixed(f) => {
                            sum = sum + f.sum.clone();
                            count = count + f.count.clone();
                        }
                        Item::Growing { .. } => return Err(format!("{name} over a growing range")),
                    }
                }
                let count = bind(&mut self.lets, count);
                let quotient = divide(&mut self.lets, sum, count.clone());
                let mean = bind(&mut self.lets, quotient);
                // The sum of squared deviations, taken from the mean as Excel does.
                let mut squares = lit(0.0);
                for item in &spread.items {
                    match item {
                        Item::Rows(e) => {
                            let d = e.clone() - mean.clone();
                            squares = squares + when(e.clone().is_null()).then(lit(0.0)).otherwise(d.clone() * d);
                        }
                        Item::Fixed(f) => squares = squares + f.sum_squares.clone() - lit(2.0) * mean.clone() * f.sum.clone() + f.count.clone() * mean.clone() * mean.clone(),
                        Item::Growing { .. } => unreachable!(),
                    }
                }
                let denominator = if population { count.clone() } else { count.clone() - lit(1.0) };
                let enough = if population { count.gt_eq(lit(1.0)) } else { count.gt_eq(lit(2.0)) };
                let variance = when(enough).then(squares / denominator).otherwise(null_num());
                let variance = when(variance.clone().lt(lit(0.0))).then(lit(0.0)).otherwise(variance);
                T::Num(spread.guard(if root { variance.pow(lit(0.5)) } else { variance }))
            }
            // Blank cells do not take part; all blank gives 0.
            "MIN" | "MAX" => {
                let smallest = name == "MIN";
                let spread = self.spread(args)?;
                let mut exprs = Vec::new();
                for item in &spread.items {
                    match item {
                        Item::Rows(e) => exprs.push(e.clone()),
                        Item::Fixed(s) if s.empty == Some(true) => {}
                        Item::Fixed(s) => exprs.push(if smallest { s.min.clone() } else { s.max.clone() }),
                        Item::Growing { max, min, .. } => exprs.push(if smallest { min.clone() } else { max.clone() }),
                    }
                }
                if exprs.is_empty() {
                    // No number at all: 0, unless an error among the arguments is the result.
                    T::Num(spread.guard(lit(0.0)))
                } else {
                    let combined = if smallest { min_horizontal(exprs) } else { max_horizontal(exprs) };
                    T::Num(spread.guard(combined.map_err(|e| e.to_string())?.fill_null(lit(0.0))))
                }
            }
            "ABS" => {
                arity(1, 1)?;
                T::Num(self.number(&args[0])?.abs())
            }
            "INT" => {
                arity(1, 1)?;
                T::Num(self.number(&args[0])?.floor())
            }
            "SIGN" => {
                arity(1, 1)?;
                let x = self.number(&args[0])?;
                T::Num(x.clone().gt(lit(0.0)).cast(DataType::Float64) - x.lt(lit(0.0)).cast(DataType::Float64))
            }
            "SQRT" => {
                arity(1, 1)?;
                let x = self.number(&args[0])?;
                T::Num(when(x.clone().lt(lit(0.0))).then(null_num()).otherwise(x.pow(lit(0.5))))
            }
            "LN" | "LOG10" => {
                arity(1, 1)?;
                let x = self.number(&args[0])?;
                let base = if name == "LN" { std::f64::consts::E } else { 10.0 };
                T::Num(when(x.clone().lt_eq(lit(0.0))).then(null_num()).otherwise(x.log(lit(base))))
            }
            "EXP" => {
                arity(1, 1)?;
                T::Num(self.number(&args[0])?.exp())
            }
            "POWER" => {
                arity(2, 2)?;
                T::Num(self.number(&args[0])?.pow(self.number(&args[1])?))
            }
            "MOD" => {
                arity(2, 2)?;
                let (n, d) = (self.number(&args[0])?, self.number(&args[1])?);
                let result = n.clone() - d.clone() * (n / d.clone()).floor();
                T::Num(when(d.eq(lit(0.0))).then(null_num()).otherwise(result))
            }
            "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "TRUNC" => {
                arity(if name == "TRUNC" { 1 } else { 2 }, 2)?;
                let digits = match args.get(1) {
                    Some(a) => constant(a, "digits")?,
                    None => 0.0,
                };
                let scale = 10f64.powi(digits as i32);
                let x = self.number(&args[0])?;
                // Excel rounds halves away from zero.
                // It also sees 8.255 as 8.255 although the binary number is a hair below:
                // values are taken at 15 significant digits.
                let scaled = x.clone().abs() * lit(scale);
                let hair = scaled.clone() * lit(2e-15);
                let whole = match name {
                    "ROUND" => (scaled + lit(0.5) + hair).floor(),
                    "ROUNDUP" => (scaled - hair).ceil(),
                    _ => (scaled + hair).floor(),
                };
                let magnitude = whole / lit(scale);
                T::Num(when(x.lt(lit(0.0))).then(lit(0.0) - magnitude.clone()).otherwise(magnitude))
            }
            "IF" => {
                arity(2, 3)?;
                let condition = self.translate(&args[0])?.truth()?;
                let yes = self.translate(&args[1])?;
                let no = match args.get(2) {
                    Some(a) => self.translate(a)?,
                    None => T::Bool(lit(false)),
                };
                match choose(&mut self.lets, condition, yes, no) {
                    // A truth value in one branch, text in the other: with a fixed condition the
                    // branch is known now.
                    Err(e) if e == "truth value mixed with text" && self.is_fixed(&args[0], 0) => match self.fixed_number(&args[0])? {
                        Some(v) if v != 0.0 => self.translate(&args[1])?,
                        Some(_) => match args.get(2) {
                            Some(a) => self.translate(a)?,
                            None => T::Bool(lit(false)),
                        },
                        None => return Err(e),
                    },
                    other => other?,
                }
            }
            "IFERROR" => {
                arity(2, 2)?;
                let value = self.translate(&args[0])?.bound(&mut self.lets);
                let instead = self.translate(&args[1])?;
                choose(&mut self.lets, value.is_error(), instead, value)?
            }
            "VLOOKUP" | "HLOOKUP" => {
                arity(3, 4)?;
                let key = self.translate(&args[0])?.bound(&mut self.lets);
                // The kind of match: fixed, or per row (both are computed and one taken).
                let exact: Option<bool> = match args.get(3).map(core) {
                    None => Some(false),
                    Some(Fx::Missing) => Some(true),
                    Some(Fx::Bool(b)) => Some(b.eq_ignore_ascii_case("FALSE")),
                    Some(Fx::Number(n)) => Some(n.parse::<f64>() == Ok(0.0)),
                    Some(Fx::Call { name: f, args: none }) if none.is_empty() && (f.eq_ignore_ascii_case("TRUE") || f.eq_ignore_ascii_case("FALSE")) => Some(f.eq_ignore_ascii_case("FALSE")),
                    Some(a) if self.is_fixed(a, 0) => match self.fixed_number(a)? {
                        Some(v) => Some(v == 0.0),
                        None => return Ok(T::Num(null_num())),
                    },
                    Some(_) => None,
                };
                if name == "VLOOKUP" {
                    if let Some((table, lo, hi)) = self.moving_table(&args[1])? {
                        let index = self.number(&args[2])?;
                        return match exact {
                            Some(e) => self.lookup_in(key, table, index, true, e, Some((lo, hi))),
                            None => Err("VLOOKUP in a moving table with a computed kind of match".into()),
                        };
                    }
                }
                let Some(table) = self.table(&args[1])? else {
                    // The table is #REF!: so is the result, unless the key is an error itself.
                    let key_error = key.is_error();
                    self.last_na = Some((key_error, lit(false)));
                    return Ok(T::Num(null_num()));
                };
                let index = self.number(&args[2])?;
                let vertical = name == "VLOOKUP";
                match exact {
                    Some(e) => self.lookup_at(key, table, index, vertical, e)?,
                    None => {
                        let approximate = self.translate(&args[3])?.truth()?;
                        let index = bind(&mut self.lets, index);
                        let a = self.lookup_at(key.clone(), table, index.clone(), vertical, false)?;
                        let e = self.lookup_at(key, table, index, vertical, true)?;
                        choose(&mut self.lets, approximate, a, e)?
                    }
                }
            }
            "OFFSET" => match self.offset(args)? {
                Target::Nowhere => T::Num(null_num()),
                Target::Fixed(b) if b.r1 == b.r2 && b.c1 == b.c2 => self.constant(b.sheet, b.r1, b.c1)?,
                Target::Shifted { sheet, col, offset, height: 1, width: 1 } => {
                    if (self.row as i64 + offset) < 1 {
                        T::Num(null_num())
                    } else {
                        T::Dyn(self.column(sheet, col, offset)?)
                    }
                }
                Target::Gather { sheet, vertical, line, index } => self.gather(sheet, vertical, line, index)?,
                _ => return Err("OFFSET range outside an aggregate".into()),
            },
            "FIND" | "SEARCH" => {
                arity(2, 3)?;
                let search = name == "SEARCH";
                let pattern = self.string(&args[0])?;
                if search {
                    // SEARCH reads * ? ~ as wildcards: only for patterns written out.
                    match core(&args[0]) {
                        Fx::Str(p) if !unquote(p).contains(['*', '?', '~']) => {}
                        _ => return Err("SEARCH with wildcards or a computed pattern".into()),
                    }
                }
                let text = self.string(&args[1])?;
                let start = match args.get(2) {
                    Some(a) => self.number(a)?.floor(),
                    None => lit(1.0),
                };
                let (pattern, text) = if search { (pattern.str().to_lowercase(), text.str().to_lowercase()) } else { (pattern, text) };
                let text = bind(&mut self.lets, text);
                let pattern = bind(&mut self.lets, pattern);
                let rest = bind(&mut self.lets, text.clone().str().slice((start.clone() - lit(1.0)).cast(DataType::Int64), lit(40_000i64)));
                let found = bind(&mut self.lets, rest.clone().str().find_literal(pattern.clone()));
                // The position is counted in bytes; the text before it gives the characters.
                let before = rest.clone().str().split(pattern.clone()).list().first().str().len_chars();
                let chars = when(pattern.str().len_chars().eq(lit(0u32))).then(lit(0.0)).otherwise(before.cast(DataType::Float64));
                let position = chars + start.clone();
                let valid = start.clone().gt_eq(lit(1.0)).and(start.lt_eq(text.str().len_chars().cast(DataType::Float64)));
                T::Num(when(found.is_not_null().and(valid)).then(position).otherwise(null_num()))
            }
            "ISERROR" => {
                arity(1, 1)?;
                T::Bool(self.translate(&args[0])?.is_error())
            }
            // #N/A against the other errors: only where the argument cannot give both.
            "ISERR" | "ISNA" => {
                arity(1, 1)?;
                self.last_na = None;
                let value = self.translate(&args[0])?;
                // A lookup knows which of its errors are #N/A.
                // Through ABS, INT, a sign or parentheses an error stays the same error.
                fn peel(e: &Fx) -> &Fx {
                    match core(e) {
                        Fx::Unary { op: '-' | '+', expr } => peel(expr),
                        Fx::Call { name, args } if args.len() == 1 && matches!(name.to_ascii_uppercase().as_str(), "ABS" | "INT") => peel(&args[0]),
                        other => other,
                    }
                }
                if let (Fx::Call { name: f, args: inner }, Some((key_error, na))) = (peel(&args[0]), self.last_na.take()) {
                    if matches!(f.to_ascii_uppercase().as_str(), "VLOOKUP" | "HLOOKUP" | "MATCH") && !inner.is_empty() {
                        let key_kinds = self.kinds(&inner[0])?.err;
                        let key_na = match (key_kinds & NA != 0, key_kinds & OTHER != 0) {
                            (true, true) => return Err(format!("{name} over a lookup key with errors of several kinds")),
                            (true, false) => lit(true),
                            _ => lit(false),
                        };
                        let na = when(key_error).then(key_na).otherwise(na);
                        let na = bind(&mut self.lets, na);
                        return Ok(T::Bool(if name == "ISNA" { na } else { value.is_error().and(na.not()) }));
                    }
                }
                let kinds = self.kinds(&args[0])?.err;
                let (wanted, other) = if name == "ISNA" { (NA, OTHER) } else { (OTHER, NA) };
                if kinds & wanted != 0 && kinds & other != 0 {
                    return Err(format!("{name} over errors of several kinds"));
                }
                T::Bool(if kinds & wanted != 0 { value.is_error() } else { lit(false) })
            }
            "ISNUMBER" => {
                arity(1, 1)?;
                T::Bool(match self.translate(&args[0])?.bound(&mut self.lets) {
                    T::Num(e) => e.clone().is_not_null().and(e.is_finite().fill_null(lit(false))),
                    T::Bool(_) | T::Text(_) => lit(false),
                    T::Dyn(d) => {
                        let n = d.num.clone();
                        n.clone().is_not_null().and(n.is_finite().fill_null(lit(false)))
                    }
                })
            }
            "ISTEXT" | "ISNONTEXT" => {
                arity(1, 1)?;
                let is_text = match self.translate(&args[0])? {
                    T::Text(e) => e.is_not_null(),
                    T::Dyn(Dyn { text: Some(t), .. }) => t.is_not_null(),
                    _ => lit(false),
                };
                T::Bool(if name == "ISTEXT" { is_text } else { is_text.not() })
            }
            "ISBLANK" => {
                arity(1, 1)?;
                T::Bool(match self.translate(&args[0])? {
                    T::Dyn(d) => d.is_blank(),
                    _ => lit(false),
                })
            }
            "N" => {
                arity(1, 1)?;
                T::Num(match self.translate(&args[0])? {
                    T::Text(_) => lit(0.0),
                    T::Dyn(d) => match &d.text {
                        Some(t) => when(t.clone().is_not_null()).then(lit(0.0)).otherwise(d.number_part()),
                        None => d.number_part(),
                    },
                    other => other.num(),
                })
            }
            "NA" => {
                arity(0, 0)?;
                T::Num(null_num())
            }
            "TRUE" | "FALSE" => {
                arity(0, 0)?;
                T::Bool(lit(name == "TRUE"))
            }
            "PI" => {
                arity(0, 0)?;
                T::Num(lit(std::f64::consts::PI))
            }
            "AND" | "OR" => {
                let mut items = Vec::new();
                for a in args {
                    let truth = self.translate(a)?.truth()?;
                    items.push(bind(&mut self.lets, truth));
                }
                if items.is_empty() {
                    return Err(format!("{name} without arguments"));
                }
                // An error in any argument is the result.
                let error = items.iter().cloned().map(|e| e.is_null()).reduce(|a, b| a.or(b)).unwrap();
                let all = items.into_iter().reduce(if name == "AND" { |a: Expr, b: Expr| a.and(b) } else { |a: Expr, b: Expr| a.or(b) }).unwrap();
                T::Bool(when(error).then(null_bool()).otherwise(all))
            }
            "NOT" => {
                arity(1, 1)?;
                T::Bool(self.translate(&args[0])?.truth()?.not())
            }
            "CHOOSE" => {
                if args.len() < 2 {
                    return Err("CHOOSE without values".into());
                }
                let kept = self.number(&args[0])?.floor();
                let index = bind(&mut self.lets, kept);
                // Outside 1..n is an error.
                let mut result = T::Num(null_num());
                for (i, a) in args.iter().enumerate().skip(1).rev() {
                    let value = self.translate(a)?;
                    let here = when(index.clone().is_null()).then(lit(false)).otherwise(index.clone().eq(lit(i as f64)));
                    result = choose(&mut self.lets, here, value, result)?;
                }
                result
            }
            "CONCATENATE" => {
                if args.is_empty() {
                    return Err("CONCATENATE without arguments".into());
                }
                let mut joined: Option<Expr> = None;
                for a in args {
                    let piece = self.translate(a)?.text(&mut self.lets);
                    joined = Some(match joined {
                        Some(j) => j + piece,
                        None => piece,
                    });
                }
                T::Text(joined.unwrap())
            }
            "LEN" => {
                arity(1, 1)?;
                T::Num(self.string(&args[0])?.str().len_chars().cast(DataType::Float64))
            }
            "UPPER" | "LOWER" => {
                arity(1, 1)?;
                let t = self.string(&args[0])?;
                T::Text(if name == "UPPER" { t.str().to_uppercase() } else { t.str().to_lowercase() })
            }
            "LEFT" | "RIGHT" => {
                arity(1, 2)?;
                let t = self.string(&args[0])?;
                let n = match args.get(1) {
                    Some(a) => {
                        let kept = self.number(a)?.floor();
                        bind(&mut self.lets, kept)
                    }
                    None => lit(1.0),
                };
                let cut = if name == "LEFT" { t.str().head(n.clone().cast(DataType::Int64)) } else { t.str().tail(n.clone().cast(DataType::Int64)) };
                // Nothing for a count of 0, an error for a negative one.
                T::Text(when(n.clone().lt(lit(0.0))).then(null_text()).otherwise(when(n.eq(lit(0.0))).then(lit("")).otherwise(cut)))
            }
            "MID" => {
                arity(3, 3)?;
                let t = self.string(&args[0])?;
                let kept = self.number(&args[1])?.floor();
                let start = bind(&mut self.lets, kept);
                let kept = self.number(&args[2])?.floor();
                let n = bind(&mut self.lets, kept);
                let cut = t.str().slice((start.clone() - lit(1.0)).cast(DataType::Int64), n.clone().cast(DataType::Int64));
                T::Text(when(start.lt(lit(1.0)).or(n.lt(lit(0.0)))).then(null_text()).otherwise(cut))
            }
            "EXACT" => {
                arity(2, 2)?;
                T::Bool(self.string(&args[0])?.eq(self.string(&args[1])?))
            }
            "VALUE" => {
                arity(1, 1)?;
                T::Num(self.number(&args[0])?)
            }
            "YEAR" | "MONTH" | "DAY" => {
                arity(1, 1)?;
                let serial = self.number(&args[0])?;
                let (days, zero, leap) = self.serial_days(serial);
                let (y, m, d) = civil(&mut self.lets, days);
                // Serial 0 reads as the 0th of January 1900, serial 60 as 29 February 1900.
                let (part, at_zero, at_leap) = match name {
                    "YEAR" => (y, 1900.0, 1900.0),
                    "MONTH" => (m, 1.0, 2.0),
                    _ => (d, 0.0, 29.0),
                };
                T::Num(when(zero).then(lit(at_zero)).otherwise(when(leap).then(lit(at_leap)).otherwise(part)))
            }
            "WEEKDAY" => {
                arity(1, 2)?;
                let kind = match args.get(1) {
                    Some(a) => constant(a, "type")?,
                    None => 1.0,
                };
                let kept = self.number(&args[0])?.floor() + lit(if self.wb.date1904 { 1462.0 } else { 0.0 });
                let serial = bind(&mut self.lets, kept);
                // Excel's own count of days, the day that never was included: serial 1 is a Sunday.
                let lets = &mut self.lets;
                let mut modulo = |e: Expr| {
                    let e = bind(lets, e);
                    e.clone() - floor_div(e, 7.0) * lit(7.0)
                };
                let day = match kind as i64 {
                    1 => modulo(serial.clone() - lit(1.0)) + lit(1.0),
                    2 => modulo(serial.clone() - lit(2.0)) + lit(1.0),
                    3 => modulo(serial.clone() - lit(2.0)),
                    _ => return Err("WEEKDAY type".into()),
                };
                T::Num(when(serial.lt(lit(0.0))).then(null_num()).otherwise(day))
            }
            "DATE" => {
                arity(3, 3)?;
                let kept = self.number(&args[0])?.floor();
                let year = bind(&mut self.lets, kept);
                let month = self.number(&args[1])?.floor();
                let day = self.number(&args[2])?.floor();
                // Years below 1900 count from 1900.
                let year = when(year.clone().lt(lit(1900.0))).then(year.clone() + lit(1900.0)).otherwise(year);
                let kept = days_from_civil(&mut self.lets, year, month, day);
                let days = bind(&mut self.lets, kept);
                let serial = if self.wb.date1904 {
                    days + lit(24_107.0)
                } else {
                    // Up to 28 February 1900 Excel's count is one behind.
                    when(days.clone().lt(lit(-25_508.0))).then(days.clone() + lit(25_568.0)).otherwise(days + lit(25_569.0))
                };
                let serial = bind(&mut self.lets, serial);
                T::Num(when(serial.clone().lt(lit(0.0))).then(null_num()).otherwise(serial))
            }
            other => return self.call_more(other, args),
        })
    }
}

fn col_expr(name: &str) -> Expr {
    col(name)
}

/// Division; by zero is an error.
fn divide(lets: &mut Lets, a: Expr, b: Expr) -> Expr {
    let b = bind(lets, b);
    when(b.clone().eq(lit(0.0))).then(null_num()).otherwise(a / b)
}

/// Excel and LibreOffice take a sum or difference as exactly 0 when it is vanishingly
/// small against its terms (the rounding noise of binary arithmetic).
fn near_zero(result: Expr, sizes: Vec<Expr>) -> Expr {
    let scale = max_horizontal(sizes).expect("at least two terms");
    when(result.clone().abs().lt(scale * lit(3.6e-15))).then(lit(0.0)).otherwise(result)
}

/// A run translated into Polars expressions over a frame with one row per sheet row.
///
/// The frame holds the sheet row number in `_row` (1-based), the numbers of each
/// column read in `s<sheet>c<column>` (columns 1-based; null where the cell is empty
/// or holds text, NaN where it holds an error value) and, for the columns in
/// `text_columns`, their text in `s<sheet>c<column>t` (null where there is none).
///
/// The translation is typed by what the columns hold in the rows the run reads: a
/// column without text there is read as numbers only.
pub struct LiftedRun {
    /// The result as a number; null where it is text or an error.
    pub expr: Expr,
    /// The result as text where it is text; `None` if it never is.
    pub text: Option<Expr>,
    pub is_bool: bool,
    /// Intermediate columns `_t<n>` the result is built from, to be added to the
    /// frame one after the other in this order.
    pub lets: Vec<(String, Expr)>,
    /// Conditions per row under which the translation does not hold, with the reason:
    /// such rows are left to the cell evaluator.
    pub guards: Vec<(String, Expr)>,
    pub columns: Vec<(u32, u32)>,
    pub text_columns: Vec<(u32, u32)>,
    /// Columns whose text is also given as a number (`s<sheet>c<column>p`), for arithmetic on text.
    pub parsed_columns: Vec<(u32, u32)>,
    /// How values of the workbook went into the translation (`BAKED_*`, or-ed).
    pub baked: u32,
    /// What the expressions hold under (see [`Assumptions`]).
    pub assumptions: Assumptions,
    /// Smallest and largest row offset read.
    pub reach: (i64, i64),
    /// The formula reads its own column at other rows (offsets, all of one sign): the
    /// frame then holds `_own`/`_ownt`, the column's number and text, and the result
    /// is found by evaluating repeatedly until it no longer changes (see `evaluate`).
    pub own: Option<(i64, i64)>,
}

fn index(wb: &Workbook) -> HashMap<(u32, u32, u32), usize> {
    let c = &wb.cells;
    (0..c.len()).map(|i| ((c.sheet[i], c.row[i], c.col[i]), i)).collect()
}

fn translate_run(ex: &dyn Executor, param: bool, wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, ranges: &RangeCache, last_rows: &[u32], external_from: u32, run: &Run) -> Lifted<LiftedRun> {
    match translate_once(ex, param, wb, at, ranges, last_rows, external_from, run, false) {
        Err(reason) if reason == "reference to its own column" => {
            if run.len > MAX_ITERATED {
                return Err("long column reading its own earlier rows".into());
            }
            translate_once(ex, param, wb, at, ranges, last_rows, external_from, run, true)
        }
        other => other,
    }
}

/// Longest run computed by iteration (each round settles at least one more row).
const MAX_ITERATED: u32 = 6_000;

#[allow(clippy::too_many_arguments)]
fn translate_once(ex: &dyn Executor, param: bool, wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, ranges: &RangeCache, last_rows: &[u32], external_from: u32, run: &Run, recursive: bool) -> Lifted<LiftedRun> {
    let parsed = parse(&run.formula).map_err(|_| "formula not parsed".to_string())?;
    // A defined name spelled like a cell (`NXB3`, valid in .xls with 256 columns, a cell in
    // .xlsx): the file is ambiguous, so the formula is not computed.
    if !wb.defined_names.is_empty() {
        let mut clash = false;
        parsed.walk(&mut |e| {
            if let Fx::Ref { prefix: None, kind: RefKind::Cell, text } = e {
                let plain = text.replace('$', "");
                if wb.defined_names.iter().any(|n| n.name.eq_ignore_ascii_case(&plain)) {
                    clash = true;
                }
            }
        });
        if clash {
            return Err("defined name spelled like a cell reference".into());
        }
    }
    let mut ctx = Context {
        baked: std::cell::Cell::new(0),
        param: std::cell::Cell::new(param),
        reads: Default::default(),
        extents: Default::default(),
        cell_inputs: Default::default(),
        range_inputs: Default::default(),
        pinned_ranges: Default::default(),
        column_kinds: BTreeMap::new(),
        ex,
        wb,
        at,
        ranges,
        sheet: run.sheet,
        row: run.row + 1,
        run_col: run.col + 1,
        len: run.len,
        columns: BTreeSet::new(),
        text_columns: BTreeSet::new(),
        parsed_columns: BTreeSet::new(),
        reach: (0, 0),
        in_name: 0,
        lets: Vec::new(),
        guards: Vec::new(),
        own: None,
        own_dir: 0,
        last_na: None,
        recursive: false,
        array: at.get(&(run.sheet, run.row, run.col)).is_some_and(|&i| wb.cells.array_ref[i].is_some()),
        summing: false,
        external_from,
        last_rows,
    };
    ctx.recursive = recursive;
    // A single cell: its relative references point at fixed cells.
    let parsed = if run.len == 1 { fixed_refs(&parsed) } else { parsed };
    let parsed = ctx.resolve(&parsed)?;
    // An array formula over fixed ranges whose result is an array: worked out now.
    let mut fixed = None;
    if ctx.array && !matches!(core(&parsed), Fx::Call { name, .. } if RANGE_FUNCTIONS.contains(&name.to_ascii_uppercase().as_str()) && !name.eq_ignore_ascii_case("INDEX")) && ctx.is_fixed(&parsed, 0) && ctx.shape(match core(&parsed) { Fx::Call { name, args } if name.eq_ignore_ascii_case("INDEX") && !args.is_empty() => &args[0], other => other })?.is_some_and(|(h, w)| h as u64 * w as u64 > 400) {
        fixed = ctx.array_corner(&parsed).map_err(|e| { if std::env::var("LIFT_DEBUG").is_ok() { eprintln!("array: {e}"); } e }).ok();
    }
    // An array formula whose result is an array shows its first element in its cell.
    let parsed = if ctx.array && !matches!(core(&parsed), Fx::Call { name, .. } if RANGE_FUNCTIONS.contains(&name.to_ascii_uppercase().as_str())) && ctx.shape(&parsed)?.is_some() {
        ctx.element(&parsed, 0, 0)?
    } else {
        parsed
    };
    let t = match fixed {
        Some(t) => t,
        None => match ctx.recurrence(&parsed)? {
            Some(t) => t,
            None => ctx.translate(&parsed)?,
        },
    };
    let is_bool = matches!(t, T::Bool(_));
    if ctx.own.is_some() && is_bool {
        return Err("truth values computed from the column's own earlier rows".into());
    }
    let (expr, text) = match t {
        T::Text(e) => (null_num(), Some(e)),
        T::Dyn(d) => (d.number_part(), d.text.clone()),
        other => (other.num(), None),
    };
    let assumptions = ctx.assumptions();
    Ok(LiftedRun { expr, text, is_bool, lets: std::mem::take(&mut ctx.lets), guards: std::mem::take(&mut ctx.guards), columns: ctx.columns.into_iter().collect(), text_columns: ctx.text_columns.into_iter().collect(), parsed_columns: ctx.parsed_columns.into_iter().collect(), reach: ctx.reach, own: ctx.own, baked: ctx.baked.get(), assumptions })
}

/// The translation of a column or cell of a workbook as a program takes it: the plan
/// (fixed cells and ranges as inputs), and what the plan reads and holds under.
pub struct Translation {
    pub run: Run,
    /// Steps (columns added in layers), then the outputs `_r` (the number), `_rt` (the
    /// text) and one `_g<k>` for each of `guards` (true where the translation does not hold).
    pub plan: Plan,
    pub guards: Vec<String>,
    pub lifted: LiftedRun,
}

/// Translate a run of `wb` for a program: fixed cells and ranges are inputs of the plan.
/// `Err` says why the engine has no translation.
pub fn translate_program(ex: &dyn Executor, wb: &Workbook, run: &Run) -> Lifted<Translation> {
    translate_program_in(ex, wb, &index(wb), &last_rows_of(wb), run)
}

/// The last row (1-based) of each sheet that holds a cell.
pub fn last_rows_of(wb: &Workbook) -> Vec<u32> {
    let mut last_rows = vec![0u32; wb.sheets.len()];
    for i in 0..wb.cells.len() {
        let last = &mut last_rows[wb.cells.sheet[i] as usize];
        *last = (*last).max(wb.cells.row[i] + 1);
    }
    last_rows
}

/// [`translate_program`] with the cells of the workbook by place and [`last_rows_of`] it
/// (for many runs of one workbook).
pub fn translate_program_in(ex: &dyn Executor, wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, last_rows: &[u32], run: &Run) -> Lifted<Translation> {
    let lifted = translate_run(ex, true, wb, at, &RangeCache::default(), last_rows, wb.sheets.len() as u32, run)?;
    if lifted.lets.len() > 8_000 {
        return Err("expression too large".into());
    }
    let text = lifted.text.clone().unwrap_or_else(null_text);
    let steps = layers(&lifted.lets).into_iter().map(|layer| layer.into_iter().map(|k| lifted.lets[k].clone()).collect()).collect();
    let mut outputs = vec![("_r".to_string(), lifted.expr.clone().cast(DataType::Float64)), ("_rt".to_string(), text.cast(DataType::String))];
    for (k, (_, g)) in lifted.guards.iter().enumerate() {
        outputs.push((format!("_g{k}"), g.clone().cast(DataType::Boolean)));
    }
    let guards = lifted.guards.iter().map(|(why, _)| why.clone()).collect();
    Ok(Translation { run: run.clone(), plan: Plan { steps, outputs, broadcast: true }, guards, lifted })
}

/// Evaluate a lifted run over its frame. A run that reads its own column is evaluated
/// again and again, its own column replaced by the last result, until nothing changes:
/// the recurrence has one solution for the given start, so the fixed point is it, and
/// each round settles at least the next row.
/// The numbers of a result column, row by row.
struct Nums(Values);

impl Nums {
    fn get(&self, k: usize) -> Option<f64> {
        self.0.f64(k)
    }
    fn iter(&self) -> impl Iterator<Item = Option<f64>> + '_ {
        (0..self.0.len()).map(|k| self.0.f64(k))
    }
}

/// The texts of a result column, row by row.
struct Texts(Values);

impl Texts {
    fn get(&self, k: usize) -> Option<&str> {
        self.0.str(k)
    }
    fn iter(&self) -> impl Iterator<Item = Option<&str>> + '_ {
        (0..self.0.len()).map(|k| self.0.str(k))
    }
}

/// A fixed expression worked out on one row (`_row` is the formula's row), after the
/// intermediate columns it reads.
fn fixed_value(ex: &dyn Executor, row: u32, lets: Lets, expr: Expr) -> Result<Values, String> {
    let mut input = Table::new(1);
    input.push(ROW, vec![row as i64]);
    let plan = std::sync::Arc::new(Plan { steps: lets.into_iter().map(|l| vec![l]).collect(), outputs: vec![("_v".into(), expr)], broadcast: false });
    let mut out = ex.run(&plan, &input)?;
    out.columns.pop().map(|(_, v)| v).ok_or_else(|| "no result".to_string())
}

fn evaluate(ex: &dyn Executor, mut frame: Table, head: usize, run: &Run, lifted: &LiftedRun, wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>) -> Lifted<(Nums, Texts, Table)> {
    let text = lifted.text.clone().unwrap_or_else(null_text);
    let height = frame.height;
    let inside = |k: usize| k >= head && k < head + run.len as usize;
    if lifted.own.is_some() {
        // The column's cells outside the run as they are; inside, nothing yet.
        let first = run.row as i64 - head as i64;
        let (mut numbers, mut strings): (Vec<Option<f64>>, Vec<Option<String>>) = (Vec::with_capacity(height), Vec::with_capacity(height));
        for k in 0..height {
            let row = first + k as i64;
            let cell = if inside(k) || row < 0 { None } else { at.get(&(run.sheet, row as u32, run.col)) };
            let (n, t) = match cell {
                None => (None, None),
                Some(&i) => match wb.cells.kind[i] {
                    Kind::Blank if wb.cells.formula[i].is_some() => (None, Some(String::new())),
                    Kind::Blank => (None, None),
                    Kind::Number | Kind::Bool => (Some(wb.cells.num[i]), None),
                    Kind::Text => (None, Some(wb.cells.text[i].as_deref().unwrap_or("").to_string())),
                    Kind::Error => (Some(f64::NAN), None),
                },
            };
            numbers.push(n);
            strings.push(t);
        }
        frame.set(OWN_NUM, numbers);
        frame.set(OWN_TEXT, strings);
    }
    let rounds = if lifted.own.is_some() { run.len as usize + 2 } else { 1 };
    if lifted.lets.len() > 8_000 {
        return Err("expression too large".into());
    }
    // The steps in layers: each layer only reads earlier ones.
    let steps = layers(&lifted.lets).into_iter().map(|layer| layer.into_iter().map(|k| lifted.lets[k].clone()).collect()).collect();
    let mut outputs = vec![("_r".to_string(), lifted.expr.clone().cast(DataType::Float64)), ("_rt".to_string(), text.cast(DataType::String))];
    for (k, (_, g)) in lifted.guards.iter().enumerate() {
        outputs.push((format!("_g{k}"), g.clone().cast(DataType::Boolean)));
    }
    let plan = std::sync::Arc::new(Plan { steps, outputs, broadcast: true });
    for _ in 0..rounds {
        let out = ex.run(&plan, &frame).map_err(|e| format!("polars: {}", e.lines().next().unwrap_or("")))?;
        let (Some(values @ Values::OptF64(_)), Some(texts @ Values::OptStr(_))) = (out.column("_r"), out.column("_rt")) else {
            return Err("polars: result type".into());
        };
        let (values, texts) = (Nums(values.clone()), Texts(texts.clone()));
        if lifted.own.is_none() {
            return Ok((values, texts, out));
        }
        // The new own column: numbers, NaN for errors, text where the result is text.
        let (Some(old_n), Some(old_t)) = (frame.column(OWN_NUM), frame.column(OWN_TEXT)) else { return Err("own column".into()) };
        let mut changed = false;
        let (mut numbers, mut strings): (Vec<Option<f64>>, Vec<Option<String>>) = (Vec::with_capacity(height), Vec::with_capacity(height));
        for k in 0..height {
            let (n, t) = if inside(k) {
                match (values.get(k), texts.get(k)) {
                    (_, Some(t)) => (None, Some(t.to_string())),
                    (Some(v), None) if v.is_finite() => (Some(v), None),
                    _ => (Some(f64::NAN), None),
                }
            } else {
                (old_n.f64(k), old_t.str(k).map(|t| t.to_string()))
            };
            let same_n = match (n, old_n.f64(k)) {
                (Some(a), Some(b)) => a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
                (None, None) => true,
                _ => false,
            };
            if !same_n || t.as_deref() != old_t.str(k) {
                changed = true;
            }
            numbers.push(n);
            strings.push(t);
        }
        if !changed {
            return Ok((values, texts, out));
        }
        frame.set(OWN_NUM, numbers);
        frame.set(OWN_TEXT, strings);
    }
    Err("iteration did not settle".into())
}

/// The intermediate columns grouped so that each group reads only earlier groups.
fn layers(lets: &Lets) -> Vec<Vec<usize>> {
    let index: HashMap<&str, usize> = lets.iter().enumerate().map(|(k, (n, _))| (n.as_str(), k)).collect();
    let mut level = vec![0usize; lets.len()];
    for (k, (_, e)) in lets.iter().enumerate() {
        let mut deepest = 0;
        for name in e.columns() {
            if let Some(&j) = index.get(name.as_str()) {
                if j < k {
                    deepest = deepest.max(level[j] + 1);
                }
            }
        }
        level[k] = deepest;
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (k, l) in level.into_iter().enumerate() {
        if out.len() <= l {
            out.resize(l + 1, Vec::new());
        }
        out[l].push(k);
    }
    out
}

/// The columns a run reads as a frame: one row per sheet row from `reach.0` rows
/// above the run to `reach.1` rows below it. Returns the frame and the number of
/// rows before the run's first row.
fn inputs(wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, run: &Run, lifted: &LiftedRun) -> Lifted<(Table, usize)> {
    let c = &wb.cells;
    let first = run.row as i64 + lifted.reach.0;
    let last = (run.row + run.len - 1) as i64 + lifted.reach.1;
    let height = (last - first + 1) as usize;
    let mut frame = Table::new(height);
    frame.push(ROW, (first..=last).map(|r| r + 1).collect::<Vec<i64>>());
    for &(sheet, column) in &lifted.columns {
        let mut values: Vec<Option<f64>> = Vec::with_capacity(height);
        for row in first..=last {
            // Rows above the sheet read as empty.
            let cell = if row < 0 { None } else { at.get(&(sheet, row as u32, column - 1)) };
            values.push(match cell {
                None => None,
                Some(&i) => match c.kind[i] {
                    Kind::Blank | Kind::Text => None,
                    Kind::Number | Kind::Bool => Some(c.num[i]),
                    Kind::Error => Some(f64::NAN),
                },
            });
        }
        frame.push(column_name(sheet, column), values);
    }
    for &(sheet, column) in &lifted.text_columns {
        let mut values: Vec<Option<&str>> = Vec::with_capacity(height);
        for row in first..=last {
            let cell = if row < 0 { None } else { at.get(&(sheet, row as u32, column - 1)) };
            values.push(match cell {
                None => None,
                Some(&i) => match c.kind[i] {
                    Kind::Text => Some(c.text[i].as_deref().unwrap_or("")),
                    // A formula without a stored value gave the empty text.
                    Kind::Blank if c.formula[i].is_some() => Some(""),
                    _ => None,
                },
            });
        }
        frame.push(text_column_name(sheet, column), values);
    }
    for &(sheet, column) in &lifted.parsed_columns {
        let mut values: Vec<Option<f64>> = Vec::with_capacity(height);
        for row in first..=last {
            let cell = if row < 0 { None } else { at.get(&(sheet, row as u32, column - 1)) };
            values.push(match cell {
                Some(&i) if c.kind[i] == Kind::Text => text_as_number(c.text[i].as_deref().unwrap_or(""), wb.date1904),
                _ => None,
            });
        }
        frame.push(parsed_column_name(sheet, column), values);
    }
    for &(key, _) in &lifted.assumptions.ranges {
        let s = summarize(wb, at, key);
        let some = |v: f64| (s.count > 0.0).then_some(v);
        for (what, v) in [('s', Some(s.sum)), ('n', Some(s.count)), ('l', some(s.min)), ('h', some(s.max)), ('q', Some(s.sum_squares))] {
            frame.push(range_input_name(key, what), vec![v; height]);
        }
    }
    for &(sheet, row, col, text) in &lifted.assumptions.cell_inputs {
        let cell = at.get(&(sheet, row, col)).copied();
        if text {
            let value: Option<String> = cell.and_then(|i| (c.kind[i] == Kind::Text).then(|| c.text[i].as_deref().unwrap_or("").to_string()));
            frame.push(cell_input_name(sheet, row, col, true), vec![value; height]);
        } else {
            let value: Option<f64> = cell.and_then(|i| match c.kind[i] {
                Kind::Number | Kind::Bool => Some(c.num[i]),
                _ => None,
            });
            frame.push(cell_input_name(sheet, row, col, false), vec![value; height]);
        }
    }
    Ok((frame, (-lifted.reach.0) as usize))
}

/// The workbook with what the file kept of other workbooks added as sheets named
/// `[1]Sheet1`, as formulas name them.
fn with_externals(wb: &Workbook) -> Workbook {
    let mut out = wb.clone();
    let sheet_ref = regex_lite_sheets();
    for (n, ext) in wb.externals.iter().enumerate() {
        // Its names, with their sheets named as the appended sheets are.
        for (name, value) in ext.names.iter().filter(|(_, v)| !v.is_empty()) {
            // Some writers put the workbook's own index before its sheets: drop it first.
            let mut plain = String::with_capacity(value.len());
            let mut rest = value.as_str();
            while let Some(open) = rest.find('[') {
                match rest[open..].find(']') {
                    Some(close) if rest[open + 1..open + close].bytes().all(|b| b.is_ascii_digit()) => {
                        plain.push_str(&rest[..open]);
                        rest = &rest[open + close + 1..];
                    }
                    _ => {
                        plain.push_str(&rest[..=open]);
                        rest = &rest[open + 1..];
                    }
                }
            }
            plain.push_str(rest);
            let value = sheet_ref(&plain, n + 1);
            out.defined_names.push(xl_cells::DefinedName { hidden: false, name: format!("[{}]!{}", n + 1, name), local_sheet: None, value });
        }
        let first = out.sheets.len() as u32;
        for name in &ext.sheets {
            out.sheets.push(xl_cells::Sheet { name: format!("[{}]{}", n + 1, name), ..xl_cells::Sheet::default() });
        }
        for (sheet, row, col, value) in &ext.cells {
            if (*sheet as usize) < ext.sheets.len() {
                out.cells.push(first + sheet, *row, *col, value.clone(), None, 0);
            }
        }
    }
    out
}

/// Rewrites `Sheet!` and `'Sheet'!` in a formula of another workbook to `'[n]Sheet'!`.
fn regex_lite_sheets() -> impl Fn(&str, usize) -> String {
    |formula: &str, n: usize| {
        let mut out = String::with_capacity(formula.len() + 8);
        let chars: Vec<char> = formula.chars().collect();
        let mut i = 0;
        let mut in_string = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                in_string = !in_string;
                out.push(c);
                i += 1;
                continue;
            }
            if in_string {
                out.push(c);
                i += 1;
                continue;
            }
            if c == '\'' {
                // A quoted sheet name up to the closing quote (doubled quotes inside).
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len() {
                    if chars[j] == '\'' && chars.get(j + 1) == Some(&'\'') {
                        name.push_str("''");
                        j += 2;
                    } else if chars[j] == '\'' {
                        break;
                    } else {
                        name.push(chars[j]);
                        j += 1;
                    }
                }
                if chars.get(j + 1) == Some(&'!') {
                    out.push_str(&format!("'[{n}]{name}'!"));
                    i = j + 2;
                    continue;
                }
                out.push(c);
                i += 1;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.') {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();
                if chars.get(j) == Some(&'!') && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '$')) {
                    out.push_str(&format!("'[{n}]{word}'!"));
                    i = j + 1;
                } else {
                    out.push_str(&word);
                    i = j;
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }
}

/// Lift every run of at least `min_run` cells (1: every formula cell) of a workbook,
/// with the expressions run by `ex`, and check each against the stored values.
pub fn lift_with(ex: &dyn Executor, original: &Workbook, min_run: usize) -> Report {
    lift_opts(ex, original, min_run, Options::default())
}

/// [`lift_with`] with the translation set by `opts`.
pub fn lift_opts(ex: &dyn Executor, original: &Workbook, min_run: usize, opts: Options) -> Report {
    let derived;
    let wb = if original.externals.iter().any(|e| !e.cells.is_empty() || !e.names.is_empty()) {
        derived = with_externals(original);
        &derived
    } else {
        original
    };
    let at = index(wb);
    let c = &wb.cells;
    let mut report = Report::default();
    let ranges = RangeCache::default();
    let show = std::env::var("LIFT_SHOW").ok();
    let mut last_rows = vec![0u32; wb.sheets.len()];
    for i in 0..c.len() {
        let last = &mut last_rows[c.sheet[i] as usize];
        *last = (*last).max(c.row[i] + 1);
    }
    // A column formula that cannot be lifted as a whole is tried cell by cell: in a
    // single cell every reference is fixed, so texts, criteria and moving tables are known.
    let mut queue: std::collections::VecDeque<(Run, Option<String>)> = runs_from(wb, min_run).into_iter().map(|r| (r, None)).collect();
    let cells_of_run = |run: &Run, reason: String| -> Vec<(Run, Option<String>)> {
        (0..run.len)
            .filter_map(|k| {
                let i = *at.get(&(run.sheet, run.row + k, run.col))?;
                Some((Run { sheet: run.sheet, col: run.col, row: run.row + k, len: 1, formula: c.formula[i].as_deref().unwrap_or("").to_string() }, Some(reason.clone())))
            })
            .collect()
    };
    // Cell by cell costs time per cell: at most a minute per workbook.
    let started = std::time::Instant::now();
    let may_split = |run: &Run| run.len > 1 && started.elapsed().as_secs() < 60;
    let slow = std::env::var_os("LIFT_SLOW").is_some();
    let mut last: Option<(std::time::Instant, String)> = None;
    while let Some((run, parent)) = queue.pop_front() {
        if let Some((t, f)) = last.take() {
            if slow && t.elapsed().as_millis() > 500 {
                eprintln!("slow\t{} ms\t={}", t.elapsed().as_millis(), f.chars().take(200).collect::<String>());
            }
        }
        if slow {
            last = Some((std::time::Instant::now(), run.formula.clone()));
        }
        let child = parent.is_some();
        if !child {
            report.runs += 1;
            report.run_cells += run.len as u64;
        }
        let refuse = |report: &mut Report, reason: String| {
            if show.as_deref().is_some_and(|s| reason.contains(s)) {
                eprintln!("{reason}\t{}\t={}", run.len, run.formula.chars().take(160).collect::<String>());
            }
            *report.unsupported.entry(reason).or_default() += run.len as u64
        };
        if let Some(reason) = parent.filter(|_| started.elapsed().as_secs() >= 60) {
            refuse(&mut report, reason);
            continue;
        }
        let lifted = match translate_run(ex, opts.param, wb, &at, &ranges, &last_rows, original.sheets.len() as u32, &run) {
            Ok(l) => l,
            Err(reason) if may_split(&run) => {
                queue.extend(cells_of_run(&run, reason));
                continue;
            }
            Err(reason) => {
                refuse(&mut report, reason);
                continue;
            }
        };
        if std::env::var_os("LIFT_DEBUG").is_some() {
            eprintln!("run {}!R{}C{} len {} reach {:?} columns {} ={}", run.sheet, run.row + 1, run.col + 1, run.len, lifted.reach, lifted.columns.len(), run.formula.chars().take(120).collect::<String>());
        }
        *report.baked.entry(lifted.baked).or_default() += run.len as u64;
        let (frame, head) = match inputs(wb, &at, &run, &lifted) {
            Ok(f) => f,
            Err(reason) => {
                refuse(&mut report, reason);
                continue;
            }
        };
        let (values, texts, out) = match evaluate(ex, frame, head, &run, &lifted, wb, &at) {
            Ok(r) => r,
            Err(reason) if may_split(&run) => {
                queue.extend(cells_of_run(&run, reason));
                continue;
            }
            Err(reason) => {
                refuse(&mut report, reason);
                continue;
            }
        };
        // Rows the translation does not hold for go to the cell evaluator.
        let uncovered = lifted.guards.iter().enumerate().find(|(k, _)| {
            out.column(&format!("_g{k}")).is_some_and(|b| (head..(head + run.len as usize).min(b.len())).any(|r| b.bool(r) == Some(true)))
        });
        if let Some((_, (reason, _))) = uncovered {
            if may_split(&run) {
                queue.extend(cells_of_run(&run, format!("rows not covered: {reason}")));
                continue;
            }
            refuse(&mut report, format!("rows not covered: {reason}"));
            continue;
        }
        let mut wrong = 0u64;
        let mut unknown = 0u64;
        for (k, (value, text)) in values.iter().zip(texts.iter()).skip(head).take(run.len as usize).enumerate() {
            let i = at[&(run.sheet, run.row + k as u32, run.col)];
            let failed = text.is_none() && value.is_none_or(|v| !v.is_finite());
            let same = match c.kind[i] {
                Kind::Number => text.is_none() && value.is_some_and(|v| (v - c.num[i]).abs() <= 1e-9 * c.num[i].abs().max(1.0)) && !lifted.is_bool,
                Kind::Bool => lifted.is_bool && value == Some(c.num[i]),
                Kind::Error => failed,
                Kind::Blank => {
                    unknown += 1;
                    true
                }
                Kind::Text => text.is_some_and(|t| Some(t) == c.text[i].as_deref()),
            };
            if !same {
                wrong += 1;
                if report.examples.len() < 8 {
                    report.examples.push(format!("{}!R{}C{} ={}: stored {:?}, computed {:?} {:?}", wb.sheets[run.sheet as usize].name, run.row + k as u32 + 1, run.col + 1, run.formula, c.value(i), value, text));
                }
            }
        }
        if unknown == run.len as u64 {
            refuse(&mut report, "no stored values to check against".into());
        } else if wrong == 0 {
            report.lifted_runs += 1;
            report.lifted_cells += run.len as u64;
        } else {
            if show.as_deref().is_some_and(|s| "mismatch".contains(s)) {
                eprintln!("mismatch\t{}\t={}", run.len, run.formula.chars().take(160).collect::<String>());
            }
            report.mismatch_runs += 1;
            report.mismatch_cells += run.len as u64;
        }
    }
    report
}

/// The reason of a cell whose result is some error, the expression not telling which.
pub const ERROR_RESULT: &str = "the result is an error (which one the expression does not tell)";

/// The formula cells on circular references (indices into `wb.cells`), as `calculate`
/// finds them: by the cells and ranges the formulas read, names followed.
pub fn circular_cells(wb: &Workbook) -> Vec<usize> {
    let (edges, _) = deps::graph(wb);
    let mut out = deps::cyclic(&edges);
    out.sort_unstable();
    out
}

/// The workbook's calculation settings (`<calcPr>`): iteration of circular references,
/// and whether results keep full precision.
#[derive(Debug, Clone, Copy)]
struct CalcSettings {
    iterate: bool,
    count: u32,
    delta: f64,
    full_precision: bool,
}

impl CalcSettings {
    fn of(wb: &Workbook) -> Self {
        let mut s = CalcSettings { iterate: false, count: 100, delta: 0.001, full_precision: true };
        let Some((_, xml)) = wb.extra.iter().find(|(n, _)| n == "calcPr") else { return s };
        let attr = |name: &str| {
            let at = xml.find(&format!(" {name}=\""))? + name.len() + 3;
            xml[at..].split('"').next().map(str::to_string)
        };
        s.iterate = attr("iterate").is_some_and(|v| v == "1" || v == "true");
        if let Some(c) = attr("iterateCount").and_then(|v| v.parse().ok()) {
            s.count = c;
        }
        if let Some(d) = attr("iterateDelta").and_then(|v| v.parse::<f64>().ok()).filter(|d| d.is_finite() && *d >= 0.0) {
            s.delta = d;
        }
        s.full_precision = !attr("fullPrecision").is_some_and(|v| v == "0" || v == "false");
        s
    }
}

/// What [`calculate`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Calculation {
    /// Formula cells.
    pub formula_cells: u64,
    /// Cells given a value by a lifted expression, and of those, how many changed.
    pub computed: u64,
    pub changed: u64,
    /// Cells left with the value they had, by reason (no translation, the result is an
    /// error whose kind the expression does not tell, ...).
    pub not_computed: BTreeMap<String, u64>,
    /// Rounds over all formulas until no value changed.
    pub rounds: u32,
    /// The values still moved after the last round (a cycle, or deeper than the limit).
    pub settled: bool,
    /// Cells on circular references worked out by iterative calculation.
    pub iterated: u64,
    /// The formula cells (indexes into the cells) left with the value they had, sorted,
    /// each with the reason.
    pub uncomputed: Vec<(usize, String)>,
}

/// The value of a cell: (kind, number, text).
fn value_of(c: &xl_cells::Cells, i: usize) -> (Kind, f64, Option<&str>) {
    (c.kind[i], c.num[i], c.text[i].as_deref())
}

/// Every formula value of a workbook removed and calculated again, as text: a line of
/// what happened, then one line per formula cell (`sheet!row,col`, the value, and the
/// reason where it was not calculated). Cells on circular references keep their value,
/// 1 % off, to start from. Two executors (or two versions of the engine) that agree
/// give the same text.
pub fn recalculation(ex: &dyn Executor, stored: &Workbook) -> String {
    recalculation_opts(ex, stored, Options::default())
}

/// [`recalculation`] with the translation set by `opts`.
pub fn recalculation_opts(ex: &dyn Executor, stored: &Workbook, opts: Options) -> String {
    let mut out = String::new();
    let mut blank = stored.clone();
    let on_cycle: std::collections::HashSet<usize> = circular_cells(stored).into_iter().collect();
    for i in 0..blank.cells.len() {
        if blank.cells.formula[i].is_none() {
            continue;
        }
        if on_cycle.contains(&i) && blank.cells.kind[i] == Kind::Number {
            blank.cells.num[i] *= 1.01;
            continue;
        }
        blank.cells.kind[i] = Kind::Blank;
        blank.cells.text[i] = None;
        blank.cells.num[i] = 0.0;
    }
    let (got, calc) = calculate_opts(ex, &blank, opts);
    out += &format!("calc\tcells {} computed {} changed {} rounds {} settled {} iterated {} why {:?}\n", calc.formula_cells, calc.computed, calc.changed, calc.rounds, calc.settled, calc.iterated, calc.not_computed);
    let reasons: HashMap<usize, &str> = calc.uncomputed.iter().map(|(i, r)| (*i, r.as_str())).collect();
    let c = &got.cells;
    for i in 0..c.len() {
        if c.formula[i].is_none() {
            continue;
        }
        let value = match c.kind[i] {
            Kind::Blank => "blank".to_string(),
            Kind::Number => format!("n {:016x}", c.num[i].to_bits()),
            Kind::Bool => format!("b {}", c.num[i]),
            Kind::Text => format!("t {:?}", c.text[i].as_deref().unwrap_or("")),
            Kind::Error => format!("e {}", c.text[i].as_deref().unwrap_or("")),
        };
        out += &format!("{}!{},{}\t{value}\t{}\n", c.sheet[i], c.row[i], c.col[i], reasons.get(&i).copied().unwrap_or(""));
    }
    out
}

/// Calculate the formulas of a workbook with their column expressions, run by `ex`: every
/// formula cell whose expression holds gets its value, in rounds over all formulas
/// until no value changes (each round settles at least one more level of dependence).
/// Cells without a translation keep the value they had and are counted by reason;
/// with any such cell the caller should leave the rest to the spreadsheet program
/// (calculation on load).
pub fn calculate_with(ex: &dyn Executor, original: &Workbook) -> (Workbook, Calculation) {
    calculate_opts(ex, original, Options::default())
}

/// [`calculate_with`] with the translation set by `opts`.
pub fn calculate_opts(ex: &dyn Executor, original: &Workbook, opts: Options) -> (Workbook, Calculation) {
    const MAX_ROUNDS: u32 = 64;
    let mut out = original.clone();
    let mut calc = Calculation::default();
    let has_externals = original.externals.iter().any(|e| !e.cells.is_empty() || !e.names.is_empty());
    let mut wb = if has_externals { with_externals(original) } else { original.clone() };
    let at = index(&wb);
    let own_cells = original.cells.len();
    let runs = runs_from(&wb, 1);
    calc.formula_cells = runs.iter().map(|r| r.len as u64).sum();
    let settings = CalcSettings::of(original);
    // With precision as displayed every result is rounded to its number format: not modelled.
    if !settings.full_precision {
        calc.not_computed.insert("precision as displayed (results rounded to their format)".into(), calc.formula_cells);
        calc.uncomputed = (0..original.cells.len()).filter(|&i| original.cells.formula[i].is_some()).map(|i| (i, "precision as displayed".to_string())).collect();
        return (out, calc);
    }
    // Cells on a cycle: with iterative calculation they are worked out by repetition until
    // no value moves by more than the workbook's delta, else they are left alone.
    let (edges, dynamic) = deps::graph(original);
    let on_cycle: std::collections::HashSet<usize> = deps::cyclic(&edges).into_iter().collect();
    let mut last_rows = vec![0u32; wb.sheets.len()];
    for i in 0..wb.cells.len() {
        let last = &mut last_rows[wb.cells.sheet[i] as usize];
        *last = (*last).max(wb.cells.row[i] + 1);
    }
    // Runs that cannot be lifted as a whole are tried cell by cell once, as in `lift`.
    let mut work: Vec<Run> = Vec::new();
    for run in runs {
        let ranges = RangeCache::default();
        match translate_run(ex, opts.param, &wb, &at, &ranges, &last_rows, original.sheets.len() as u32, &run) {
            Ok(_) => work.push(run),
            Err(_) if run.len > 1 => {
                for k in 0..run.len {
                    if let Some(&i) = at.get(&(run.sheet, run.row + k, run.col)) {
                        work.push(Run { sheet: run.sheet, col: run.col, row: run.row + k, len: 1, formula: wb.cells.formula[i].clone().unwrap_or_default() });
                    }
                }
            }
            Err(_) => work.push(run),
        }
    }
    let mut reasons: HashMap<(u32, u32, u32), String> = HashMap::new();
    let mut computed: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // XL_TRACE=sheet,row,col (0-based): the values one cell takes, round by round.
    let traced: Vec<(u32, u32, u32)> = std::env::var("XL_TRACE")
        .ok()
        .map(|t| {
            t.split(';')
                .filter_map(|c| {
                    let v: Vec<u32> = c.split(',').filter_map(|x| x.parse().ok()).collect();
                    (v.len() == 3).then(|| (v[0], v[1], v[2]))
                })
                .collect()
        })
        .unwrap_or_default();
    // Cells whose value still changed in the last round.
    let mut changed_last: Vec<usize> = Vec::new();
    let max_rounds = if settings.iterate { MAX_ROUNDS + settings.count } else { MAX_ROUNDS };
    // Iteration starts from the values the cells have; one without a value starts at 0
    // (as a new formula does in Excel), not as the empty text of a formula.
    if settings.iterate {
        for &i in &on_cycle {
            if wb.cells.kind[i] == Kind::Blank {
                wb.cells.kind[i] = Kind::Number;
                wb.cells.num[i] = 0.0;
            }
        }
    }
    for round in 1..=max_rounds {
        calc.rounds = round;
        let mut moved = false;
        let mut cycle_change = 0.0f64;
        changed_last.clear();
        reasons.clear();
        let ranges = RangeCache::default();
        for run in &work {
            let fail = |reasons: &mut HashMap<(u32, u32, u32), String>, why: &str| {
                if traced.iter().any(|t| t.0 == run.sheet && t.2 == run.col && (run.row..run.row + run.len).contains(&t.1)) {
                    eprintln!("round {round} fail {}:{}:{}: {why}", run.sheet, run.row, run.col);
                }
                for k in 0..run.len {
                    reasons.insert((run.sheet, run.row + k, run.col), why.to_string());
                }
            };
            let lifted = match translate_run(ex, opts.param, &wb, &at, &ranges, &last_rows, original.sheets.len() as u32, run) {
                Ok(l) => l,
                Err(why) => {
                    fail(&mut reasons, &why);
                    continue;
                }
            };
            let (frame, head) = match inputs(&wb, &at, run, &lifted) {
                Ok(f) => f,
                Err(why) => {
                    fail(&mut reasons, &why);
                    continue;
                }
            };
            let (values, texts, frame_out) = match evaluate(ex, frame, head, run, &lifted, &wb, &at) {
                Ok(r) => r,
                Err(why) => {
                    fail(&mut reasons, &why);
                    continue;
                }
            };
            for k in 0..run.len as usize {
                let key = (run.sheet, run.row + k as u32, run.col);
                let Some(&i) = at.get(&key) else { continue };
                if traced.contains(&key) {
                    eprintln!("round {round} reached {key:?}: text {:?} value {:?}", texts.get(head + k), values.get(head + k));
                }
                // A row the translation does not hold for keeps its value.
                let uncovered = lifted.guards.iter().enumerate().find(|(g, _)| {
                    frame_out.column(&format!("_g{g}")).and_then(|b| b.bool(head + k)).unwrap_or(false)
                });
                if let Some((_, (why, _))) = uncovered {
                    reasons.insert(key, format!("rows not covered: {why}"));
                    continue;
                }
                let new = match (texts.get(head + k), values.get(head + k)) {
                    (Some(t), _) => (Kind::Text, 0.0, Some(t.to_string())),
                    (None, Some(v)) if v.is_finite() && lifted.is_bool => (Kind::Bool, if v != 0.0 { 1.0 } else { 0.0 }, None),
                    (None, Some(v)) if v.is_finite() => (Kind::Number, if v == 0.0 { 0.0 } else { v }, None),
                    _ => {
                        // An error: which one the expression does not say. Readers see an
                        // error (as they would in Excel); the cell keeps its own value.
                        reasons.insert(key, ERROR_RESULT.into());
                        let c = &mut wb.cells;
                        if c.kind[i] != Kind::Error {
                            if on_cycle.contains(&i) {
                                cycle_change = f64::INFINITY;
                            } else {
                                moved = true;
                            }
                            changed_last.push(i);
                            c.kind[i] = Kind::Error;
                            c.num[i] = 0.0;
                            c.text[i] = Some(std::sync::Arc::from("#VALUE!"));
                        }
                        continue;
                    }
                };
                if traced.contains(&key) {
                    eprintln!("round {round} {key:?}: {:?} -> {:?}", value_of(&wb.cells, i), new);
                }
                let old = value_of(&wb.cells, i);
                let same = old.0 == new.0 && old.1.to_bits() == new.1.to_bits() && old.2 == new.2.as_deref();
                computed.insert(i);
                if !same {
                    let numbers = old.0 == Kind::Number && new.0 == Kind::Number;
                    if on_cycle.contains(&i) {
                        let step = if numbers { (new.1 - old.1).abs() } else { f64::INFINITY };
                        cycle_change = cycle_change.max(step);
                    } else if !(settings.iterate && numbers && (new.1 - old.1).abs() <= 1e-12 * new.1.abs().max(old.1.abs())) {
                        // While iterating, cells after a cycle follow its last digits round
                        // after round; that is no movement of their own.
                        moved = true;
                    }
                    changed_last.push(i);
                    let c = &mut wb.cells;
                    c.kind[i] = new.0;
                    c.num[i] = new.1;
                    c.text[i] = new.2.map(std::sync::Arc::from);
                    c.rich[i] = None;
                    c.iso[i] = xl_cells::IsoDate::No;
                }
            }
        }
        // Settled: nothing off the cycles moved, and on them (when iterating) nothing by
        // more than the delta. Without iteration the cycles do not count: they are not
        // trusted anyway.
        // Excel stops as soon as no value moves by more than the delta; with its own
        // values as the start that is close to where the cycle settles. We go on to a
        // thousandth of the delta (within the same count), so that the result does not
        // depend on how far off the start was.
        let cycles_still = settings.iterate && cycle_change > settings.delta / 1000.0;
        if std::env::var_os("XL_CYCLES").is_some() && settings.iterate {
            eprintln!("round {round}: moved {moved}, largest change on cycles {cycle_change}");
        }
        if !moved && !cycles_still {
            calc.settled = true;
            break;
        }
    }
    // Not to be trusted: cells on a cycle, cells not computed, and every cell that reads
    // one of them (directly or through others); a cell that reads cells only known when
    // evaluated (INDIRECT, OFFSET, ...) as soon as any cell is not computed.
    let mut bad: HashMap<usize, String> = HashMap::new();
    for (key, why) in &reasons {
        if let Some(&i) = at.get(key) {
            bad.insert(i, why.clone());
        }
    }
    if !(settings.iterate && calc.settled) {
        let why = if settings.iterate { format!("circular reference that does not settle within {} iterations", settings.count) } else { "circular reference".to_string() };
        for &i in &on_cycle {
            bad.insert(i, why.clone());
        }
    }
    // A formula cell no run covers was never calculated: it keeps its old value, which
    // its readers must not trust.
    let mut covered = vec![false; wb.cells.len()];
    for run in &work {
        for k in 0..run.len {
            if let Some(&i) = at.get(&(run.sheet, run.row + k, run.col)) {
                covered[i] = true;
            }
        }
    }
    for i in 0..own_cells {
        if original.cells.formula[i].as_deref().is_some_and(|f| !f.is_empty()) && !covered[i] {
            bad.entry(i).or_insert_with(|| "not taken into the calculation (no run covers it)".into());
        }
    }
    // Values that still moved after the last round are not final, nor is what reads them.
    if !calc.settled {
        for &i in &changed_last {
            bad.entry(i).or_insert_with(|| format!("not settled after {MAX_ROUNDS} rounds"));
        }
    }
    let mut readers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (&i, targets) in &edges {
        for &t in targets {
            readers.entry(t).or_default().push(i);
        }
    }
    // An error result does not spoil its readers: they see an error either way, unless
    // they ask which error it is.
    let kind_of_error = |i: usize| {
        original.cells.formula[i].as_deref().is_some_and(|f| {
            let f = f.to_ascii_uppercase();
            ["ISNA(", "IFNA(", "ERROR.TYPE(", "ISERR("].iter().any(|n| f.contains(n))
        })
    };
    let mut queue: Vec<usize> = bad.iter().filter(|(_, why)| why.as_str() != ERROR_RESULT).map(|(i, _)| *i).collect();
    for (i, why) in bad.clone() {
        if why == ERROR_RESULT {
            for &r in readers.get(&i).map(|v| v.as_slice()).unwrap_or(&[]) {
                if kind_of_error(r) && !bad.contains_key(&r) {
                    bad.insert(r, "asks which error a cell has".into());
                    queue.push(r);
                }
            }
        }
    }
    if !bad.is_empty() {
        for &i in &dynamic {
            if !bad.contains_key(&i) {
                bad.insert(i, "reads cells known only when evaluated".into());
                queue.push(i);
            }
        }
    }
    while let Some(i) = queue.pop() {
        for &r in readers.get(&i).map(|v| v.as_slice()).unwrap_or(&[]) {
            // An error result that read a cell not computed is no error we know of: it may
            // be one only because that cell had no value. It spoils its readers too.
            if bad.get(&r).is_none_or(|why| why == ERROR_RESULT) {
                bad.insert(r, "reads a cell not computed".into());
                queue.push(r);
            }
        }
    }
    if std::env::var_os("XL_CYCLES").is_some() {
        let mut why: BTreeMap<&str, usize> = BTreeMap::new();
        for i in &on_cycle {
            *why.entry(bad.get(i).map(|s| s.as_str()).unwrap_or("computed")).or_default() += 1;
        }
        eprintln!("cells on cycles: {} (settled {}, rounds {}): {why:?}", on_cycle.len(), calc.settled, calc.rounds);
        let mut some: Vec<usize> = on_cycle.iter().copied().collect();
        some.sort_unstable();
        for &i in some.iter().take(12) {
            let c = &original.cells;
            eprintln!("  {}!{}{} = {} : start {:?}, now {:?}", original.sheets[c.sheet[i] as usize].name, xl_formula::col_name(c.col[i] + 1), c.row[i] + 1, c.formula[i].as_deref().unwrap_or(""), c.value(i), wb.cells.value(i));
        }
    }
    for key in &traced {
        for run in work.iter().filter(|r| r.sheet == key.0 && r.col == key.2 && (r.row..r.row + r.len).contains(&key.1)) {
            eprintln!("trace {key:?}: run from row {} len {}: {}", run.row, run.len, run.formula);
        }
        let i = at.get(key);
        eprintln!("trace {key:?}: reason {:?}, bad {:?}", reasons.get(key), i.and_then(|i| bad.get(i)));
    }
    for (i, why) in &bad {
        computed.remove(i);
        *calc.not_computed.entry(why.clone()).or_default() += 1;
        if *i < own_cells {
            calc.uncomputed.push((*i, why.clone()));
        }
    }
    calc.uncomputed.sort_unstable();
    for &i in &computed {
        if i >= own_cells {
            continue;
        }
        calc.computed += 1;
        if on_cycle.contains(&i) {
            calc.iterated += 1;
        }
        let (c, o) = (&wb.cells, &mut out.cells);
        if value_of(c, i) != value_of(o, i) {
            calc.changed += 1;
            o.kind[i] = c.kind[i];
            o.num[i] = c.num[i];
            o.text[i] = c.text[i].clone();
            o.rich[i] = None;
            o.iso[i] = xl_cells::IsoDate::No;
        }
    }
    (out, calc)
}

/// How translations made on one workbook hold on another (see [`reuse_check`]).
#[derive(Debug, Clone, Default)]
pub struct Reuse {
    /// Formula cells of the runs that were translated.
    pub cells: u64,
    /// Cells whose value the old translation gives on the new data, as the new workbook has it.
    pub ok: u64,
    /// The same, with the assumptions of the translation violated by the new data (expected
    /// to differ; counted whether they do or not).
    pub violated: u64,
    /// Cells where the assumptions hold and the value is not the new workbook's: a fault.
    pub wrong: u64,
    /// Cells of runs the plan could not be evaluated for on the new data.
    pub failed: u64,
    /// Cells of runs not translated at all (by the engine's own reasons).
    pub untranslated: u64,
    /// Cells the calculation of the new data left with their value: not compared.
    pub left: u64,
    pub examples: Vec<String>,
    /// The first assumption violated, by reason, in cells.
    pub why_violated: BTreeMap<String, u64>,
}

impl Reuse {
    pub fn add(&mut self, o: &Reuse) {
        self.cells += o.cells;
        self.ok += o.ok;
        self.violated += o.violated;
        self.wrong += o.wrong;
        self.failed += o.failed;
        self.untranslated += o.untranslated;
        self.left += o.left;
        for (k, v) in &o.why_violated {
            *self.why_violated.entry(k.clone()).or_default() += v;
        }
        self.examples.extend(o.examples.iter().take(60usize.saturating_sub(self.examples.len())).cloned());
    }
}

/// The first assumption of a translation made on `old` that does not hold on `data`.
fn violated(a: &Assumptions, old: &Workbook, old_at: &HashMap<(u32, u32, u32), usize>, data: &Workbook, data_at: &HashMap<(u32, u32, u32), usize>) -> Option<String> {
    let same = |key: (u32, u32, u32)| match (old_at.get(&key), data_at.get(&key)) {
        (None, None) => true,
        (Some(&i), Some(&j)) => {
            let (a, b) = (&old.cells, &data.cells);
            a.kind[i] == b.kind[j] && a.num[i].to_bits() == b.num[j].to_bits() && a.text[i] == b.text[j]
        }
        _ => false,
    };
    for r in &a.pinned {
        if (r.first..=r.last).any(|row| !same((r.sheet, row, r.col))) {
            return Some("a value read while translating changed".into());
        }
    }
    let extent = |wb: &Workbook, sheet: u32| (0..wb.cells.len()).filter(|&i| wb.cells.sheet[i] == sheet).map(|i| wb.cells.row[i] + 1).max().unwrap_or(0);
    if a.extents.iter().any(|&s| extent(old, s) != extent(data, s)) {
        return Some("the extent of a sheet changed".into());
    }
    for &(s, r, c, text) in &a.cell_inputs {
        let kind = data_at.get(&(s, r, c)).map(|&i| data.cells.kind[i]);
        if matches!(kind, Some(Kind::Text)) != text || !matches!(kind, Some(Kind::Number | Kind::Bool | Kind::Text)) {
            return Some("a fixed cell changed its kind".into());
        }
    }
    for &(key, error) in &a.ranges {
        if summarize(data, data_at, key).error != error {
            return Some("a fixed range changed its errors".into());
        }
    }
    for k in &a.columns {
        let (text, error) = holds_in(data, data_at, k.sheet, k.col, k.first, k.last);
        if (text && !k.text) || (error && !k.error) {
            return Some("a column holds a kind it did not".into());
        }
    }
    None
}

/// Do translations made on `old` hold on `data`? `data` has the layout and formulas of
/// `old` and other values, formulas calculated (cells the engine leaves alone keep what
/// they hold). Every run of `old` is translated once (with `opts`), its plan is run on the
/// inputs of `data`, and what it gives is compared with `data`'s values. Where the
/// translation's assumptions are violated by `data` a difference is expected and counted
/// apart; elsewhere it is a fault. Cells of `data` in `left` (cells the calculation left
/// with the value they had) are not compared.
pub fn reuse_check(ex: &dyn Executor, old: &Workbook, data: &Workbook, left: &HashSet<usize>, opts: Options) -> Reuse {
    let mut out = Reuse::default();
    let (at, data_at) = (index(old), index(data));
    let mut last_rows = vec![0u32; old.sheets.len()];
    for i in 0..old.cells.len() {
        let last = &mut last_rows[old.cells.sheet[i] as usize];
        *last = (*last).max(old.cells.row[i] + 1);
    }
    let external_from = old.sheets.len() as u32;
    let mut work: Vec<Run> = Vec::new();
    for run in runs_from(old, 1) {
        let ranges = RangeCache::default();
        match translate_run(ex, opts.param, old, &at, &ranges, &last_rows, external_from, &run) {
            Ok(_) => work.push(run),
            Err(_) if run.len > 1 => {
                for k in 0..run.len {
                    if let Some(&i) = at.get(&(run.sheet, run.row + k, run.col)) {
                        work.push(Run { sheet: run.sheet, col: run.col, row: run.row + k, len: 1, formula: old.cells.formula[i].clone().unwrap_or_default() });
                    }
                }
            }
            Err(_) => work.push(run),
        }
    }
    let d = &data.cells;
    for run in work {
        let ranges = RangeCache::default();
        let cells = run.len as u64;
        let Ok(lifted) = translate_run(ex, opts.param, old, &at, &ranges, &last_rows, external_from, &run) else {
            out.untranslated += cells;
            continue;
        };
        out.cells += cells;
        let Ok((frame, head)) = inputs(data, &data_at, &run, &lifted) else {
            out.failed += cells;
            continue;
        };
        let Ok((values, texts, table)) = evaluate(ex, frame, head, &run, &lifted, data, &data_at) else {
            out.failed += cells;
            continue;
        };
        let broken = violated(&lifted.assumptions, old, &at, data, &data_at);
        for k in 0..run.len as usize {
            let Some(&i) = data_at.get(&(run.sheet, run.row + k as u32, run.col)) else { continue };
            if left.contains(&i) {
                out.left += 1;
                continue;
            }
            // A row the translation does not hold for is left to the cell evaluator.
            if lifted.guards.iter().enumerate().any(|(g, _)| table.column(&format!("_g{g}")).and_then(|b| b.bool(head + k)).unwrap_or(false)) {
                out.failed += 1;
                continue;
            }
            let (value, text) = (values.get(head + k), texts.get(head + k));
            let failed = text.is_none() && value.is_none_or(|v| !v.is_finite());
            let same = match d.kind[i] {
                Kind::Number => text.is_none() && value.is_some_and(|v| (v - d.num[i]).abs() <= 1e-9 * d.num[i].abs().max(1.0)) && !lifted.is_bool,
                Kind::Bool => lifted.is_bool && value == Some(d.num[i]),
                Kind::Error => failed,
                Kind::Blank => true,
                Kind::Text => text.is_some_and(|t| Some(t) == d.text[i].as_deref()),
            };
            match (&broken, same) {
                (None, true) => out.ok += 1,
                (None, false) => {
                    out.wrong += 1;
                    if out.examples.len() < 60 {
                        out.examples.push(format!("{}!R{}C{} ={}: new {:?}, computed {:?} {:?}", old.sheets[run.sheet as usize].name, run.row + k as u32 + 1, run.col + 1, run.formula, data.cells.value(i), value, text));
                    }
                }
                (Some(why), _) => {
                    out.violated += 1;
                    *out.why_violated.entry(why.clone()).or_default() += 1;
                }
            }
        }
    }
    out
}
