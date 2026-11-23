//! The engine of `xl-engine` on the Polars built into this crate.
//!
//! * [`builtin`]: the executor of the formula engine's expressions on Polars.
//! * [`convert`]: the engine's frames as Polars frames and back; [`Polars`], the
//!   backend of stores (Parquet files written and read by Polars).
//! * [`lift`], [`store`]: the engine's functions with this executor and backend.
//! * [`cells_frame`]: the cell table of a workbook as a Polars `DataFrame` (one row
//!   per cell).
//!
//! The other modules are those of `xl-engine`, unchanged.

pub mod builtin;
pub mod convert;

pub use xl_engine::{apply, codegen, data, frame, ir, long, plans, regions, rows, stack, view};

pub use polars::prelude::DataFrame;
use polars::prelude::*;
use xl_cells::{Kind, Workbook};

/// The formula engine, its expressions run by the built-in Polars.
pub mod lift {
    pub use xl_engine::lift::*;
    use xl_cells::Workbook;

    /// The translation options of the entry points below: fixed cells as inputs
    /// (`Options::param`) when the environment variable `XL_PARAM` is set, to run the
    /// tests and measurements in that mode.
    pub fn default_options() -> Options {
        Options { param: std::env::var_os("XL_PARAM").is_some() }
    }

    /// Calculate the formulas of a workbook (see [`calculate_with`]).
    pub fn calculate(original: &Workbook) -> (Workbook, Calculation) {
        calculate_opts(&crate::builtin::PolarsExecutor::default(), original, default_options())
    }

    /// Lift every run of a workbook and check it against the stored values.
    pub fn lift(original: &Workbook) -> Report {
        lift_from(original, MIN_RUN)
    }

    /// As `lift`, for runs of at least `min_run` cells (1: every formula cell).
    pub fn lift_from(original: &Workbook, min_run: usize) -> Report {
        lift_opts(&crate::builtin::PolarsExecutor::default(), original, min_run, default_options())
    }
}

/// Stores (see `xl_engine::store`) with Parquet files written and read by Polars and
/// formulas calculated by the built-in Polars.
pub mod store {
    pub use xl_engine::store::*;
    use std::path::Path;
    use xl_cells::Workbook;
    use xl_engine::data::Frame;

    fn b() -> crate::Polars {
        crate::Polars::default()
    }

    pub fn create(dir: &Path, original: &[u8], wb: &Workbook, is_xlsx: bool) -> Result<Vec<(String, usize)>, String> {
        xl_engine::store::create(&b(), dir, original, wb, is_xlsx)
    }

    pub fn run(dir: &Path) -> Result<RunReport, String> {
        xl_engine::store::run(&b(), dir)
    }

    pub fn read_region(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_region(&b(), dir, name)
    }

    pub fn read_frame(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_frame(&b(), dir, name)
    }

    pub fn read_wide(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_wide(&b(), dir, name)
    }

    pub fn write_frame(dir: &Path, name: &str, df: &Frame) -> Result<(), String> {
        xl_engine::store::write_frame(&b(), dir, name, df)
    }

    pub fn recalculate(dir: &Path) -> Result<xl_engine::plans::Recalculation, String> {
        xl_engine::store::recalculate(&b(), dir)
    }

    pub fn add_sheet(dir: &Path, like: &str, name: &str) -> Result<Vec<String>, String> {
        xl_engine::store::add_sheet(&b(), dir, like, name)
    }

    pub fn rename_region(dir: &Path, region: &str, name: &str) -> Result<RunReport, String> {
        xl_engine::store::rename_region(&b(), dir, region, name)
    }

    pub fn set_cells(dir: &Path, cells: &[(String, String, serde_json::Value)]) -> Result<RunReport, String> {
        xl_engine::store::set_cells(&b(), dir, cells)
    }

    pub fn patch(dir: &Path, region: &str, patch: &serde_json::Value) -> Result<usize, String> {
        xl_engine::store::patch(&b(), dir, region, patch)
    }

    pub fn tables(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
        xl_engine::store::tables(&b(), dir)
    }
}

/// The backend of stores on Polars: the built-in executor, Parquet by Polars.
#[derive(Default)]
pub struct Polars {
    executor: builtin::PolarsExecutor,
}

impl xl_engine::store::Backend for Polars {
    fn executor(&self) -> &dyn xl_engine::ir::Executor {
        &self.executor
    }

    fn write_parquet(&self, path: &std::path::Path, frame: &xl_engine::data::Frame, note: Option<&str>) -> Result<(), String> {
        let mut df = convert::to_polars(frame).map_err(|e| e.to_string())?;
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let metadata = note.map(|n| KeyValueMetadata::from_static(vec![(xl_engine::store::NOTE_KEY.to_string(), n.to_string())]));
        // Statistics of a single row say nothing, and cost time for each of many columns.
        let statistics = if df.height() <= 1 { StatisticsOptions::empty() } else { StatisticsOptions::default() };
        ParquetWriter::new(file).with_key_value_metadata(metadata).with_statistics(statistics).finish(&mut df).map(|_| ()).map_err(|e| e.to_string())
    }

    fn read_note(&self, path: &std::path::Path) -> Result<Option<String>, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut reader = ParquetReader::new(file);
        let metadata = reader.get_metadata().map_err(|e| e.to_string())?;
        Ok(metadata.key_value_metadata().iter().flatten().find(|kv| kv.key == xl_engine::store::NOTE_KEY).and_then(|kv| kv.value.clone()))
    }

    fn read_parquet(&self, path: &std::path::Path) -> Result<xl_engine::data::Frame, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let df = ParquetReader::new(file).finish().map_err(|e| e.to_string())?;
        convert::from_polars(&df)
    }
}

/// The cells of a workbook as a frame with the columns sheet, row, col (0-based),
/// kind (0 blank, 1 number, 2 text, 3 bool, 4 error), num, text, formula, style.
pub fn cells_frame(wb: &Workbook) -> PolarsResult<DataFrame> {
    let c = &wb.cells;
    let num: Vec<Option<f64>> = (0..c.len()).map(|i| matches!(c.kind[i], Kind::Number | Kind::Bool).then_some(c.num[i])).collect();
    let text: Vec<Option<&str>> = c.text.iter().map(|t| t.as_deref()).collect();
    let formula: Vec<Option<&str>> = c.formula.iter().map(|f| f.as_deref()).collect();
    let kind: Vec<u8> = c.kind.iter().map(|k| *k as u8).collect();
    df!(
        "sheet" => &c.sheet,
        "row" => &c.row,
        "col" => &c.col,
        "kind" => kind,
        "num" => num,
        "text" => text,
        "formula" => formula,
        "style" => &c.style,
    )
}
