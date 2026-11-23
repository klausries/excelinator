//! Python extension module: a thin wrapper over `xl-compat`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use xl_compat::{Font, Formula, Label, Value, Workbook, XFStyle};

fn err(e: xl_compat::Error) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Builds a workbook cell by cell.
#[pyclass]
struct Writer {
    wb: Workbook,
}

#[pymethods]
impl Writer {
    #[new]
    fn new() -> Self {
        Writer { wb: Workbook::new() }
    }

    fn add_sheet(&mut self, name: &str) -> usize {
        self.wb.add_sheet(name)
    }

    #[pyo3(signature = (num_format="General", bold=false, italic=false))]
    fn add_style(&mut self, num_format: &str, bold: bool, italic: bool) -> u32 {
        self.wb.add_style(&XFStyle { num_format_str: num_format.to_string(), font: Font { bold, italic } })
    }

    /// kind: 0 blank, 1 number, 2 text, 3 bool, 4 error. `formula` without `=`.
    /// `array_ref` makes the formula an array formula over that range; `float_text`
    /// keeps a whole number spelled as `1.0`.
    /// `rich`: for a text cell, its formatted runs as string item XML.
    /// `iso`: write a number as an ISO 8601 date cell (1 date, 2 date and time, 3 time).
    #[pyo3(signature = (sheet, row, col, kind, num=0.0, text=None, formula=None, style=0, array_ref=None, float_text=false, rich=None, iso=0))]
    #[allow(clippy::too_many_arguments)]
    fn write(&mut self, sheet: usize, row: u32, col: u32, kind: u8, num: f64, text: Option<String>, formula: Option<&str>, style: u32, array_ref: Option<&str>, float_text: bool, rich: Option<&str>, iso: u8) -> PyResult<()> {
        if sheet >= self.wb.sheet_count() {
            return Err(PyValueError::new_err(format!("no sheet {sheet}")));
        }
        let value = match kind {
            0 => Value::Blank,
            1 => Value::Number(num),
            2 => Value::Text(text.unwrap_or_default()),
            3 => Value::Bool(num != 0.0),
            4 => Value::Error(text.unwrap_or_default()),
            k => return Err(PyValueError::new_err(format!("unknown cell kind {k}"))),
        };
        let label = match (formula, value) {
            (Some(f), v) => Label::FormulaWithValue(Formula::new(f), v),
            (None, Value::Blank) => Label::Blank,
            (None, Value::Number(n)) => Label::Number(n),
            (None, Value::Text(s)) => Label::Text(s),
            (None, Value::Bool(b)) => Label::Bool(b),
            (None, Value::Error(e)) => Label::Error(e),
        };
        self.wb.write_styled(sheet, row, col, label, style);
        if array_ref.is_some() || float_text {
            self.wb.mark_last(array_ref, float_text);
        }
        if let Some(r) = rich {
            self.wb.mark_last_rich(r).map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        if iso != 0 {
            self.wb.mark_last_iso(iso).map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        Ok(())
    }

    fn merge(&mut self, sheet: usize, r1: u32, r2: u32, c1: u32, c2: u32) {
        self.wb.merge(sheet, r1, r2, c1, c2)
    }

    fn set_col_width(&mut self, sheet: usize, col: u32, width: u32) {
        self.wb.set_col_width(sheet, col, width)
    }

    /// Width of the columns first..=last in characters.
    fn set_col_width_chars(&mut self, sheet: usize, first: u32, last: u32, width: f64) {
        self.wb.set_col_width_chars(sheet, first, last, width)
    }

    /// Register a style from its `key=value` description (fields separated by U+001F).
    /// The named cell styles: (name, format as a style spec, builtin id or -1, hidden).
    fn set_named_styles(&mut self, styles: Vec<(String, String, i64, bool)>) {
        self.wb.cells_mut().named_styles = styles
            .into_iter()
            .map(|(name, spec, builtin, hidden)| xl_compat::cells::NamedStyle { name, style: xl_compat::cells::Style::from_spec(&spec), builtin_id: u32::try_from(builtin).ok(), hidden, ..Default::default() })
            .collect();
    }

    fn add_style_spec(&mut self, spec: &str) -> u32 {
        self.wb.add_style_spec(spec)
    }

    /// Replace the style list index for index (before writing a loaded workbook).
    fn set_styles(&mut self, specs: Vec<String>) {
        self.wb.set_styles(&specs)
    }

    fn set_sheet_kind(&mut self, sheet: usize, kind: u8) {
        self.wb.set_sheet_kind(sheet, kind)
    }

    /// `local_sheet` is the index of the sheet the name belongs to, or None for the workbook.
    #[pyo3(signature = (name, value, local_sheet=None, hidden=false))]
    fn add_defined_name(&mut self, name: String, value: String, local_sheet: Option<u32>, hidden: bool) {
        self.wb.cells_mut().defined_names.push(xl_compat::cells::DefinedName { hidden, name, local_sheet, value });
    }

    #[pyo3(signature = (sheet, cell, target=None, location=None, tooltip=None, display=None))]
    fn add_hyperlink(&mut self, sheet: usize, cell: String, target: Option<String>, location: Option<String>, tooltip: Option<String>, display: Option<String>) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.hyperlinks.push(xl_compat::cells::Hyperlink { cell, target, location, tooltip, display });
        }
    }

    fn set_auto_filter(&mut self, sheet: usize, range: Option<String>) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.auto_filter = range;
        }
    }

    /// `attrs` are the attributes of the rule as written in the file (type, operator,
    /// allowBlank, sqref, error, ...).
    #[pyo3(signature = (sheet, attrs, formula1=None, formula2=None))]
    fn add_validation(&mut self, sheet: usize, attrs: Vec<(String, String)>, formula1: Option<String>, formula2: Option<String>) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.validations.push(xl_compat::cells::Validation { attrs, formula1, formula2 });
        }
    }

    /// Conditional formatting of `sqref`; each rule is (attributes, content XML, format XML or None).
    #[allow(clippy::type_complexity)]
    fn add_cond_format(&mut self, sheet: usize, sqref: String, rules: Vec<(Vec<(String, String)>, String, Option<String>)>) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            let rules = rules.into_iter().map(|(attrs, inner, dxf)| xl_compat::cells::CondRule { attrs, inner, dxf }).collect();
            s.cond_formats.push(xl_compat::cells::CondFormat { sqref, rules });
        }
    }

    /// Place a picture (`image` = (extension, file content)) or a chart (its part as
    /// XML). `from` and `to` are (column, column offset, row, row offset), offsets and
    /// `size` (width, height) in EMU.
    #[pyo3(signature = (sheet, from, to=None, size=None, image=None, chart=None))]
    #[allow(clippy::type_complexity)]
    fn add_drawing(&mut self, sheet: usize, from: (u32, i64, u32, i64), to: Option<(u32, i64, u32, i64)>, size: Option<(i64, i64)>, image: Option<(String, Vec<u8>)>, chart: Option<String>) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.drawings.push(xl_compat::cells::Drawing {
                anchor: xl_compat::cells::Anchor { from, to, ext: size },
                image: image.map(|(ext, data)| (std::sync::Arc::from(data), ext)),
                chart: chart.map(|c| std::sync::Arc::from(c.as_str())),
            });
        }
    }

    fn add_comment(&mut self, sheet: usize, cell: String, author: String, text: String) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.comments.push(xl_compat::cells::Comment { cell, author, text });
        }
    }

    /// A child of `<workbook>` (`workbookProtection`, `bookViews`, `calcPr`, ...) as XML;
    /// empty removes it.
    fn set_workbook_xml(&mut self, name: String, xml: String) {
        let wb = self.wb.cells_mut();
        wb.extra.retain(|e| e.0 != name);
        if !xml.is_empty() {
            wb.extra.push((name, xml));
        }
    }

    /// A document property part (`core`, `app`, `custom`) as XML; empty removes it.
    fn set_properties_xml(&mut self, kind: String, xml: String) {
        let wb = self.wb.cells_mut();
        wb.properties.retain(|e| e.0 != kind);
        if !xml.is_empty() {
            wb.properties.push((kind, xml));
        }
    }

    /// A table (list object) as the XML of its table part.
    fn add_table(&mut self, sheet: usize, xml: String) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.tables.push(xml);
        }
    }

    fn set_sheet_origin(&mut self, sheet: usize, origin: u32) {
        self.wb.set_sheet_origin(sheet, origin)
    }

    fn set_row_height(&mut self, sheet: usize, row: u32, height: f64) {
        self.wb.set_row_height(sheet, row, height)
    }

    /// All attributes of a row (0-based), replacing what it had.
    #[pyo3(signature = (sheet, row, height=None, custom_height=false, hidden=false, outline_level=0, collapsed=false, style=None, custom_format=false, thick_top=false, thick_bot=false, phonetic=false))]
    #[allow(clippy::too_many_arguments)]
    fn set_row(&mut self, sheet: usize, row: u32, height: Option<f64>, custom_height: bool, hidden: bool, outline_level: u8, collapsed: bool, style: Option<u32>, custom_format: bool, thick_top: bool, thick_bot: bool, phonetic: bool) {
        self.wb.set_row(sheet, xl_compat::cells::Row { row, height, custom_height, hidden, outline_level, collapsed, style, custom_format, thick_top, thick_bot, phonetic })
    }

    /// All attributes of the columns first..=last (0-based), replacing what they had.
    #[pyo3(signature = (sheet, first, last, width=None, custom_width=false, hidden=false, best_fit=false, outline_level=0, collapsed=false, style=None, phonetic=false))]
    #[allow(clippy::too_many_arguments)]
    fn set_columns(&mut self, sheet: usize, first: u32, last: u32, width: Option<f64>, custom_width: bool, hidden: bool, best_fit: bool, outline_level: u8, collapsed: bool, style: Option<u32>, phonetic: bool) {
        self.wb.set_columns(sheet, xl_compat::cells::Column { first, last, width, custom_width, hidden, best_fit, outline_level, collapsed, style, phonetic })
    }

    /// A child of `<worksheet>` as XML (`pageSetup`, `sheetProtection`, `sheetViews`, …);
    /// replaces earlier ones of that name. Empty `xml` removes it.
    fn set_sheet_xml(&mut self, sheet: usize, name: String, xml: String) {
        if let Some(s) = self.wb.cells_mut().sheets.get_mut(sheet) {
            s.extra.retain(|e| e.0 != name);
            if !xml.is_empty() {
                s.extra.push((name, xml));
            }
        }
    }

    fn set_freeze(&mut self, sheet: usize, rows: u32, cols: u32) {
        self.wb.set_freeze(sheet, rows, cols)
    }

    /// The file written onto `original`; returns (bytes, kept_original).
    fn to_bytes_onto<'py>(&self, py: Python<'py>, original: &[u8]) -> PyResult<(Bound<'py, PyBytes>, bool)> {
        let (data, kept) = self.wb.to_bytes_onto(original.to_vec()).map_err(err)?;
        Ok((PyBytes::new(py, &data), kept))
    }

    fn set_date1904(&mut self, on: bool) {
        self.wb.set_date1904(on)
    }

    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.wb.to_bytes().map_err(err)?))
    }
}

/// Read a workbook into a dict of equally long columns (one entry per cell).
#[pyfunction]
fn read<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyDict>> {
    let t = xl_compat::read_table(data.to_vec()).map_err(err)?;
    let n = t.len();
    let text: Vec<Option<&str>> = (0..n).map(|i| matches!(t.kind[i], 2 | 4).then(|| t.text(i))).collect();
    let formula: Vec<Option<&str>> = (0..n).map(|i| Some(t.formula(i)).filter(|f| !f.is_empty())).collect();
    let d = PyDict::new(py);
    d.set_item("sheet_names", &t.sheet_names)?;
    d.set_item("date1904", t.date1904)?;
    d.set_item("sheet", &t.sheet)?;
    d.set_item("row", &t.row)?;
    d.set_item("col", &t.col)?;
    d.set_item("kind", pyo3::types::PyList::new(py, &t.kind)?)?;
    d.set_item("num", &t.num)?;
    d.set_item("is_date", pyo3::types::PyList::new(py, t.is_date.iter().map(|&b| b != 0))?)?;
    d.set_item("float_text", pyo3::types::PyList::new(py, t.float_text.iter().map(|&b| b != 0))?)?;
    d.set_item("text", text)?;
    d.set_item("formula", formula)?;
    d.set_item("style", &t.style)?;
    d.set_item("num_format", &t.num_format)?;
    d.set_item("style_bold", pyo3::types::PyList::new(py, t.bold.iter().map(|&b| b != 0))?)?;
    d.set_item("style_italic", pyo3::types::PyList::new(py, t.italic.iter().map(|&b| b != 0))?)?;
    d.set_item("rich", &t.rich)?;
    d.set_item("iso", &t.iso)?;
    d.set_item("array_ref", t.array_ref.iter().map(|a| Some(a.as_str()).filter(|a| !a.is_empty())).collect::<Vec<_>>())?;
    d.set_item("defined_names", &t.defined_names)?;
    d.set_item("hyperlinks", &t.hyperlinks)?;
    d.set_item("auto_filter", &t.auto_filter)?;
    d.set_item("validations", &t.validations)?;
    d.set_item("cond_formats", &t.cond_formats)?;
    d.set_item("comments", &t.comments)?;
    d.set_item("tables", &t.tables)?;
    d.set_item("dimensions", &t.dimensions)?;
    d.set_item("workbook_extra", &t.workbook_extra)?;
    d.set_item("properties", &t.properties)?;
    let drawings = pyo3::types::PyList::empty(py);
    for (sheet, dr) in &t.drawings {
        let image = dr.image.as_ref().map(|(data, ext)| (ext.clone(), PyBytes::new(py, data)));
        drawings.append((*sheet, dr.anchor.from, dr.anchor.to, dr.anchor.ext, image, dr.chart.as_deref()))?;
    }
    d.set_item("drawings", drawings)?;
    d.set_item("sheet_kind", pyo3::types::PyList::new(py, &t.sheet_kind)?)?;
    d.set_item("style_spec", &t.style_spec)?;
    d.set_item("named_styles", &t.named_styles)?;
    d.set_item("rows", t.rows.iter().map(|(s, r)| (*s, r.row, r.height, r.custom_height, r.hidden, r.outline_level, r.collapsed, r.style, r.custom_format, r.thick_top, r.thick_bot, r.phonetic)).collect::<Vec<_>>())?;
    d.set_item("freeze", &t.freeze)?;
    d.set_item("extra", &t.extra)?;
    d.set_item("merges", &t.merges)?;
    d.set_item("columns", t.columns.iter().map(|(s, c)| (*s, c.first, c.last, c.width, c.custom_width, c.hidden, c.best_fit, c.outline_level, c.collapsed, c.style, c.phonetic)).collect::<Vec<_>>())?;
    Ok(d)
}

/// Serial number of a date and time in the 1900 date system.
#[pyfunction]
#[pyo3(signature = (year, month, day, hour=0, minute=0, second=0, microsecond=0))]
fn date_serial(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32, microsecond: u32) -> Option<f64> {
    xl_compat::date_serial(year, month, day, hour, minute, second, microsecond)
}

/// A formula (without `=`) as it reads when moved `rows` down and `cols` right: relative
/// references move, absolute ones stay, references that leave the sheet become `#REF!`.
/// `None` if the formula cannot be parsed.
#[pyfunction]
fn translate_formula(formula: &str, rows: i64, cols: i64) -> Option<String> {
    let expr = xl_formula::parse(formula).ok()?;
    Some(expr.shifted(rows, cols).to_string())
}

#[pymodule]
fn _xlcells(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Writer>()?;
    m.add_function(wrap_pyfunction!(read, m)?)?;
    m.add_function(wrap_pyfunction!(date_serial, m)?)?;
    m.add_function(wrap_pyfunction!(translate_formula, m)?)?;
    Ok(())
}
