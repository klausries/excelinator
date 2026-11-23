//! JavaScript/TypeScript binding (WebAssembly): a thin wrapper over `xl-compat`.

use wasm_bindgen::prelude::*;
use xl_compat::{Font, Formula, Label, Value, Workbook, XFStyle};

fn err(e: xl_compat::Error) -> JsError {
    JsError::new(&e.to_string())
}

/// Builds a workbook cell by cell.
#[wasm_bindgen]
pub struct Writer {
    wb: Workbook,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Writer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Writer {
        Writer { wb: Workbook::new() }
    }

    /// Add a worksheet; returns its index.
    #[wasm_bindgen(js_name = addSheet)]
    pub fn add_sheet(&mut self, name: &str) -> usize {
        self.wb.add_sheet(name)
    }

    /// Register a style; returns its index.
    #[wasm_bindgen(js_name = addStyle)]
    pub fn add_style(&mut self, num_format: &str, bold: bool, italic: bool) -> u32 {
        self.wb.add_style(&XFStyle { num_format_str: num_format.to_string(), font: Font { bold, italic } })
    }

    /// kind: 0 blank, 1 number, 2 text, 3 bool, 4 error. `formula` without `=`.
    #[allow(clippy::too_many_arguments)]
    pub fn write(&mut self, sheet: usize, row: u32, col: u32, kind: u8, num: f64, text: Option<String>, formula: Option<String>, style: u32) -> Result<(), JsError> {
        if sheet >= self.wb.sheet_count() {
            return Err(JsError::new(&format!("no sheet {sheet}")));
        }
        let value = match kind {
            0 => Value::Blank,
            1 => Value::Number(num),
            2 => Value::Text(text.unwrap_or_default()),
            3 => Value::Bool(num != 0.0),
            4 => Value::Error(text.unwrap_or_default()),
            k => return Err(JsError::new(&format!("unknown cell kind {k}"))),
        };
        let label = match (formula, value) {
            (Some(f), v) => Label::FormulaWithValue(Formula::new(&f), v),
            (None, Value::Blank) => Label::Blank,
            (None, Value::Number(n)) => Label::Number(n),
            (None, Value::Text(s)) => Label::Text(s),
            (None, Value::Bool(b)) => Label::Bool(b),
            (None, Value::Error(e)) => Label::Error(e),
        };
        self.wb.write_styled(sheet, row, col, label, style);
        Ok(())
    }

    pub fn merge(&mut self, sheet: usize, r1: u32, r2: u32, c1: u32, c2: u32) {
        self.wb.merge(sheet, r1, r2, c1, c2)
    }

    /// Column width in 1/256 of a character.
    #[wasm_bindgen(js_name = setColWidth)]
    pub fn set_col_width(&mut self, sheet: usize, col: u32, width: u32) {
        self.wb.set_col_width(sheet, col, width)
    }

    #[wasm_bindgen(js_name = setDate1904)]
    pub fn set_date1904(&mut self, on: bool) {
        self.wb.set_date1904(on)
    }

    /// The xlsx file.
    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Result<Vec<u8>, JsError> {
        self.wb.to_bytes().map_err(err)
    }
}

/// The cells of a workbook as equally long columns (one entry per cell).
#[wasm_bindgen]
pub struct Table {
    t: xl_compat::Table,
}

#[wasm_bindgen]
impl Table {
    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.t.len()
    }
    #[wasm_bindgen(getter, js_name = sheetNames)]
    pub fn sheet_names(&self) -> Vec<String> {
        self.t.sheet_names.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn date1904(&self) -> bool {
        self.t.date1904
    }
    #[wasm_bindgen(getter)]
    pub fn sheet(&self) -> Vec<u32> {
        self.t.sheet.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn row(&self) -> Vec<u32> {
        self.t.row.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn col(&self) -> Vec<u32> {
        self.t.col.clone()
    }
    /// 0 blank, 1 number, 2 text, 3 bool, 4 error
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> Vec<u8> {
        self.t.kind.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn num(&self) -> Vec<f64> {
        self.t.num.clone()
    }
    /// 1 where the number is a date serial
    #[wasm_bindgen(getter, js_name = isDate)]
    pub fn is_date(&self) -> Vec<u8> {
        self.t.is_date.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn style(&self) -> Vec<u32> {
        self.t.style.clone()
    }
    #[wasm_bindgen(getter, js_name = numFormat)]
    pub fn num_format(&self) -> Vec<String> {
        self.t.num_format.clone()
    }
    /// Text of text and error cells, `undefined` otherwise.
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> Vec<JsValue> {
        (0..self.t.len()).map(|i| if matches!(self.t.kind[i], 2 | 4) { JsValue::from_str(self.t.text(i)) } else { JsValue::UNDEFINED }).collect()
    }
    /// Formula text without `=`, `undefined` for cells without a formula.
    #[wasm_bindgen(getter)]
    pub fn formula(&self) -> Vec<JsValue> {
        (0..self.t.len()).map(|i| if self.t.formula(i).is_empty() { JsValue::UNDEFINED } else { JsValue::from_str(self.t.formula(i)) }).collect()
    }
}

/// Read a workbook.
#[wasm_bindgen]
pub fn read(data: &[u8]) -> Result<Table, JsError> {
    Ok(Table { t: xl_compat::read_table(data.to_vec()).map_err(err)? })
}

/// Serial number of a date and time in the 1900 date system; `undefined` if not representable.
#[wasm_bindgen(js_name = dateSerial)]
pub fn date_serial(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32, millisecond: u32) -> Option<f64> {
    xl_compat::date_serial(year, month, day, hour, minute, second, millisecond * 1000)
}
