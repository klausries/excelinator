//! xlcells as one WebAssembly module (`wasm32-wasip1`): reading and writing workbooks,
//! the formula engine, regions, stores and the MCP server, for any processor.
//!
//! The host is Python (`xlcells._wasm`, with `wasmtime`): it gives the module the files
//! of the machine (WASI, a drive `C:` as `/c`), runs the engine's plans on its Polars and
//! reads and writes Parquet files (see `host`). Nothing of this is built on the machine
//! that runs it.
//!
//! The module's functions:
//!
//! * `xl_alloc(len) -> ptr`, `xl_free(ptr, len)`: memory for messages.
//! * `xl_call(ptr, len) -> ptr << 32 | len`: one call, a message (see `rpc`)
//!   `{"fn": name, "args": [...]}` in, `{"ok": value}` or `{"err": text}` out; the
//!   message in is the module's from then on, the one out the host frees.
//!
//! The names are those of the Python modules this replaces (`_xlcells`, `_engine`),
//! objects are held here under a number (`writer_new`, `book_new`, `looker_new`, then
//! `<kind>_call` with the number, the method and its arguments; `drop`), and `mcp_*`
//! run the server.

#![recursion_limit = "256"]

mod engine;
mod host;
mod rpc;
mod slim;

use rpc::{In, Msg, Out};
use serde_json::{json, Value as J};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

enum Object {
    Writer(xl_compat::Workbook),
    Book(engine::Book),
    Looker(engine::Looker),
}

thread_local! {
    static OBJECTS: RefCell<HashMap<u32, Object>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
    /// The server, once `mcp_launch` has said how it runs.
    static SERVER: RefCell<Option<xl_mcp::Launch>> = const { RefCell::new(None) };
}

fn keep(o: Object) -> J {
    let n = NEXT.with(|n| {
        let k = n.get();
        n.set(k + 1);
        k
    });
    OBJECTS.with(|m| m.borrow_mut().insert(n, o));
    json!(n)
}

fn with_object<T>(a: &In, f: impl FnOnce(&mut Object, &In) -> Result<T, String>) -> Result<T, String> {
    let n = a.u32(0)?;
    // The rest of the arguments are the method's.
    let rest = In { args: a.args.iter().skip(1).cloned().collect(), blobs: a.blobs.clone() };
    OBJECTS.with(|m| match m.borrow_mut().get_mut(&n) {
        Some(o) => f(o, &rest),
        None => Err(format!("no object {n} (it was dropped, or the module was started anew)")),
    })
}

/// The server's way of running (stdio or HTTP) and its configuration.
fn server<T>(f: impl FnOnce(&xl_mcp::Launch) -> T) -> Result<T, String> {
    SERVER.with(|s| s.borrow().as_ref().map(f).ok_or_else(|| "the server has not been launched (mcp_launch)".to_string()))
}

fn dispatch(name: &str, a: &In, out: &mut Out) -> Result<J, String> {
    match name {
        // The host says how it names paths, before anything else.
        "init" => {
            let windows = a.flag(0, false);
            host::PATHS.with(|p| p.borrow_mut().windows = windows);
            let cwd = host::inbound(a.str(1)?).to_string_lossy().into_owned();
            host::PATHS.with(|p| p.borrow_mut().cwd = cwd);
            xl_mcp::set_backend(host::backend);
            xl_mcp::set_paths(xl_mcp::Paths { inbound: host::inbound, outbound: host::outbound });
            xl_mcp::set_port_check(host::check_port);
            Ok(json!({ "version": env!("CARGO_PKG_VERSION") }))
        }
        "read" => slim::read(a.bytes(0)?, out),
        "date_serial" => slim::date_serial(a),
        "translate_formula" => slim::translate_formula(a),
        "engine_info" => Ok(json!(["wasm", host::polars_version()])),
        "drop" => {
            OBJECTS.with(|m| m.borrow_mut().remove(&a.u32(0).unwrap_or(0)));
            Ok(J::Null)
        }
        "writer_new" => Ok(keep(Object::Writer(xl_compat::Workbook::new()))),
        // Calls of a writer gathered by the host: [[method, [args]], …]; their results.
        "writer_call" => with_object(a, |o, rest| {
            let Object::Writer(wb) = o else { return Err("not a writer".into()) };
            let mut results = Vec::new();
            for c in rest.arg(0).as_array().into_iter().flatten() {
                let method = c[0].as_str().unwrap_or("");
                let args = In { args: c[1].as_array().cloned().unwrap_or_default(), blobs: rest.blobs.clone() };
                results.push(match method {
                    "to_bytes" => out.bytes(wb.to_bytes().map_err(|e| e.to_string())?),
                    "to_bytes_onto" => {
                        let (data, kept) = wb.to_bytes_onto(args.bytes(0)?.to_vec()).map_err(|e| e.to_string())?;
                        json!([out.bytes(data), kept])
                    }
                    _ => slim::writer_call(wb, method, &args)?,
                });
            }
            Ok(J::Array(results))
        }),
        "book_new" => Ok(keep(Object::Book(engine::Book::new(a.bytes(0)?)?))),
        "book_call" => with_object(a, |o, rest| {
            let Object::Book(b) = o else { return Err("not a book".into()) };
            let method = rest.str(0)?.to_string();
            b.call(&method, &In { args: rest.args[1..].to_vec(), blobs: rest.blobs.clone() }, out)
        }),
        "looker_new" => Ok(keep(Object::Looker(engine::Looker::new(a.bytes(0)?)?))),
        "looker_call" => with_object(a, |o, rest| {
            let Object::Looker(l) = o else { return Err("not a looker".into()) };
            l.looks(rest)
        }),
        "looks" => engine::Looker::new(a.bytes(0)?)?.looks(&In { args: a.args[1..].to_vec(), blobs: Vec::new() }),
        // The server: its arguments; how it runs.
        "mcp_launch" => {
            let args: Vec<String> = a.arg(0).as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
            let launch = xl_mcp::launch(&args)?;
            let answer = match &launch {
                xl_mcp::Launch::Stdio(_) => json!({ "mode": "stdio" }),
                xl_mcp::Launch::Http { addr, token, .. } => json!({ "mode": "http", "addr": addr, "token": token, "max_body": xl_mcp::MAX_BODY }),
            };
            SERVER.with(|s| *s.borrow_mut() = Some(launch));
            Ok(answer)
        }
        "mcp_banner" => server(|l| match l {
            xl_mcp::Launch::Http { token, .. } => json!(xl_mcp::banner(a.str(0).unwrap_or(""), token)),
            xl_mcp::Launch::Stdio(_) => J::Null,
        }),
        "mcp_line" => {
            let line = a.str(0)?;
            server(|l| match l {
                xl_mcp::Launch::Stdio(cfg) | xl_mcp::Launch::Http { cfg, .. } => json!(xl_mcp::answer_line(cfg, line)),
            })
        }
        // One HTTP request: method, address, headers ({name: value}), body.
        "mcp_http" => {
            let (method, url, headers, body) = (a.str(0)?, a.str(1)?, a.arg(2), a.bytes(3)?);
            let reply = server(|l| match l {
                xl_mcp::Launch::Http { cfg, token, .. } => {
                    let header = |name: &str| headers.as_object().and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name))).and_then(|(_, v)| v.as_str().map(str::to_string));
                    Ok(xl_mcp::handle(cfg, token, method, url, &header, body))
                }
                xl_mcp::Launch::Stdio(_) => Err("the server was launched for stdio".to_string()),
            })??;
            Ok(json!({ "status": reply.status, "headers": reply.headers, "body": out.bytes(reply.body) }))
        }
        // The app's interface on a workspace, without a server (as the desktop app has it).
        "app_call" => Ok(json!(xl_mcp::app_call(&host::inbound(a.str(0)?), a.str(1)?, a.str(2)?)?)),
        other => engine::call(other, a, out),
    }
}

#[no_mangle]
pub extern "C" fn xl_alloc(len: u32) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must be those of `xl_alloc` or of an answer of `xl_call`.
#[no_mangle]
pub unsafe extern "C" fn xl_free(ptr: *mut u8, len: u32) {
    drop(Vec::from_raw_parts(ptr, 0, len as usize));
}

/// # Safety
/// `ptr` and `len` must be a message written into memory from `xl_alloc(len)`.
#[no_mangle]
pub unsafe extern "C" fn xl_call(ptr: *mut u8, len: u32) -> u64 {
    let input = Vec::from_raw_parts(ptr, len as usize, len as usize);
    let mut out = Out::default();
    let json = match Msg::decode(&input) {
        Err(e) => json!({ "err": e }),
        Ok(m) => {
            let name = m.json["fn"].as_str().unwrap_or("").to_string();
            let a = In { args: m.json["args"].as_array().cloned().unwrap_or_default(), blobs: m.blobs };
            match dispatch(&name, &a, &mut out) {
                Ok(v) => json!({ "ok": v }),
                Err(e) => json!({ "err": e }),
            }
        }
    };
    let answer = Msg { json, blobs: out.blobs }.encode().into_boxed_slice();
    let len = answer.len() as u64;
    let ptr = Box::into_raw(answer) as *mut u8 as u64;
    (ptr << 32) | len
}
