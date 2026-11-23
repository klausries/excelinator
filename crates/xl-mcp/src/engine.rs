//! The engine the server works with, and how paths of the machine are seen.
//!
//! The formula engine and the store are those of `xl-engine`; what runs their plans and
//! reads and writes Parquet (the *backend*) is set once at the start. The program
//! `xl-mcp` uses the Polars built into it (feature `native`); built for WebAssembly
//! (`bindings/wasm`) the host gives the backend: the Polars of its Python.
//!
//! Paths: where the server runs as WebAssembly it sees the host's files under other
//! names (`C:\Users\x` is `/c/Users/x`). [`set_paths`] gives the two ways of naming;
//! without it a path is what it says.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use xl_engine::store::Backend;

pub use xl_engine::{apply, codegen, data, frame, lift, long, plans, port, regions, view};

/// Makes the backend of one call.
pub type MakeBackend = fn() -> Box<dyn Backend>;

static MAKE: OnceLock<MakeBackend> = OnceLock::new();

/// The backend of every call from now on (once; later calls change nothing).
pub fn set_backend(make: MakeBackend) {
    let _ = MAKE.set(make);
}

pub(crate) fn backend() -> Box<dyn Backend> {
    match MAKE.get() {
        Some(make) => make(),
        None => built_in(),
    }
}

#[cfg(feature = "native")]
fn built_in() -> Box<dyn Backend> {
    Box::new(xl_polars::Polars::default())
}

#[cfg(not(feature = "native"))]
fn built_in() -> Box<dyn Backend> {
    panic!("xl-mcp without the feature `native` needs a backend: call set_backend first")
}

/// Runs the check of a port where Python can run it (see `xl_engine::port::Check`).
pub type PortCheck = fn(&Path) -> Result<Vec<(String, String, String)>, String>;

static CHECK: OnceLock<PortCheck> = OnceLock::new();

/// How ports are checked as they are written (once); without it they are not.
pub fn set_port_check(check: PortCheck) {
    let _ = CHECK.set(check);
}

pub(crate) fn port_check() -> Option<PortCheck> {
    CHECK.get().copied()
}

/// The two ways of naming a path: as the person or agent writes it (`inbound`, to a
/// path the program can open), and as it is shown to them (`outbound`).
pub struct Paths {
    pub inbound: fn(&str) -> PathBuf,
    pub outbound: fn(&Path) -> String,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

/// How paths are named from now on (once).
pub fn set_paths(paths: Paths) {
    let _ = PATHS.set(paths);
}

/// A path as written by a person or an agent, as the program opens it.
pub fn inbound(text: &str) -> PathBuf {
    match PATHS.get() {
        Some(p) => (p.inbound)(text),
        None => PathBuf::from(text),
    }
}

/// A path as the person or agent knows it.
pub fn outbound(path: &Path) -> String {
    match PATHS.get() {
        Some(p) => (p.outbound)(path),
        None => path.display().to_string(),
    }
}

/// Stores (see `xl_engine::store`) on the backend set.
pub mod store {
    use super::backend;
    use std::path::Path;
    use xl_cells::Workbook;
    use xl_engine::data::Frame;
    pub use xl_engine::store::*;

    pub fn create(dir: &Path, original: &[u8], wb: &Workbook, is_xlsx: bool) -> Result<Vec<(String, usize)>, String> {
        xl_engine::store::create(&*backend(), dir, original, wb, is_xlsx)
    }

    pub fn run(dir: &Path) -> Result<RunReport, String> {
        xl_engine::store::run(&*backend(), dir)
    }

    pub fn read_region(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_region(&*backend(), dir, name)
    }

    pub fn read_frame(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_frame(&*backend(), dir, name)
    }

    pub fn read_wide(dir: &Path, name: &str) -> Result<Frame, String> {
        xl_engine::store::read_wide(&*backend(), dir, name)
    }

    pub fn write_frame(dir: &Path, name: &str, df: &Frame) -> Result<(), String> {
        xl_engine::store::write_frame(&*backend(), dir, name, df)
    }

    pub fn recalculate(dir: &Path) -> Result<xl_engine::plans::Recalculation, String> {
        xl_engine::store::recalculate(&*backend(), dir)
    }

    pub fn add_sheet(dir: &Path, like: &str, name: &str) -> Result<Vec<String>, String> {
        xl_engine::store::add_sheet(&*backend(), dir, like, name)
    }

    pub fn rename_region(dir: &Path, region: &str, name: &str) -> Result<RunReport, String> {
        xl_engine::store::rename_region(&*backend(), dir, region, name)
    }

    pub fn set_cells(dir: &Path, cells: &[(String, String, serde_json::Value)]) -> Result<RunReport, String> {
        xl_engine::store::set_cells(&*backend(), dir, cells)
    }

    pub fn patch(dir: &Path, region: &str, patch: &serde_json::Value) -> Result<usize, String> {
        xl_engine::store::patch(&*backend(), dir, region, patch)
    }

    pub fn tables(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
        xl_engine::store::tables(&*backend(), dir)
    }
}
