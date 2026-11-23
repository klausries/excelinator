//! Frames as an application shows and edits them: rows as JSON records, the rows of a
//! view (filtered, sorted), and a frame with rows changed.

use crate::data::{Column, Data, Frame};
use serde_json::{json, Value as J};
use std::collections::{HashMap, HashSet};

/// A number as it is shown: to 15 significant digits (7.140000000000001 is 7.14).
fn shown(x: f64) -> f64 {
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// Rows of a frame as JSON records (dates and times as text), from `offset`, at most `limit`.
pub fn rows_json(df: &Frame, offset: usize, limit: usize) -> Result<J, String> {
    let part = df.slice(offset, limit);
    let mut columns: Vec<(String, Vec<J>)> = Vec::new();
    for c in &part.columns {
        let n = c.data.len();
        let values: Vec<J> = match &c.data {
            Data::Bool(v) => v.iter().map(|v| v.map_or(J::Null, J::Bool)).collect(),
            Data::Int64(v) => v.iter().map(|v| v.map_or(J::Null, |x| json!(x))).collect(),
            Data::Float64(v) => v.iter().map(|v| v.filter(|x| x.is_finite()).map_or(J::Null, |x| json!(shown(x)))).collect(),
            Data::Null(_) => vec![J::Null; n],
            d => (0..n).map(|k| d.text(k).map_or(J::Null, J::String)).collect(),
        };
        columns.push((c.name.clone(), values));
    }
    let rows: Vec<J> = (0..part.height).map(|k| J::Object(columns.iter().map(|(n, v)| (n.clone(), v[k].clone())).collect())).collect();
    Ok(J::Array(rows))
}

fn digits(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `2026-01-31` as days since 1970.
pub fn parse_date(s: &str) -> Option<i64> {
    let mut p = s.trim().splitn(3, '-');
    let (y, m, d) = (digits(p.next()?)?, digits(p.next()?)?, digits(p.next()?)?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| crate::data::days_from_civil(y, m, d))
}

/// `12:30:00`, `12:30:00.250`, `12:30` as nanoseconds since midnight.
pub fn parse_clock(s: &str) -> Option<i64> {
    let (main, frac) = s.trim().split_once('.').unwrap_or((s.trim(), ""));
    let parts: Vec<&str> = main.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let (h, m) = (digits(parts[0])?, digits(parts[1])?);
    let sec = if parts.len() == 3 { digits(parts[2])? } else { 0 };
    if h > 23 || m > 59 || sec > 59 || frac.len() > 9 || (!frac.is_empty() && digits(frac).is_none()) {
        return None;
    }
    let f = if frac.is_empty() { 0 } else { digits(frac)? * 10i64.pow(9 - frac.len() as u32) };
    Some(((h * 60 + m) * 60 + sec) * 1_000_000_000 + f)
}

/// `2026-01-31 12:30:00` (or with `T`) as milliseconds since 1970; a date alone is midnight.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = match s.find([' ', 'T']) {
        Some(k) => (&s[..k], Some(&s[k + 1..])),
        None => (s, None),
    };
    let days = parse_date(date)?;
    let ns = match time {
        Some(t) => parse_clock(t)?,
        None => 0,
    };
    Some(days * 86_400_000 + ns / 1_000_000)
}

/// A duration as `rows_json` writes it (`1d 2h 3m 4s 5ms`) in milliseconds.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = s.strip_prefix('-').map_or((1, s), |r| (-1, r));
    let mut total = 0i64;
    for part in s.split_whitespace() {
        let k = part.find(|c: char| !c.is_ascii_digit())?;
        let n = digits(&part[..k])?;
        total += n * match &part[k..] {
            "d" => 86_400_000,
            "h" => 3_600_000,
            "m" => 60_000,
            "s" => 1000,
            "ms" => 1,
            _ => return None,
        };
    }
    Some(sign * total)
}

/// A frame from JSON records. The columns are those of `like` that the records name,
/// with its types (dates and times given as text, `2026-01-31`, `12:30:00`); other keys
/// follow, typed by their values.
pub fn frame_from_rows(rows: &J, like: &Frame) -> Result<Frame, String> {
    let rows = rows.as_array().ok_or("rows must be a list of objects, one per row: [{\"column\": value, …}, …]")?;
    let mut names: Vec<String> = like.names().into_iter().map(str::to_string).filter(|n| rows.iter().any(|r| r.get(n).is_some())).collect();
    for r in rows {
        for k in r.as_object().ok_or("every row must be an object: {\"column\": value, …}")?.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    let mut columns: Vec<Column> = Vec::new();
    for name in &names {
        let values: Vec<&J> = rows.iter().map(|r| r.get(name).unwrap_or(&J::Null)).collect();
        let want = like.column(name).map(|c| &c.data);
        let all = |f: fn(&J) -> bool| values.iter().all(|v| v.is_null() || f(v));
        let texts = || values.iter().map(|v| v.as_str());
        let data = match want {
            Some(Data::Date(_)) if all(J::is_string) => Data::Date(texts().map(|t| t.and_then(parse_date).map(|d| d as i32)).collect()),
            Some(Data::Datetime(_)) if all(J::is_string) => Data::Datetime(texts().map(|t| t.and_then(parse_datetime)).collect()),
            Some(Data::Time(_)) if all(J::is_string) => Data::Time(texts().map(|t| t.and_then(parse_clock)).collect()),
            Some(Data::Duration(_)) if all(J::is_string) => Data::Duration(texts().map(|t| t.and_then(parse_duration)).collect()),
            _ if all(J::is_boolean) && values.iter().any(|v| v.is_boolean()) => Data::Bool(values.iter().map(|v| v.as_bool()).collect()),
            _ if all(J::is_number) && values.iter().any(|v| v.is_number()) => {
                if all(|v| v.is_i64()) && !matches!(want, Some(Data::Float64(_))) {
                    Data::Int64(values.iter().map(|v| v.as_i64()).collect())
                } else {
                    Data::Float64(values.iter().map(|v| v.as_f64()).collect())
                }
            }
            _ if values.iter().all(|v| v.is_null()) => Data::Null(values.len()),
            _ => Data::String(values.iter().map(|v| match v { J::Null => None, J::String(s) => Some(s.clone()), other => Some(other.to_string()) }).collect()),
        };
        columns.push(Column::new(name.clone(), data));
    }
    Frame::new(rows.len(), columns)
}

/// The rows of a view of a frame, in the view's order. A view is
/// `{"filter": [{"column": "qty", "op": ">", "value": 2}], "search": "nor", "sort": [{"column": "qty", "desc": true}]}`.
/// Every filter must hold: `=`, `!=`, `<`, `<=`, `>`, `>=` (numbers as numbers, anything
/// else as text, without regard to case), `contains`, `empty`, `not empty`. `search` is a
/// text some column of the row contains. Rows without a value come last when sorted.
pub fn rows(df: &Frame, view: &J) -> Result<Vec<usize>, String> {
    let column = |name: &J| {
        let name = name.as_str().unwrap_or("");
        df.column(name).map(|c| &c.data).ok_or_else(|| format!("no column {name:?}; the columns are {:?}", df.names()))
    };
    let lower = |d: &Data, k: usize| d.text(k).map(|t| t.to_lowercase());
    let mut keep: Vec<usize> = (0..df.height).collect();
    for f in view["filter"].as_array().into_iter().flatten() {
        let data = column(&f["column"])?;
        let op = f["op"].as_str().unwrap_or("=");
        let numbers = data.numbers();
        let number = f["value"].as_f64().or_else(|| f["value"].as_str().and_then(|t| t.trim().parse().ok()));
        let text = match &f["value"] {
            J::String(t) => t.to_lowercase(),
            J::Null => String::new(),
            other => other.to_string(),
        };
        let holds: Box<dyn Fn(usize) -> bool> = match op {
            "empty" => Box::new(|k| lower(data, k).is_none_or(|t| t.is_empty())),
            "not empty" => Box::new(|k| lower(data, k).is_some_and(|t| !t.is_empty())),
            "contains" => Box::new(|k| lower(data, k).is_some_and(|t| t.contains(&text))),
            "=" | "!=" | "<" | "<=" | ">" | ">=" => Box::new(|k| {
                let order = match (numbers[k], number) {
                    (Some(a), Some(b)) => a.partial_cmp(&b),
                    _ => lower(data, k).map(|t| t.cmp(&text)),
                };
                match (order, op) {
                    (None, _) => op == "!=",
                    (Some(o), "=") => o.is_eq(),
                    (Some(o), "!=") => o.is_ne(),
                    (Some(o), "<") => o.is_lt(),
                    (Some(o), "<=") => o.is_le(),
                    (Some(o), ">") => o.is_gt(),
                    (Some(o), _) => o.is_ge(),
                }
            }),
            other => return Err(format!("no filter {other:?}; there are =, !=, <, <=, >, >=, contains, empty, not empty")),
        };
        keep.retain(|&k| holds(k));
    }
    if let Some(search) = view["search"].as_str().map(str::to_lowercase).filter(|t| !t.is_empty()) {
        keep.retain(|&k| df.columns.iter().any(|c| lower(&c.data, k).is_some_and(|t| t.contains(&search))));
    }
    // The last key first: the sort keeps the order of what it finds equal.
    for key in view["sort"].as_array().into_iter().flatten().rev() {
        let data = column(&key["column"])?;
        let (numbers, desc) = (data.numbers(), key["desc"].as_bool().unwrap_or(false));
        let texts: Vec<Option<String>> = if matches!(data, Data::String(_)) { (0..df.height).map(|k| lower(data, k)).collect() } else { Vec::new() };
        keep.sort_by(|&a, &b| {
            let order = match texts.is_empty() {
                true => match (numbers[a], numbers[b]) {
                    (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                    (x, y) => return y.is_some().cmp(&x.is_some()),
                },
                false => match (&texts[a], &texts[b]) {
                    (Some(x), Some(y)) => x.cmp(y),
                    (x, y) => return y.is_some().cmp(&x.is_some()),
                },
            };
            if desc { order.reverse() } else { order }
        });
    }
    Ok(keep)
}

/// What the columns of numbers add up to in some rows of a frame: by column its
/// `count` (of values), `sum`, `min` and `max`.
pub fn summary(df: &Frame, rows: &[usize]) -> J {
    let mut out = serde_json::Map::new();
    for c in df.columns.iter().filter(|c| matches!(c.data, Data::Int64(_) | Data::Float64(_))) {
        let numbers = c.data.numbers();
        let values: Vec<f64> = rows.iter().filter_map(|&k| numbers[k]).filter(|x| x.is_finite()).collect();
        if !values.is_empty() {
            let sum: f64 = values.iter().sum();
            out.insert(c.name.clone(), json!({ "count": values.len(), "sum": shown(sum), "min": values.iter().copied().fold(f64::INFINITY, f64::min), "max": values.iter().copied().fold(f64::NEG_INFINITY, f64::max) }));
        }
    }
    J::Object(out)
}

/// The rows of a frame in another order or a part of them.
pub fn taken(df: &Frame, rows: &[usize]) -> Frame {
    Frame { height: rows.len(), columns: df.columns.iter().map(|c| Column::new(c.name.clone(), c.data.take(rows))).collect() }
}

/// A frame with rows changed:
/// `{"set": [{"row": 0, "column": "qty", "value": 5}], "delete": [3], "insert": {"at": 1, "rows": [{"qty": 1}]}}`.
/// Rows are counted from 0 in the frame as it is; new rows come in before its row `at`
/// (at its end without `at`). A value is taken as its column is (see [`frame_from_rows`]).
pub fn patched(df: &Frame, patch: &J) -> Result<Frame, String> {
    let n = df.height;
    let list = |v: &'_ J| v.as_array().cloned().unwrap_or_default();
    let row_of = |v: &J| v.as_u64().map(|r| r as usize).filter(|r| *r < n).ok_or_else(|| format!("row {v} is not a row of the table: it has the rows 0 to {}", n.saturating_sub(1)));
    let known = |name: &str| df.column(name).map(|_| ()).ok_or_else(|| format!("no column {name:?}; the columns are {:?}", df.names()));
    let gone: HashSet<usize> = list(&patch["delete"]).iter().map(row_of).collect::<Result<_, _>>()?;
    let mut sets: HashMap<(usize, String), J> = HashMap::new();
    for set in list(&patch["set"]) {
        let name = set["column"].as_str().ok_or("a change needs `row`, `column` and `value`")?;
        known(name)?;
        sets.insert((row_of(&set["row"])?, name.to_string()), set["value"].clone());
    }
    let new = list(&patch["insert"]["rows"]);
    for name in new.iter().flat_map(|r| r.as_object().into_iter().flat_map(|o| o.keys())) {
        known(name)?;
    }
    let at = patch["insert"]["at"].as_u64().map_or(n, |a| (a as usize).min(n));
    // The rows to be: one there is (by its number), or a new one.
    let mut order: Vec<Result<usize, &J>> = Vec::new();
    for k in 0..=n {
        if k == at {
            order.extend(new.iter().map(Err));
        }
        if k < n && !gone.contains(&k) {
            order.push(Ok(k));
        }
    }
    let mut columns = Vec::new();
    for c in &df.columns {
        let one = |v: &J| frame_from_rows(&json!([{ c.name.as_str(): v }]), df).map(|f| f.columns.into_iter().next().map_or(Data::Null(1), |x| x.data));
        let (mut parts, mut run): (Vec<Data>, Vec<usize>) = (Vec::new(), Vec::new());
        for src in &order {
            let value = match src {
                Ok(k) => sets.get(&(*k, c.name.clone())),
                Err(row) => Some(row.get(&c.name).unwrap_or(&J::Null)),
            };
            match (src, value) {
                (Ok(k), None) => run.push(*k),
                (_, Some(v)) => {
                    parts.push(c.data.take(&std::mem::take(&mut run)));
                    parts.push(one(v)?);
                }
                (Err(_), None) => {}
            }
        }
        parts.push(c.data.take(&run));
        columns.push(Column::new(c.name.clone(), Data::concat(&parts.iter().collect::<Vec<_>>())));
    }
    Frame::new(order.len(), columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times_come_back_from_their_text() {
        assert_eq!(parse_date("2026-01-31"), Some(20_484));
        assert_eq!(parse_datetime("1970-01-02 00:00:01.500"), Some(86_401_500));
        assert_eq!(parse_clock("12:30:00"), Some(45_000_000_000_000));
        assert_eq!(parse_duration("1d 2h"), Some(93_600_000));
        let like = Frame::of(vec![Column::new("d", Data::Duration(vec![Some(1)]))]).unwrap();
        let back = frame_from_rows(&json!([{"d": "1d 2h 3m 4s 5ms"}]), &like).unwrap();
        assert_eq!(rows_json(&back, 0, 10).unwrap(), json!([{"d": "1d 2h 3m 4s 5ms"}]));
    }

    #[test]
    fn a_view_filters_and_sorts_and_a_patch_changes_rows() {
        let df = crate::frame![("item", vec!["b", "a", "c", "d"]), ("qty", vec![2i64, 1, 3, 4]), ("price", vec![2.5, 1.0, 2.5, 9.0])].unwrap();
        let view = json!({"filter": [{"column": "qty", "op": "<", "value": 4}], "sort": [{"column": "price", "desc": true}, {"column": "item"}]});
        assert_eq!(rows(&df, &view).unwrap(), vec![0, 2, 1]);
        assert_eq!(rows(&df, &json!({"search": "D"})).unwrap(), vec![3]);
        assert_eq!(summary(&df, &[0, 2])["qty"], json!({"count": 2, "sum": 5.0, "min": 2.0, "max": 3.0}));
        assert!(rows(&df, &json!({"filter": [{"column": "nope"}]})).unwrap_err().contains("the columns are"));

        let patch = json!({"set": [{"row": 1, "column": "qty", "value": 10}], "delete": [2], "insert": {"at": 0, "rows": [{"item": "new", "price": 1}]}});
        let got = patched(&df, &patch).unwrap();
        assert_eq!(rows_json(&got, 0, 9).unwrap(), json!([
            {"item": "new", "qty": null, "price": 1.0},
            {"item": "b", "qty": 2, "price": 2.5},
            {"item": "a", "qty": 10, "price": 1.0},
            {"item": "d", "qty": 4, "price": 9.0}]));
        assert!(patched(&df, &json!({"delete": [9]})).unwrap_err().contains("0 to 3"));
    }
}
