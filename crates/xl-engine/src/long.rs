//! Single cells as rows: the form a group of single cells has in a store.
//!
//! Where a group of single cells is worked with (regions, writing back, plans) it is one
//! row with a column per cell. A model of thousands of cells is a poor table that way: as
//! wide as it has cells, each column of its own type. In a store it is a table with a
//! row per cell:
//!
//! ```text
//! name     String    the name of the cell
//! cell     String    its address (B7)
//! dtype    String    Int64, Float64, Boolean, String, Date, Datetime, Time, Duration;
//!                    Null for a cell without a value
//! number   Float64   the value of a number; of a boolean 1 or 0
//! text     String    the value of a text; a date or time as text (2026-01-31, 12:30:00)
//! ```
//!
//! To change a cell, give its row another `number` or `text`. What a row then holds is
//! read by its `dtype` where that fits (a text for a `Date` is a date), else by where
//! the value is: a `number` is a number, a `text` a text.

use crate::data::{Column, Data, Frame};
use std::collections::HashMap;

pub const NAME: &str = "name";
pub const CELL: &str = "cell";
pub const DTYPE: &str = "dtype";
pub const NUMBER: &str = "number";
pub const TEXT: &str = "text";

/// Whether a frame is single cells as rows: names, and values in `number` or `text`.
pub fn is_long(df: &Frame) -> bool {
    matches!(df.column(NAME).map(|c| &c.data), Some(Data::String(_) | Data::Null(_))) && (df.column(NUMBER).is_some() || df.column(TEXT).is_some())
}

/// One row with a column per cell as rows. `cells` are the names and addresses, in the
/// order of the rows to be; a cell the frame has no column for is a row without a value.
pub fn to_long(wide: &Frame, cells: &[(String, String)]) -> Frame {
    let n = cells.len();
    let (mut dtypes, mut numbers, mut texts) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for (name, _) in cells {
        let data = wide.column(name).map(|c| &c.data);
        let (number, text) = match data {
            Some(Data::Int64(v)) => (v.first().copied().flatten().map(|x| x as f64), None),
            Some(Data::Float64(v)) => (v.first().copied().flatten(), None),
            Some(Data::Bool(v)) => (v.first().copied().flatten().map(|b| if b { 1.0 } else { 0.0 }), None),
            Some(Data::Null(_)) | None => (None, None),
            Some(other) => (None, other.text(0)),
        };
        // A column without any value is stored as text: it says nothing about the cell.
        let dtype = match data {
            Some(d) if number.is_some() || text.is_some() => d.type_name(),
            _ => "Null",
        };
        dtypes.push(Some(dtype.to_string()));
        numbers.push(number);
        texts.push(text);
    }
    Frame {
        height: n,
        columns: vec![
            Column::new(NAME, Data::String(cells.iter().map(|(name, _)| Some(name.clone())).collect())),
            Column::new(CELL, Data::String(cells.iter().map(|(_, cell)| Some(cell.clone())).collect())),
            Column::new(DTYPE, Data::String(dtypes)),
            Column::new(NUMBER, Data::Float64(numbers)),
            Column::new(TEXT, Data::String(texts)),
        ],
    }
}

fn texts_of(df: &Frame, name: &str) -> Vec<Option<String>> {
    match df.column(name).map(|c| &c.data) {
        Some(Data::String(v)) => v.clone(),
        Some(other) => (0..other.len()).map(|k| other.text(k)).collect(),
        None => vec![None; df.height],
    }
}

fn numbers_of(df: &Frame, name: &str) -> Result<Vec<Option<f64>>, String> {
    match df.column(name).map(|c| &c.data) {
        Some(Data::Float64(v)) => Ok(v.clone()),
        Some(Data::Int64(v)) => Ok(v.iter().map(|x| x.map(|x| x as f64)).collect()),
        Some(Data::Bool(v)) => Ok(v.iter().map(|x| x.map(|b| if b { 1.0 } else { 0.0 })).collect()),
        Some(Data::Null(n)) => Ok(vec![None; *n]),
        Some(Data::String(v)) if v.iter().all(Option::is_none) => Ok(vec![None; v.len()]),
        Some(other) => Err(format!("the column `{name}` holds {}: it is for numbers (text, dates and times go into `{TEXT}`)", other.type_name())),
        None => Ok(vec![None; df.height]),
    }
}

/// The value of one row as a column of one value.
fn value(dtype: &str, number: Option<f64>, text: Option<&str>) -> Data {
    use crate::store::{parse_clock, parse_date, parse_datetime, parse_duration};
    if let Some(t) = text {
        let moment = match dtype {
            "Date" => parse_date(t).map(|d| Data::Date(vec![Some(d as i32)])),
            "Datetime" => parse_datetime(t).map(|ms| Data::Datetime(vec![Some(ms)])),
            "Time" => parse_clock(t).map(|ns| Data::Time(vec![Some(ns)])),
            "Duration" => parse_duration(t).map(|ms| Data::Duration(vec![Some(ms)])),
            _ => None,
        };
        if let Some(m) = moment {
            return m;
        }
        if dtype == "String" || number.is_none() {
            return Data::String(vec![Some(t.to_string())]);
        }
    }
    match number {
        Some(x) if dtype == "Boolean" => Data::Bool(vec![Some(x != 0.0)]),
        Some(x) if dtype == "Int64" && x == x.trunc() && x.abs() < 9e15 => Data::Int64(vec![Some(x as i64)]),
        Some(x) => Data::Float64(vec![Some(x)]),
        None => Data::Null(1),
    }
}

/// Single cells as rows from names and JSON values: a number is a number, true and
/// false a boolean; a text is taken as what the cell is (a date for a cell that holds
/// dates), else as text.
pub fn from_values(values: &[(String, serde_json::Value)]) -> Result<Frame, String> {
    use serde_json::Value as J;
    let (mut dtypes, mut numbers, mut texts) = (Vec::new(), Vec::new(), Vec::new());
    for (name, v) in values {
        let (dtype, number, text) = match v {
            J::Null => (Some("Null"), None, None),
            J::Bool(b) => (Some("Boolean"), Some(*b as u8 as f64), None),
            J::Number(n) => (Some(if n.is_i64() { "Int64" } else { "Float64" }), n.as_f64(), None),
            J::String(t) => (None, None, Some(t.clone())),
            other => return Err(format!("the value of {name:?} must be a number, text, true, false or null, not {other}")),
        };
        dtypes.push(dtype.map(str::to_string));
        numbers.push(number);
        texts.push(text);
    }
    Frame::of(vec![
        Column::new(NAME, Data::String(values.iter().map(|(n, _)| Some(n.clone())).collect())),
        Column::new(DTYPE, Data::String(dtypes)),
        Column::new(NUMBER, Data::Float64(numbers)),
        Column::new(TEXT, Data::String(texts)),
    ])
}

/// Rows of single cells as one row with a column per cell.
pub fn to_wide(long: &Frame) -> Result<Frame, String> {
    let names = texts_of(long, NAME);
    let (dtypes, numbers, texts) = (texts_of(long, DTYPE), numbers_of(long, NUMBER)?, texts_of(long, TEXT));
    let mut columns = Vec::with_capacity(long.height);
    for k in 0..long.height {
        let name = names[k].clone().ok_or_else(|| format!("row {} has no name", k + 1))?;
        columns.push(Column::new(name, value(dtypes[k].as_deref().unwrap_or(""), numbers[k], texts[k].as_deref())));
    }
    Frame::new(1, columns)
}

/// The rows of `old` with those of `new` in their place, by name. `new` is rows (names
/// with `number` or `text`; a row keeps its `dtype` where `new` gives none) or one row
/// with a column per cell. A name `old` does not have is an error.
pub fn merged(old: &Frame, new: &Frame, region: &str) -> Result<Frame, String> {
    let names = texts_of(old, NAME);
    let place: HashMap<&str, usize> = names.iter().enumerate().filter_map(|(k, n)| n.as_deref().map(|n| (n, k))).collect();
    let unknown = |name: &str| {
        let have: Vec<&str> = names.iter().flatten().map(String::as_str).take(60).collect();
        format!("{region:?} has no cell {name:?}; it has {have:?}{}", if names.len() > 60 { " …" } else { "" })
    };
    let (mut dtypes, mut numbers, mut texts) = (texts_of(old, DTYPE), numbers_of(old, NUMBER)?, texts_of(old, TEXT));
    if is_long(new) {
        if let Some(other) = new.columns.iter().find(|c| ![NAME, CELL, DTYPE, NUMBER, TEXT].contains(&c.name.as_str())) {
            return Err(format!("{region:?} is single cells, one row each ({NAME}, {CELL}, {DTYPE}, {NUMBER}, {TEXT}); the column {:?} is none of these. To set a cell give its row another `{NUMBER}` or `{TEXT}`, or give one row with a column per cell", other.name));
        }
        let (given, kinds) = (texts_of(new, NAME), new.column(DTYPE).map(|_| texts_of(new, DTYPE)));
        let (n, t) = (numbers_of(new, NUMBER)?, texts_of(new, TEXT));
        for k in 0..new.height {
            let name = given[k].as_deref().ok_or_else(|| format!("row {} has no name", k + 1))?;
            let at = *place.get(name).ok_or_else(|| unknown(name))?;
            numbers[at] = n[k];
            texts[at] = t[k].clone();
            if let Some(kind) = kinds.as_ref().and_then(|d| d[k].clone()) {
                dtypes[at] = Some(kind);
            }
        }
    } else {
        if new.height != 1 {
            return Err(format!("{region:?} is single cells: give rows with `{NAME}` and `{NUMBER}` or `{TEXT}`, or one row with a column per cell (this frame has {} rows and no `{NAME}`)", new.height));
        }
        let cells: Vec<(String, String)> = new.columns.iter().map(|c| (c.name.clone(), String::new())).collect();
        let rows = to_long(new, &cells);
        let (kinds, n, t) = (texts_of(&rows, DTYPE), numbers_of(&rows, NUMBER)?, texts_of(&rows, TEXT));
        for (k, (name, _)) in cells.iter().enumerate() {
            let at = *place.get(name.as_str()).ok_or_else(|| unknown(name))?;
            numbers[at] = n[k];
            texts[at] = t[k].clone();
            dtypes[at] = kinds[k].clone();
        }
    }
    Frame::new(
        old.height,
        vec![
            Column::new(NAME, Data::String(names.clone())),
            Column::new(CELL, Data::String(texts_of(old, CELL))),
            Column::new(DTYPE, Data::String(dtypes)),
            Column::new(NUMBER, Data::Float64(numbers)),
            Column::new(TEXT, Data::String(texts)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide() -> Frame {
        Frame::of(vec![
            Column::new("Rate", Data::Float64(vec![Some(0.19)])),
            Column::new("Count", Data::Int64(vec![Some(4)])),
            Column::new("Open", Data::Bool(vec![Some(true)])),
            Column::new("Label", Data::String(vec![Some("net".into())])),
            Column::new("Due", Data::Date(vec![Some(20_484)])),
            Column::new("At", Data::Time(vec![Some(45_000_000_000_000)])),
            Column::new("Empty", Data::String(vec![None])),
        ])
        .unwrap()
    }

    fn cells(df: &Frame) -> Vec<(String, String)> {
        df.columns.iter().enumerate().map(|(k, c)| (c.name.clone(), format!("B{}", k + 1))).collect()
    }

    #[test]
    fn single_cells_go_into_rows_and_come_back() {
        let wide = wide();
        let long = to_long(&wide, &cells(&wide));
        assert!(is_long(&long) && !is_long(&wide));
        assert_eq!(long.shape(), (7, 5));
        assert_eq!(texts_of(&long, DTYPE).into_iter().flatten().collect::<Vec<_>>(), vec!["Float64", "Int64", "Boolean", "String", "Date", "Time", "Null"]);
        assert_eq!(numbers_of(&long, NUMBER).unwrap(), vec![Some(0.19), Some(4.0), Some(1.0), None, None, None, None]);
        assert_eq!(texts_of(&long, TEXT)[3..6], [Some("net".to_string()), Some("2026-01-31".to_string()), Some("12:30:00".to_string())]);
        let back = to_wide(&long).unwrap();
        // A cell without a value has no type.
        let mut want = wide.clone();
        want.columns[6].data = Data::Null(1);
        assert_eq!(back, want);
    }

    #[test]
    fn rows_are_changed_by_name() {
        let wide = wide();
        let long = to_long(&wide, &cells(&wide));
        // Rows by name: a number, a date as text, a text where a number was.
        let new = Frame::of(vec![
            Column::new(NAME, Data::String(vec![Some("Rate".into()), Some("Due".into()), Some("Count".into())])),
            Column::new(NUMBER, Data::Float64(vec![Some(0.07), None, None])),
            Column::new(TEXT, Data::String(vec![None, Some("2026-02-01".into()), Some("many".into())])),
        ])
        .unwrap();
        let back = to_wide(&merged(&long, &new, "Model").unwrap()).unwrap();
        assert_eq!(back.column("Rate").unwrap().data, Data::Float64(vec![Some(0.07)]));
        assert_eq!(back.column("Due").unwrap().data, Data::Date(vec![Some(20_485)]));
        assert_eq!(back.column("Count").unwrap().data, Data::String(vec![Some("many".into())]));
        assert_eq!(back.column("Open").unwrap().data, Data::Bool(vec![Some(true)]));
        // One row with a column per cell.
        let one = Frame::of(vec![Column::new("Count", Data::Int64(vec![Some(7)])), Column::new("Open", Data::Bool(vec![Some(false)]))]).unwrap();
        let back = to_wide(&merged(&long, &one, "Model").unwrap()).unwrap();
        assert_eq!((back.column("Count").unwrap().data.clone(), back.column("Open").unwrap().data.clone()), (Data::Int64(vec![Some(7)]), Data::Bool(vec![Some(false)])));
        // What is not a cell of the region is refused, and so is a column that is no part of rows.
        let other = Frame::of(vec![Column::new("Nope", Data::Int64(vec![Some(1)]))]).unwrap();
        assert!(merged(&long, &other, "Model").unwrap_err().contains("has no cell \"Nope\""));
        let mut extra = long.clone();
        extra.columns.push(Column::new("Rate", Data::Float64(vec![Some(1.0); 7])));
        assert!(merged(&long, &extra, "Model").unwrap_err().contains("one row each"));
    }
}
