//! C interface: a thin wrapper over `xl-compat`. Header: `include/xlcells.h`.
//!
//! Strings are passed as pointer and length in UTF-8 (a null pointer means "none").
//! Every object returned by a `*_new`, `xl_read` or `*_to_bytes` call must be released
//! with its `*_free` function. Column pointers of a table stay valid until the table
//! is freed.

use std::panic::{catch_unwind, AssertUnwindSafe};
use xl_compat::{Font, Formula, Label, Table, Value, Workbook, XFStyle};

pub struct XlWriter {
    wb: Workbook,
}

pub struct XlBytes {
    data: Vec<u8>,
}

pub struct XlTable {
    t: Table,
}

/// # Safety
/// `ptr` must be null or point to `len` readable bytes.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).ok()
}

/// Store an error message in `*error` (if `error` is not null) as an [`XlBytes`].
unsafe fn set_error(error: *mut *mut XlBytes, message: String) {
    if !error.is_null() {
        *error = Box::into_raw(Box::new(XlBytes { data: message.into_bytes() }));
    }
}

/// Allocate `len` bytes for passing data in. Release with `xl_dealloc`.
#[no_mangle]
pub extern "C" fn xl_alloc(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len.max(1)].into_boxed_slice();
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `xl_alloc(len)` with the same `len`.
#[no_mangle]
pub unsafe extern "C" fn xl_dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len.max(1))));
    }
}

#[no_mangle]
pub extern "C" fn xl_writer_new() -> *mut XlWriter {
    Box::into_raw(Box::new(XlWriter { wb: Workbook::new() }))
}

/// # Safety
/// `w` must come from `xl_writer_new` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_free(w: *mut XlWriter) {
    if !w.is_null() {
        drop(Box::from_raw(w));
    }
}

/// Add a worksheet. Returns its index, or -1 if the name is not valid UTF-8.
/// # Safety
/// `w` must be a live writer; `name` must point to `name_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_add_sheet(w: *mut XlWriter, name: *const u8, name_len: usize) -> i32 {
    match text(name, name_len) {
        Some(n) => (*w).wb.add_sheet(n) as i32,
        None => -1,
    }
}

/// Register a style. Returns its index (0 is the default style).
/// # Safety
/// `w` must be a live writer; `num_format` null or `num_format_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_add_style(w: *mut XlWriter, num_format: *const u8, num_format_len: usize, bold: bool, italic: bool) -> u32 {
    let fmt = text(num_format, num_format_len).unwrap_or("General");
    (*w).wb.add_style(&XFStyle { num_format_str: fmt.to_string(), font: Font { bold, italic } })
}

/// Write one cell. kind: 0 blank, 1 number, 2 text, 3 bool, 4 error. `formula`
/// without `=`. Returns 0, or -1 for an unknown sheet or kind or invalid UTF-8.
/// # Safety
/// `w` must be a live writer; string pointers null or valid for their lengths.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn xl_writer_write(w: *mut XlWriter, sheet: u32, row: u32, col: u32, kind: u8, num: f64, txt: *const u8, txt_len: usize, formula: *const u8, formula_len: usize, style: u32) -> i32 {
    let w = &mut *w;
    if sheet as usize >= w.wb.sheet_count() || (!txt.is_null() && text(txt, txt_len).is_none()) || (!formula.is_null() && text(formula, formula_len).is_none()) {
        return -1;
    }
    let t = text(txt, txt_len).unwrap_or("").to_string();
    let value = match kind {
        0 => Value::Blank,
        1 => Value::Number(num),
        2 => Value::Text(t),
        3 => Value::Bool(num != 0.0),
        4 => Value::Error(t),
        _ => return -1,
    };
    let label = match (text(formula, formula_len), value) {
        (Some(f), v) => Label::FormulaWithValue(Formula::new(f), v),
        (None, Value::Blank) => Label::Blank,
        (None, Value::Number(n)) => Label::Number(n),
        (None, Value::Text(s)) => Label::Text(s),
        (None, Value::Bool(b)) => Label::Bool(b),
        (None, Value::Error(e)) => Label::Error(e),
    };
    w.wb.write_styled(sheet as usize, row, col, label, style);
    0
}

/// # Safety
/// `w` must be a live writer.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_merge(w: *mut XlWriter, sheet: u32, r1: u32, r2: u32, c1: u32, c2: u32) {
    (*w).wb.merge(sheet as usize, r1, r2, c1, c2)
}

/// Column width in 1/256 of a character.
/// # Safety
/// `w` must be a live writer.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_set_col_width(w: *mut XlWriter, sheet: u32, col: u32, width: u32) {
    (*w).wb.set_col_width(sheet as usize, col, width)
}

/// # Safety
/// `w` must be a live writer.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_set_date1904(w: *mut XlWriter, on: bool) {
    (*w).wb.set_date1904(on)
}

/// The xlsx file. On failure returns null and stores the message in `*error`.
/// # Safety
/// `w` must be a live writer; `error` null or a valid place for a pointer.
#[no_mangle]
pub unsafe extern "C" fn xl_writer_to_bytes(w: *const XlWriter, error: *mut *mut XlBytes) -> *mut XlBytes {
    let w = &*w;
    match catch_unwind(AssertUnwindSafe(|| w.wb.to_bytes())) {
        Ok(Ok(data)) => Box::into_raw(Box::new(XlBytes { data })),
        Ok(Err(e)) => {
            set_error(error, e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_error(error, "internal error".into());
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `b` must be a live bytes object.
#[no_mangle]
pub unsafe extern "C" fn xl_bytes_data(b: *const XlBytes) -> *const u8 {
    (*b).data.as_ptr()
}

/// # Safety
/// `b` must be a live bytes object.
#[no_mangle]
pub unsafe extern "C" fn xl_bytes_len(b: *const XlBytes) -> usize {
    (*b).data.len()
}

/// # Safety
/// `b` must come from this library and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn xl_bytes_free(b: *mut XlBytes) {
    if !b.is_null() {
        drop(Box::from_raw(b));
    }
}

/// Read a workbook. On failure returns null and stores the message in `*error`.
/// # Safety
/// `data` must point to `len` bytes; `error` null or a valid place for a pointer.
#[no_mangle]
pub unsafe extern "C" fn xl_read(data: *const u8, len: usize, error: *mut *mut XlBytes) -> *mut XlTable {
    let bytes = std::slice::from_raw_parts(data, len).to_vec();
    match catch_unwind(|| xl_compat::read_table(bytes)) {
        Ok(Ok(t)) => Box::into_raw(Box::new(XlTable { t })),
        Ok(Err(e)) => {
            set_error(error, e.to_string());
            std::ptr::null_mut()
        }
        Err(_) => {
            set_error(error, "internal error".into());
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `t` must come from `xl_read` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn xl_table_free(t: *mut XlTable) {
    if !t.is_null() {
        drop(Box::from_raw(t));
    }
}

/// Number of cells.
/// # Safety
/// `t` must be a live table (also for all `xl_table_*` functions below).
#[no_mangle]
pub unsafe extern "C" fn xl_table_len(t: *const XlTable) -> usize {
    (*t).t.len()
}

#[no_mangle]
pub unsafe extern "C" fn xl_table_date1904(t: *const XlTable) -> bool {
    (*t).t.date1904
}

macro_rules! column {
    ($name:ident, $field:ident, $ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(t: *const XlTable) -> *const $ty {
            (*t).t.$field.as_ptr()
        }
    };
}

column!(xl_table_sheet, sheet, u32);
column!(xl_table_row, row, u32);
column!(xl_table_col, col, u32);
column!(xl_table_kind, kind, u8);
column!(xl_table_num, num, f64);
column!(xl_table_is_date, is_date, u8);
column!(xl_table_style, style, u32);
column!(xl_table_text_offsets, text_offsets, u32);
column!(xl_table_text_data, text_data, u8);
column!(xl_table_formula_offsets, formula_offsets, u32);
column!(xl_table_formula_data, formula_data, u8);

#[no_mangle]
pub unsafe extern "C" fn xl_table_sheet_count(t: *const XlTable) -> usize {
    (*t).t.sheet_names.len()
}

/// Name of sheet `i` (UTF-8, not terminated); its length goes to `*len`.
#[no_mangle]
pub unsafe extern "C" fn xl_table_sheet_name(t: *const XlTable, i: usize, len: *mut usize) -> *const u8 {
    match (&(*t).t.sheet_names).get(i) {
        Some(s) => {
            *len = s.len();
            s.as_ptr()
        }
        None => {
            *len = 0;
            std::ptr::null()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn xl_table_num_format_count(t: *const XlTable) -> usize {
    (*t).t.num_format.len()
}

/// Number format `i` (the `style` column indexes these); its length goes to `*len`.
#[no_mangle]
pub unsafe extern "C" fn xl_table_num_format(t: *const XlTable, i: usize, len: *mut usize) -> *const u8 {
    match (&(*t).t.num_format).get(i) {
        Some(s) => {
            *len = s.len();
            s.as_ptr()
        }
        None => {
            *len = 0;
            std::ptr::null()
        }
    }
}

/// Serial number of a date and time in the 1900 date system; NaN if not representable.
#[no_mangle]
pub extern "C" fn xl_date_serial(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32, microsecond: u32) -> f64 {
    xl_compat::date_serial(year, month, day, hour, minute, second, microsecond).unwrap_or(f64::NAN)
}
