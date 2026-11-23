//! What the host does for the module: the Polars of its Python runs the engine's plans
//! and reads and writes Parquet files (with their notes).
//!
//! The host gives two functions in the import module `xl`: `host_call(ptr, len) -> n`
//! takes a message (see `rpc`) and answers with `n` bytes, which `host_take(ptr)` then
//! copies into the module. Paths in messages are written as the host writes them.

use crate::rpc::Msg;
use serde_json::{json, Value as J};
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use xl_engine::data::Frame;
use xl_engine::ir::{Executor, Plan, Table};
use xl_engine::store::Backend;

#[cfg(target_family = "wasm")]
#[link(wasm_import_module = "xl")]
extern "C" {
    fn host_call(ptr: *const u8, len: u32) -> u32;
    fn host_take(ptr: *mut u8);
}

#[cfg(not(target_family = "wasm"))]
unsafe fn host_call(_: *const u8, _: u32) -> u32 {
    panic!("the host is there only in WebAssembly")
}

#[cfg(not(target_family = "wasm"))]
unsafe fn host_take(_: *mut u8) {}

/// A call of the host: its name, its arguments, attachments.
pub fn call(name: &str, args: J, blobs: Vec<Vec<u8>>) -> Result<Msg, String> {
    let request = Msg { json: json!({ "fn": name, "args": args }), blobs }.encode();
    let n = unsafe { host_call(request.as_ptr(), request.len() as u32) };
    let mut answer = vec![0u8; n as usize];
    unsafe { host_take(answer.as_mut_ptr()) };
    let m = Msg::decode(&answer)?;
    match m.json.get("err").and_then(J::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(m),
    }
}

/// Paths go to the host as it writes them.
fn path(p: &Path) -> String {
    outbound(p)
}

/// Keys of plans as the host knows them: one for every plan sent.
static KEYS: AtomicU64 = AtomicU64::new(1);

/// Runs plans on the host's Polars. A plan run again (the rounds of a formula that
/// reads its own column) is sent once.
#[derive(Default)]
pub struct HostExecutor {
    last: RefCell<Option<(Weak<Plan>, u64)>>,
}

impl Executor for HostExecutor {
    fn run(&self, plan: &Arc<Plan>, input: &Table) -> Result<Table, String> {
        let mut last = self.last.borrow_mut();
        let known = last.as_ref().filter(|(p, _)| p.upgrade().is_some_and(|q| Arc::ptr_eq(&q, plan))).map(|(_, k)| *k);
        let (key, spec) = match known {
            Some(k) => (k, J::Null),
            None => {
                let k = KEYS.fetch_add(1, Ordering::Relaxed);
                *last = Some((Arc::downgrade(plan), k));
                (k, J::String(xl_engine::ir::wire::plan_json(plan)))
            }
        };
        let done = call("run_plan", json!([key, spec]), vec![xl_arrow::ipc::table_to_ipc(input)?]).and_then(|m| xl_arrow::ipc::table_from_ipc(m.blobs.first().ok_or("the host gave no table")?));
        if done.is_err() {
            // The plan is sent again after a failure (the host may not have kept it).
            *last = None;
        }
        done
    }
}

/// Stores whose Parquet files the host's Polars writes and reads.
#[derive(Default)]
pub struct HostBackend {
    executor: HostExecutor,
}

impl Backend for HostBackend {
    fn executor(&self) -> &dyn Executor {
        &self.executor
    }

    fn write_parquet(&self, p: &Path, frame: &Frame, note: Option<&str>) -> Result<(), String> {
        call("write_parquet", json!([path(p), xl_engine::store::NOTE_KEY, note]), vec![xl_arrow::ipc::frame_to_ipc(frame)?]).map(|_| ())
    }

    fn read_parquet(&self, p: &Path) -> Result<Frame, String> {
        let m = call("read_parquet", json!([path(p)]), Vec::new())?;
        xl_arrow::ipc::frame_from_ipc(m.blobs.first().ok_or("the host gave no frame")?)
    }

    fn read_note(&self, p: &Path) -> Result<Option<String>, String> {
        let m = call("read_note", json!([path(p), xl_engine::store::NOTE_KEY]), Vec::new())?;
        Ok(m.json.get("ok").and_then(J::as_str).map(str::to_string))
    }
}

pub fn backend() -> Box<dyn Backend> {
    Box::new(HostBackend::default())
}

/// The check of a port, run by the host's Python (see `xl_engine::port::Check`).
pub fn check_port(dest: &Path) -> Result<Vec<(String, String, String)>, String> {
    let m = call("check_port", json!([path(dest)]), Vec::new())?;
    Ok(m.json["ok"].as_array().into_iter().flatten().filter_map(|i| Some((i.get(0)?.as_str()?.to_string(), i.get(1)?.as_str()?.to_string(), i.get(2)?.as_str()?.to_string()))).collect())
}

/// The version of the host's Polars.
pub fn polars_version() -> String {
    call("polars_version", json!([]), Vec::new()).ok().and_then(|m| m.json.get("ok").and_then(J::as_str).map(str::to_string)).unwrap_or_else(|| "unknown".into())
}

/// How the host names paths (set by it at the start, see `lib.rs`): where its drives
/// are, its way of writing a path, its working directory.
pub struct HostPaths {
    /// `windows`: the host's drive `C:` is `/c` here; otherwise its `/` is `/`.
    pub windows: bool,
    /// The host's working directory, as seen here.
    pub cwd: String,
}

thread_local! {
    pub static PATHS: RefCell<HostPaths> = const { RefCell::new(HostPaths { windows: false, cwd: String::new() }) };
}

/// A path as the host writes it (`C:\Users\x`, `./a.xlsx`), as seen here (`/c/Users/x`).
pub fn inbound(text: &str) -> std::path::PathBuf {
    PATHS.with(|p| {
        let p = p.borrow();
        let b = text.as_bytes();
        let seen = if p.windows && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            format!("/{}/{}", (b[0] as char).to_ascii_lowercase(), text[2..].replace('\\', "/").trim_start_matches('/'))
        } else if p.windows {
            text.replace('\\', "/")
        } else {
            text.to_string()
        };
        if seen.starts_with('/') || p.cwd.is_empty() {
            std::path::PathBuf::from(seen)
        } else {
            std::path::Path::new(&p.cwd).join(seen)
        }
    })
}

/// A path as seen here, as the host writes it.
pub fn outbound(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    PATHS.with(|p| {
        let b = text.as_bytes();
        if p.borrow().windows && b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
            format!("{}:\\{}", (b[1] as char).to_ascii_uppercase(), text.get(3..).unwrap_or("").replace('/', "\\"))
        } else {
            text
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(windows: bool, cwd: &str) {
        PATHS.with(|p| *p.borrow_mut() = HostPaths { windows, cwd: cwd.into() });
    }

    #[test]
    fn windows_paths_go_in_and_come_out_as_windows_writes_them() {
        host(true, "/c/Users/me");
        assert_eq!(inbound(r"C:\Users\me\Plan.xlsx"), Path::new("/c/Users/me/Plan.xlsx"));
        assert_eq!(inbound(r"d:/data/a.xlsx"), Path::new("/d/data/a.xlsx"));
        assert_eq!(inbound(r"sub\a.xlsx"), Path::new("/c/Users/me/sub/a.xlsx"));
        assert_eq!(inbound("C:\\"), Path::new("/c/"));
        assert_eq!(outbound(Path::new("/c/Users/me/Plan.xlsx.store")), r"C:\Users\me\Plan.xlsx.store");
        assert_eq!(outbound(Path::new("/d")), r"D:\");
        assert_eq!(outbound(&inbound(r"E:\x\y.parquet")), r"E:\x\y.parquet");
    }

    #[test]
    fn other_paths_stay_as_they_are() {
        host(false, "/home/me");
        assert_eq!(inbound("/tmp/a.xlsx"), Path::new("/tmp/a.xlsx"));
        assert_eq!(inbound("a.xlsx"), Path::new("/home/me/a.xlsx"));
        assert_eq!(outbound(Path::new("/c/x")), "/c/x");
    }
}
