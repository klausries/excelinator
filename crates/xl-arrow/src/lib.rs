//! The frames of `xl-engine` ([`Frame`], and the executor's [`Table`]) as streams of the
//! Arrow C stream interface, and such streams as frames. This is how data leaves Rust
//! for Polars, pyarrow or DuckDB, whatever their version, and comes back.
//!
//! Out, the types every reader knows: `Int64`, `Float64`, `Boolean`, large strings,
//! `Date32`, timestamps in milliseconds, `Time64` in nanoseconds, durations in
//! milliseconds. In, what a cell can hold: integers become `Int64`, other numbers
//! `Float64`, any kind of string or dictionary of strings text, timestamps milliseconds
//! (a time zone is dropped, the instant kept), times nanoseconds, durations
//! milliseconds; other types are refused with the column's name.

use arrow_array::cast::AsArray;
use arrow_array::ffi_stream::{FFI_ArrowArrayStream, RecordBatchStreamReader};
use arrow_array::types::*;
use arrow_array::{Array, ArrayRef, BooleanArray, Date32Array, DurationMillisecondArray, Float64Array, Int64Array, LargeStringArray, NullArray, RecordBatch, RecordBatchIterator, RecordBatchReader, Time64NanosecondArray, TimestampMillisecondArray};
use arrow_schema::{ArrowError, DataType, Field, Schema, TimeUnit};
use std::sync::Arc;
use xl_engine::data::{Column, Data, Frame};
use xl_engine::ir::{Table, Values};

pub use arrow_array::ffi_stream::FFI_ArrowArrayStream as ArrowArrayStream;

fn array(d: &Data) -> ArrayRef {
    match d {
        Data::Null(n) => Arc::new(NullArray::new(*n)),
        Data::Bool(v) => Arc::new(BooleanArray::from(v.clone())),
        Data::Int64(v) => Arc::new(Int64Array::from(v.clone())),
        Data::Float64(v) => Arc::new(Float64Array::from(v.clone())),
        Data::String(v) => Arc::new(LargeStringArray::from(v.iter().map(|s| s.as_deref()).collect::<Vec<_>>())),
        Data::Date(v) => Arc::new(Date32Array::from(v.clone())),
        Data::Datetime(v) => Arc::new(TimestampMillisecondArray::from(v.clone())),
        Data::Time(v) => Arc::new(Time64NanosecondArray::from(v.clone())),
        Data::Duration(v) => Arc::new(DurationMillisecondArray::from(v.clone())),
    }
}

/// Columns as a schema and its record batches (one, or none for a frame without columns).
fn batches(names: Vec<String>, arrays: Vec<ArrayRef>, height: usize) -> (Arc<Schema>, Vec<Result<RecordBatch, ArrowError>>) {
    let fields: Vec<Field> = names.iter().zip(&arrays).map(|(n, a)| Field::new(n.as_str(), a.data_type().clone(), true)).collect();
    let schema = Arc::new(Schema::new(fields));
    let batches = if arrays.is_empty() {
        Vec::new()
    } else {
        vec![RecordBatch::try_new_with_options(schema.clone(), arrays, &arrow_array::RecordBatchOptions::new().with_row_count(Some(height)))]
    };
    (schema, batches)
}

fn stream(names: Vec<String>, arrays: Vec<ArrayRef>, height: usize) -> FFI_ArrowArrayStream {
    let (schema, batches) = batches(names, arrays, height);
    FFI_ArrowArrayStream::new(Box::new(RecordBatchIterator::new(batches, schema)))
}

fn frame_columns(f: &Frame) -> (Vec<String>, Vec<ArrayRef>, usize) {
    (f.columns.iter().map(|c| c.name.clone()).collect(), f.columns.iter().map(|c| array(&c.data)).collect(), f.height)
}

fn table_columns(t: &Table) -> (Vec<String>, Vec<ArrayRef>, usize) {
    (t.columns.iter().map(|(n, _)| n.clone()).collect(), t.columns.iter().map(|(_, v)| values_array(v)).collect(), t.height)
}

/// A frame as an Arrow C stream (one record batch).
pub fn frame_to_stream(f: &Frame) -> FFI_ArrowArrayStream {
    let (names, arrays, height) = frame_columns(f);
    stream(names, arrays, height)
}

fn values_array(v: &Values) -> ArrayRef {
    match v {
        Values::F64(x) => Arc::new(Float64Array::from(x.clone())),
        Values::OptF64(x) => Arc::new(Float64Array::from(x.clone())),
        Values::I64(x) => Arc::new(Int64Array::from(x.clone())),
        Values::Str(x) => Arc::new(LargeStringArray::from(x.iter().map(|s| Some(s.as_str())).collect::<Vec<_>>())),
        Values::OptStr(x) => Arc::new(LargeStringArray::from(x.iter().map(|s| s.as_deref()).collect::<Vec<_>>())),
        Values::OptBool(x) => Arc::new(BooleanArray::from(x.clone())),
    }
}

/// An executor's table as an Arrow C stream.
pub fn table_to_stream(t: &Table) -> FFI_ArrowArrayStream {
    let (names, arrays, height) = table_columns(t);
    stream(names, arrays, height)
}

type Read = (Vec<Field>, Vec<ArrayRef>, usize);

/// The record batches of a stream, put together: (field names, one array per column).
///
/// # Safety
/// `stream` must point to a valid Arrow C stream; it is consumed.
unsafe fn read(stream: *mut FFI_ArrowArrayStream) -> Result<Read, String> {
    collect(RecordBatchStreamReader::from_raw(stream).map_err(|e| e.to_string())?)
}

/// The record batches of a reader, put together.
fn collect(reader: impl RecordBatchReader) -> Result<Read, String> {
    let schema = reader.schema();
    let mut chunks: Vec<Vec<ArrayRef>> = vec![Vec::new(); schema.fields().len()];
    let mut rows = 0;
    for batch in reader {
        let batch = batch.map_err(|e| e.to_string())?;
        rows += batch.num_rows();
        for (k, c) in batch.columns().iter().enumerate() {
            chunks[k].push(c.clone());
        }
    }
    let mut arrays = Vec::with_capacity(chunks.len());
    for (field, parts) in schema.fields().iter().zip(chunks) {
        arrays.push(if parts.is_empty() {
            arrow_array::new_empty_array(field.data_type())
        } else {
            let refs: Vec<&dyn Array> = parts.iter().map(|a| a.as_ref()).collect();
            arrow_cast::cast::cast(&arrow_select_concat(&refs)?, field.data_type()).map_err(|e| e.to_string())?
        });
    }
    Ok((schema.fields().iter().map(|f| f.as_ref().clone()).collect(), arrays, rows))
}

/// Arrays of one type as one array.
fn arrow_select_concat(parts: &[&dyn Array]) -> Result<ArrayRef, String> {
    if parts.len() == 1 {
        return Ok(arrow_array::make_array(parts[0].to_data()));
    }
    // Through the generic builder of array data: concatenation without arrow-select.
    let datas: Vec<arrow_data::ArrayData> = parts.iter().map(|a| a.to_data()).collect();
    let refs: Vec<&arrow_data::ArrayData> = datas.iter().collect();
    let total: usize = datas.iter().map(|d| d.len()).sum();
    let mut m = arrow_data::transform::MutableArrayData::new(refs, true, total);
    for (k, d) in datas.iter().enumerate() {
        m.try_extend(k, 0, d.len()).map_err(|e| e.to_string())?;
    }
    Ok(arrow_array::make_array(m.freeze()))
}

fn to(a: &ArrayRef, t: &DataType) -> Result<ArrayRef, String> {
    arrow_cast::cast::cast(a, t).map_err(|e| e.to_string())
}

/// Physical values of a temporal array in another unit (truncated toward zero, as Polars casts).
fn rescale(v: impl Iterator<Item = Option<i64>>, from: i64, to_per: i64) -> Vec<Option<i64>> {
    v.map(|x| x.map(|x| if from >= to_per { x * (from / to_per) } else { x / (to_per / from) })).collect()
}

fn per_second(u: &TimeUnit) -> i64 {
    match u {
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 1000,
        TimeUnit::Microsecond => 1_000_000,
        TimeUnit::Nanosecond => 1_000_000_000,
    }
}

/// An Arrow array as the engine's values.
pub fn data_of(name: &str, a: &ArrayRef) -> Result<Data, String> {
    let n = a.len();
    Ok(match a.data_type() {
        DataType::Null => Data::Null(n),
        DataType::Boolean => Data::Bool(a.as_boolean().iter().collect()),
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 | DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            Data::Int64(to(a, &DataType::Int64)?.as_primitive::<Int64Type>().iter().collect())
        }
        DataType::Float16 | DataType::Float32 | DataType::Float64 | DataType::Decimal32(_, _) | DataType::Decimal64(_, _) | DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => {
            Data::Float64(to(a, &DataType::Float64)?.as_primitive::<Float64Type>().iter().collect())
        }
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Dictionary(_, _) => {
            Data::String(to(a, &DataType::LargeUtf8).map_err(|_| format!("column {name:?} has the type {}, which a cell cannot hold", a.data_type()))?.as_string::<i64>().iter().map(|s| s.map(str::to_string)).collect())
        }
        DataType::Date32 => Data::Date(a.as_primitive::<Date32Type>().iter().collect()),
        DataType::Date64 => Data::Date(a.as_primitive::<Date64Type>().iter().map(|v| v.map(|ms| ms.div_euclid(86_400_000) as i32)).collect()),
        DataType::Timestamp(u, _) => {
            let raw: Vec<Option<i64>> = match u {
                TimeUnit::Second => a.as_primitive::<TimestampSecondType>().iter().collect(),
                TimeUnit::Millisecond => a.as_primitive::<TimestampMillisecondType>().iter().collect(),
                TimeUnit::Microsecond => a.as_primitive::<TimestampMicrosecondType>().iter().collect(),
                TimeUnit::Nanosecond => a.as_primitive::<TimestampNanosecondType>().iter().collect(),
            };
            Data::Datetime(rescale(raw.into_iter(), per_second(u), 1000))
        }
        DataType::Time32(u) => {
            let raw: Vec<Option<i64>> = match u {
                TimeUnit::Second => a.as_primitive::<Time32SecondType>().iter().map(|v| v.map(i64::from)).collect(),
                _ => a.as_primitive::<Time32MillisecondType>().iter().map(|v| v.map(i64::from)).collect(),
            };
            Data::Time(rescale(raw.into_iter(), per_second(u), 1_000_000_000))
        }
        DataType::Time64(u) => {
            let raw: Vec<Option<i64>> = match u {
                TimeUnit::Microsecond => a.as_primitive::<Time64MicrosecondType>().iter().collect(),
                _ => a.as_primitive::<Time64NanosecondType>().iter().collect(),
            };
            Data::Time(rescale(raw.into_iter(), per_second(u), 1_000_000_000))
        }
        DataType::Duration(u) => {
            let raw: Vec<Option<i64>> = match u {
                TimeUnit::Second => a.as_primitive::<DurationSecondType>().iter().collect(),
                TimeUnit::Millisecond => a.as_primitive::<DurationMillisecondType>().iter().collect(),
                TimeUnit::Microsecond => a.as_primitive::<DurationMicrosecondType>().iter().collect(),
                TimeUnit::Nanosecond => a.as_primitive::<DurationNanosecondType>().iter().collect(),
            };
            Data::Duration(rescale(raw.into_iter(), per_second(u), 1000))
        }
        other => return Err(format!("column {name:?} has the type {other}, which a cell cannot hold")),
    })
}

/// A frame from an Arrow C stream (what Polars, pyarrow or DuckDB give through
/// `__arrow_c_stream__`).
///
/// # Safety
/// `stream` must point to a valid Arrow C stream; it is consumed.
pub unsafe fn frame_from_stream(stream: *mut FFI_ArrowArrayStream) -> Result<Frame, String> {
    frame_of(read(stream)?)
}

fn frame_of((fields, arrays, rows): Read) -> Result<Frame, String> {
    let columns = fields.iter().zip(&arrays).map(|(f, a)| Ok(Column::new(f.name().clone(), data_of(f.name(), a)?))).collect::<Result<Vec<_>, String>>()?;
    Frame::new(rows, columns)
}

/// An executor's table from an Arrow C stream: numbers, text and booleans as they are,
/// other types as numbers (as the built-in executor reads them).
///
/// # Safety
/// `stream` must point to a valid Arrow C stream; it is consumed.
pub unsafe fn table_from_stream(stream: *mut FFI_ArrowArrayStream) -> Result<Table, String> {
    table_of(read(stream)?)
}

fn table_of((fields, arrays, rows): Read) -> Result<Table, String> {
    let mut t = Table::new(rows);
    for (f, a) in fields.iter().zip(&arrays) {
        let v = match a.data_type() {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Values::OptStr(to(a, &DataType::LargeUtf8)?.as_string::<i64>().iter().map(|s| s.map(str::to_string)).collect()),
            DataType::Boolean => Values::OptBool(a.as_boolean().iter().collect()),
            _ => Values::OptF64(to(a, &DataType::Float64)?.as_primitive::<Float64Type>().iter().collect()),
        };
        t.push(f.name().as_str(), v);
    }
    Ok(t)
}

/// Frames and tables as bytes of the Arrow IPC stream format: the way they cross a
/// border that pointers do not (between WebAssembly and its host).
#[cfg(feature = "ipc")]
pub mod ipc {
    use super::*;
    use arrow_ipc::reader::StreamReader;
    use arrow_ipc::writer::StreamWriter;

    fn write((names, arrays, height): (Vec<String>, Vec<ArrayRef>, usize)) -> Result<Vec<u8>, String> {
        let (schema, batches) = batches(names, arrays, height);
        let mut w = StreamWriter::try_new(Vec::new(), &schema).map_err(|e| e.to_string())?;
        for b in batches {
            w.write(&b.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        w.into_inner().map_err(|e| e.to_string())
    }

    fn reader(bytes: &[u8]) -> Result<StreamReader<std::io::Cursor<&[u8]>>, String> {
        StreamReader::try_new(std::io::Cursor::new(bytes), None).map_err(|e| format!("not an Arrow IPC stream: {e}"))
    }

    pub fn frame_to_ipc(f: &Frame) -> Result<Vec<u8>, String> {
        write(frame_columns(f))
    }

    pub fn table_to_ipc(t: &Table) -> Result<Vec<u8>, String> {
        write(table_columns(t))
    }

    pub fn frame_from_ipc(bytes: &[u8]) -> Result<Frame, String> {
        frame_of(collect(reader(bytes)?)?)
    }

    pub fn table_from_ipc(bytes: &[u8]) -> Result<Table, String> {
        table_of(collect(reader(bytes)?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_goes_out_and_comes_back() {
        let f = Frame::of(vec![
            Column::new("n", Data::Null(2)),
            Column::new("b", Data::Bool(vec![Some(true), None])),
            Column::new("i", Data::Int64(vec![Some(1), Some(-2)])),
            Column::new("x", Data::Float64(vec![Some(0.5), None])),
            Column::new("s", Data::String(vec![Some("a".into()), None])),
            Column::new("d", Data::Date(vec![Some(20_000), None])),
            Column::new("dt", Data::Datetime(vec![Some(1_700_000_000_000), None])),
            Column::new("t", Data::Time(vec![Some(45_000_000_000_000), None])),
            Column::new("du", Data::Duration(vec![Some(1500), None])),
        ])
        .unwrap();
        let mut s = frame_to_stream(&f);
        let back = unsafe { frame_from_stream(&mut s) }.unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn a_table_goes_out_and_comes_back() {
        let mut t = Table::new(2);
        t.push("x", vec![Some(1.5), None]);
        t.push("s", vec![Some("a"), None]);
        t.push("g", vec![Some(true), None]);
        let mut s = table_to_stream(&t);
        let back = unsafe { table_from_stream(&mut s) }.unwrap();
        assert_eq!(back.column("x"), Some(&Values::OptF64(vec![Some(1.5), None])));
        assert_eq!(back.column("s"), Some(&Values::OptStr(vec![Some("a".into()), None])));
        assert_eq!(back.column("g"), Some(&Values::OptBool(vec![Some(true), None])));
    }

    #[cfg(feature = "ipc")]
    #[test]
    fn a_frame_goes_through_ipc_and_comes_back() {
        let f = Frame::of(vec![Column::new("i", Data::Int64(vec![Some(1), None])), Column::new("s", Data::String(vec![Some("ä".into()), None])), Column::new("d", Data::Date(vec![Some(3), None]))]).unwrap();
        assert_eq!(ipc::frame_from_ipc(&ipc::frame_to_ipc(&f).unwrap()).unwrap(), f);
        let mut t = Table::new(1);
        t.push("x", vec![Some(2.5)]);
        assert_eq!(ipc::table_from_ipc(&ipc::table_to_ipc(&t).unwrap()).unwrap().column("x"), Some(&Values::OptF64(vec![Some(2.5)])));
    }
}
