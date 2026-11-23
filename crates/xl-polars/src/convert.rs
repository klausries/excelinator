//! The engine's frames as Polars frames, and Polars frames as the engine's.

use polars::prelude::*;
use xl_engine::data::{Column as XColumn, Data, Frame};

/// A frame of the engine as a Polars frame.
pub fn to_polars(f: &Frame) -> PolarsResult<DataFrame> {
    let mut columns: Vec<Column> = Vec::with_capacity(f.columns.len());
    for c in &f.columns {
        let name: PlSmallStr = c.name.as_str().into();
        let s = match &c.data {
            Data::Null(n) => Series::new_null(name, *n),
            Data::Bool(v) => Series::new(name, v),
            Data::Int64(v) => Series::new(name, v),
            Data::Float64(v) => Series::new(name, v),
            Data::String(v) => Series::new(name, v),
            Data::Date(v) => Series::new(name, v).cast(&DataType::Date)?,
            Data::Datetime(v) => Series::new(name, v).cast(&DataType::Datetime(TimeUnit::Milliseconds, None))?,
            Data::Time(v) => Series::new(name, v).cast(&DataType::Time)?,
            Data::Duration(v) => Series::new(name, v).cast(&DataType::Duration(TimeUnit::Milliseconds))?,
        };
        columns.push(s.into());
    }
    DataFrame::new(f.height, columns)
}

fn ints(s: &Series) -> Result<Vec<Option<i64>>, String> {
    Ok(s.cast(&DataType::Int64).map_err(|e| e.to_string())?.i64().map_err(|e| e.to_string())?.iter().collect())
}

/// A Polars column as the engine's values: integers as Int64, other numbers as
/// Float64, categories as text, date times in milliseconds without a time zone,
/// durations in milliseconds.
pub fn data_of(s: &Series) -> Result<Data, String> {
    let fail = |e: PolarsError| e.to_string();
    Ok(match s.dtype() {
        DataType::Null => Data::Null(s.len()),
        DataType::Boolean => Data::Bool(s.bool().map_err(fail)?.iter().collect()),
        DataType::String => Data::String(s.str().map_err(fail)?.iter().map(|v| v.map(str::to_string)).collect()),
        DataType::Categorical(_, _) | DataType::Enum(_, _) => Data::String(s.cast(&DataType::String).map_err(fail)?.str().map_err(fail)?.iter().map(|v| v.map(str::to_string)).collect()),
        DataType::Date => Data::Date(ints(&s.to_physical_repr())?.into_iter().map(|v| v.map(|d| d as i32)).collect()),
        DataType::Datetime(_, _) => Data::Datetime(ints(&s.cast(&DataType::Datetime(TimeUnit::Milliseconds, None)).map_err(fail)?.to_physical_repr())?),
        DataType::Time => Data::Time(ints(&s.to_physical_repr())?),
        DataType::Duration(_) => Data::Duration(ints(&s.cast(&DataType::Duration(TimeUnit::Milliseconds)).map_err(fail)?.to_physical_repr())?),
        d if d.is_integer() => Data::Int64(ints(s)?),
        d if d.is_primitive_numeric() || d.is_decimal() => Data::Float64(s.cast(&DataType::Float64).map_err(fail)?.f64().map_err(fail)?.iter().collect()),
        other => return Err(format!("column {:?} has the type {other}, which a cell cannot hold", s.name())),
    })
}

/// A Polars frame as a frame of the engine.
pub fn from_polars(df: &DataFrame) -> Result<Frame, String> {
    let columns = df.columns().iter().map(|c| Ok(XColumn::new(c.name().to_string(), data_of(c.as_materialized_series())?))).collect::<Result<Vec<_>, String>>()?;
    Frame::new(df.height(), columns)
}
