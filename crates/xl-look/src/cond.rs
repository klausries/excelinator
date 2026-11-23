//! Conditional formatting: which rules hold for a cell, and what colour scales, data
//! bars and icon sets give it.
//!
//! Rules with formulas (`expression`, `cellIs`, and the text, blank, error and date
//! rules, for which Excel writes the formula into the file) are evaluated by the engine
//! of `xl-polars`: each formula, filled to the cell as Excel fills it and with its
//! references qualified by the rule's sheet, becomes a cell of a scratch sheet, and the
//! workbook is calculated over the values the file holds (formula cells keep their
//! stored values; only those without one are worked out). A result the engine cannot vouch for leaves the rule
//! undecided for that cell; nothing is guessed. Rules over the values of their ranges
//! (top/bottom, above average, duplicates, colour scales, data bars, icon sets) are
//! computed here from the cell values.

use crate::color::Spec;
use crate::parts::{self, Partial};
use crate::table::parse_range;
use crate::tree::{self, Node};
use std::collections::HashMap;
use xl_cells::{Kind, Value, Workbook};
use xl_formula::Expr;

/// One rule of a sheet.
#[derive(Debug, Clone)]
pub struct Rule {
    pub sheet: u32,
    pub kind: String,
    pub sqref: String,
    pub areas: Vec<(u32, u32, u32, u32)>,
    pub priority: i64,
    pub stop: bool,
    pub attrs: Vec<(String, String)>,
    pub formulas: Vec<String>,
    pub content: Node,
    pub look: Partial,
}

impl Rule {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    pub fn covers(&self, row: u32, col: u32) -> bool {
        self.areas.iter().any(|&(r0, c0, r1, c1)| (r0..=r1).contains(&row) && (c0..=c1).contains(&col))
    }

    /// The cell the rule's formulas are written for: the top left corner of the range's
    /// bounding box (the first row and the first column of any area, which need not be
    /// a cell of the range), wherever the areas stand in `sqref`. Checked against Excel
    /// with `ROW(A1)=1` rules over `C5:C10 A1:A3`, `A5:A10 C1:C3` and `C1:C3 A5:A10`.
    pub fn anchor(&self) -> Option<(u32, u32)> {
        let r0 = self.areas.iter().map(|a| a.0).min()?;
        let c0 = self.areas.iter().map(|a| a.1).min()?;
        Some((r0, c0))
    }

    /// Decided by a formula (Excel stores one for every rule type but those over a
    /// range's values).
    pub fn by_formula(&self) -> bool {
        !matches!(self.kind.as_str(), "top10" | "aboveAverage" | "duplicateValues" | "uniqueValues" | "colorScale" | "dataBar" | "iconSet")
    }
}

/// The rules of every sheet, highest priority (lowest number) first.
pub fn rules(wb: &Workbook) -> Vec<Vec<Rule>> {
    let mut out = Vec::new();
    for (s, sheet) in wb.sheets.iter().enumerate() {
        let mut list = Vec::new();
        let mut order = 0i64;
        for cf in &sheet.cond_formats {
            let areas: Vec<_> = cf.sqref.split_whitespace().filter_map(parse_range).collect();
            for r in &cf.rules {
                order += 1;
                let get = |k: &str| r.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
                let content = tree::parse(&r.inner).unwrap_or_default();
                list.push(Rule {
                    sheet: s as u32,
                    kind: get("type").unwrap_or("").to_string(),
                    sqref: cf.sqref.clone(),
                    areas: areas.clone(),
                    priority: get("priority").and_then(|v| v.parse().ok()).unwrap_or(i64::MAX / 2 + order),
                    stop: matches!(get("stopIfTrue"), Some("1" | "true")),
                    attrs: r.attrs.clone(),
                    formulas: content.all("formula").map(|f| f.text.trim().trim_start_matches('=').to_string()).collect(),
                    look: r.dxf.as_deref().map(parts::of_dxf).unwrap_or_default(),
                    content,
                });
            }
        }
        list.sort_by_key(|r| r.priority);
        out.push(list);
    }
    out
}

/// Whether a rule holds for a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Yes,
    No,
    /// Not decided, and why.
    Unknown(String),
}

fn quote_sheet(name: &str) -> String {
    format!("'{}'!", name.replace('\'', "''"))
}

/// What a formula of a rule needs to know of where and when it is evaluated: the
/// cell (`ROW()`, `COLUMN()` without arguments) and the moment (`TODAY()`, `NOW()`).
struct Place<'a> {
    prefix: &'a str,
    local: &'a dyn Fn(&str) -> bool,
    cell: String,
    now: f64,
}

/// The expression with every unqualified reference qualified by the sheet, the
/// unqualified names that are local to the sheet too, `ROW()` and `COLUMN()` of the
/// cell, and `TODAY()` and `NOW()` as numbers.
fn qualified(e: &Expr, p: &Place) -> Expr {
    let (prefix, local) = (p.prefix, p.local);
    let q = |x: &Expr| Box::new(qualified(x, p));
    let is_empty = |args: &[Expr]| args.is_empty() || (args.len() == 1 && matches!(args[0].unspaced(), Expr::Missing));
    match e {
        Expr::Call { name, args } if is_empty(args) && matches!(name.to_ascii_uppercase().as_str(), "ROW" | "COLUMN") => {
            Expr::Call { name: name.clone(), args: vec![Expr::Ref { prefix: Some(prefix.to_string()), kind: xl_formula::RefKind::Cell, text: p.cell.clone() }] }
        }
        Expr::Call { name, args } if is_empty(args) && name.eq_ignore_ascii_case("TODAY") => Expr::Number(format!("{}", p.now.floor())),
        Expr::Call { name, args } if is_empty(args) && name.eq_ignore_ascii_case("NOW") => Expr::Number(format!("{}", p.now)),
        Expr::Ref { prefix: None, kind, text } => Expr::Ref { prefix: Some(prefix.to_string()), kind: *kind, text: text.clone() },
        Expr::Name { prefix: None, text } if local(text) => Expr::Name { prefix: Some(prefix.to_string()), text: text.clone() },
        Expr::Call { name, args } => Expr::Call { name: name.clone(), args: args.iter().map(|a| qualified(a, p)).collect() },
        Expr::Apply { callee, args } => Expr::Apply { callee: q(callee), args: args.iter().map(|a| qualified(a, p)).collect() },
        Expr::Unary { op, expr } => Expr::Unary { op: *op, expr: q(expr) },
        Expr::Postfix { expr, op } => Expr::Postfix { expr: q(expr), op: *op },
        Expr::Binary { op, op_text, lhs, rhs } => Expr::Binary { op: *op, op_text: op_text.clone(), lhs: q(lhs), rhs: q(rhs) },
        Expr::Paren(x) => Expr::Paren(q(x)),
        Expr::Spaced { before, expr, after } => Expr::Spaced { before: before.clone(), expr: q(expr), after: after.clone() },
        Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|x| qualified(x, p)).collect()).collect()),
        other => other.clone(),
    }
}

/// The formula of a rule as it reads for the cell (row, col), qualified, without `=`;
/// `now` is the moment as a serial date (for `TODAY()` and `NOW()`).
pub fn formula_for(wb: &Workbook, rule: &Rule, row: u32, col: u32, now: f64) -> Result<String, String> {
    let sheet = &wb.sheets[rule.sheet as usize];
    let prefix = quote_sheet(&sheet.name);
    let (r0, c0) = rule.anchor().ok_or("no range")?;
    let local = |n: &str| wb.defined_names.iter().any(|d| d.local_sheet == Some(rule.sheet) && d.name.eq_ignore_ascii_case(n));
    let place = Place { prefix: &prefix, local: &local, cell: format!("${}${}", xl_formula::col_name(col + 1), row + 1), now };
    let fill = |f: &str| -> Result<String, String> {
        let e = xl_formula::parse(f).map_err(|e| format!("formula not understood: {e:?}"))?;
        let e = e.shifted(row as i64 - r0 as i64, col as i64 - c0 as i64);
        Ok(qualified(&e, &place).to_string())
    };
    let cell = format!("{prefix}${}${}", xl_formula::col_name(col + 1), row + 1);
    match rule.kind.as_str() {
        "cellIs" => {
            let a = fill(rule.formulas.first().ok_or("no formula")?)?;
            let b = rule.formulas.get(1).map(|f| fill(f)).transpose()?;
            let op = rule.get("operator").unwrap_or("equal");
            Ok(match (op, b) {
                ("between", Some(b)) => format!("OR(AND({cell}>=({a}),{cell}<=({b})),AND({cell}>=({b}),{cell}<=({a})))"),
                ("notBetween", Some(b)) => format!("NOT(OR(AND({cell}>=({a}),{cell}<=({b})),AND({cell}>=({b}),{cell}<=({a}))))"),
                ("equal", _) => format!("{cell}=({a})"),
                ("notEqual", _) => format!("{cell}<>({a})"),
                ("greaterThan", _) => format!("{cell}>({a})"),
                ("lessThan", _) => format!("{cell}<({a})"),
                ("greaterThanOrEqual", _) => format!("{cell}>=({a})"),
                ("lessThanOrEqual", _) => format!("{cell}<=({a})"),
                (op, _) => return Err(format!("cellIs operator {op} not understood")),
            })
        }
        _ => fill(rule.formulas.first().ok_or_else(|| format!("{} rule without a formula", rule.kind))?),
    }
}

/// A rule Excel decides by its type rather than by the formula it stores: blanks,
/// errors and periods of time (checked against Excel: `notContainsBlanks` holds for an
/// error value, whose formula `LEN(TRIM(A1))>0` is an error). `None` for other rules.
/// `now` is the moment as a serial of the workbook's date system.
pub fn by_type(rule: &Rule, value: &Value, now: f64, date1904: bool) -> Option<Hit> {
    let blank = match value {
        Value::Blank => true,
        Value::Text(t) => t.trim().is_empty(),
        _ => false,
    };
    let error = matches!(value, Value::Error(_));
    let yes = |b: bool| Some(if b { Hit::Yes } else { Hit::No });
    match rule.kind.as_str() {
        "containsBlanks" => yes(blank),
        "notContainsBlanks" => yes(!blank),
        "containsErrors" => yes(error),
        "notContainsErrors" => yes(!error),
        "timePeriod" => {
            let Value::Number(n) = value else { return yes(false) };
            let system = if date1904 { xl_serial::DateSystem::V1904 } else { xl_serial::DateSystem::V1900 };
            let (day, today) = (n.floor() as i64, now.floor() as i64);
            // Weeks run Sunday to Saturday, as WEEKDAY(…, 1) counts them.
            let week_of = |d: i64| xl_serial::weekday(d, system).map(|w| d - (w as i64 - 1));
            let month_of = |d: i64| xl_serial::ymd_from_serial(d, system).map(|(y, m, _)| y as i64 * 12 + m as i64 - 1);
            let weeks = (week_of(day), week_of(today));
            let months = (month_of(day), month_of(today));
            match rule.get("timePeriod").unwrap_or("") {
                "today" => yes(day == today),
                "yesterday" => yes(day == today - 1),
                "tomorrow" => yes(day == today + 1),
                "last7Days" => yes((today - 6..=today).contains(&day)),
                "thisWeek" | "lastWeek" | "nextWeek" => match weeks {
                    (Some(w), Some(t)) => yes(w - t == match rule.get("timePeriod") { Some("lastWeek") => -7, Some("nextWeek") => 7, _ => 0 }),
                    _ => yes(false),
                },
                "thisMonth" | "lastMonth" | "nextMonth" => match months {
                    (Some(m), Some(t)) => yes(m - t == match rule.get("timePeriod") { Some("lastMonth") => -1, Some("nextMonth") => 1, _ => 0 }),
                    _ => yes(false),
                },
                _ => None,
            }
        }
        _ => None,
    }
}

/// Decide formula rules for many cells at once: (sheet, index of the rule in its sheet,
/// row, col) each.
pub fn decide(wb: &Workbook, rules: &[Vec<Rule>], targets: &[(u32, usize, u32, u32)], now: f64, ex: &dyn xl_engine::ir::Executor) -> Vec<Hit> {
    let mut out = vec![Hit::Unknown(String::new()); targets.len()];
    let mut scratch = wb.clone();
    // The rules see the values the file holds, as the range rules do (and as Excel shows
    // them): formula cells with a stored value become that value. Only formula cells
    // without one are worked out.
    for i in 0..scratch.cells.len() {
        if scratch.cells.formula[i].is_some() && scratch.cells.kind[i] != Kind::Blank {
            scratch.cells.formula[i] = None;
            scratch.cells.array_ref[i] = None;
        }
    }
    let s = scratch.sheets.len() as u32;
    let mut name = "xl-look scratch".to_string();
    while wb.sheets.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
        name.push('_');
    }
    scratch.sheets.push(xl_cells::Sheet { name, ..Default::default() });
    let mut placed: Vec<(usize, usize)> = Vec::new();
    for (k, &(sheet, r, row, col)) in targets.iter().enumerate() {
        match formula_for(wb, &rules[sheet as usize][r], row, col, now) {
            Ok(f) => {
                let i = scratch.cells.len();
                let n = placed.len() as u32;
                scratch.cells.push_raw(s, n, 0, Kind::Blank, 0.0, None, Some(f), 0);
                placed.push((k, i));
            }
            Err(why) => out[k] = Hit::Unknown(why),
        }
    }
    if placed.is_empty() {
        return out;
    }
    let (done, calc) = xl_engine::lift::calculate_with(ex, &scratch);
    let unknown: HashMap<usize, &str> = calc.uncomputed.iter().map(|(i, why)| (*i, why.as_str())).collect();
    for (k, i) in placed {
        out[k] = if let Some(why) = unknown.get(&i) {
            // A formula whose result is an error does not apply its format.
            if *why == xl_engine::lift::ERROR_RESULT {
                Hit::No
            } else {
                Hit::Unknown(format!("formula not computed: {why}"))
            }
        } else {
            match done.cells.value(i) {
                Value::Bool(b) => if b { Hit::Yes } else { Hit::No },
                Value::Number(n) => if n != 0.0 { Hit::Yes } else { Hit::No },
                // The calculation gives every cell it works out a value: none, not worked out.
                Value::Blank => Hit::Unknown("formula not computed".into()),
                _ => Hit::No,
            }
        };
    }
    out
}

/// The numbers of the cells a rule covers.
pub fn numbers(wb: &Workbook, rule: &Rule) -> Vec<f64> {
    let c = &wb.cells;
    (0..c.len()).filter(|&i| c.sheet[i] == rule.sheet && rule.covers(c.row[i], c.col[i]) && c.kind[i] == Kind::Number).map(|i| c.num[i]).collect()
}

/// PERCENTILE.INC of sorted numbers, p in 0..=1.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let h = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

/// Looks up the number in a cell a `cfvo` names (`$T$5`, `'Sheet 2'!B1`).
pub type CellNumber<'a> = &'a dyn Fn(&str) -> Option<f64>;

/// The value a `cfvo` stands for over sorted numbers: a number, or a single cell.
fn cfvo(n: &Node, sorted: &[f64], cell: CellNumber) -> Result<f64, String> {
    let (min, max) = (sorted.first().copied().unwrap_or(0.0), sorted.last().copied().unwrap_or(0.0));
    let val = || {
        let v = n.get("val").unwrap_or("").trim().trim_start_matches('=');
        v.parse::<f64>().ok().or_else(|| cell(v)).ok_or_else(|| format!("cfvo value {:?} is neither a number nor a cell with one", n.get("val")))
    };
    match n.get("type").unwrap_or("") {
        "min" | "autoMin" => Ok(min),
        "max" | "autoMax" => Ok(max),
        "num" | "formula" => val(),
        "percent" => Ok(min + (max - min) * val()? / 100.0),
        "percentile" => Ok(percentile(sorted, val()? / 100.0)),
        t => Err(format!("cfvo type {t} not understood")),
    }
}

/// What a range rule gives a cell with the number `v`.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Hit(Hit),
    Fill(Spec),
    Bar { color: Option<Spec>, fraction: f64 },
    Icon { set: String, index: usize },
}

/// Linear per channel, the step from the lower colour cut toward zero (as LibreOffice
/// computes colour scales; Excel publishes no rounding).
fn lerp(a: &str, b: &str, t: f64) -> String {
    let ch = |s: &str, i: usize| crate::color::channel(s, i) as f64;
    let mut out = String::new();
    for i in 0..3 {
        let v = ch(a, i) + ((ch(b, i) - ch(a, i)) * t).trunc();
        out.push_str(&format!("{:02X}", v.clamp(0.0, 255.0) as u8));
    }
    out
}

/// A range rule for a cell's value; `sorted` are the numbers of the rule's cells, sorted,
/// and `all` the values of all of them (for duplicates).
pub fn range_rule(rule: &Rule, value: &Value, sorted: &[f64], all: &[Value], colors: &crate::color::Colors, cell: CellNumber) -> Mark {
    let num = match value {
        Value::Number(n) => Some(*n),
        _ => None,
    };
    let yes = |b: bool| Mark::Hit(if b { Hit::Yes } else { Hit::No });
    match rule.kind.as_str() {
        "top10" => {
            let Some(v) = num else { return yes(false) };
            let rank: f64 = rule.get("rank").and_then(|r| r.parse().ok()).unwrap_or(10.0);
            let percent = matches!(rule.get("percent"), Some("1" | "true"));
            let bottom = matches!(rule.get("bottom"), Some("1" | "true"));
            let n = sorted.len();
            let count = if percent { ((n as f64) * rank / 100.0).floor() as usize } else { rank as usize }.clamp(1, n.max(1));
            if n == 0 {
                return yes(false);
            }
            if bottom {
                yes(v <= sorted[count - 1])
            } else {
                yes(v >= sorted[n - count])
            }
        }
        "aboveAverage" => {
            let Some(v) = num else { return yes(false) };
            if sorted.is_empty() {
                return yes(false);
            }
            let n = sorted.len() as f64;
            let mean = sorted.iter().sum::<f64>() / n;
            let above = !matches!(rule.get("aboveAverage"), Some("0" | "false"));
            let equal = matches!(rule.get("equalAverage"), Some("1" | "true"));
            let sd_count: f64 = rule.get("stdDev").and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if sd_count != 0.0 {
                // The standard deviation of the population (STDEV.P); see the golden test.
                let sd = (sorted.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
                let bound = if above { mean + sd_count * sd } else { mean - sd_count * sd };
                return yes(if above { v > bound || (equal && v == bound) } else { v < bound || (equal && v == bound) });
            }
            yes(if above { v > mean || (equal && v == mean) } else { v < mean || (equal && v == mean) })
        }
        "duplicateValues" | "uniqueValues" => {
            let key = |v: &Value| match v {
                Value::Number(n) => Some(format!("n{n}")),
                Value::Text(t) if !t.is_empty() => Some(format!("t{}", t.to_lowercase())),
                Value::Bool(b) => Some(format!("b{b}")),
                Value::Error(e) => Some(format!("e{e}")),
                _ => None,
            };
            let Some(k) = key(value) else { return yes(false) };
            let count = all.iter().filter(|v| key(v).as_deref() == Some(k.as_str())).count();
            yes(if rule.kind == "duplicateValues" { count > 1 } else { count == 1 })
        }
        "colorScale" => {
            let Some(v) = num else { return Mark::Hit(Hit::No) };
            let Some(cs) = rule.content.child("colorScale") else { return Mark::Hit(Hit::Unknown("colour scale without content".into())) };
            let points: Result<Vec<f64>, String> = cs.all("cfvo").map(|n| cfvo(n, sorted, cell)).collect();
            let cols: Vec<Option<String>> = cs.all("color").map(|c| Spec::of(c).and_then(|s| colors.rgb(&s, Some("000000")))).collect();
            let points = match points {
                Ok(p) => p,
                Err(why) => return Mark::Hit(Hit::Unknown(why)),
            };
            if points.len() < 2 || cols.len() != points.len() || cols.iter().any(Option::is_none) {
                return Mark::Hit(Hit::Unknown("colour scale not understood".into()));
            }
            let cols: Vec<String> = cols.into_iter().map(Option::unwrap).collect();
            let rgb = if v <= points[0] {
                cols[0].clone()
            } else if v >= points[points.len() - 1] {
                cols[cols.len() - 1].clone()
            } else {
                let k = (1..points.len()).find(|&k| v <= points[k]).unwrap_or(points.len() - 1);
                let span = points[k] - points[k - 1];
                let t = if span == 0.0 { 1.0 } else { (v - points[k - 1]) / span };
                lerp(&cols[k - 1], &cols[k], t)
            };
            Mark::Fill(Spec::Rgb(rgb, 0.0))
        }
        "dataBar" => {
            let Some(v) = num else { return Mark::Hit(Hit::No) };
            let Some(db) = rule.content.child("dataBar") else { return Mark::Hit(Hit::Unknown("data bar without content".into())) };
            let p: Result<Vec<f64>, String> = db.all("cfvo").map(|n| cfvo(n, sorted, cell)).collect();
            match p {
                Ok(p) if p.len() == 2 => {
                    let span = p[1] - p[0];
                    let fraction = if span <= 0.0 { 1.0 } else { ((v - p[0]) / span).clamp(0.0, 1.0) };
                    Mark::Bar { color: db.child("color").and_then(Spec::of), fraction }
                }
                Ok(_) => Mark::Hit(Hit::Unknown("data bar not understood".into())),
                Err(why) => Mark::Hit(Hit::Unknown(why)),
            }
        }
        "iconSet" => {
            let Some(v) = num else { return Mark::Hit(Hit::No) };
            let Some(is) = rule.content.child("iconSet") else { return Mark::Hit(Hit::Unknown("icon set without content".into())) };
            let set = is.get("iconSet").unwrap_or("3TrafficLights1").to_string();
            let reverse = is.bool_attr("reverse").unwrap_or(false);
            let mut index = 0;
            let list: Vec<&Node> = is.all("cfvo").collect();
            for (k, n) in list.iter().enumerate().skip(1) {
                let t = match cfvo(n, sorted, cell) {
                    Ok(t) => t,
                    Err(why) => return Mark::Hit(Hit::Unknown(why)),
                };
                let gte = n.bool_attr("gte").unwrap_or(true);
                if if gte { v >= t } else { v > t } {
                    index = k;
                }
            }
            if reverse {
                index = list.len().saturating_sub(1) - index;
            }
            Mark::Icon { set, index }
        }
        k => Mark::Hit(Hit::Unknown(format!("rule type {k} not understood"))),
    }
}
