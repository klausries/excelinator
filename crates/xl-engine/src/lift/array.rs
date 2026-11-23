//! Array formulas over fixed ranges, worked out element by element at translation.
//!
//! An array formula in a single cell (or one whose ranges are all fixed) means the same
//! array in every cell of its run. Unrolled into one expression per element it grows with
//! the size of its ranges; here the array is computed once, natively, with Excel's rules
//! for each element, and the aggregate around it takes the result as a fixed range.

use super::*;

/// One element of an array: what a cell can hold.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum V {
    Num(f64),
    Text(String),
    Bool(bool),
    Empty,
    Err(&'static str),
}

/// An array of `h` rows and `w` columns, row by row. A single value is 1 × 1.
#[derive(Debug, Clone)]
pub(super) struct Arr {
    h: u32,
    w: u32,
    v: Vec<V>,
}

/// The most elements an array formula may compute at translation.
const ELEMENTS: usize = 4_000_000;

impl Arr {
    fn one(v: V) -> Arr {
        Arr { h: 1, w: 1, v: vec![v] }
    }

    /// The element at (i, j) when the array is spread to a larger size: a single row or
    /// column repeats, positions beyond a smaller array are #N/A.
    fn at(&self, i: u32, j: u32) -> &V {
        const NA: V = V::Err("#N/A");
        let i = if self.h == 1 { 0 } else { i };
        let j = if self.w == 1 { 0 } else { j };
        if i >= self.h || j >= self.w {
            return &NA;
        }
        &self.v[(i * self.w + j) as usize]
    }
}

fn error_of(text: &str) -> &'static str {
    match text.to_ascii_uppercase().as_str() {
        "#NULL!" => "#NULL!",
        "#DIV/0!" => "#DIV/0!",
        "#VALUE!" => "#VALUE!",
        "#REF!" => "#REF!",
        "#NAME?" => "#NAME?",
        "#NUM!" => "#NUM!",
        _ => "#N/A",
    }
}

/// The number an element is in arithmetic: empty is 0, a truth value 1 or 0, text that
/// reads as a number that number.
fn as_number(v: &V, date1904: bool) -> Result<f64, &'static str> {
    match v {
        V::Num(x) => Ok(*x),
        V::Bool(b) => Ok(*b as u8 as f64),
        V::Empty => Ok(0.0),
        V::Text(t) => text_as_number(t, date1904).ok_or("#VALUE!"),
        V::Err(e) => Err(e),
    }
}

fn as_text(v: &V) -> Result<String, &'static str> {
    match v {
        V::Num(x) => Ok(number_text(*x)),
        V::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.into()),
        V::Empty => Ok(String::new()),
        V::Text(t) => Ok(t.clone()),
        V::Err(e) => Err(e),
    }
}

/// A number as Excel writes it in text: up to 15 significant digits, no trailing zeros.
fn number_text(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let digits = 15 - (x.abs().log10().floor() as i32 + 1);
    if (-300..=300).contains(&digits) && x.abs() >= 1e-9 && x.abs() < 1e15 {
        let s = format!("{:.*}", digits.max(0) as usize, x);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        return s;
    }
    let s = format!("{:.14E}", x);
    let (m, e) = s.split_once('E').unwrap_or((&s, "0"));
    let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
    let e: i32 = e.parse().unwrap_or(0);
    format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// Excel's comparison: an empty cell is 0, "" or FALSE as the other side is; numbers come
/// before text, text before truth values; text compares without case.
fn compare(a: &V, b: &V) -> Result<std::cmp::Ordering, &'static str> {
    use std::cmp::Ordering;
    let rank = |v: &V| match v {
        V::Num(_) => 0,
        V::Text(_) => 1,
        V::Bool(_) => 2,
        _ => 3,
    };
    let (a, b) = match (a, b) {
        (V::Err(e), _) | (_, V::Err(e)) => return Err(e),
        (V::Empty, V::Empty) => return Ok(Ordering::Equal),
        (V::Empty, other) => (empty_as(other), other.clone()),
        (other, V::Empty) => (other.clone(), empty_as(other)),
        (a, b) => (a.clone(), b.clone()),
    };
    Ok(match (&a, &b) {
        (V::Num(x), V::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (V::Text(x), V::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (V::Bool(x), V::Bool(y)) => x.cmp(y),
        _ => rank(&a).cmp(&rank(&b)),
    })
}

fn empty_as(other: &V) -> V {
    match other {
        V::Text(_) => V::Text(String::new()),
        V::Bool(_) => V::Bool(false),
        _ => V::Num(0.0),
    }
}

/// Whether an element counts as true in IF: numbers other than 0, TRUE; empty is false;
/// text is #VALUE!.
fn truth(v: &V) -> Result<bool, &'static str> {
    match v {
        V::Num(x) => Ok(*x != 0.0),
        V::Bool(b) => Ok(*b),
        V::Empty => Ok(false),
        V::Text(_) => Err("#VALUE!"),
        V::Err(e) => Err(e),
    }
}

impl Context<'_> {
    /// The value a cell holds, as an array element.
    fn element_of(&self, sheet: u32, row: u32, col: u32) -> V {
        let c = &self.wb.cells;
        match self.peek(&(sheet, row - 1, col - 1)) {
            None => V::Empty,
            Some(&i) => match c.kind[i] {
                Kind::Blank if c.formula[i].is_some() => V::Text(String::new()),
                Kind::Blank => V::Empty,
                Kind::Number => V::Num(c.num[i]),
                Kind::Bool => V::Bool(c.num[i] != 0.0),
                Kind::Text => V::Text(c.text[i].as_deref().unwrap_or("").to_string()),
                Kind::Error => V::Err(error_of(c.text[i].as_deref().unwrap_or("#N/A"))),
            },
        }
    }

    fn block_array(&self, b: Block) -> Lifted<Arr> {
        self.not_own_cells(b)?;
        let (h, w) = (b.r2 - b.r1 + 1, b.c2 - b.c1 + 1);
        if h as usize * w as usize > ELEMENTS {
            return Err("large range in an array formula".into());
        }
        let mut v = Vec::with_capacity((h * w) as usize);
        for r in b.r1..=b.r2 {
            for c in b.c1..=b.c2 {
                v.push(self.element_of(b.sheet, r, c));
            }
        }
        Ok(Arr { h, w, v })
    }

    /// Element by element over two arrays spread to their common size.
    fn zip(a: &Arr, b: &Arr, f: impl Fn(&V, &V) -> V) -> Lifted<Arr> {
        let (h, w) = (a.h.max(b.h), a.w.max(b.w));
        if h as usize * w as usize > ELEMENTS {
            return Err("large array formula".into());
        }
        let mut v = Vec::with_capacity((h * w) as usize);
        for i in 0..h {
            for j in 0..w {
                v.push(f(a.at(i, j), b.at(i, j)));
            }
        }
        Ok(Arr { h, w, v })
    }

    fn map(a: Arr, f: impl Fn(&V) -> V) -> Arr {
        Arr { h: a.h, w: a.w, v: a.v.iter().map(f).collect() }
    }

    /// A fixed array expression, computed now. Fails for anything it does not know, so
    /// the caller can fall back to the expression per element.
    pub(super) fn fixed_array(&mut self, e: &Fx) -> Lifted<Arr> {
        let was = self.param.replace(false);
        let r = self.fixed_array_in(e);
        self.param.set(was);
        r
    }

    fn fixed_array_in(&mut self, e: &Fx) -> Lifted<Arr> {
        self.baked.set(self.baked.get() | BAKED_BLOCK);
        let d1904 = self.wb.date1904;
        let num = move |v: &V| as_number(v, d1904);
        Ok(match e {
            Fx::Spaced { expr, .. } | Fx::Paren(expr) => self.fixed_array(expr)?,
            Fx::Number(x) => Arr::one(V::Num(x.parse::<f64>().map_err(|_| "number form")?)),
            Fx::Str(s) => Arr::one(V::Text(unquote(s))),
            Fx::Bool(b) => Arr::one(V::Bool(b.eq_ignore_ascii_case("TRUE"))),
            Fx::Error(t) => Arr::one(V::Err(error_of(t))),
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                let r = parse_cell_ref(text).ok_or("reference form")?;
                if !r.abs_row || (self.in_name > 0 && !r.abs_col) {
                    return Err("array over a moving reference".into());
                }
                let sheet = self.sheet_of(prefix)?;
                self.block_array(Block { sheet, r1: r.row, r2: r.row, c1: r.col, c2: r.col })?
            }
            Fx::Ref { .. } | Fx::Binary { op: BinaryOp::Range, .. } => match self.table(e)? {
                Some(b) => self.block_array(b)?,
                None => Arr::one(V::Err("#REF!")),
            },
            Fx::Name { prefix, text } => {
                let formula = self.name(&qualified(prefix, text))?;
                self.in_name += 1;
                let result = self.fixed_array(&formula);
                self.in_name -= 1;
                result?
            }
            Fx::Unary { op: '-', expr } => Self::map(self.fixed_array(expr)?, |v| match num(v) {
                Ok(x) => V::Num(-x),
                Err(e) => V::Err(e),
            }),
            Fx::Unary { op: '+', expr } => self.fixed_array(expr)?,
            Fx::Postfix { expr, op: '%' } => Self::map(self.fixed_array(expr)?, |v| match num(v) {
                Ok(x) => V::Num(x / 100.0),
                Err(e) => V::Err(e),
            }),
            Fx::Binary { op: BinaryOp::Intersect | BinaryOp::Union, .. } => return Err("array expression form".into()),
            Fx::Binary { op, lhs, rhs, .. } => {
                let (a, b) = (self.fixed_array(lhs)?, self.fixed_array(rhs)?);
                let op = op.clone();
                Self::zip(&a, &b, |x, y| binary(&op, x, y, d1904))?
            }
            Fx::Call { name, args } => self.fixed_call(&name.to_ascii_uppercase(), args)?,
            _ => return Err("array expression form".into()),
        })
    }

    fn fixed_call(&mut self, name: &str, args: &[Fx]) -> Lifted<Arr> {
        let d1904 = self.wb.date1904;
        let arity = |lo: usize, hi: usize| if (lo..=hi).contains(&args.len()) { Ok(()) } else { Err(format!("{name} with {} arguments", args.len())) };
        let number_fn = |ctx: &mut Self, f: &dyn Fn(f64) -> Result<f64, &'static str>| -> Lifted<Arr> {
            let a = ctx.fixed_array(&args[0])?;
            Ok(Self::map(a, |v| match as_number(v, d1904).and_then(f) {
                Ok(x) => V::Num(x),
                Err(e) => V::Err(e),
            }))
        };
        Ok(match name {
            "IF" => {
                arity(2, 3)?;
                let cond = self.fixed_array(&args[0])?;
                let yes = self.fixed_array(&args[1])?;
                let no = match args.get(2) {
                    Some(a) if !matches!(core(a), Fx::Missing) => self.fixed_array(a)?,
                    Some(_) => Arr::one(V::Num(0.0)),
                    None => Arr::one(V::Bool(false)),
                };
                let (h, w) = (cond.h.max(yes.h).max(no.h), cond.w.max(yes.w).max(no.w));
                if h as usize * w as usize > ELEMENTS {
                    return Err("large array formula".into());
                }
                let mut v = Vec::with_capacity((h * w) as usize);
                for i in 0..h {
                    for j in 0..w {
                        // An empty branch gives 0.
                        let pick = |a: &V| if *a == V::Empty { V::Num(0.0) } else { a.clone() };
                        v.push(match truth(cond.at(i, j)) {
                            Ok(true) => pick(yes.at(i, j)),
                            Ok(false) => pick(no.at(i, j)),
                            Err(e) => V::Err(e),
                        });
                    }
                }
                Arr { h, w, v }
            }
            "ROWS" | "COLUMNS" => {
                arity(1, 1)?;
                let a = self.fixed_array(&args[0])?;
                Arr::one(V::Num(if name == "ROWS" { a.h } else { a.w } as f64))
            }
            "OFFSET" => match self.offset(args)? {
                Target::Fixed(b) => self.block_array(b)?,
                Target::Nowhere => Arr::one(V::Err("#REF!")),
                _ => return Err("OFFSET that moves in an array".into()),
            },
            "YEAR" | "MONTH" | "DAY" => {
                arity(1, 1)?;
                let part = match name {
                    "YEAR" => 0,
                    "MONTH" => 1,
                    _ => 2,
                };
                number_fn(self, &|x| {
                    let s = x.floor() + if d1904 { 1462.0 } else { 0.0 };
                    if !(0.0..2_958_466.0).contains(&s) {
                        return Err("#NUM!");
                    }
                    // Serial 0 is 0 January 1900, serial 60 the 29 February 1900 that never was.
                    let ymd = if s == 0.0 {
                        (1900, 1, 0)
                    } else if s == 60.0 {
                        (1900, 2, 29)
                    } else {
                        let days = if s < 60.0 { s as i64 - 25_568 } else { s as i64 - 25_569 };
                        civil_days(days)
                    };
                    Ok([ymd.0, ymd.1, ymd.2][part] as f64)
                })?
            }
            "ABS" => {
                arity(1, 1)?;
                number_fn(self, &|x| Ok(x.abs()))?
            }
            "INT" => {
                arity(1, 1)?;
                number_fn(self, &|x| Ok(x.floor()))?
            }
            "NOT" => {
                arity(1, 1)?;
                let a = self.fixed_array(&args[0])?;
                Self::map(a, |v| match v {
                    V::Text(_) => V::Err("#VALUE!"),
                    other => match truth(other) {
                        Ok(b) => V::Bool(!b),
                        Err(e) => V::Err(e),
                    },
                })
            }
            "ISNUMBER" | "ISTEXT" | "ISBLANK" | "ISERROR" | "ISNA" | "ISERR" | "ISLOGICAL" | "ISNONTEXT" => {
                arity(1, 1)?;
                let a = self.fixed_array(&args[0])?;
                Self::map(a, |v| {
                    V::Bool(match name {
                        "ISNUMBER" => matches!(v, V::Num(_)),
                        "ISTEXT" => matches!(v, V::Text(_)),
                        "ISNONTEXT" => !matches!(v, V::Text(_)),
                        "ISBLANK" => matches!(v, V::Empty),
                        "ISERROR" => matches!(v, V::Err(_)),
                        "ISNA" => matches!(v, V::Err("#N/A")),
                        "ISERR" => matches!(v, V::Err(e) if *e != "#N/A"),
                        _ => matches!(v, V::Bool(_)),
                    })
                })
            }
            "SUM" | "COUNT" | "MAX" | "MIN" | "AVERAGE" => {
                let mut s = Summary { sum: 0.0, count: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY, sum_squares: 0.0, error: false };
                let mut first_error = None;
                for a in args {
                    // A truth value or text written directly as an argument counts; in a range or array it does not.
                    let direct = !matches!(core(a), Fx::Ref { .. } | Fx::Name { .. } | Fx::Binary { op: BinaryOp::Range, .. } | Fx::Call { .. } | Fx::Binary { .. });
                    let arr = self.fixed_array(a)?;
                    for v in &arr.v {
                        let x = match v {
                            V::Num(x) => Some(*x),
                            V::Bool(b) if direct => Some(*b as u8 as f64),
                            V::Text(t) if direct => match text_as_number(t, d1904) {
                                Some(x) => Some(x),
                                None => {
                                    first_error.get_or_insert("#VALUE!");
                                    None
                                }
                            },
                            V::Err(e) => {
                                first_error.get_or_insert(*e);
                                None
                            }
                            _ => None,
                        };
                        if let Some(x) = x {
                            s.sum += x;
                            s.count += 1.0;
                            s.min = s.min.min(x);
                            s.max = s.max.max(x);
                        }
                    }
                }
                Arr::one(match (name, first_error) {
                    ("COUNT", _) => V::Num(s.count),
                    (_, Some(e)) => V::Err(e),
                    ("SUM", _) => V::Num(s.sum),
                    ("MAX", _) => V::Num(if s.count > 0.0 { s.max } else { 0.0 }),
                    ("MIN", _) => V::Num(if s.count > 0.0 { s.min } else { 0.0 }),
                    _ => if s.count > 0.0 { V::Num(s.sum / s.count) } else { V::Err("#DIV/0!") },
                })
            }
            "INDEX" => {
                arity(2, 3)?;
                let a = self.fixed_array(&args[0])?;
                let at = |ctx: &mut Self, k: usize| -> Lifted<Option<f64>> {
                    match args.get(k) {
                        Some(x) if !matches!(core(x), Fx::Missing) => {
                            let v = ctx.fixed_array(x)?;
                            Ok(as_number(&v.v[0], d1904).ok().map(f64::trunc))
                        }
                        _ => Ok(Some(0.0)),
                    }
                };
                let (Some(mut r), Some(mut c)) = (at(self, 1)?, at(self, 2)?) else { return Ok(Arr::one(V::Err("#VALUE!"))) };
                // A single row or column takes one number as the position along it.
                if args.len() == 2 && a.h == 1 {
                    (r, c) = (1.0, r);
                }
                if r < 0.0 || c < 0.0 || r > a.h as f64 || c > a.w as f64 {
                    return Ok(Arr::one(V::Err("#REF!")));
                }
                let (r, c) = (r as u32, c as u32);
                let rows: Vec<u32> = if r == 0 { (0..a.h).collect() } else { vec![r - 1] };
                let cols: Vec<u32> = if c == 0 { (0..a.w).collect() } else { vec![c - 1] };
                let v = rows.iter().flat_map(|&i| cols.iter().map(move |&j| (i, j))).map(|(i, j)| a.at(i, j).clone()).collect();
                Arr { h: rows.len() as u32, w: cols.len() as u32, v }
            }
            "TRUE" | "FALSE" => {
                arity(0, 0)?;
                Arr::one(V::Bool(name == "TRUE"))
            }
            "LINEST" => {
                arity(1, 4)?;
                let y = self.fixed_array(&args[0])?;
                let x = match args.get(1) {
                    Some(a) if !matches!(core(a), Fx::Missing) => Some(self.fixed_array(a)?),
                    _ => None,
                };
                let flag = |ctx: &mut Self, k: usize, default: bool| -> Lifted<bool> {
                    match args.get(k) {
                        Some(a) if !matches!(core(a), Fx::Missing) => {
                            let v = ctx.fixed_array(a)?;
                            truth(&v.v[0]).map_err(|_| "LINEST flag".to_string())
                        }
                        _ => Ok(default),
                    }
                };
                let (constant, stats) = (flag(self, 2, true)?, flag(self, 3, false)?);
                linest(&y, x.as_ref(), constant, stats)?
            }
            _ => return Err(format!("{name} in an array formula")),
        })
    }

    /// The first element of a fixed array, as the formula's cell shows it.
    pub(super) fn array_corner(&mut self, e: &Fx) -> Lifted<T> {
        let a = self.fixed_array(e)?;
        Ok(match &a.v[0] {
            V::Num(x) => T::Num(lit(*x)),
            V::Empty => T::Num(lit(0.0)),
            V::Bool(b) => T::Bool(lit(*b)),
            V::Text(t) => T::Text(lit(t.clone())),
            V::Err(e) => return self.translate(&Fx::Error((*e).to_string())),
        })
    }

    /// The numbers of a fixed array, as an aggregate takes them: text, truth values and
    /// empty elements do not take part, an error is the result.
    pub(super) fn array_summary(&mut self, e: &Fx) -> Lifted<Summary> {
        let a = self.fixed_array(e)?;
        let mut s = Summary { sum: 0.0, count: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY, sum_squares: 0.0, error: false };
        for v in &a.v {
            match v {
                V::Num(x) => {
                    s.sum += x;
                    s.count += 1.0;
                    s.min = s.min.min(*x);
                    s.max = s.max.max(*x);
                    s.sum_squares += x * x;
                }
                V::Err(_) => s.error = true,
                _ => {}
            }
        }
        Ok(s)
    }
}

/// Year, month and day of a day count since 1970-01-01.
fn civil_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn binary(op: &BinaryOp, a: &V, b: &V, d1904: bool) -> V {
    use std::cmp::Ordering::*;
    let arith = |f: &dyn Fn(f64, f64) -> Result<f64, &'static str>| match (as_number(a, d1904), as_number(b, d1904)) {
        (Err(e), _) | (_, Err(e)) => V::Err(e),
        (Ok(x), Ok(y)) => match f(x, y) {
            Ok(r) if r.is_finite() => V::Num(r),
            Ok(_) => V::Err("#NUM!"),
            Err(e) => V::Err(e),
        },
    };
    let cmp = |want: &dyn Fn(std::cmp::Ordering) -> bool| match compare(a, b) {
        Ok(o) => V::Bool(want(o)),
        Err(e) => V::Err(e),
    };
    match op {
        BinaryOp::Add => arith(&|x, y| Ok(x + y)),
        BinaryOp::Sub => arith(&|x, y| Ok(x - y)),
        BinaryOp::Mul => arith(&|x, y| Ok(x * y)),
        BinaryOp::Div => arith(&|x, y| if y == 0.0 { Err("#DIV/0!") } else { Ok(x / y) }),
        BinaryOp::Pow => arith(&|x, y| if x == 0.0 && y == 0.0 { Err("#NUM!") } else { Ok(x.powf(y)) }),
        BinaryOp::Concat => match (as_text(a), as_text(b)) {
            (Err(e), _) | (_, Err(e)) => V::Err(e),
            (Ok(x), Ok(y)) => V::Text(x + &y),
        },
        BinaryOp::Eq => cmp(&|o| o == Equal),
        BinaryOp::Ne => cmp(&|o| o != Equal),
        BinaryOp::Lt => cmp(&|o| o == Less),
        BinaryOp::Gt => cmp(&|o| o == Greater),
        BinaryOp::Le => cmp(&|o| o != Greater),
        BinaryOp::Ge => cmp(&|o| o != Less),
        _ => V::Err("#VALUE!"),
    }
}

/// LINEST by least squares (Householder QR): coefficients from the last x to the first,
/// then the intercept; with `stats` four more rows of statistics, as Excel lays them out.
fn linest(y: &Arr, x: Option<&Arr>, constant: bool, stats: bool) -> Lifted<Arr> {
    if y.w != 1 {
        return Err("LINEST over a row of y values".into());
    }
    let n = y.h as usize;
    let numbers = |a: &Arr| -> Option<Vec<f64>> { a.v.iter().map(|v| if let V::Num(x) = v { Some(*x) } else { None }).collect() };
    let Some(ys) = numbers(y) else { return Ok(Arr::one(V::Err("#VALUE!"))) };
    let (k, xs) = match x {
        None => (1, (1..=n).map(|i| i as f64).collect::<Vec<f64>>()),
        Some(a) if a.h as usize == n => match numbers(a) {
            Some(v) => (a.w as usize, v),
            None => return Ok(Arr::one(V::Err("#VALUE!"))),
        },
        Some(_) => return Err("LINEST with x values of another shape".into()),
    };
    let p = k + constant as usize;
    if n < p.max(1) {
        return Ok(Arr::one(V::Err("#NUM!")));
    }
    // Design matrix, column-major: the x columns, then a column of ones.
    let mut a: Vec<Vec<f64>> = (0..k).map(|j| (0..n).map(|i| xs[i * k + j]).collect()).collect();
    if constant {
        a.push(vec![1.0; n]);
    }
    // With an intercept the columns are centred first (as Excel does), which keeps the
    // problem well conditioned.
    let (y_mean, x_means): (f64, Vec<f64>) = if constant {
        (ys.iter().sum::<f64>() / n as f64, (0..k).map(|j| a[j].iter().sum::<f64>() / n as f64).collect())
    } else {
        (0.0, vec![0.0; k])
    };
    let mut q: Vec<Vec<f64>> = (0..k).map(|j| a[j].iter().map(|v| v - x_means[j]).collect()).collect();
    let mut b: Vec<f64> = ys.iter().map(|v| v - y_mean).collect();
    let mut r = vec![vec![0.0; k]; k];
    for j in 0..k {
        // The part of the column below the rows already settled.
        let norm = q[j].iter().skip(j).map(|v| v * v).sum::<f64>().sqrt();
        let scale = a[j].iter().map(|v| v.abs()).fold(0.0, f64::max).max(1e-300);
        if norm <= 1e-12 * scale * (n as f64).sqrt() {
            return Err("LINEST with x columns that depend on each other".into());
        }
        let alpha = if q[j][j] > 0.0 { -norm } else { norm };
        let mut v = q[j].clone();
        for vi in v.iter_mut().take(j) {
            *vi = 0.0;
        }
        v[j] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        if vv > 0.0 {
            for col in q.iter_mut().skip(j) {
                let d: f64 = v.iter().zip(col.iter()).map(|(a, b)| a * b).sum::<f64>() * 2.0 / vv;
                for (c, vi) in col.iter_mut().zip(v.iter()) {
                    *c -= d * vi;
                }
            }
            let d: f64 = v.iter().zip(b.iter()).map(|(a, b)| a * b).sum::<f64>() * 2.0 / vv;
            for (c, vi) in b.iter_mut().zip(v.iter()) {
                *c -= d * vi;
            }
        }
        for i in 0..=j {
            r[i][j] = q[j][i];
        }
    }
    // Back substitution.
    let mut m = vec![0.0; k];
    for i in (0..k).rev() {
        let s: f64 = (i + 1..k).map(|j| r[i][j] * m[j]).sum();
        m[i] = (b[i] - s) / r[i][i];
    }
    let intercept = if constant { y_mean - (0..k).map(|j| m[j] * x_means[j]).sum::<f64>() } else { 0.0 };
    let w = k + 1;
    let mut out = vec![V::Err("#N/A"); if stats { 5 * w } else { w }];
    for j in 0..k {
        out[k - 1 - j] = V::Num(m[j]);
    }
    out[k] = V::Num(intercept);
    if stats {
        let fitted: Vec<f64> = (0..n).map(|i| intercept + (0..k).map(|j| m[j] * xs[i * k + j]).sum::<f64>()).collect();
        let sse: f64 = (0..n).map(|i| (ys[i] - fitted[i]).powi(2)).sum();
        let ssreg: f64 = if constant { fitted.iter().map(|f| (f - y_mean).powi(2)).sum() } else { fitted.iter().map(|f| f * f).sum() };
        let df = n as f64 - p as f64;
        let sey = if df > 0.0 { (sse / df).sqrt() } else { f64::NAN };
        // (RᵀR)⁻¹ diagonal: from R⁻¹.
        let mut rinv = vec![vec![0.0; k]; k];
        for c in 0..k {
            for i in (0..=c).rev() {
                let s: f64 = (i + 1..=c).map(|j| r[i][j] * rinv[j][c]).sum();
                rinv[i][c] = ((i == c) as u8 as f64 - s) / r[i][i];
            }
        }
        let var: Vec<f64> = (0..k).map(|i| (0..k).map(|c| rinv[i][c] * rinv[i][c]).sum()).collect();
        let num = |x: f64| if x.is_finite() { V::Num(x) } else { V::Err("#NUM!") };
        for j in 0..k {
            out[w + k - 1 - j] = num((var[j]).sqrt() * sey);
        }
        out[w + k] = if constant {
            // Variance of the intercept: 1/n plus x̄ᵀ (XcᵀXc)⁻¹ x̄.
            let mut quad = 0.0;
            for i in 0..k {
                for j in 0..k {
                    let cov: f64 = (0..k).map(|c| rinv[i][c] * rinv[j][c]).sum();
                    quad += x_means[i] * cov * x_means[j];
                }
            }
            num(sey * (1.0 / n as f64 + quad).sqrt())
        } else {
            V::Err("#N/A")
        };
        let r2 = if ssreg + sse == 0.0 { f64::NAN } else { ssreg / (ssreg + sse) };
        out[2 * w] = num(r2);
        out[2 * w + 1] = num(sey);
        let v1 = k as f64;
        out[3 * w] = if sse == 0.0 { V::Err("#NUM!") } else { num((ssreg / v1) / (sse / df)) };
        out[3 * w + 1] = num(df);
        out[4 * w] = num(ssreg);
        out[4 * w + 1] = num(sse);
    }
    Ok(Arr { h: if stats { 5 } else { 1 }, w: w as u32, v: out })
}
