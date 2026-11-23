//! Typed tables: the form in which sheets and regions leave the engine and data comes
//! back into it. Executors and bindings turn them into their own frames (Polars, Arrow).

/// The values of a column, one data type for all; `None` is null.
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    /// A column without any value.
    Null(usize),
    Bool(Vec<Option<bool>>),
    Int64(Vec<Option<i64>>),
    Float64(Vec<Option<f64>>),
    String(Vec<Option<String>>),
    /// Days since 1970-01-01.
    Date(Vec<Option<i32>>),
    /// Milliseconds since 1970-01-01, without a time zone.
    Datetime(Vec<Option<i64>>),
    /// Nanoseconds since midnight.
    Time(Vec<Option<i64>>),
    /// Milliseconds.
    Duration(Vec<Option<i64>>),
}

impl Data {
    pub fn len(&self) -> usize {
        match self {
            Data::Null(n) => *n,
            Data::Bool(v) => v.len(),
            Data::Int64(v) => v.len(),
            Data::Float64(v) => v.len(),
            Data::String(v) => v.len(),
            Data::Date(v) => v.len(),
            Data::Datetime(v) | Data::Time(v) | Data::Duration(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The name of the type, as Polars names it.
    pub fn type_name(&self) -> &'static str {
        match self {
            Data::Null(_) => "Null",
            Data::Bool(_) => "Boolean",
            Data::Int64(_) => "Int64",
            Data::Float64(_) => "Float64",
            Data::String(_) => "String",
            Data::Date(_) => "Date",
            Data::Datetime(_) => "Datetime",
            Data::Time(_) => "Time",
            Data::Duration(_) => "Duration",
        }
    }

    /// A column of the same type with every value null.
    pub fn nulls_like(&self, n: usize) -> Data {
        match self {
            Data::Null(_) => Data::Null(n),
            Data::Bool(_) => Data::Bool(vec![None; n]),
            Data::Int64(_) => Data::Int64(vec![None; n]),
            Data::Float64(_) => Data::Float64(vec![None; n]),
            Data::String(_) => Data::String(vec![None; n]),
            Data::Date(_) => Data::Date(vec![None; n]),
            Data::Datetime(_) => Data::Datetime(vec![None; n]),
            Data::Time(_) => Data::Time(vec![None; n]),
            Data::Duration(_) => Data::Duration(vec![None; n]),
        }
    }

    /// The values of the given rows, in their order.
    pub fn take(&self, rows: &[usize]) -> Data {
        fn of<T: Clone>(v: &[Option<T>], rows: &[usize]) -> Vec<Option<T>> {
            rows.iter().map(|&r| v[r].clone()).collect()
        }
        match self {
            Data::Null(_) => Data::Null(rows.len()),
            Data::Bool(v) => Data::Bool(of(v, rows)),
            Data::Int64(v) => Data::Int64(of(v, rows)),
            Data::Float64(v) => Data::Float64(of(v, rows)),
            Data::String(v) => Data::String(of(v, rows)),
            Data::Date(v) => Data::Date(of(v, rows)),
            Data::Datetime(v) => Data::Datetime(of(v, rows)),
            Data::Time(v) => Data::Time(of(v, rows)),
            Data::Duration(v) => Data::Duration(of(v, rows)),
        }
    }

    /// Columns one below the other. Parts of one type keep it; whole numbers and
    /// fractions together are fractions; a part without any value takes the type of the
    /// others; anything else becomes text.
    pub fn concat(parts: &[&Data]) -> Data {
        let empty = |d: &Data| match d {
            Data::Null(_) => true,
            Data::String(v) => v.iter().all(Option::is_none),
            _ => false,
        };
        let n: usize = parts.iter().map(|d| d.len()).sum();
        let mut types: Vec<&'static str> = parts.iter().filter(|d| !empty(d)).map(|d| d.type_name()).collect();
        types.dedup();
        types.sort_unstable();
        types.dedup();
        macro_rules! join {
            ($variant:ident) => {{
                let mut out = Vec::with_capacity(n);
                for d in parts {
                    match d {
                        Data::$variant(v) => out.extend(v.iter().cloned()),
                        other => out.extend(std::iter::repeat_n(None, other.len())),
                    }
                }
                Data::$variant(out)
            }};
        }
        match types.as_slice() {
            [] => Data::Null(n),
            ["Boolean"] => join!(Bool),
            ["Int64"] => join!(Int64),
            ["Float64"] => join!(Float64),
            ["String"] => join!(String),
            ["Date"] => join!(Date),
            ["Datetime"] => join!(Datetime),
            ["Time"] => join!(Time),
            ["Duration"] => join!(Duration),
            ["Float64", "Int64"] => {
                let mut out = Vec::with_capacity(n);
                for d in parts {
                    match d {
                        Data::Float64(v) => out.extend(v.iter().copied()),
                        Data::Int64(v) => out.extend(v.iter().map(|x| x.map(|x| x as f64))),
                        other => out.extend(std::iter::repeat_n(None, other.len())),
                    }
                }
                Data::Float64(out)
            }
            _ => Data::String(parts.iter().flat_map(|d| (0..d.len()).map(|k| d.text(k))).collect()),
        }
    }

    /// As [`Data::nulls_like`], with the values of the given rows kept (rows beyond
    /// either length are left out).
    pub fn nulls_keeping(&self, n: usize, rows: &[usize]) -> Data {
        fn keep<T: Clone>(old: &[Option<T>], n: usize, rows: &[usize]) -> Vec<Option<T>> {
            let mut out = vec![None; n];
            for &r in rows.iter().filter(|&&r| r < n && r < old.len()) {
                out[r] = old[r].clone();
            }
            out
        }
        match self {
            Data::Null(_) => Data::Null(n),
            Data::Bool(v) => Data::Bool(keep(v, n, rows)),
            Data::Int64(v) => Data::Int64(keep(v, n, rows)),
            Data::Float64(v) => Data::Float64(keep(v, n, rows)),
            Data::String(v) => Data::String(keep(v, n, rows)),
            Data::Date(v) => Data::Date(keep(v, n, rows)),
            Data::Datetime(v) => Data::Datetime(keep(v, n, rows)),
            Data::Time(v) => Data::Time(keep(v, n, rows)),
            Data::Duration(v) => Data::Duration(keep(v, n, rows)),
        }
    }

    /// The values as numbers (booleans 1 and 0; dates and times their physical value;
    /// text and null `None`).
    pub fn numbers(&self) -> Vec<Option<f64>> {
        match self {
            Data::Null(n) => vec![None; *n],
            Data::Bool(v) => v.iter().map(|x| x.map(|b| b as u8 as f64)).collect(),
            Data::Int64(v) | Data::Datetime(v) | Data::Time(v) | Data::Duration(v) => v.iter().map(|x| x.map(|i| i as f64)).collect(),
            Data::Float64(v) => v.clone(),
            Data::String(v) => vec![None; v.len()],
            Data::Date(v) => v.iter().map(|x| x.map(|i| i as f64)).collect(),
        }
    }

    /// The value at a row as text, the way Polars casts it to a string; `None` for null.
    pub fn text(&self, k: usize) -> Option<String> {
        match self {
            Data::Null(_) => None,
            Data::Bool(v) => v.get(k).copied().flatten().map(|b| b.to_string()),
            Data::Int64(v) => v.get(k).copied().flatten().map(|i| i.to_string()),
            Data::Float64(v) => v.get(k).copied().flatten().map(float_text),
            Data::String(v) => v.get(k).cloned().flatten(),
            Data::Date(v) => v.get(k).copied().flatten().map(|d| date_text(d as i64)),
            Data::Datetime(v) => v.get(k).copied().flatten().map(|ms| {
                let rest = ms.rem_euclid(86_400_000);
                format!("{} {}.{:03}", date_text(ms.div_euclid(86_400_000)), clock_text(rest / 1000 * 1_000_000_000), rest % 1000)
            }),
            Data::Time(v) => v.get(k).copied().flatten().map(clock_text),
            Data::Duration(v) => v.get(k).copied().flatten().map(duration_text),
        }
    }
}

/// A float as Polars casts it to text: `1.0`, `0.5`, `1e20`.
pub fn float_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x.fract() == 0.0 && x.abs() < 1e16 {
        return format!("{x:.1}");
    }
    let a = x.abs();
    if a >= 1e16 || a < 1e-5 {
        return format!("{x:e}");
    }
    format!("{x}")
}

/// (year, month, day) of days since 1970-01-01.
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// Days since 1970-01-01 of a date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn date_text(days: i64) -> String {
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A time of day in nanoseconds as `hh:mm:ss`, a fraction in 3, 6 or 9 digits as needed.
fn clock_text(ns: i64) -> String {
    let s = ns.div_euclid(1_000_000_000);
    let frac = ns.rem_euclid(1_000_000_000);
    let base = format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60);
    match frac {
        0 => base,
        f if f % 1_000_000 == 0 => format!("{base}.{:03}", f / 1_000_000),
        f if f % 1000 == 0 => format!("{base}.{:06}", f / 1000),
        f => format!("{base}.{f:09}"),
    }
}

/// A duration in milliseconds as Polars shows it: `1d 2h 3m 4s 5ms`, `0ms`.
fn duration_text(ms: i64) -> String {
    if ms == 0 {
        return "0ms".into();
    }
    let sign = if ms < 0 { "-" } else { "" };
    let mut rest = ms.unsigned_abs();
    let mut parts = Vec::new();
    for (unit, size) in [("d", 86_400_000u64), ("h", 3_600_000), ("m", 60_000), ("s", 1000), ("ms", 1)] {
        if rest >= size {
            parts.push(format!("{}{unit}", rest / size));
            rest %= size;
        }
    }
    format!("{sign}{}", parts.join(" "))
}

impl From<Vec<&str>> for Data {
    fn from(v: Vec<&str>) -> Self {
        Data::String(v.into_iter().map(|s| Some(s.to_string())).collect())
    }
}
impl From<Vec<i64>> for Data {
    fn from(v: Vec<i64>) -> Self {
        Data::Int64(v.into_iter().map(Some).collect())
    }
}
impl From<Vec<f64>> for Data {
    fn from(v: Vec<f64>) -> Self {
        Data::Float64(v.into_iter().map(Some).collect())
    }
}

/// A frame from (name, values) pairs, as Polars' `df!`: `frame![("qty", vec![1i64, 2])]`.
#[macro_export]
macro_rules! frame {
    ($(($name:expr, $values:expr)),* $(,)?) => {
        $crate::data::Frame::of(vec![$($crate::data::Column::new($name, $crate::data::Data::from($values))),*])
    };
}

/// A named column.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub data: Data,
}

impl Column {
    pub fn new(name: impl Into<String>, data: Data) -> Self {
        Column { name: name.into(), data }
    }
}

/// A table of named columns of equal length.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub height: usize,
    pub columns: Vec<Column>,
}

impl Frame {
    /// A frame of columns; they must all have `height` values.
    pub fn new(height: usize, columns: Vec<Column>) -> Result<Frame, String> {
        if let Some(c) = columns.iter().find(|c| c.data.len() != height) {
            return Err(format!("column {:?} has {} values, not {height}", c.name, c.data.len()));
        }
        let mut seen = std::collections::HashSet::new();
        if let Some(c) = columns.iter().find(|c| !seen.insert(c.name.as_str())) {
            return Err(format!("the column name {:?} is used twice", c.name));
        }
        Ok(Frame { height, columns })
    }

    /// A frame with one row per value of the longest column; all columns as long.
    pub fn of(columns: Vec<Column>) -> Result<Frame, String> {
        let height = columns.first().map_or(0, |c| c.data.len());
        Frame::new(height, columns)
    }

    pub fn empty() -> Frame {
        Frame::default()
    }

    /// (rows, columns).
    pub fn shape(&self) -> (usize, usize) {
        (self.height, self.columns.len())
    }

    pub fn width(&self) -> usize {
        self.columns.len()
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }

    /// The columns named, in that order.
    pub fn select<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Result<Frame, String> {
        let columns = names.into_iter().map(|n| self.column(n).cloned().ok_or_else(|| format!("no column {n:?}"))).collect::<Result<Vec<_>, _>>()?;
        Frame::new(self.height, columns)
    }

    /// The rows from `offset`, at most `limit`.
    pub fn slice(&self, offset: usize, limit: usize) -> Frame {
        let start = offset.min(self.height);
        let end = start.saturating_add(limit).min(self.height);
        let cut = |d: &Data| -> Data {
            match d {
                Data::Null(_) => Data::Null(end - start),
                Data::Bool(v) => Data::Bool(v[start..end].to_vec()),
                Data::Int64(v) => Data::Int64(v[start..end].to_vec()),
                Data::Float64(v) => Data::Float64(v[start..end].to_vec()),
                Data::String(v) => Data::String(v[start..end].to_vec()),
                Data::Date(v) => Data::Date(v[start..end].to_vec()),
                Data::Datetime(v) => Data::Datetime(v[start..end].to_vec()),
                Data::Time(v) => Data::Time(v[start..end].to_vec()),
                Data::Duration(v) => Data::Duration(v[start..end].to_vec()),
            }
        };
        Frame { height: end - start, columns: self.columns.iter().map(|c| Column::new(c.name.clone(), cut(&c.data))).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_as_text_as_polars_casts_them() {
        assert_eq!(float_text(1.0), "1.0");
        assert_eq!(float_text(7.5), "7.5");
        assert_eq!(Data::Date(vec![Some(0)]).text(0).as_deref(), Some("1970-01-01"));
        assert_eq!(Data::Datetime(vec![Some(86_400_000 + 1500)]).text(0).as_deref(), Some("1970-01-02 00:00:01.500"));
        assert_eq!(Data::Datetime(vec![Some(0)]).text(0).as_deref(), Some("1970-01-01 00:00:00.000"));
        assert_eq!(Data::Time(vec![Some(45_000_000_000_000)]).text(0).as_deref(), Some("12:30:00"));
        assert_eq!(days_from_civil(2026, 11, 16), 20_773);
        assert_eq!(civil(20_773), (2026, 11, 16));
    }
}
