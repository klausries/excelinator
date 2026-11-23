//! The expression form of the formula engine, free of any dataframe library.
//!
//! The engine (`lift`) translates Excel formulas into column expressions. It builds
//! them here, not as Polars expressions directly, so that the same translation can be
//! run by different executors: Polars in Rust (`xl-polars`), the
//! Polars of a Python environment (through a thin shim that builds `pl.Expr` from the
//! same nodes), later Polars for Dart or JavaScript.
//!
//! The vocabulary is the part of Polars the engine needs, with Polars' names and
//! meaning: an executor maps every node to the operation of the same name. Nothing is
//! evaluated here; [`Plan`] says which columns to add to an input [`Table`] and which
//! to give back.
//!
//! Expressions are shared, not copied, when cloned: the engine builds large graphs in
//! which one part is used in many places, and executors convert each part once.

use std::sync::Arc;

/// A column expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr(Arc<Node>);

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Column(String),
    Literal(Lit),
    Binary(Expr, Op, Expr),
    /// A function of Polars: the expression it is a method of, then its arguments.
    Function(F, Vec<Expr>),
    /// `when(condition).then(yes).otherwise(no)`.
    Ternary(Expr, Expr, Expr),
    Cast(Expr, DataType),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    F64(f64),
    Str(String),
    Series(Series),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Plus,
    Minus,
    Multiply,
    TrueDivide,
    Modulus,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataType {
    Boolean,
    Int64,
    UInt32,
    Float64,
    String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchSortedSide {
    Left,
    Right,
}

/// Functions, by their Polars name. Arguments other than expressions are part of the
/// function; expressions are in the node.
#[derive(Clone, Debug, PartialEq)]
pub enum F {
    Not,
    IsNull,
    IsNotNull,
    IsNan,
    IsFinite,
    /// `fill_null(value)`.
    FillNull,
    Abs,
    Floor,
    Ceil,
    Sqrt,
    Exp,
    /// `log(base)`.
    Log,
    /// `pow(exponent)`.
    Pow,
    Sum,
    Min,
    Max,
    Product,
    Count,
    First,
    Last,
    Reverse,
    Any(bool),
    All(bool),
    /// The flag is `reverse`.
    CumSum(bool),
    CumProd(bool),
    CumMin(bool),
    CumMax(bool),
    /// `shift(n)`.
    Shift,
    /// `replace_strict(old, new, default, return_dtype)`; the flag says whether a default is given.
    ReplaceStrict { default: bool, dtype: Option<DataType> },
    /// `search_sorted(element, side, descending)`.
    SearchSorted(SearchSortedSide, bool),
    /// `gather(indices, null_on_oob)`.
    Gather(bool),
    /// `get(index, null_on_oob)`.
    Get(bool),
    /// `filter(predicate)`.
    Filter,
    /// `slice(offset, length)`.
    Slice,
    StrToLowercase,
    StrToUppercase,
    StrLenChars,
    /// `str.head(n)`.
    StrHead,
    /// `str.tail(n)`.
    StrTail,
    /// `str.slice(offset, length)`.
    StrSlice,
    /// `str.contains(pattern, strict)`, a regular expression.
    StrContains(bool),
    /// `str.replace_all(pattern, value, literal)`.
    StrReplaceAll(bool),
    /// `str.strip_chars(characters)`; null strips white space.
    StrStripChars,
    /// `str.split(by)`.
    StrSplit,
    /// `str.find_literal(pattern)`.
    StrFindLiteral,
    /// `str.extract(pattern, group_index)`.
    StrExtract(usize),
    ListFirst,
    /// `min_horizontal`: the receiver and the arguments are the expressions compared.
    MinHorizontal,
    MaxHorizontal,
}

/// A named list of values, as a literal (keys and values of a lookup, for example).
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    pub values: Values,
}

/// The values of a column.
#[derive(Clone, Debug, PartialEq)]
pub enum Values {
    F64(Vec<f64>),
    OptF64(Vec<Option<f64>>),
    I64(Vec<i64>),
    Str(Vec<String>),
    OptStr(Vec<Option<String>>),
    OptBool(Vec<Option<bool>>),
}

impl Values {
    pub fn len(&self) -> usize {
        match self {
            Values::F64(v) => v.len(),
            Values::OptF64(v) => v.len(),
            Values::I64(v) => v.len(),
            Values::Str(v) => v.len(),
            Values::OptStr(v) => v.len(),
            Values::OptBool(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number at a row (null, or not a number column: `None`).
    pub fn f64(&self, k: usize) -> Option<f64> {
        match self {
            Values::F64(v) => v.get(k).copied(),
            Values::OptF64(v) => v.get(k).copied().flatten(),
            Values::I64(v) => v.get(k).map(|&x| x as f64),
            _ => None,
        }
    }

    pub fn str(&self, k: usize) -> Option<&str> {
        match self {
            Values::Str(v) => v.get(k).map(|s| s.as_str()),
            Values::OptStr(v) => v.get(k).and_then(|s| s.as_deref()),
            _ => None,
        }
    }

    pub fn bool(&self, k: usize) -> Option<bool> {
        match self {
            Values::OptBool(v) => v.get(k).copied().flatten(),
            _ => None,
        }
    }
}

impl From<Vec<f64>> for Values {
    fn from(v: Vec<f64>) -> Self {
        Values::F64(v)
    }
}
impl From<Vec<Option<f64>>> for Values {
    fn from(v: Vec<Option<f64>>) -> Self {
        Values::OptF64(v)
    }
}
impl From<Vec<i64>> for Values {
    fn from(v: Vec<i64>) -> Self {
        Values::I64(v)
    }
}
impl From<Vec<String>> for Values {
    fn from(v: Vec<String>) -> Self {
        Values::Str(v)
    }
}
impl From<Vec<&str>> for Values {
    fn from(v: Vec<&str>) -> Self {
        Values::Str(v.into_iter().map(str::to_string).collect())
    }
}
impl From<Vec<Option<String>>> for Values {
    fn from(v: Vec<Option<String>>) -> Self {
        Values::OptStr(v)
    }
}
impl From<Vec<Option<&str>>> for Values {
    fn from(v: Vec<Option<&str>>) -> Self {
        Values::OptStr(v.into_iter().map(|s| s.map(str::to_string)).collect())
    }
}
impl From<Vec<Option<bool>>> for Values {
    fn from(v: Vec<Option<bool>>) -> Self {
        Values::OptBool(v)
    }
}

impl Series {
    pub fn new(name: String, values: impl Into<Values>) -> Self {
        Series { name, values: values.into() }
    }
}

/// The null literal, as `lit(NULL)`.
pub struct Null;
pub const NULL: Null = Null;

pub trait Literal {
    fn into_lit(self) -> Lit;
}
impl Literal for Null {
    fn into_lit(self) -> Lit {
        Lit::Null
    }
}
impl Literal for bool {
    fn into_lit(self) -> Lit {
        Lit::Bool(self)
    }
}
impl Literal for i32 {
    fn into_lit(self) -> Lit {
        Lit::I32(self)
    }
}
impl Literal for i64 {
    fn into_lit(self) -> Lit {
        Lit::I64(self)
    }
}
impl Literal for u32 {
    fn into_lit(self) -> Lit {
        Lit::U32(self)
    }
}
impl Literal for u64 {
    fn into_lit(self) -> Lit {
        Lit::U64(self)
    }
}
impl Literal for f64 {
    fn into_lit(self) -> Lit {
        Lit::F64(self)
    }
}
impl Literal for &str {
    fn into_lit(self) -> Lit {
        Lit::Str(self.to_string())
    }
}
impl Literal for String {
    fn into_lit(self) -> Lit {
        Lit::Str(self)
    }
}
impl Literal for &String {
    fn into_lit(self) -> Lit {
        Lit::Str(self.clone())
    }
}
impl Literal for Series {
    fn into_lit(self) -> Lit {
        Lit::Series(self)
    }
}

pub fn lit(value: impl Literal) -> Expr {
    Expr::new(Node::Literal(value.into_lit()))
}

pub fn col(name: impl Into<String>) -> Expr {
    Expr::new(Node::Column(name.into()))
}

fn horizontal(f: F, exprs: Vec<Expr>) -> Result<Expr, String> {
    let mut exprs = exprs.into_iter();
    let first = exprs.next().ok_or_else(|| "cannot return empty fold because the number of output rows is unknown".to_string())?;
    Ok(first.f(f, exprs.collect()))
}

/// The smallest of the expressions, row by row (nulls left out).
pub fn min_horizontal(exprs: Vec<Expr>) -> Result<Expr, String> {
    horizontal(F::MinHorizontal, exprs)
}

/// The largest of the expressions, row by row (nulls left out).
pub fn max_horizontal(exprs: Vec<Expr>) -> Result<Expr, String> {
    horizontal(F::MaxHorizontal, exprs)
}

pub fn when(condition: Expr) -> When {
    When { condition }
}

pub struct When {
    condition: Expr,
}

pub struct Then {
    condition: Expr,
    yes: Expr,
}

impl When {
    pub fn then(self, yes: Expr) -> Then {
        Then { condition: self.condition, yes }
    }
}

impl Then {
    pub fn otherwise(self, no: Expr) -> Expr {
        Expr::new(Node::Ternary(self.condition, self.yes, no))
    }
}

impl Expr {
    pub fn new(node: Node) -> Self {
        Expr(Arc::new(node))
    }

    pub fn node(&self) -> &Node {
        &self.0
    }

    /// An address that is the same for clones of one expression (for executors that
    /// convert each shared part once).
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    fn f(self, f: F, args: Vec<Expr>) -> Expr {
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(self);
        all.extend(args);
        Expr::new(Node::Function(f, all))
    }

    fn bin(self, op: Op, other: Expr) -> Expr {
        Expr::new(Node::Binary(self, op, other))
    }

    pub fn is_column(&self) -> bool {
        matches!(self.node(), Node::Column(_))
    }

    pub fn is_literal(&self) -> bool {
        matches!(self.node(), Node::Literal(_))
    }

    /// The names of the columns the expression reads, each once, in the order met.
    pub fn columns(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        let mut names_seen = std::collections::HashSet::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            if !seen.insert(e.id()) {
                continue;
            }
            match e.node() {
                Node::Column(n) => {
                    if names_seen.insert(n.clone()) {
                        names.push(n.clone());
                    }
                }
                Node::Literal(_) => {}
                Node::Binary(a, _, b) => {
                    stack.push(b);
                    stack.push(a);
                }
                Node::Function(_, args) => stack.extend(args.iter().rev()),
                Node::Ternary(c, a, b) => {
                    stack.push(b);
                    stack.push(a);
                    stack.push(c);
                }
                Node::Cast(a, _) => stack.push(a),
            }
        }
        names
    }

    /// Whether the expression reads any column.
    pub fn reads_columns(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            if !seen.insert(e.id()) {
                continue;
            }
            match e.node() {
                Node::Column(_) => return true,
                Node::Literal(_) => {}
                Node::Binary(a, _, b) => {
                    stack.push(a);
                    stack.push(b);
                }
                Node::Function(_, args) => stack.extend(args.iter()),
                Node::Ternary(c, a, b) => {
                    stack.push(c);
                    stack.push(a);
                    stack.push(b);
                }
                Node::Cast(a, _) => stack.push(a),
            }
        }
        false
    }

    /// The number of distinct nodes.
    pub fn size(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            if !seen.insert(e.id()) {
                continue;
            }
            match e.node() {
                Node::Column(_) | Node::Literal(_) => {}
                Node::Binary(a, _, b) => {
                    stack.push(a);
                    stack.push(b);
                }
                Node::Function(_, args) => stack.extend(args.iter()),
                Node::Ternary(c, a, b) => {
                    stack.push(c);
                    stack.push(a);
                    stack.push(b);
                }
                Node::Cast(a, _) => stack.push(a),
            }
        }
        seen.len()
    }

    pub fn cast(self, dtype: DataType) -> Expr {
        Expr::new(Node::Cast(self, dtype))
    }

    pub fn eq(self, other: Expr) -> Expr {
        self.bin(Op::Eq, other)
    }
    pub fn neq(self, other: Expr) -> Expr {
        self.bin(Op::NotEq, other)
    }
    pub fn lt(self, other: Expr) -> Expr {
        self.bin(Op::Lt, other)
    }
    pub fn lt_eq(self, other: Expr) -> Expr {
        self.bin(Op::LtEq, other)
    }
    pub fn gt(self, other: Expr) -> Expr {
        self.bin(Op::Gt, other)
    }
    pub fn gt_eq(self, other: Expr) -> Expr {
        self.bin(Op::GtEq, other)
    }
    pub fn and(self, other: Expr) -> Expr {
        self.bin(Op::And, other)
    }
    pub fn or(self, other: Expr) -> Expr {
        self.bin(Op::Or, other)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Expr {
        self.f(F::Not, vec![])
    }
    pub fn is_null(self) -> Expr {
        self.f(F::IsNull, vec![])
    }
    pub fn is_not_null(self) -> Expr {
        self.f(F::IsNotNull, vec![])
    }
    pub fn is_nan(self) -> Expr {
        self.f(F::IsNan, vec![])
    }
    pub fn is_finite(self) -> Expr {
        self.f(F::IsFinite, vec![])
    }
    pub fn fill_null(self, value: Expr) -> Expr {
        self.f(F::FillNull, vec![value])
    }
    pub fn abs(self) -> Expr {
        self.f(F::Abs, vec![])
    }
    pub fn floor(self) -> Expr {
        self.f(F::Floor, vec![])
    }
    pub fn ceil(self) -> Expr {
        self.f(F::Ceil, vec![])
    }
    pub fn sqrt(self) -> Expr {
        self.f(F::Sqrt, vec![])
    }
    pub fn exp(self) -> Expr {
        self.f(F::Exp, vec![])
    }
    pub fn log(self, base: Expr) -> Expr {
        self.f(F::Log, vec![base])
    }
    pub fn pow(self, exponent: Expr) -> Expr {
        self.f(F::Pow, vec![exponent])
    }
    pub fn sum(self) -> Expr {
        self.f(F::Sum, vec![])
    }
    pub fn min(self) -> Expr {
        self.f(F::Min, vec![])
    }
    pub fn max(self) -> Expr {
        self.f(F::Max, vec![])
    }
    pub fn product(self) -> Expr {
        self.f(F::Product, vec![])
    }
    pub fn count(self) -> Expr {
        self.f(F::Count, vec![])
    }
    pub fn first(self) -> Expr {
        self.f(F::First, vec![])
    }
    pub fn last(self) -> Expr {
        self.f(F::Last, vec![])
    }
    pub fn reverse(self) -> Expr {
        self.f(F::Reverse, vec![])
    }
    pub fn any(self, ignore_nulls: bool) -> Expr {
        self.f(F::Any(ignore_nulls), vec![])
    }
    pub fn all(self, ignore_nulls: bool) -> Expr {
        self.f(F::All(ignore_nulls), vec![])
    }
    pub fn cum_sum(self, reverse: bool) -> Expr {
        self.f(F::CumSum(reverse), vec![])
    }
    pub fn cum_prod(self, reverse: bool) -> Expr {
        self.f(F::CumProd(reverse), vec![])
    }
    pub fn cum_min(self, reverse: bool) -> Expr {
        self.f(F::CumMin(reverse), vec![])
    }
    pub fn cum_max(self, reverse: bool) -> Expr {
        self.f(F::CumMax(reverse), vec![])
    }
    pub fn shift(self, n: Expr) -> Expr {
        self.f(F::Shift, vec![n])
    }
    pub fn replace_strict(self, old: Expr, new: Expr, default: Option<Expr>, dtype: Option<DataType>) -> Expr {
        let has_default = default.is_some();
        let mut args = vec![old, new];
        args.extend(default);
        self.f(F::ReplaceStrict { default: has_default, dtype }, args)
    }
    pub fn search_sorted(self, element: Expr, side: SearchSortedSide, descending: bool) -> Expr {
        self.f(F::SearchSorted(side, descending), vec![element])
    }
    pub fn gather(self, indices: Expr, null_on_oob: bool) -> Expr {
        self.f(F::Gather(null_on_oob), vec![indices])
    }
    pub fn get(self, index: Expr, null_on_oob: bool) -> Expr {
        self.f(F::Get(null_on_oob), vec![index])
    }
    pub fn filter(self, predicate: Expr) -> Expr {
        self.f(F::Filter, vec![predicate])
    }
    pub fn slice(self, offset: Expr, length: Expr) -> Expr {
        self.f(F::Slice, vec![offset, length])
    }

    pub fn str(self) -> StrNs {
        StrNs(self)
    }
    pub fn list(self) -> ListNs {
        ListNs(self)
    }
}

/// The functions of text columns (`.str()` in Polars).
pub struct StrNs(Expr);

impl StrNs {
    pub fn to_lowercase(self) -> Expr {
        self.0.f(F::StrToLowercase, vec![])
    }
    pub fn to_uppercase(self) -> Expr {
        self.0.f(F::StrToUppercase, vec![])
    }
    pub fn len_chars(self) -> Expr {
        self.0.f(F::StrLenChars, vec![])
    }
    pub fn head(self, n: Expr) -> Expr {
        self.0.f(F::StrHead, vec![n])
    }
    pub fn tail(self, n: Expr) -> Expr {
        self.0.f(F::StrTail, vec![n])
    }
    pub fn slice(self, offset: Expr, length: Expr) -> Expr {
        self.0.f(F::StrSlice, vec![offset, length])
    }
    pub fn contains(self, pattern: Expr, strict: bool) -> Expr {
        self.0.f(F::StrContains(strict), vec![pattern])
    }
    pub fn replace_all(self, pattern: Expr, value: Expr, literal: bool) -> Expr {
        self.0.f(F::StrReplaceAll(literal), vec![pattern, value])
    }
    pub fn strip_chars(self, characters: Expr) -> Expr {
        self.0.f(F::StrStripChars, vec![characters])
    }
    pub fn split(self, by: Expr) -> Expr {
        self.0.f(F::StrSplit, vec![by])
    }
    pub fn find_literal(self, pattern: Expr) -> Expr {
        self.0.f(F::StrFindLiteral, vec![pattern])
    }
    pub fn extract(self, pattern: Expr, group: usize) -> Expr {
        self.0.f(F::StrExtract(group), vec![pattern])
    }
}

/// The functions of list columns (`.list()` in Polars).
pub struct ListNs(Expr);

impl ListNs {
    pub fn first(self) -> Expr {
        self.0.f(F::ListFirst, vec![])
    }
}

macro_rules! arith {
    ($trait:ident, $method:ident, $op:expr) => {
        impl std::ops::$trait for Expr {
            type Output = Expr;
            fn $method(self, other: Expr) -> Expr {
                self.bin($op, other)
            }
        }
    };
}
arith!(Add, add, Op::Plus);
arith!(Sub, sub, Op::Minus);
arith!(Mul, mul, Op::Multiply);
arith!(Div, div, Op::TrueDivide);
arith!(Rem, rem, Op::Modulus);

/// A table of named columns of equal length: the input of a plan, and its output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub height: usize,
    pub columns: Vec<(String, Values)>,
}

impl Table {
    pub fn new(height: usize) -> Self {
        Table { height, columns: Vec::new() }
    }

    pub fn push(&mut self, name: impl Into<String>, values: impl Into<Values>) {
        self.columns.push((name.into(), values.into()));
    }

    pub fn column(&self, name: &str) -> Option<&Values> {
        self.columns.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn column_mut(&mut self, name: &str) -> Option<&mut Values> {
        self.columns.iter_mut().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Put a column in, replacing one of the same name.
    pub fn set(&mut self, name: &str, values: impl Into<Values>) {
        let values = values.into();
        match self.column_mut(name) {
            Some(v) => *v = values,
            None => self.columns.push((name.to_string(), values)),
        }
    }
}

/// What to compute from an input table: columns added in steps (each step's
/// expressions read only the input and earlier steps), then the columns given back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub steps: Vec<Vec<(String, Expr)>>,
    pub outputs: Vec<(String, Expr)>,
    /// The outputs are added to the table as columns (an output that is one value is
    /// repeated for every row) before they are taken, rather than selected on their own
    /// (where such an output has one row).
    pub broadcast: bool,
}

/// Something that runs plans: Polars in Rust, Polars in Python, ...
pub trait Executor {
    fn run(&self, plan: &Arc<Plan>, input: &Table) -> Result<Table, String>;
}

/// Plans as JSON, for executors outside Rust (the Python shim builds `pl.Expr` from it).
///
/// ```text
/// {"nodes": [node, …], "steps": [[[name, node index], …], …],
///  "outputs": [[name, node index], …], "broadcast": bool}
/// ```
///
/// A node refers to earlier nodes by index; each shared part of the expressions is one
/// node. Nodes:
///
/// * `["col", name]`
/// * `["lit", type, value]` with type `null`, `bool`, `i32`, `i64`, `u32`, `u64`, `f64`, `str`
/// * `["series", name, type, [values]]` with type `f64`, `i64`, `str`, `bool` (null allowed)
/// * `["bin", op, a, b]` with op `+ - * / % == != < <= > >= and or`
/// * `["when", condition, then, otherwise]`
/// * `["cast", a, dtype]` (non-strict) with dtype `Boolean`, `Int64`, `UInt32`, `Float64`, `String`
/// * `["fn", name, [receiver, arguments…], parameters…]` with the names of [`F`] in
///   Polars' Python spelling (`cum_sum`, `str.contains`, `min_horizontal`, …)
///
/// Numbers that are not finite are written as the texts `"nan"`, `"inf"`, `"-inf"`: a plan
/// is standard JSON, so that it can be kept anywhere (see [`wire::plan_from_json`]).
pub mod wire {
    use super::*;
    use std::collections::HashMap;
    use std::fmt::Write;

    fn num(out: &mut String, x: f64) {
        if x.is_nan() {
            out.push_str("\"nan\"");
        } else if x.is_infinite() {
            out.push_str(if x > 0.0 { "\"inf\"" } else { "\"-inf\"" });
        } else if x.fract() == 0.0 && x.abs() < 1e15 {
            let _ = write!(out, "{x:.1}");
        } else {
            let _ = write!(out, "{x:?}");
        }
    }

    fn text(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    fn list<T>(out: &mut String, items: &[T], mut f: impl FnMut(&mut String, &T)) {
        out.push('[');
        for (k, x) in items.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            f(out, x);
        }
        out.push(']');
    }

    pub fn op_name(op: Op) -> &'static str {
        match op {
            Op::Plus => "+",
            Op::Minus => "-",
            Op::Multiply => "*",
            Op::TrueDivide => "/",
            Op::Modulus => "%",
            Op::Eq => "==",
            Op::NotEq => "!=",
            Op::Lt => "<",
            Op::LtEq => "<=",
            Op::Gt => ">",
            Op::GtEq => ">=",
            Op::And => "and",
            Op::Or => "or",
        }
    }

    pub fn dtype_name(d: DataType) -> &'static str {
        match d {
            DataType::Boolean => "Boolean",
            DataType::Int64 => "Int64",
            DataType::UInt32 => "UInt32",
            DataType::Float64 => "Float64",
            DataType::String => "String",
        }
    }

    /// The Python name of a function and its parameters (as JSON values).
    fn function(f: &F) -> (&'static str, String) {
        let b = |x: bool| if x { "true" } else { "false" };
        match f {
            F::Not => ("not_", String::new()),
            F::IsNull => ("is_null", String::new()),
            F::IsNotNull => ("is_not_null", String::new()),
            F::IsNan => ("is_nan", String::new()),
            F::IsFinite => ("is_finite", String::new()),
            F::FillNull => ("fill_null", String::new()),
            F::Abs => ("abs", String::new()),
            F::Floor => ("floor", String::new()),
            F::Ceil => ("ceil", String::new()),
            F::Sqrt => ("sqrt", String::new()),
            F::Exp => ("exp", String::new()),
            F::Log => ("log", String::new()),
            F::Pow => ("pow", String::new()),
            F::Sum => ("sum", String::new()),
            F::Min => ("min", String::new()),
            F::Max => ("max", String::new()),
            F::Product => ("product", String::new()),
            F::Count => ("count", String::new()),
            F::First => ("first", String::new()),
            F::Last => ("last", String::new()),
            F::Reverse => ("reverse", String::new()),
            F::Any(n) => ("any", format!(",{}", b(*n))),
            F::All(n) => ("all", format!(",{}", b(*n))),
            F::CumSum(r) => ("cum_sum", format!(",{}", b(*r))),
            F::CumProd(r) => ("cum_prod", format!(",{}", b(*r))),
            F::CumMin(r) => ("cum_min", format!(",{}", b(*r))),
            F::CumMax(r) => ("cum_max", format!(",{}", b(*r))),
            F::Shift => ("shift", String::new()),
            F::ReplaceStrict { default, dtype } => ("replace_strict", format!(",{},{}", b(*default), dtype.map_or("null".to_string(), |d| format!("\"{}\"", dtype_name(d))))),
            F::SearchSorted(side, descending) => ("search_sorted", format!(",\"{}\",{}", if *side == SearchSortedSide::Left { "left" } else { "right" }, b(*descending))),
            F::Gather(oob) => ("gather", format!(",{}", b(*oob))),
            F::Get(oob) => ("get", format!(",{}", b(*oob))),
            F::Filter => ("filter", String::new()),
            F::Slice => ("slice", String::new()),
            F::StrToLowercase => ("str.to_lowercase", String::new()),
            F::StrToUppercase => ("str.to_uppercase", String::new()),
            F::StrLenChars => ("str.len_chars", String::new()),
            F::StrHead => ("str.head", String::new()),
            F::StrTail => ("str.tail", String::new()),
            F::StrSlice => ("str.slice", String::new()),
            F::StrContains(strict) => ("str.contains", format!(",{}", b(*strict))),
            F::StrReplaceAll(literal) => ("str.replace_all", format!(",{}", b(*literal))),
            F::StrStripChars => ("str.strip_chars", String::new()),
            F::StrSplit => ("str.split", String::new()),
            F::StrFindLiteral => ("str.find_literal", String::new()),
            F::StrExtract(group) => ("str.extract", format!(",{group}")),
            F::ListFirst => ("list.first", String::new()),
            F::MinHorizontal => ("min_horizontal", String::new()),
            F::MaxHorizontal => ("max_horizontal", String::new()),
        }
    }

    fn series(out: &mut String, s: &Series) {
        out.push_str("[\"series\",");
        text(out, &s.name);
        match &s.values {
            Values::F64(v) => {
                out.push_str(",\"f64\",");
                list(out, v, |o, x| num(o, *x));
            }
            Values::OptF64(v) => {
                out.push_str(",\"f64\",");
                list(out, v, |o, x| match x {
                    Some(x) => num(o, *x),
                    None => o.push_str("null"),
                });
            }
            Values::I64(v) => {
                out.push_str(",\"i64\",");
                list(out, v, |o, x| {
                    let _ = write!(o, "{x}");
                });
            }
            Values::Str(v) => {
                out.push_str(",\"str\",");
                list(out, v, |o, x| text(o, x));
            }
            Values::OptStr(v) => {
                out.push_str(",\"str\",");
                list(out, v, |o, x| match x {
                    Some(x) => text(o, x),
                    None => o.push_str("null"),
                });
            }
            Values::OptBool(v) => {
                out.push_str(",\"bool\",");
                list(out, v, |o, x| o.push_str(match x {
                    Some(true) => "true",
                    Some(false) => "false",
                    None => "null",
                }));
            }
        }
        out.push(']');
    }

    struct Writer {
        index: HashMap<usize, usize>,
        keep: Vec<Expr>,
        nodes: Vec<String>,
    }

    impl Writer {
        /// The index of an expression's node, written with all it needs first.
        fn node(&mut self, root: &Expr) -> usize {
            // Iterative post-order: deep expressions do not exhaust the stack.
            let mut stack: Vec<(Expr, bool)> = vec![(root.clone(), false)];
            while let Some((e, ready)) = stack.pop() {
                if self.index.contains_key(&e.id()) {
                    continue;
                }
                let children: Vec<Expr> = match e.node() {
                    Node::Column(_) | Node::Literal(_) => Vec::new(),
                    Node::Binary(a, _, b) => vec![a.clone(), b.clone()],
                    Node::Function(_, args) => args.clone(),
                    Node::Ternary(c, a, b) => vec![c.clone(), a.clone(), b.clone()],
                    Node::Cast(a, _) => vec![a.clone()],
                };
                if !ready && children.iter().any(|c| !self.index.contains_key(&c.id())) {
                    stack.push((e.clone(), true));
                    for c in children.into_iter().rev() {
                        stack.push((c, false));
                    }
                    continue;
                }
                let at = |x: &Expr| self.index[&x.id()];
                let mut out = String::new();
                match e.node() {
                    Node::Column(n) => {
                        out.push_str("[\"col\",");
                        text(&mut out, n);
                        out.push(']');
                    }
                    Node::Literal(l) => match l {
                        Lit::Null => out.push_str("[\"lit\",\"null\",null]"),
                        Lit::Bool(v) => {
                            let _ = write!(out, "[\"lit\",\"bool\",{v}]");
                        }
                        Lit::I32(v) => {
                            let _ = write!(out, "[\"lit\",\"i32\",{v}]");
                        }
                        Lit::I64(v) => {
                            let _ = write!(out, "[\"lit\",\"i64\",{v}]");
                        }
                        Lit::U32(v) => {
                            let _ = write!(out, "[\"lit\",\"u32\",{v}]");
                        }
                        Lit::U64(v) => {
                            let _ = write!(out, "[\"lit\",\"u64\",{v}]");
                        }
                        Lit::F64(v) => {
                            out.push_str("[\"lit\",\"f64\",");
                            num(&mut out, *v);
                            out.push(']');
                        }
                        Lit::Str(s) => {
                            out.push_str("[\"lit\",\"str\",");
                            text(&mut out, s);
                            out.push(']');
                        }
                        Lit::Series(s) => series(&mut out, s),
                    },
                    Node::Binary(a, op, b) => {
                        let _ = write!(out, "[\"bin\",\"{}\",{},{}]", op_name(*op), at(a), at(b));
                    }
                    Node::Ternary(c, a, b) => {
                        let _ = write!(out, "[\"when\",{},{},{}]", at(c), at(a), at(b));
                    }
                    Node::Cast(a, d) => {
                        let _ = write!(out, "[\"cast\",{},\"{}\"]", at(a), dtype_name(*d));
                    }
                    Node::Function(f, args) => {
                        let (name, params) = function(f);
                        let args: Vec<usize> = args.iter().map(at).collect();
                        let _ = write!(out, "[\"fn\",\"{name}\",");
                        list(&mut out, &args, |o, k| {
                            let _ = write!(o, "{k}");
                        });
                        out.push_str(&params);
                        out.push(']');
                    }
                }
                self.index.insert(e.id(), self.nodes.len());
                self.nodes.push(out);
                // Kept so that no address is reused while the plan is written.
                self.keep.push(e);
            }
            self.index[&root.id()]
        }
    }

    /// A plan as JSON (see the module).
    pub fn plan_json(plan: &Plan) -> String {
        let mut w = Writer { index: HashMap::new(), keep: Vec::new(), nodes: Vec::new() };
        let steps: Vec<Vec<(String, usize)>> = plan.steps.iter().map(|s| s.iter().map(|(n, e)| (n.clone(), w.node(e))).collect()).collect();
        let outputs: Vec<(String, usize)> = plan.outputs.iter().map(|(n, e)| (n.clone(), w.node(e))).collect();
        let named = |o: &mut String, x: &(String, usize)| {
            o.push('[');
            text(o, &x.0);
            let _ = write!(o, ",{}]", x.1);
        };
        let mut out = String::from("{\"nodes\":[");
        out.push_str(&w.nodes.join(","));
        out.push_str("],\"steps\":");
        list(&mut out, &steps, |o, s| list(o, s, named));
        out.push_str(",\"outputs\":");
        list(&mut out, &outputs, named);
        let _ = write!(out, ",\"broadcast\":{}}}", plan.broadcast);
        out
    }


    use serde_json::Value as J;

    fn function_of(name: &str, p: &[J]) -> Option<F> {
        let b = |k: usize| p.get(k).and_then(J::as_bool);
        Some(match name {
            "not_" => F::Not,
            "is_null" => F::IsNull,
            "is_not_null" => F::IsNotNull,
            "is_nan" => F::IsNan,
            "is_finite" => F::IsFinite,
            "fill_null" => F::FillNull,
            "abs" => F::Abs,
            "floor" => F::Floor,
            "ceil" => F::Ceil,
            "sqrt" => F::Sqrt,
            "exp" => F::Exp,
            "log" => F::Log,
            "pow" => F::Pow,
            "sum" => F::Sum,
            "min" => F::Min,
            "max" => F::Max,
            "product" => F::Product,
            "count" => F::Count,
            "first" => F::First,
            "last" => F::Last,
            "reverse" => F::Reverse,
            "any" => F::Any(b(0)?),
            "all" => F::All(b(0)?),
            "cum_sum" => F::CumSum(b(0)?),
            "cum_prod" => F::CumProd(b(0)?),
            "cum_min" => F::CumMin(b(0)?),
            "cum_max" => F::CumMax(b(0)?),
            "shift" => F::Shift,
            "replace_strict" => F::ReplaceStrict {
                default: b(0)?,
                dtype: match p.get(1)? {
                    J::Null => None,
                    d => Some(dtype_of(d.as_str()?)?),
                },
            },
            "search_sorted" => F::SearchSorted(if p.first()?.as_str()? == "left" { SearchSortedSide::Left } else { SearchSortedSide::Right }, b(1)?),
            "gather" => F::Gather(b(0)?),
            "get" => F::Get(b(0)?),
            "filter" => F::Filter,
            "slice" => F::Slice,
            "str.to_lowercase" => F::StrToLowercase,
            "str.to_uppercase" => F::StrToUppercase,
            "str.len_chars" => F::StrLenChars,
            "str.head" => F::StrHead,
            "str.tail" => F::StrTail,
            "str.slice" => F::StrSlice,
            "str.contains" => F::StrContains(b(0)?),
            "str.replace_all" => F::StrReplaceAll(b(0)?),
            "str.strip_chars" => F::StrStripChars,
            "str.split" => F::StrSplit,
            "str.find_literal" => F::StrFindLiteral,
            "str.extract" => F::StrExtract(p.first()?.as_u64()? as usize),
            "list.first" => F::ListFirst,
            "min_horizontal" => F::MinHorizontal,
            "max_horizontal" => F::MaxHorizontal,
            _ => return None,
        })
    }

    fn dtype_of(name: &str) -> Option<DataType> {
        [DataType::Boolean, DataType::Int64, DataType::UInt32, DataType::Float64, DataType::String].into_iter().find(|d| dtype_name(*d) == name)
    }

    fn op_of(name: &str) -> Option<Op> {
        use Op::*;
        [Plus, Minus, Multiply, TrueDivide, Modulus, Eq, NotEq, Lt, LtEq, Gt, GtEq, And, Or].into_iter().find(|o| op_name(*o) == name)
    }

    /// A number as [`plan_json`] writes it.
    fn number(v: &J) -> Option<f64> {
        match v.as_str() {
            Some("nan") => Some(f64::NAN),
            Some("inf") => Some(f64::INFINITY),
            Some("-inf") => Some(f64::NEG_INFINITY),
            Some(_) => None,
            None => v.as_f64(),
        }
    }

    fn node_of(n: &J, done: &[Expr]) -> Option<Expr> {
        let a = n.as_array()?;
        let at = |k: usize| done.get(a.get(k)?.as_u64()? as usize).cloned();
        let text = |k: usize| a.get(k)?.as_str();
        Some(Expr::new(match text(0)? {
            "col" => Node::Column(text(1)?.to_string()),
            "lit" => Node::Literal(match text(1)? {
                "null" => Lit::Null,
                "bool" => Lit::Bool(a.get(2)?.as_bool()?),
                "i32" => Lit::I32(a.get(2)?.as_i64()? as i32),
                "i64" => Lit::I64(a.get(2)?.as_i64()?),
                "u32" => Lit::U32(a.get(2)?.as_u64()? as u32),
                "u64" => Lit::U64(a.get(2)?.as_u64()?),
                "f64" => Lit::F64(number(a.get(2)?)?),
                "str" => Lit::Str(text(2)?.to_string()),
                _ => return None,
            }),
            "series" => {
                let items = a.get(3)?.as_array()?;
                let nulls = items.iter().any(J::is_null);
                let values = match text(2)? {
                    "f64" if nulls => Values::OptF64(items.iter().map(|v| if v.is_null() { Some(None) } else { number(v).map(Some) }).collect::<Option<_>>()?),
                    "f64" => Values::F64(items.iter().map(number).collect::<Option<_>>()?),
                    "i64" => Values::I64(items.iter().map(J::as_i64).collect::<Option<_>>()?),
                    "str" if nulls => Values::OptStr(items.iter().map(|v| if v.is_null() { Some(None) } else { v.as_str().map(|s| Some(s.to_string())) }).collect::<Option<_>>()?),
                    "str" => Values::Str(items.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<_>>()?),
                    "bool" => Values::OptBool(items.iter().map(|v| if v.is_null() { Some(None) } else { v.as_bool().map(Some) }).collect::<Option<_>>()?),
                    _ => return None,
                };
                Node::Literal(Lit::Series(Series { name: text(1)?.to_string(), values }))
            }
            "bin" => Node::Binary(at(2)?, op_of(text(1)?)?, at(3)?),
            "when" => Node::Ternary(at(1)?, at(2)?, at(3)?),
            "cast" => Node::Cast(at(1)?, dtype_of(text(2)?)?),
            "fn" => {
                let args = a.get(2)?.as_array()?.iter().map(|k| done.get(k.as_u64()? as usize).cloned()).collect::<Option<Vec<_>>>()?;
                Node::Function(function_of(text(1)?, &a[3..])?, args)
            }
            _ => return None,
        }))
    }

    /// The plan of a JSON value as [`plan_json`] writes it; parts that are shared there are
    /// shared here.
    pub fn plan_from_value(v: &J) -> Result<Plan, String> {
        let mut done: Vec<Expr> = Vec::new();
        for (k, n) in v["nodes"].as_array().ok_or("a plan has nodes")?.iter().enumerate() {
            let e = node_of(n, &done).ok_or_else(|| format!("node {k} of the plan is not understood: {n}"))?;
            done.push(e);
        }
        let named = |x: &J| -> Result<(String, Expr), String> {
            let name = x[0].as_str().ok_or("a named expression is [name, node]")?;
            let e = x[1].as_u64().and_then(|k| done.get(k as usize)).ok_or_else(|| format!("{name}: no such node"))?;
            Ok((name.to_string(), e.clone()))
        };
        let steps = v["steps"].as_array().ok_or("a plan has steps")?.iter().map(|s| s.as_array().ok_or("a step is a list".to_string())?.iter().map(named).collect()).collect::<Result<_, String>>()?;
        let outputs = v["outputs"].as_array().ok_or("a plan has outputs")?.iter().map(named).collect::<Result<_, String>>()?;
        Ok(Plan { steps, outputs, broadcast: v["broadcast"].as_bool().unwrap_or(true) })
    }

    /// The plan of the text [`plan_json`] writes.
    pub fn plan_from_json(text: &str) -> Result<Plan, String> {
        plan_from_value(&serde_json::from_str(text).map_err(|e| format!("a plan is JSON: {e}"))?)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn shared_parts_are_one_node() {
            let x = col("a") + lit(1.5);
            let y = when(x.clone().gt(lit(f64::INFINITY))).then(x.clone()).otherwise(lit(NULL).cast(DataType::Float64));
            let plan = Plan { steps: vec![vec![("_t0".into(), x.clone().cum_sum(false))]], outputs: vec![("_r".into(), y)], broadcast: true };
            assert_eq!(
                plan_json(&plan),
                r#"{"nodes":[["col","a"],["lit","f64",1.5],["bin","+",0,1],["fn","cum_sum",[2],false],["lit","f64","inf"],["bin",">",2,4],["lit","null",null],["cast",6,"Float64"],["when",5,2,7]],"steps":[[["_t0",3]]],"outputs":[["_r",8]],"broadcast":true}"#
            );
        }

        #[test]
        fn a_plan_comes_back_from_its_text() {
            let keys = lit(Series::new("k".into(), vec![Some(1.5), None, Some(f64::NEG_INFINITY)]));
            let x = col("a").cast(DataType::Float64).replace_strict(keys.clone(), lit(Series::new("v".into(), vec!["x", "y\n\"z\"", ""])), Some(lit("none")), Some(DataType::String));
            let y = when(col("b").str().contains(lit("^a.*"), false)).then(x.clone()).otherwise(lit(NULL));
            let z = (col("a") + lit(0.1)).search_sorted(lit(3i64), SearchSortedSide::Right, true).gt(lit(7u32)).and(lit(true));
            let plan = Plan { steps: vec![vec![("_t0".into(), x.clone())], vec![("_t1".into(), y.clone().str().extract(lit("(a)"), 1))]], outputs: vec![("_r".into(), y), ("_g".into(), z)], broadcast: false };
            let text = plan_json(&plan);
            let back = plan_from_json(&text).unwrap();
            assert_eq!(back, plan);
            assert_eq!(plan_json(&back), text);
            assert!(plan_from_json(r#"{"nodes":[["what"]],"steps":[],"outputs":[]}"#).unwrap_err().contains("node 0"));
        }
    }
}
