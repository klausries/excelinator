//! Plans of the formula engine run by the Polars built into this crate: every node
//! becomes the Polars operation of the same name.

use xl_engine::ir::{DataType, Executor, Expr, Lit, Node, Op, Plan, SearchSortedSide, Table, Values, F};
use polars::prelude as pl;
use polars::prelude::{IntoLazy, NamedFrom};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

fn dtype(d: DataType) -> pl::DataType {
    match d {
        DataType::Boolean => pl::DataType::Boolean,
        DataType::Int64 => pl::DataType::Int64,
        DataType::UInt32 => pl::DataType::UInt32,
        DataType::Float64 => pl::DataType::Float64,
        DataType::String => pl::DataType::String,
    }
}

fn series(name: &str, v: &Values) -> pl::Series {
    let name = name.into();
    match v {
        Values::F64(x) => pl::Series::new(name, x),
        Values::OptF64(x) => pl::Series::new(name, x),
        Values::I64(x) => pl::Series::new(name, x),
        Values::Str(x) => pl::Series::new(name, x),
        Values::OptStr(x) => pl::Series::new(name, x),
        Values::OptBool(x) => pl::Series::new(name, x),
    }
}

/// Converts expressions, each shared part once (the result shares it as well).
#[derive(Default)]
pub struct Converter {
    done: HashMap<usize, (Expr, pl::Expr)>,
}

impl Converter {
    pub fn convert(&mut self, e: &Expr) -> pl::Expr {
        if let Some((_, x)) = self.done.get(&e.id()) {
            return x.clone();
        }
        let x = self.build(e);
        // The expression is kept so that its address is not reused while converting.
        self.done.insert(e.id(), (e.clone(), x.clone()));
        x
    }

    fn build(&mut self, e: &Expr) -> pl::Expr {
        match e.node() {
            Node::Column(n) => pl::col(n.as_str()),
            Node::Literal(l) => match l {
                Lit::Null => pl::lit(pl::NULL),
                Lit::Bool(b) => pl::lit(*b),
                Lit::I32(v) => pl::lit(*v),
                Lit::I64(v) => pl::lit(*v),
                Lit::U32(v) => pl::lit(*v),
                Lit::U64(v) => pl::lit(*v),
                Lit::F64(v) => pl::lit(*v),
                Lit::Str(s) => pl::lit(s.as_str()),
                Lit::Series(s) => pl::lit(series(&s.name, &s.values)),
            },
            Node::Binary(a, op, b) => {
                let (a, b) = (self.convert(a), self.convert(b));
                match op {
                    Op::Plus => a + b,
                    Op::Minus => a - b,
                    Op::Multiply => a * b,
                    Op::TrueDivide => a / b,
                    Op::Modulus => a % b,
                    Op::Eq => a.eq(b),
                    Op::NotEq => a.neq(b),
                    Op::Lt => a.lt(b),
                    Op::LtEq => a.lt_eq(b),
                    Op::Gt => a.gt(b),
                    Op::GtEq => a.gt_eq(b),
                    Op::And => a.and(b),
                    Op::Or => a.or(b),
                }
            }
            Node::Ternary(c, a, b) => {
                let (c, a, b) = (self.convert(c), self.convert(a), self.convert(b));
                pl::when(c).then(a).otherwise(b)
            }
            Node::Cast(a, d) => self.convert(a).cast(dtype(*d)),
            Node::Function(f, args) => {
                let mut a: Vec<pl::Expr> = args.iter().map(|x| self.convert(x)).collect();
                if matches!(f, F::MinHorizontal | F::MaxHorizontal) {
                    let all = a;
                    let r = if matches!(f, F::MinHorizontal) { polars::lazy::dsl::min_horizontal(all) } else { polars::lazy::dsl::max_horizontal(all) };
                    return r.expect("expressions to compare");
                }
                let x = a.remove(0);
                let mut arg = a.into_iter();
                let mut next = || arg.next().expect("argument of a function");
                match f {
                    F::Not => x.not(),
                    F::IsNull => x.is_null(),
                    F::IsNotNull => x.is_not_null(),
                    F::IsNan => x.is_nan(),
                    F::IsFinite => x.is_finite(),
                    F::FillNull => x.fill_null(next()),
                    F::Abs => x.abs(),
                    F::Floor => x.floor(),
                    F::Ceil => x.ceil(),
                    F::Sqrt => x.sqrt(),
                    F::Exp => x.exp(),
                    F::Log => x.log(next()),
                    F::Pow => x.pow(next()),
                    F::Sum => x.sum(),
                    F::Min => x.min(),
                    F::Max => x.max(),
                    F::Product => x.product(),
                    F::Count => x.count(),
                    F::First => x.first(),
                    F::Last => x.last(),
                    F::Reverse => x.reverse(),
                    F::Any(n) => x.any(*n),
                    F::All(n) => x.all(*n),
                    F::CumSum(r) => x.cum_sum(*r),
                    F::CumProd(r) => x.cum_prod(*r),
                    F::CumMin(r) => x.cum_min(*r),
                    F::CumMax(r) => x.cum_max(*r),
                    F::Shift => x.shift(next()),
                    F::ReplaceStrict { default, dtype: d } => {
                        let (old, new) = (next(), next());
                        let default = default.then(&mut next);
                        x.replace_strict(old, new, default, d.map(dtype))
                    }
                    F::SearchSorted(side, descending) => {
                        let side = match side {
                            SearchSortedSide::Left => pl::SearchSortedSide::Left,
                            SearchSortedSide::Right => pl::SearchSortedSide::Right,
                        };
                        x.search_sorted(next(), side, *descending)
                    }
                    F::Gather(oob) => x.gather(next(), *oob),
                    F::Get(oob) => x.get(next(), *oob),
                    F::Filter => x.filter(next()),
                    F::Slice => {
                        let offset = next();
                        x.slice(offset, next())
                    }
                    F::StrToLowercase => x.str().to_lowercase(),
                    F::StrToUppercase => x.str().to_uppercase(),
                    F::StrLenChars => x.str().len_chars(),
                    F::StrHead => x.str().head(next()),
                    F::StrTail => x.str().tail(next()),
                    F::StrSlice => {
                        let offset = next();
                        x.str().slice(offset, next())
                    }
                    F::StrContains(strict) => x.str().contains(next(), *strict),
                    F::StrReplaceAll(literal) => {
                        let pattern = next();
                        x.str().replace_all(pattern, next(), *literal)
                    }
                    F::StrStripChars => x.str().strip_chars(next()),
                    F::StrSplit => x.str().split(next()),
                    F::StrFindLiteral => x.str().find_literal(next()),
                    F::StrExtract(group) => x.str().extract(next(), *group),
                    F::ListFirst => x.list().first(),
                    F::MinHorizontal | F::MaxHorizontal => unreachable!("handled above"),
                }
            }
        }
    }
}

/// A table as a Polars frame.
pub fn frame(t: &Table) -> pl::PolarsResult<pl::DataFrame> {
    let columns: Vec<pl::Column> = t.columns.iter().map(|(n, v)| series(n, v).into()).collect();
    pl::DataFrame::new(t.height, columns)
}

/// A Polars column as values: numbers, text and booleans as they are, other types
/// cast to numbers.
fn values(c: &pl::Column) -> pl::PolarsResult<Values> {
    Ok(match c.dtype() {
        pl::DataType::String => Values::OptStr(c.str()?.iter().map(|s| s.map(str::to_string)).collect()),
        pl::DataType::Boolean => Values::OptBool(c.bool()?.iter().collect()),
        pl::DataType::Float64 => Values::OptF64(c.f64()?.iter().collect()),
        _ => Values::OptF64(c.cast(&pl::DataType::Float64)?.f64()?.iter().collect()),
    })
}

/// Errors are given in Polars' words, whole.
fn first_line(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The steps and outputs of a plan as Polars expressions.
struct Prepared {
    plan: Weak<Plan>,
    steps: Vec<Vec<pl::Expr>>,
    outputs: Vec<pl::Expr>,
    names: Vec<String>,
}

/// The executor of the Polars in this crate. It keeps the plan it ran last converted,
/// for a plan run again on changed input.
#[derive(Default)]
pub struct PolarsExecutor {
    last: RefCell<Option<Prepared>>,
}

impl PolarsExecutor {
    fn prepare(plan: &Arc<Plan>) -> Prepared {
        let mut c = Converter::default();
        Prepared {
            plan: Arc::downgrade(plan),
            steps: plan.steps.iter().map(|step| step.iter().map(|(n, e)| c.convert(e).alias(n.as_str())).collect()).collect(),
            outputs: plan.outputs.iter().map(|(n, e)| c.convert(e).alias(n.as_str())).collect(),
            names: plan.outputs.iter().map(|(n, _)| n.clone()).collect(),
        }
    }
}

impl Executor for PolarsExecutor {
    fn run(&self, plan: &Arc<Plan>, input: &Table) -> Result<Table, String> {
        let mut last = self.last.borrow_mut();
        if !last.as_ref().is_some_and(|p| p.plan.upgrade().is_some_and(|q| Arc::ptr_eq(&q, plan))) {
            *last = Some(Self::prepare(plan));
        }
        let p = last.as_ref().expect("prepared plan");
        let mut lazy = frame(input).map_err(first_line)?.lazy();
        for step in &p.steps {
            lazy = lazy.with_columns(step.clone());
        }
        let out = if plan.broadcast {
            lazy.with_columns(p.outputs.clone()).select(p.names.iter().map(|n| pl::col(n.as_str())).collect::<Vec<_>>())
        } else {
            lazy.select(p.outputs.clone())
        };
        let out = out.collect().map_err(first_line)?;
        let mut t = Table::new(out.height());
        for name in &p.names {
            let c = out.column(name).map_err(first_line)?;
            t.push(name.as_str(), values(c).map_err(first_line)?);
        }
        Ok(t)
    }
}
