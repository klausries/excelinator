//! Plans of the formula engine as Python source for Polars.
//!
//! Every node of a plan becomes the Polars call of the same name (see `ir`), the same
//! mapping as `xlcells_engine/_shim.py` makes when it builds expressions in memory. A
//! part used more than once, or a part so deeply nested that Python's parser would
//! refuse it, is assigned to a variable first.

use crate::ir::{DataType, Expr, Lit, Node, Op, Plan, SearchSortedSide, Values, F};
use std::collections::HashMap;

/// A plan as statements. `df` is a Polars DataFrame with the plan's input columns: the
/// steps add columns to it, and the outputs end up in the DataFrame `out`, which holds
/// the plan's outputs and nothing else. `prefix` begins the names of the helper variables.
pub fn plan_python(plan: &Plan, df: &str, out: &str, prefix: &str) -> Result<Vec<String>, String> {
    if plan.outputs.is_empty() {
        return Err("a plan without outputs".into());
    }
    let mut w = Writer::new(prefix);
    w.count(plan);
    let steps: Vec<Vec<(String, String)>> = plan.steps.iter().map(|s| s.iter().map(|(n, e)| Ok((n.clone(), w.expr(e)?))).collect::<Result<_, String>>()).collect::<Result<_, String>>()?;
    let outputs: Vec<(String, String)> = plan.outputs.iter().map(|(n, e)| Ok((n.clone(), w.expr(e)?))).collect::<Result<_, String>>()?;
    let mut lines = w.statements;
    for step in &steps {
        lines.push(format!("{df} = {df}.with_columns("));
        for (name, code) in step {
            lines.push(format!("    {}.alias({}),", atom(code), quote(name)));
        }
        lines.push(")".into());
    }
    lines.push(format!("{out} = {df}.with_columns("));
    for (name, code) in &outputs {
        lines.push(format!("    {}.alias({}),", atom(code), quote(name)));
    }
    let names: Vec<String> = outputs.iter().map(|(n, _)| quote(n)).collect();
    lines.push(format!(").select({})", names.join(", ")));
    Ok(lines)
}

/// An expression text that a method can be called on.
fn atom(code: &str) -> String {
    code.to_string()
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn float(x: f64) -> String {
    if x.is_nan() {
        "float(\"nan\")".into()
    } else if x.is_infinite() {
        if x > 0.0 { "float(\"inf\")".into() } else { "float(\"-inf\")".into() }
    } else if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.1}")
    } else {
        format!("{x:?}")
    }
}

fn dtype(d: DataType) -> &'static str {
    match d {
        DataType::Boolean => "pl.Boolean",
        DataType::Int64 => "pl.Int64",
        DataType::UInt32 => "pl.UInt32",
        DataType::Float64 => "pl.Float64",
        DataType::String => "pl.String",
    }
}

fn series(name: &str, v: &Values) -> String {
    let list = |items: Vec<String>| format!("[{}]", items.join(", "));
    let opt = |x: Option<String>| x.unwrap_or_else(|| "None".into());
    match v {
        Values::F64(x) => format!("pl.Series({}, {}, dtype=pl.Float64)", quote(name), list(x.iter().map(|v| float(*v)).collect())),
        Values::OptF64(x) => format!("pl.Series({}, {}, dtype=pl.Float64)", quote(name), list(x.iter().map(|v| opt(v.map(float))).collect())),
        Values::I64(x) => format!("pl.Series({}, {}, dtype=pl.Int64)", quote(name), list(x.iter().map(|v| v.to_string()).collect())),
        Values::Str(x) => format!("pl.Series({}, {}, dtype=pl.String)", quote(name), list(x.iter().map(|v| quote(v)).collect())),
        Values::OptStr(x) => format!("pl.Series({}, {}, dtype=pl.String)", quote(name), list(x.iter().map(|v| opt(v.as_deref().map(quote))).collect())),
        Values::OptBool(x) => format!("pl.Series({}, {}, dtype=pl.Boolean)", quote(name), list(x.iter().map(|v| opt(v.map(|b| if b { "True".into() } else { "False".into() }))).collect())),
    }
}

/// How deep an inline expression may nest before it gets a variable (Python's parser
/// stops at 200 levels of brackets; every level here costs one or two).
const MAX_HEIGHT: usize = 24;

struct Writer<'a> {
    prefix: &'a str,
    /// How often each node is used.
    uses: HashMap<usize, usize>,
    /// The code of each node written, and its height inline (0 for a variable).
    done: HashMap<usize, (String, usize)>,
    statements: Vec<String>,
    keep: Vec<Expr>,
    variables: usize,
}

impl<'a> Writer<'a> {
    fn new(prefix: &'a str) -> Self {
        Writer { prefix, uses: HashMap::new(), done: HashMap::new(), statements: Vec::new(), keep: Vec::new(), variables: 0 }
    }

    /// Count the uses of every node, over the whole plan.
    fn count(&mut self, plan: &Plan) {
        let mut seen: HashMap<usize, ()> = HashMap::new();
        let roots = plan.steps.iter().flatten().map(|(_, e)| e).chain(plan.outputs.iter().map(|(_, e)| e));
        let mut stack: Vec<&Expr> = roots.collect();
        // Every root is used once by its statement; a node reached again is used again.
        for r in &stack {
            *self.uses.entry(r.id()).or_default() += 1;
        }
        while let Some(e) = stack.pop() {
            if seen.insert(e.id(), ()).is_some() {
                continue;
            }
            let children: Vec<&Expr> = match e.node() {
                Node::Column(_) | Node::Literal(_) => Vec::new(),
                Node::Binary(a, _, b) => vec![a, b],
                Node::Function(_, args) => args.iter().collect(),
                Node::Ternary(c, a, b) => vec![c, a, b],
                Node::Cast(a, _) => vec![a],
            };
            for c in children {
                *self.uses.entry(c.id()).or_default() += 1;
                stack.push(c);
            }
        }
    }

    /// The code of an expression, its nodes written (bottom-up, without recursion).
    fn expr(&mut self, root: &Expr) -> Result<String, String> {
        let mut stack: Vec<(Expr, bool)> = vec![(root.clone(), false)];
        while let Some((e, ready)) = stack.pop() {
            if self.done.contains_key(&e.id()) {
                continue;
            }
            let children: Vec<Expr> = match e.node() {
                Node::Column(_) | Node::Literal(_) => Vec::new(),
                Node::Binary(a, _, b) => vec![a.clone(), b.clone()],
                Node::Function(_, args) => args.clone(),
                Node::Ternary(c, a, b) => vec![c.clone(), a.clone(), b.clone()],
                Node::Cast(a, _) => vec![a.clone()],
            };
            if !ready && children.iter().any(|c| !self.done.contains_key(&c.id())) {
                stack.push((e.clone(), true));
                for c in children.into_iter().rev() {
                    stack.push((c, false));
                }
                continue;
            }
            let (code, height) = self.write(&e)?;
            let trivial = matches!(e.node(), Node::Column(_)) || matches!(e.node(), Node::Literal(l) if !matches!(l, Lit::Series(_)));
            let shared = self.uses.get(&e.id()).copied().unwrap_or(1) > 1;
            let big = matches!(e.node(), Node::Literal(Lit::Series(s)) if s.values.len() > 8);
            let entry = if !trivial && (shared || height >= MAX_HEIGHT || big) {
                let name = format!("{}{}", self.prefix, self.variables);
                self.variables += 1;
                self.statements.push(format!("{name} = {code}"));
                (name, 0)
            } else {
                (code, height)
            };
            self.done.insert(e.id(), entry);
            self.keep.push(e);
        }
        Ok(self.done[&root.id()].0.clone())
    }

    fn write(&self, e: &Expr) -> Result<(String, usize), String> {
        let at = |x: &Expr| &self.done[&x.id()];
        Ok(match e.node() {
            Node::Column(n) => (format!("pl.col({})", quote(n)), 0),
            Node::Literal(l) => (
                match l {
                    Lit::Null => "pl.lit(None)".into(),
                    Lit::Bool(b) => format!("pl.lit({})", if *b { "True" } else { "False" }),
                    Lit::I32(v) => format!("pl.lit({v}, dtype=pl.Int32)"),
                    Lit::I64(v) => format!("pl.lit({v}, dtype=pl.Int64)"),
                    Lit::U32(v) => format!("pl.lit({v}, dtype=pl.UInt32)"),
                    Lit::U64(v) => format!("pl.lit({v}, dtype=pl.UInt64)"),
                    Lit::F64(v) => format!("pl.lit({})", float(*v)),
                    Lit::Str(s) => format!("pl.lit({})", quote(s)),
                    Lit::Series(s) => format!("pl.lit({})", series(&s.name, &s.values)),
                },
                0,
            ),
            Node::Binary(a, op, b) => {
                let (a, b) = (at(a), at(b));
                let sym = match op {
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
                    Op::And => "&",
                    Op::Or => "|",
                };
                (format!("({} {sym} {})", a.0, b.0), 1 + a.1.max(b.1))
            }
            Node::Ternary(c, a, b) => {
                let (c, a, b) = (at(c), at(a), at(b));
                (format!("pl.when({}).then({}).otherwise({})", c.0, a.0, b.0), 1 + c.1.max(a.1).max(b.1))
            }
            Node::Cast(a, d) => {
                let a = at(a);
                (format!("{}.cast({}, strict=False)", a.0, dtype(*d)), 1 + a.1)
            }
            Node::Function(f, args) => {
                let parts: Vec<&(String, usize)> = args.iter().map(at).collect();
                let height = 1 + parts.iter().map(|p| p.1).max().unwrap_or(0);
                let code = function(f, &parts.iter().map(|p| p.0.as_str()).collect::<Vec<_>>());
                (code, height)
            }
        })
    }
}

/// `receiver.method(arguments…)` for a function of the plan; `a[0]` is the receiver.
fn function(f: &F, a: &[&str]) -> String {
    let b = |x: bool| if x { "True" } else { "False" };
    let call = |name: &str, extra: &str| {
        let mut args: Vec<String> = a[1..].iter().map(|s| s.to_string()).collect();
        if !extra.is_empty() {
            args.push(extra.to_string());
        }
        format!("{}.{name}({})", a[0], args.join(", "))
    };
    match f {
        F::Not => call("not_", ""),
        F::IsNull => call("is_null", ""),
        F::IsNotNull => call("is_not_null", ""),
        F::IsNan => call("is_nan", ""),
        F::IsFinite => call("is_finite", ""),
        F::FillNull => call("fill_null", ""),
        F::Abs => call("abs", ""),
        F::Floor => call("floor", ""),
        F::Ceil => call("ceil", ""),
        F::Sqrt => call("sqrt", ""),
        F::Exp => call("exp", ""),
        F::Log => call("log", ""),
        F::Pow => call("pow", ""),
        F::Sum => call("sum", ""),
        F::Min => call("min", ""),
        F::Max => call("max", ""),
        F::Product => call("product", ""),
        F::Count => call("count", ""),
        F::First => call("first", ""),
        F::Last => call("last", ""),
        F::Reverse => call("reverse", ""),
        F::Any(n) => call("any", &format!("ignore_nulls={}", b(*n))),
        F::All(n) => call("all", &format!("ignore_nulls={}", b(*n))),
        F::CumSum(r) => call("cum_sum", &format!("reverse={}", b(*r))),
        F::CumProd(r) => call("cum_prod", &format!("reverse={}", b(*r))),
        F::CumMin(r) => call("cum_min", &format!("reverse={}", b(*r))),
        F::CumMax(r) => call("cum_max", &format!("reverse={}", b(*r))),
        F::Shift => call("shift", ""),
        F::ReplaceStrict { default, dtype: d } => {
            // The arguments are old, new and, if there is one, the default.
            let mut extra = Vec::new();
            if *default {
                extra.push(format!("default={}", a[3]));
            }
            if let Some(d) = d {
                extra.push(format!("return_dtype={}", dtype(*d)));
            }
            format!("{}.replace_strict({}, {}{}{})", a[0], a[1], a[2], if extra.is_empty() { "" } else { ", " }, extra.join(", "))
        }
        F::SearchSorted(side, descending) => call("search_sorted", &format!("side=\"{}\", descending={}", if *side == SearchSortedSide::Left { "left" } else { "right" }, b(*descending))),
        F::Gather(oob) => call("gather", &format!("null_on_oob={}", b(*oob))),
        F::Get(oob) => call("get", &format!("null_on_oob={}", b(*oob))),
        F::Filter => call("filter", ""),
        F::Slice => call("slice", ""),
        F::StrToLowercase => format!("{}.str.to_lowercase()", a[0]),
        F::StrToUppercase => format!("{}.str.to_uppercase()", a[0]),
        F::StrLenChars => format!("{}.str.len_chars()", a[0]),
        F::StrHead => format!("{}.str.head({})", a[0], a[1]),
        F::StrTail => format!("{}.str.tail({})", a[0], a[1]),
        F::StrSlice => format!("{}.str.slice({}, {})", a[0], a[1], a[2]),
        F::StrContains(strict) => format!("{}.str.contains({}, literal=False, strict={})", a[0], a[1], b(*strict)),
        F::StrReplaceAll(literal) => format!("{}.str.replace_all({}, {}, literal={})", a[0], a[1], a[2], b(*literal)),
        F::StrStripChars => format!("{}.str.strip_chars({})", a[0], a[1]),
        F::StrSplit => format!("{}.str.split({}, literal=True)", a[0], a[1]),
        F::StrFindLiteral => format!("{}.str.find({}, literal=True)", a[0], a[1]),
        F::StrExtract(group) => format!("{}.str.extract({}, group_index={group})", a[0], a[1]),
        F::ListFirst => format!("{}.list.first()", a[0]),
        F::MinHorizontal => format!("pl.min_horizontal({})", a.join(", ")),
        F::MaxHorizontal => format!("pl.max_horizontal({})", a.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::*;

    #[test]
    fn shared_parts_are_variables() {
        let x = col("a") + lit(1.5);
        let y = when(x.clone().gt(lit(0.0))).then(x.clone()).otherwise(lit(NULL).cast(DataType::Float64));
        let plan = Plan { steps: vec![vec![("_t0".into(), x.clone().cum_sum(false))]], outputs: vec![("_r".into(), y)], broadcast: true };
        let lines = plan_python(&plan, "df", "out", "_x").unwrap();
        assert_eq!(
            lines.join("\n"),
            "_x0 = (pl.col(\"a\") + pl.lit(1.5))\ndf = df.with_columns(\n    _x0.cum_sum(reverse=False).alias(\"_t0\"),\n)\nout = df.with_columns(\n    pl.when((_x0 > pl.lit(0.0))).then(_x0).otherwise(pl.lit(None).cast(pl.Float64, strict=False)).alias(\"_r\"),\n).select(\"_r\")"
        );
    }

    #[test]
    fn deep_chains_get_variables() {
        let mut e = col("a");
        for _ in 0..200 {
            e = when(e.clone().gt(lit(1.0))).then(e + lit(1.0)).otherwise(lit(0.0));
        }
        let plan = Plan { steps: vec![], outputs: vec![("_r".into(), e)], broadcast: true };
        let lines = plan_python(&plan, "df", "out", "_x").unwrap();
        let deepest = lines.iter().map(|l| l.matches('(').count()).max().unwrap();
        assert!(deepest < 150, "{deepest}");
    }
}
