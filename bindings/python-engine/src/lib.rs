//! `xlcells_engine._engine` (and `xlcells_engine_bundled._engine`): the part of xlcells
//! that calculates and gives dataframes.
//!
//! It holds the formula engine, the typed frames handed over as Arrow, the regions of a
//! workbook, the store, and the look of cells (whose conditional formats the engine
//! decides). The package `xlcells` (reading and writing, the openpyxl and pyExcelerator
//! interfaces) works without it; its methods that need the engine (`wb.calculate()`,
//! `ws.to_polars()`, `wb.regions`, `wb.to_store()`, `cell.look`,
//! `xlcells.polars.read_excel`) ask for this package.
//!
//! Two builds of the same code: by default the engine's expressions run on the Polars
//! installed in the Python environment (through `xlcells_engine._shim`), and Parquet
//! files are written by it; with the feature `bundled` they run on the Polars built in.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::sync::Mutex;
use xl_engine::data::Frame as XFrame;

#[cfg(feature = "bundled")]
mod engine {
    use xl_engine::ir::Executor;
    use xl_engine::store::Backend;

    pub const KIND: &str = "bundled";

    pub fn executor() -> Box<dyn Executor> {
        Box::new(xl_polars::builtin::PolarsExecutor::default())
    }

    pub fn backend() -> Box<dyn Backend> {
        Box::new(xl_polars::Polars::default())
    }

    pub fn polars_version(_py: pyo3::Python<'_>) -> String {
        "0.55.2 (Rust, built in)".into()
    }
}

#[cfg(not(feature = "bundled"))]
mod engine {
    use pyo3::prelude::*;
    use std::cell::RefCell;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Weak};
    use xl_engine::data::Frame;
    use xl_engine::ir::{Executor, Plan, Table};
    use xl_engine::store::Backend;

    pub const KIND: &str = "environment";

    const SHIM: &str = "xlcells_engine._shim";

    /// Keys of plans as the shim knows them: one for every plan sent.
    static KEYS: AtomicU64 = AtomicU64::new(1);

    fn text(e: PyErr) -> String {
        e.to_string()
    }

    /// Runs plans on the Polars of the Python environment, through the shim. A plan run
    /// again (the rounds of a formula reading its own column) is sent once.
    #[derive(Default)]
    pub struct PyExecutor {
        last: RefCell<Option<(Weak<Plan>, u64)>>,
    }

    impl Executor for PyExecutor {
        fn run(&self, plan: &Arc<Plan>, input: &Table) -> Result<Table, String> {
            let mut last = self.last.borrow_mut();
            let known = last.as_ref().filter(|(p, _)| p.upgrade().is_some_and(|q| Arc::ptr_eq(&q, plan))).map(|(_, k)| *k);
            let (key, spec) = match known {
                Some(k) => (k, None),
                None => {
                    let k = KEYS.fetch_add(1, Ordering::Relaxed);
                    *last = Some((Arc::downgrade(plan), k));
                    (k, Some(xl_engine::ir::wire::plan_json(plan)))
                }
            };
            Python::attach(|py| -> PyResult<Table> {
                let shim = py.import(SHIM)?;
                let table = crate::Arrow::new(xl_arrow::table_to_stream(input));
                let out = shim.call_method1("run", (key, spec, table))?;
                crate::with_stream(&out, |ptr| unsafe { xl_arrow::table_from_stream(ptr) })
            })
            .map_err(|e| {
                // The plan is sent again after a failure (the shim may not have kept it).
                *last = None;
                text(e)
            })
        }
    }

    pub fn executor() -> Box<dyn Executor> {
        Box::new(PyExecutor::default())
    }

    /// Stores with Parquet written and read by the Polars of the environment.
    #[derive(Default)]
    pub struct PyBackend {
        executor: PyExecutor,
    }

    impl Backend for PyBackend {
        fn executor(&self) -> &dyn Executor {
            &self.executor
        }

        fn write_parquet(&self, path: &Path, frame: &Frame, note: Option<&str>) -> Result<(), String> {
            Python::attach(|py| -> PyResult<()> {
                let table = crate::Arrow::new(xl_arrow::frame_to_stream(frame));
                py.import(SHIM)?.call_method1("write_parquet", (path.to_string_lossy().to_string(), table, xl_engine::store::NOTE_KEY, note))?;
                Ok(())
            })
            .map_err(text)
        }

        fn read_note(&self, path: &Path) -> Result<Option<String>, String> {
            Python::attach(|py| py.import(SHIM)?.call_method1("read_note", (path.to_string_lossy().to_string(), xl_engine::store::NOTE_KEY))?.extract::<Option<String>>()).map_err(text)
        }

        fn read_parquet(&self, path: &Path) -> Result<Frame, String> {
            Python::attach(|py| -> PyResult<Frame> {
                let df = py.import(SHIM)?.call_method1("read_parquet", (path.to_string_lossy().to_string(),))?;
                crate::with_stream(&df, |ptr| unsafe { xl_arrow::frame_from_stream(ptr) })
            })
            .map_err(text)
        }
    }

    pub fn backend() -> Box<dyn Backend> {
        Box::new(PyBackend::default())
    }

    pub fn polars_version(py: Python<'_>) -> String {
        py.import(SHIM).and_then(|m| m.call_method0("version")).and_then(|v| v.extract::<String>()).unwrap_or_else(|e| format!("unavailable: {e}"))
    }
}

/// Which build this is (`environment`: the Polars of the Python environment runs the
/// engine's expressions; `bundled`: the Polars built in) and that Polars' version.
#[pyfunction]
fn engine_info(py: Python<'_>) -> (&'static str, String) {
    (engine::KIND, engine::polars_version(py))
}

/// Read the Arrow stream of something with `__arrow_c_stream__` (a Polars DataFrame, a
/// pyarrow Table): `read` gets the stream, which lives as long as its capsule, held here.
fn with_stream<T>(obj: &Bound<'_, PyAny>, read: impl FnOnce(*mut xl_arrow::ArrowArrayStream) -> Result<T, String>) -> PyResult<T> {
    use pyo3::types::PyCapsuleMethods;
    let capsule = obj.call_method0("__arrow_c_stream__").map_err(|_| PyValueError::new_err("a dataframe is needed (Polars or pyarrow): something with __arrow_c_stream__"))?;
    let capsule = capsule.cast::<pyo3::types::PyCapsule>()?;
    let ptr = capsule.pointer_checked(Some(c"arrow_array_stream"))?.as_ptr() as *mut xl_arrow::ArrowArrayStream;
    read(ptr).map_err(PyValueError::new_err)
}

fn err(e: xl_compat::Error) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Calculate the formulas of a workbook with their Polars expressions. Returns the cells
/// that got a value, as (sheet, row, col, kind, num, text) with kind 1 number, 2 text,
/// 3 bool, and what happened: formula cells, computed, changed, settled, rounds, and the
/// cells not computed by reason.
#[pyfunction]
fn calculate<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyDict>> {
    let wb = xl_cells::read_bytes(data.to_vec()).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let (out, calc) = py.detach(|| xl_engine::lift::calculate_with(&*engine::executor(), &wb));
    let c = &out.cells;
    let mut values = Vec::new();
    for i in 0..c.len() {
        let before = &wb.cells;
        if c.formula[i].is_none() || (before.kind[i] == c.kind[i] && before.num[i].to_bits() == c.num[i].to_bits() && before.text[i] == c.text[i]) {
            continue;
        }
        values.push((c.sheet[i], c.row[i], c.col[i], c.kind[i] as u8, c.num[i], c.text[i].as_deref().map(str::to_string)));
    }
    let d = PyDict::new(py);
    d.set_item("values", values)?;
    d.set_item("formula_cells", calc.formula_cells)?;
    d.set_item("computed", calc.computed)?;
    d.set_item("changed", calc.changed)?;
    d.set_item("settled", calc.settled)?;
    d.set_item("rounds", calc.rounds)?;
    d.set_item("not_computed", calc.not_computed.into_iter().collect::<Vec<_>>())?;
    Ok(d)
}

/// The kind of a source and its parts: the name (style, row number, column letter, table,
/// range of the conditional format), then for a table its style and element, for a
/// conditional format the rule's priority and type.
fn source_parts(s: &xl_look::Source) -> (&'static str, Vec<String>) {
    use xl_look::Source;
    let parts = match s {
        Source::Default | Source::Cell => Vec::new(),
        Source::Style(n) => vec![n.clone()],
        Source::Row(r) => vec![(r + 1).to_string()],
        Source::Column(c) => vec![xl_formula::col_name(c + 1)],
        Source::Table { table, style, element } => vec![table.clone(), style.clone(), element.clone()],
        Source::Conditional { range, priority, kind } => vec![range.clone(), priority.to_string(), kind.clone()],
    };
    (s.kind(), parts)
}

/// The range of the conditional format a data bar or an icon comes from.
fn source_range(s: &xl_look::Source) -> Option<String> {
    match s {
        xl_look::Source::Conditional { range, .. } => Some(range.clone()),
        _ => None,
    }
}

fn looks_to_py<'py>(py: Python<'py>, looks: Vec<xl_look::Look>) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let mut out = Vec::with_capacity(looks.len());
    for l in looks {
        let d = PyDict::new(py);
        for (k, a) in &l.attrs {
            let value: Py<PyAny> = match &a.value {
                xl_look::Val::Text(t) => t.clone().into_pyobject(py)?.into_any().unbind(),
                xl_look::Val::Number(n) => n.into_pyobject(py)?.into_any().unbind(),
                xl_look::Val::Flag(b) => pyo3::types::PyBool::new(py, *b).to_owned().into_any().unbind(),
                xl_look::Val::Rgb(c) => c.clone().into_pyobject(py)?.into_any().unbind(),
                xl_look::Val::None => py.None(),
            };
            let (kind, parts) = source_parts(&a.source);
            d.set_item(*k, (value, kind, pyo3::types::PyTuple::new(py, parts)?, a.certain, a.spec.clone()))?;
        }
        match &l.data_bar {
            Some(b) => d.set_item("data_bar", (b.color.clone(), b.fraction, source_range(&b.source)))?,
            None => d.set_item("data_bar", py.None())?,
        }
        match &l.icon {
            Some(i) => d.set_item("icon", (i.set.clone(), i.index, source_range(&i.source)))?,
            None => d.set_item("icon", py.None())?,
        }
        d.set_item("undecided", l.undecided.clone())?;
        out.push(d);
    }
    Ok(out)
}

/// The looks of the cells of one workbook file, read once: what was worked out for one
/// call of `looks` (formats, rules decided) serves the next.
#[pyclass]
struct Looker {
    inner: xl_look::Looker<'static>,
}

#[pymethods]
impl Looker {
    #[new]
    fn new(py: Python<'_>, data: &[u8]) -> PyResult<Self> {
        let inner = py.detach(|| -> std::result::Result<_, String> {
            let wb = xl_cells::read_bytes(data.to_vec()).map_err(|e| e.to_string())?;
            let mut looker = xl_look::Looker::owning_with_engine(wb, engine::executor);
            looker.pivots = xl_look::pivot::pivot_areas(data);
            Ok(looker)
        });
        Ok(Looker { inner: inner.map_err(PyValueError::new_err)? })
    }

    /// For each (sheet, row, col), 0-based, a dict of every attribute as (value, source
    /// kind, source parts, certain, colour as stated), and `data_bar`, `icon`, `undecided`.
    fn looks<'py>(&mut self, py: Python<'py>, cells: Vec<(u32, u32, u32)>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let inner = &mut self.inner;
        let looks = py.detach(|| inner.looks(&cells));
        looks_to_py(py, looks)
    }
}

/// The look of cells of a file; see [`Looker::looks`].
#[pyfunction]
fn looks<'py>(py: Python<'py>, data: &[u8], cells: Vec<(u32, u32, u32)>) -> PyResult<Vec<Bound<'py, PyDict>>> {
    Looker::new(py, data)?.looks(py, cells)
}

/// An Arrow stream on its way into a capsule. The stream is only ever touched by the one
/// who holds the capsule.
#[repr(transparent)]
struct Stream(xl_arrow::ArrowArrayStream);
unsafe impl Send for Stream {}

/// An Arrow stream handed to Python once (`__arrow_c_stream__`), for Polars to read.
#[pyclass]
struct Arrow {
    stream: Mutex<Option<Stream>>,
}

impl Arrow {
    #[allow(dead_code)]
    fn new(stream: xl_arrow::ArrowArrayStream) -> Self {
        Arrow { stream: Mutex::new(Some(Stream(stream))) }
    }
}

#[pymethods]
impl Arrow {
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_stream__<'py>(&self, py: Python<'py>, requested_schema: Option<Py<PyAny>>) -> PyResult<Bound<'py, pyo3::types::PyCapsule>> {
        let _ = requested_schema;
        let stream = self.stream.lock().map_err(|_| PyValueError::new_err("stream"))?.take().ok_or_else(|| PyValueError::new_err("the stream was read already"))?;
        pyo3::types::PyCapsule::new_with_value(py, stream, c"arrow_array_stream")
    }
}

/// A table of typed columns, held in Rust. It gives itself to Polars, pyarrow or
/// DuckDB through the Arrow stream interface (`__arrow_c_stream__`), without passing
/// through Python objects.
#[pyclass]
struct Frame {
    df: XFrame,
}

#[pymethods]
impl Frame {
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_stream__<'py>(&self, py: Python<'py>, requested_schema: Option<Py<PyAny>>) -> PyResult<Bound<'py, pyo3::types::PyCapsule>> {
        let _ = requested_schema;
        pyo3::types::PyCapsule::new_with_value(py, Stream(xl_arrow::frame_to_stream(&self.df)), c"arrow_array_stream")
    }

    /// (rows, columns).
    #[getter]
    fn shape(&self) -> (usize, usize) {
        self.df.shape()
    }

    #[getter]
    fn columns(&self) -> Vec<String> {
        self.df.names().into_iter().map(str::to_string).collect()
    }

    fn __repr__(&self) -> String {
        let cols: Vec<String> = self.df.columns.iter().map(|c| format!("{}: {}", c.name, c.data.type_name())).collect();
        format!("<Frame {} rows: {}>", self.df.height, cols.join(", "))
    }
}

/// A workbook file (xlsx or xls) read once and held in Rust: what is asked of it is
/// worked out there.
#[pyclass]
struct Book {
    wb: xl_cells::Workbook,
    /// The file as read: what is written goes onto it.
    original: Vec<u8>,
    /// The regions of `wb`, found when first asked for; forgotten when `wb` changes.
    found: std::sync::OnceLock<Vec<xl_engine::regions::Region>>,
}

/// A frame from anything that gives an Arrow stream (`__arrow_c_stream__`): a Polars
/// DataFrame, a pyarrow Table, a `Frame`.
fn frame_from_py(obj: &Bound<'_, PyAny>) -> PyResult<XFrame> {
    if let Ok(frame) = obj.cast::<Frame>() {
        return Ok(frame.borrow().df.clone());
    }
    // Columns without a value are handed over as nulls of another type (see the module).
    let obj = obj.py().import("xlcells._arrow_compat")?.call_method1("prepare", (obj,))?;
    // The capsule holds a stream by the Arrow PyCapsule interface; it is read once.
    with_stream(&obj, |ptr| unsafe { xl_arrow::frame_from_stream(ptr) })
}

impl Book {
    fn region(&self, name: &str) -> PyResult<xl_engine::regions::Region> {
        let all = self.found.get_or_init(|| xl_engine::regions::regions(&self.wb));
        all.iter().find(|r| r.name == name).cloned().ok_or_else(|| PyValueError::new_err(format!("no region named {name:?}; the regions are {:?}", all.iter().map(|r| r.name.as_str()).collect::<Vec<_>>())))
    }
}

#[pymethods]
impl Book {
    #[new]
    fn new(py: Python<'_>, data: &[u8]) -> PyResult<Self> {
        let wb = py.detach(|| xl_compat::read_any(data.to_vec())).map_err(err)?;
        Ok(Book { wb, original: data.to_vec(), found: std::sync::OnceLock::new() })
    }

    #[getter]
    fn sheet_names(&self) -> Vec<String> {
        self.wb.sheets.iter().map(|s| s.name.clone()).collect()
    }

    /// The used block of a sheet as (first row, first column, last row, last column),
    /// 0-based; `None` for a sheet without cells.
    fn used_area(&self, sheet: u32) -> Option<(u32, u32, u32, u32)> {
        xl_engine::frame::used_area(&self.wb, sheet).map(|a| (a.first_row, a.first_col, a.last_row, a.last_col))
    }

    /// An area of a sheet (`area`: first row, first column, last row, last column,
    /// 0-based; default the used block) as a typed frame.
    #[pyo3(signature = (sheet, area=None, has_header=true))]
    fn frame(&self, py: Python<'_>, sheet: u32, area: Option<(u32, u32, u32, u32)>, has_header: bool) -> PyResult<Frame> {
        if sheet as usize >= self.wb.sheets.len() {
            return Err(PyValueError::new_err(format!("no sheet {sheet}; the workbook has {} sheets", self.wb.sheets.len())));
        }
        let df = py.detach(|| match area {
            Some((r0, c0, r1, c1)) if r0 <= r1 && c0 <= c1 => xl_engine::frame::frame(&self.wb, xl_engine::frame::Area { sheet, first_row: r0, first_col: c0, last_row: r1, last_col: c1 }, has_header),
            Some(_) => Ok(XFrame::empty()),
            None => xl_engine::frame::sheet_frame(&self.wb, sheet, has_header),
        });
        Ok(Frame { df: df.map_err(PyValueError::new_err)? })
    }

    /// What the workbook holds: tables, groups of single cells and blocks, each a dict
    /// with name, kind, sheet (index), ref, has_header, source, columns (name, 0-based
    /// column, dtype, formula, logic, exceptions: (0-based row of the sheet, formula, logic)),
    /// first_row (0-based row of the sheet of the first data row) and cells (name, row,
    /// column, dtype, formula, logic).
    fn regions<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let mut out = Vec::new();
        for r in self.found.get_or_init(|| xl_engine::regions::regions(&self.wb)) {
            let d = PyDict::new(py);
            d.set_item("name", &r.name)?;
            d.set_item("kind", r.kind.as_str())?;
            d.set_item("sheet", r.area.sheet)?;
            d.set_item("ref", r.range())?;
            d.set_item("has_header", r.has_header)?;
            d.set_item("source", r.source)?;
            d.set_item("first_row", r.data_rows().map_or(0, |x| x.0))?;
            d.set_item("columns", r.columns.iter().map(|c| (c.name.clone(), c.col, c.dtype.clone(), c.formula.clone(), c.logic.clone(), c.exceptions.iter().map(|e| (e.row, e.formula.clone(), e.logic.clone())).collect::<Vec<_>>())).collect::<Vec<_>>())?;
            d.set_item("cells", r.cells.iter().map(|c| (c.name.clone(), c.row, c.col, c.dtype.clone(), c.formula.clone(), c.logic.clone())).collect::<Vec<_>>())?;
            out.push(d);
        }
        Ok(out)
    }

    /// A region as a frame (single cells: one row, a column per cell).
    fn region_frame(&self, name: &str) -> PyResult<Frame> {
        let region = self.region(name)?;
        let df = xl_engine::regions::region_frame(&self.wb, &region).map_err(PyValueError::new_err)?;
        Ok(Frame { df })
    }

    /// Put a dataframe into the cells of a region; returns the region's range afterwards.
    fn write(&mut self, name: &str, frame: &Bound<'_, PyAny>) -> PyResult<String> {
        let region = self.region(name)?;
        let df = frame_from_py(frame)?;
        let after = xl_engine::apply::write_region(&mut self.wb, &region, &df).map_err(PyValueError::new_err)?;
        self.found = std::sync::OnceLock::new();
        Ok(after.range())
    }

    /// The cells of a region whose value in the dataframe differs from the workbook's.
    fn verify(&self, name: &str, frame: &Bound<'_, PyAny>) -> PyResult<Frame> {
        let region = self.region(name)?;
        let df = frame_from_py(frame)?;
        Ok(Frame { df: xl_engine::apply::verify(&self.wb, &region, &df).map_err(PyValueError::new_err)? })
    }

    /// Calculate the formulas; returns (formula cells, computed, not computed by reason).
    fn calculate(&mut self, py: Python<'_>) -> (u64, u64, Vec<(String, u64)>) {
        let (done, calc) = py.detach(|| xl_engine::lift::calculate_with(&*engine::executor(), &self.wb));
        self.wb = done;
        self.found = std::sync::OnceLock::new();
        (calc.formula_cells, calc.computed, calc.not_computed.into_iter().collect())
    }

    /// The workbook as a Polars program in Python (see `xl_engine::codegen`): the source;
    /// for every calculated column and cell (region, name, formula, how, reason, line
    /// where its statements begin, the statement that takes it from the engine, an
    /// expression for its value as a frame); the line where the final check begins; the
    /// statement that asks the engine for its values.
    /// `exact`: everything by the engine; `engine`: (region, name) pairs left to it.
    #[pyo3(signature = (file, exact=false, engine=Vec::new()))]
    #[allow(clippy::type_complexity)]
    fn polars_program(&self, file: &str, exact: bool, engine: Vec<(String, String)>) -> (String, Vec<(String, String, String, &'static str, Option<String>, usize, String, String)>, usize, String) {
        let accuracy = if exact { xl_engine::codegen::Accuracy::Exact } else { xl_engine::codegen::Accuracy::Plain };
        let ex = engine::executor();
        let p = xl_engine::codegen::polars_program(&self.wb, file, accuracy, &engine, Some(&*ex));
        (p.source, p.items.into_iter().map(|i| (i.region, i.name, i.formula, i.how, i.reason, i.line, i.fallback, i.check)).collect(), p.check_line, p.exact_statement)
    }

    /// The workbook as an xlsx file, written onto the file it was read from.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = if xl_compat::is_xls(&self.original) { xl_cells::write_bytes(&self.wb) } else { xl_cells::update_bytes(self.original.clone(), &self.wb) };
        Ok(PyBytes::new(py, &data.map_err(|e| PyValueError::new_err(e.to_string()))?))
    }
}

/// Make a store (see `xl_engine::store`) in a directory from a workbook file; returns
/// the regions and their rows.
#[pyfunction]
fn store_create(py: Python<'_>, dir: &str, data: &[u8]) -> PyResult<Vec<(String, usize)>> {
    py.detach(|| {
        let wb = xl_compat::read_any(data.to_vec()).map_err(|e| e.to_string())?;
        xl_engine::store::create(&*engine::backend(), std::path::Path::new(dir), data, &wb, !xl_compat::is_xls(data))
    })
    .map_err(PyValueError::new_err)
}

/// Bring a store up to date; returns (formula cells, computed, not computed by reason,
/// regions and rows, logic changed).
#[pyfunction]
#[allow(clippy::type_complexity)]
fn store_run(py: Python<'_>, dir: &str) -> PyResult<(u64, u64, Vec<(String, u64)>, Vec<(String, usize)>, Vec<(String, String)>)> {
    let r = py.detach(|| xl_engine::store::run(&*engine::backend(), std::path::Path::new(dir))).map_err(PyValueError::new_err)?;
    Ok((r.formula_cells, r.computed, r.not_computed, r.regions, r.logic_changed))
}

/// The frame of a region of a store (single cells: a row per cell).
#[pyfunction]
fn store_read(py: Python<'_>, dir: &str, name: &str) -> PyResult<Frame> {
    Ok(Frame { df: py.detach(|| xl_engine::store::read_region(&*engine::backend(), std::path::Path::new(dir), name)).map_err(PyValueError::new_err)? })
}

/// The frame of a region of a store with single cells as one row, a column per cell.
#[pyfunction]
fn store_read_wide(py: Python<'_>, dir: &str, name: &str) -> PyResult<Frame> {
    Ok(Frame { df: py.detach(|| xl_engine::store::read_wide(&*engine::backend(), std::path::Path::new(dir), name)).map_err(PyValueError::new_err)? })
}

/// Replace the frame of a region of a store.
#[pyfunction]
fn store_write(py: Python<'_>, dir: &str, name: &str, frame: &Bound<'_, PyAny>) -> PyResult<()> {
    let df = frame_from_py(frame)?;
    py.detach(|| xl_engine::store::write_frame(&*engine::backend(), std::path::Path::new(dir), name, &df)).map_err(PyValueError::new_err)
}

/// Add a sheet to a store's workbook as a copy of another; the regions of the new sheet.
#[pyfunction]
fn store_add_sheet(py: Python<'_>, dir: &str, like: &str, name: &str) -> PyResult<Vec<String>> {
    py.detach(|| xl_engine::store::add_sheet(&*engine::backend(), std::path::Path::new(dir), like, name)).map_err(PyValueError::new_err)
}

/// Change rows of a region of a store, or of a directory of Parquet files with notes: a
/// patch as JSON (see `xl_engine::view::patched`); the rows the region has afterwards.
#[pyfunction]
fn store_patch(py: Python<'_>, dir: &str, name: &str, patch: &str) -> PyResult<usize> {
    let patch: serde_json::Value = serde_json::from_str(patch).map_err(|e| PyValueError::new_err(e.to_string()))?;
    py.detach(|| xl_engine::store::patch(&*engine::backend(), std::path::Path::new(dir), name, &patch)).map_err(PyValueError::new_err)
}

/// The tables of a store, or of a directory of Parquet files, as their notes describe
/// them (see `xl_engine::store::tables`), as JSON.
#[pyfunction]
fn store_tables(py: Python<'_>, dir: &str) -> PyResult<String> {
    py.detach(|| xl_engine::store::tables(&*engine::backend(), std::path::Path::new(dir))).map(|t| serde_json::Value::Array(t).to_string()).map_err(PyValueError::new_err)
}

/// Give a region of a store another name and run the store; returns what `store_run` returns.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn store_rename_region(py: Python<'_>, dir: &str, region: &str, name: &str) -> PyResult<(u64, u64, Vec<(String, u64)>, Vec<(String, usize)>, Vec<(String, String)>)> {
    let r = py.detach(|| xl_engine::store::rename_region(&*engine::backend(), std::path::Path::new(dir), region, name)).map_err(PyValueError::new_err)?;
    Ok((r.formula_cells, r.computed, r.not_computed, r.regions, r.logic_changed))
}

/// Keep the state of a store as it is, to come back to with `store_undo`.
#[pyfunction]
fn store_remember(dir: &str, what: &str) -> PyResult<()> {
    xl_engine::store::remember(std::path::Path::new(dir), what).map_err(PyValueError::new_err)
}

/// Bring back the state before the last change kept; what was undone, or `None`.
#[pyfunction]
fn store_undo(dir: &str) -> PyResult<Option<String>> {
    xl_engine::store::undo(std::path::Path::new(dir)).map_err(PyValueError::new_err)
}

/// Set cells of a store's workbook as typed into them (`[[sheet, address, input], …]` as
/// JSON) and run the store; returns what `store_run` returns.
#[pyfunction]
#[allow(clippy::type_complexity)]
fn store_set_cells(py: Python<'_>, dir: &str, cells: &str) -> PyResult<(u64, u64, Vec<(String, u64)>, Vec<(String, usize)>, Vec<(String, String)>)> {
    let cells: Vec<(String, String, serde_json::Value)> = serde_json::from_str(cells).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let r = py.detach(|| xl_engine::store::set_cells(&*engine::backend(), std::path::Path::new(dir), &cells)).map_err(PyValueError::new_err)?;
    Ok((r.formula_cells, r.computed, r.not_computed, r.regions, r.logic_changed))
}

type Recalculated = (Vec<(String, String)>, Vec<(String, String, String, String)>, Vec<(String, String, String)>);

fn recalculated(r: xl_engine::plans::Recalculation) -> Recalculated {
    (r.done, r.kept, r.from_kept)
}

/// Calculate the data of a store again from the notes of its Parquet files, without the
/// workbook (see `xl_engine::store::recalculate`); returns (done, kept, from kept).
#[pyfunction]
fn store_recalculate(py: Python<'_>, dir: &str) -> PyResult<Recalculated> {
    py.detach(|| xl_engine::store::recalculate(&*engine::backend(), std::path::Path::new(dir))).map(recalculated).map_err(PyValueError::new_err)
}

/// Calculate Parquet files again from the notes they carry; returns (done, kept, from kept).
#[pyfunction]
fn recalculate_files(py: Python<'_>, paths: Vec<String>) -> PyResult<Recalculated> {
    py.detach(|| {
        let b = engine::backend();
        let mut files = Vec::new();
        for p in &paths {
            let path = std::path::PathBuf::from(p);
            let note = b.read_note(&path)?.ok_or_else(|| format!("{p} carries no note (file metadata {:?}): it was not written by a store, or written anew by a tool that dropped it", xl_engine::store::NOTE_KEY))?;
            files.push((path, serde_json::from_str(&note).map_err(|e| format!("the note of {p} is not JSON: {e}"))?));
        }
        xl_engine::store::recalculate_files(&*b, &files)
    })
    .map(recalculated)
    .map_err(PyValueError::new_err)
}

/// The store as a Python project on Polars in `dest` (see `xl_engine::port`): how every
/// calculated column and cell is ported (region, name, formula, logic, how, reason,
/// line), and how far the port is, in a sentence.
#[pyfunction]
#[pyo3(signature = (dir, dest, exact=false, replace=false, check=false))]
#[allow(clippy::type_complexity)]
fn store_port(py: Python<'_>, dir: &str, dest: &str, exact: bool, replace: bool, check: bool) -> PyResult<(Vec<(String, String, String, Option<String>, &'static str, Option<String>, usize)>, String)> {
    let accuracy = if exact { xl_engine::codegen::Accuracy::Exact } else { xl_engine::codegen::Accuracy::Plain };
    // The project's check, run by Python (xlcells._port_check).
    let run_check = |folder: &std::path::Path| -> Result<Vec<(String, String, String)>, String> {
        Python::attach(|py| py.import("xlcells._port_check")?.call_method1("differing", (folder.to_string_lossy().to_string(),))?.extract::<Vec<(String, String, String)>>()).map_err(|e| e.to_string())
    };
    let p = py
        .detach(|| {
            let checker: Option<xl_engine::port::Check> = if check { Some(&run_check) } else { None };
            xl_engine::port::write(&*engine::backend(), std::path::Path::new(dir), std::path::Path::new(dest), accuracy, replace, checker)
        })
        .map_err(PyValueError::new_err)?;
    let summary = p.summary();
    Ok((p.items.into_iter().map(|i| (i.region, i.name, i.formula, i.logic, i.how, i.reason, i.line)).collect(), summary))
}

/// The note a Parquet file carries (JSON), if any.
#[pyfunction]
fn read_note(py: Python<'_>, path: &str) -> PyResult<Option<String>> {
    py.detach(|| engine::backend().read_note(std::path::Path::new(path))).map_err(PyValueError::new_err)
}

/// Every formula value of a workbook file removed and calculated again, cell by cell,
/// as text (see `xl_engine::lift::recalculation`): for comparing executors.
#[pyfunction]
fn recalculation(py: Python<'_>, data: &[u8]) -> PyResult<String> {
    let wb = xl_compat::read_any(data.to_vec()).map_err(err)?;
    Ok(py.detach(|| xl_engine::lift::recalculation(&*engine::executor(), &wb)))
}

#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Looker>()?;
    m.add_class::<Book>()?;
    m.add_class::<Frame>()?;
    m.add_class::<Arrow>()?;
    m.add_function(wrap_pyfunction!(engine_info, m)?)?;
    m.add_function(wrap_pyfunction!(recalculation, m)?)?;
    m.add_function(wrap_pyfunction!(calculate, m)?)?;
    m.add_function(wrap_pyfunction!(looks, m)?)?;
    m.add_function(wrap_pyfunction!(store_create, m)?)?;
    m.add_function(wrap_pyfunction!(store_run, m)?)?;
    m.add_function(wrap_pyfunction!(store_read, m)?)?;
    m.add_function(wrap_pyfunction!(store_read_wide, m)?)?;
    m.add_function(wrap_pyfunction!(store_add_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(store_patch, m)?)?;
    m.add_function(wrap_pyfunction!(store_rename_region, m)?)?;
    m.add_function(wrap_pyfunction!(store_tables, m)?)?;
    m.add_function(wrap_pyfunction!(store_remember, m)?)?;
    m.add_function(wrap_pyfunction!(store_undo, m)?)?;
    m.add_function(wrap_pyfunction!(store_set_cells, m)?)?;
    m.add_function(wrap_pyfunction!(store_write, m)?)?;
    m.add_function(wrap_pyfunction!(store_recalculate, m)?)?;
    m.add_function(wrap_pyfunction!(recalculate_files, m)?)?;
    m.add_function(wrap_pyfunction!(read_note, m)?)?;
    m.add_function(wrap_pyfunction!(store_port, m)?)?;
    m.add("NOTE_KEY", xl_engine::store::NOTE_KEY)?;
    Ok(())
}
