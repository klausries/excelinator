//! `xl-mcp`: a Model Context Protocol server through which an agent moves an Excel
//! workbook into a store of tables and logic, works on it and brings it back, without
//! Python.
//!
//! A workbook is opened into a *store* (see `xl_polars::store`): a directory with the
//! data of its regions as Parquet files, a model document and the workbook. The tools
//! read and write the regions as rows, change the logic of calculated columns and
//! cells, run the store (everything recalculated by the formula engine) and give the
//! workbook back in its old layout.
//!
//! Three ways to run it:
//!
//! * `xl-mcp`: JSON-RPC over stdin and stdout, one message per line, for an agent on
//!   the same machine. Paths are paths of that machine.
//! * `xl-mcp --http ADDR --workspace DIR --token SECRET [--public-url URL]`: the
//!   Streamable HTTP transport, for an agent elsewhere (Microsoft Copilot Studio,
//!   ChatGPT). `POST /mcp/SECRET` takes one JSON-RPC message and answers with JSON.
//!   Such an agent sees no files of this machine: workbooks live in the workspace
//!   directory and are named, not addressed by path; they come in and go out through
//!   `/files/SECRET` (a page to upload and download) and `/files/SECRET/<name>` (PUT
//!   and GET). Nothing outside the workspace can be named.
//!   `/app/SECRET/` is the app (see `web/`), `POST /api/SECRET/<call>` its interface
//!   (see `api.rs`).
//! * `xl-mcp --app DIR [--port N]`: the same server for this machine only
//!   (`127.0.0.1`, port 8787), with the app at `http://127.0.0.1:8787/`.
//! * In a chat that shows apps (MCP Apps) the same app appears inside the conversation:
//!   the tool `show_app` names it as a resource (`ui://excelinator/app.html`, one page
//!   with all it needs), and the app's calls come back as the tool `app_call`.
//!   `/app/SECRET/chat.html?store=NAME.store` shows it the way such a chat does.
//! * `--log FILE` (all ways): every tool call (and the client's `initialize`, and each
//!   `resources/read`) is appended to FILE as a line of JSON
//!   (tool, arguments, answer, error or not, milliseconds).

mod api;
pub mod engine;
mod http;
mod tools;

pub use engine::{set_backend, set_paths, set_port_check, MakeBackend, Paths, PortCheck};
pub use http::{banner, handle, Reply, MAX_BODY};

use serde_json::{json, Value as J};
use std::io::Write;
#[cfg(feature = "native")]
use std::io::BufRead;
use std::path::{Path, PathBuf};
use tools::{call, tools, INSTRUCTIONS};

/// Where the server works: anywhere (stdio), or inside a workspace (HTTP).
pub struct Config {
    pub(crate) workspace: Option<PathBuf>,
    /// The address agents and people reach the server at, with the secret: the base of
    /// download links (`https://host/files/SECRET`).
    pub(crate) files_url: Option<String>,
    /// A file every tool call is appended to (`--log FILE`): a line of JSON with the
    /// tool, its arguments, the answer and how long it took.
    log: Option<PathBuf>,
    /// The files of the app are read from here, not from the program (`--web-dir DIR`).
    pub(crate) web_dir: Option<PathBuf>,
}

/// Append a tool call to the log, if one is kept. Failing to write is not an error of
/// the call.
pub(crate) fn log_call(cfg: &Config, tool: &str, arguments: &J, answer: &Result<String, String>, took: std::time::Duration) {
    let Some(path) = &cfg.log else { return };
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (text, error) = match answer {
        Ok(t) => (t, false),
        Err(t) => (t, true),
    };
    let line = json!({ "time": time, "tool": tool, "arguments": arguments, "error": error, "ms": took.as_millis() as u64, "answer": text });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// A name of the workspace: one path component, nothing that leads elsewhere.
pub(crate) fn plain_name(name: &str) -> Result<&str, String> {
    let bad = name.is_empty() || name.starts_with('.') || name.contains(['/', '\\', '\0']) || name == ".." || name.contains("..");
    if bad {
        return Err(format!("{name:?} is not a name of the workspace: give the file's name only, without folders"));
    }
    Ok(name)
}

impl Config {
    /// The place a file or store argument names.
    pub(crate) fn place(&self, arg: &str) -> Result<PathBuf, String> {
        match &self.workspace {
            Some(w) => Ok(w.join(plain_name(arg)?)),
            None => Ok(engine::inbound(arg)),
        }
    }

    /// A store as the agent names it.
    pub(crate) fn shown(&self, dir: &Path) -> String {
        match &self.workspace {
            Some(_) => dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            None => engine::outbound(dir),
        }
    }

    pub(crate) fn workbooks(&self) -> Result<Vec<(String, bool)>, String> {
        let Some(w) = &self.workspace else {
            return Err("this server works on paths of its machine: name the Excel file by its path in open_workbook".into());
        };
        let mut out = Vec::new();
        for e in std::fs::read_dir(w).map_err(|e| format!("cannot read the workspace: {e}"))?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if e.path().is_file() && matches!(ext.as_str(), "xlsx" | "xlsm" | "xls") && !name.starts_with(['.', '~']) {
                out.push((name.clone(), w.join(format!("{name}.store")).join("model.json").exists()));
            }
        }
        out.sort();
        Ok(out)
    }
}

/// The app as a resource a chat can show (MCP Apps), and the type such a page has.
pub(crate) const APP_URI: &str = "ui://excelinator/app.html";
const APP_MIME: &str = "text/html;profile=mcp-app";

/// One call of the app's interface (see `api.rs`) on a workspace directory, without a
/// server: the name of the call and its arguments as JSON, the answer as JSON. This is
/// what a program that has the engine built in (the desktop app) calls.
pub fn app_call(workspace: &Path, op: &str, args: &str) -> Result<String, String> {
    let cfg = Config { workspace: Some(workspace.to_path_buf()), files_url: None, log: None, web_dir: None };
    let args: J = if args.trim().is_empty() { json!({}) } else { serde_json::from_str(args).map_err(|e| format!("not JSON: {e}"))? };
    guarded(|| api::call(&cfg, op, &args)).map(|v| v.to_string())
}

/// A call that does not take the program down with it.
pub(crate) fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("the call failed unexpectedly (a bug in xl-mcp)".to_string()))
}

/// The answer to one message; `None` for a notification.
pub(crate) fn answer(cfg: &Config, msg: &J) -> Option<J> {
    let id = msg.get("id")?.clone();
    let method = msg["method"].as_str().unwrap_or("");
    // Who the client is and what it can (an MCP App needs the extension
    // `io.modelcontextprotocol/ui`): kept in the log with the calls.
    if method == "initialize" || method == "resources/read" {
        log_call(cfg, method, &msg["params"], &Ok(String::new()), std::time::Duration::ZERO);
    }
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
            "serverInfo": { "name": "xl-mcp", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        // The way to port a workbook, for clients that offer prompts to the person.
        "prompts/list" => Ok(json!({ "prompts": [{ "name": "port_workbook", "title": "Port a workbook to Python", "description": "Port an Excel workbook to a Python project on Polars, checked against the workbook, and build the application around it.", "arguments": [{ "name": "workbook", "description": "The Excel file (a path, or its name in the workspace).", "required": true }, { "name": "goal", "description": "What the program is for (what data comes in, what goes out).", "required": false }] }] })),
        "prompts/get" => match msg["params"]["name"].as_str() {
            Some("port_workbook") => {
                let a = &msg["params"]["arguments"];
                let goal = a["goal"].as_str().map(|g| format!(" The application is for: {g}.")).unwrap_or_default();
                let text = format!("Port the workbook {} to Python and Polars with the tools of this server, following this guide; report at the end what check.py says.{goal}\n\n{}", a["workbook"].as_str().unwrap_or("(name it)"), tools::GUIDE);
                Ok(json!({ "description": "Port a workbook to Python", "messages": [{ "role": "user", "content": { "type": "text", "text": text } }] }))
            }
            other => Err(json!({ "code": -32602, "message": format!("no prompt {other:?}") })),
        },
        "tools/list" => Ok(json!({ "tools": tools(cfg.workspace.is_some()) })),
        // The app as a chat shows it (an MCP App): one page, with all it needs inside.
        "resources/list" => Ok(json!({ "resources": [{ "uri": APP_URI, "name": "excelinator", "description": "The app: a store as tables, logic and sheets.", "mimeType": APP_MIME }] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" if msg["params"]["uri"] == APP_URI => Ok(json!({ "contents": [{ "uri": APP_URI, "mimeType": APP_MIME, "text": http::app_page(cfg), "_meta": { "ui": { "prefersBorder": true } } }] })),
        "resources/read" => Err(json!({ "code": -32002, "message": format!("no resource {}", msg["params"]["uri"]) })),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let start = std::time::Instant::now();
            let done = guarded(|| call(cfg, name, &msg["params"]["arguments"]));
            log_call(cfg, name, &msg["params"]["arguments"], &done, start.elapsed());
            Ok(match done {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                Err(why) => json!({ "content": [{ "type": "text", "text": why }], "isError": true }),
            })
        }
        other => Err(json!({ "code": -32601, "message": format!("method {other:?} not found") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

/// The answer to one line of the stdio transport (a JSON-RPC message), if it has one.
pub fn answer_line(cfg: &Config, line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let reply = match serde_json::from_str::<J>(line) {
        Ok(msg) => answer(cfg, &msg),
        Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("not JSON: {e}") } })),
    };
    reply.map(|r| r.to_string())
}

#[cfg(feature = "native")]
fn stdio(cfg: &Config) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Some(reply) = answer_line(cfg, &line) {
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

pub const USAGE: &str = "usage: xl-mcp [--log FILE]                                              (an agent on this machine, over stdin and stdout)
       xl-mcp --app DIR [--port N] [--log FILE]                             (the app in the browser, for this machine only)
       xl-mcp --http ADDR --workspace DIR --token SECRET [--public-url URL] [--log FILE]   (agents elsewhere)";

/// How the server runs, as its arguments say.
pub enum Launch {
    /// JSON-RPC over stdin and stdout.
    Stdio(Config),
    /// HTTP on an address; `token` is part of every address.
    Http { cfg: Config, addr: String, token: String },
}

/// The arguments of the program (without its name) as the way it runs; an error is
/// what to tell the person (the usage, or what is wrong). Paths go through
/// [`engine::inbound`]; the workspace is made if it is not there.
pub fn launch(args: &[String]) -> Result<Launch, String> {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    // Every argument is a flag of these, or the value of one.
    let flags = ["--log", "--app", "--port", "--http", "--workspace", "--token", "--public-url", "--web-dir"];
    if args.iter().enumerate().any(|(i, a)| if flags.contains(&a.as_str()) { args.get(i + 1).is_none() } else { i == 0 || !flags.contains(&args[i - 1].as_str()) }) {
        return Err(USAGE.into());
    }
    let path = |flag: &str| value(flag).map(|v| engine::inbound(&v));
    let (log, web_dir) = (path("--log"), path("--web-dir"));
    let (addr, workspace, token) = match (value("--app"), value("--http")) {
        (None, None) => return Ok(Launch::Stdio(Config { workspace: None, files_url: None, log, web_dir })),
        // The app: this machine only, so the address needs no secret.
        (Some(dir), _) => (format!("127.0.0.1:{}", value("--port").unwrap_or_else(|| "8787".into())), dir, http::LOCAL.to_string()),
        (None, Some(addr)) => {
            let (Some(workspace), Some(token)) = (value("--workspace"), value("--token")) else {
                return Err("--http needs --workspace DIR (where the workbooks live) and --token SECRET (part of the address)".into());
            };
            if token.len() < 16 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return Err("the token must be at least 16 letters, digits, - or _ (it is all that protects the server)".into());
            }
            (addr, workspace, token)
        }
    };
    let workspace = engine::inbound(&workspace);
    std::fs::create_dir_all(&workspace).map_err(|e| format!("cannot use the workspace {}: {e}", engine::outbound(&workspace)))?;
    let public = value("--public-url").unwrap_or_else(|| format!("http://{addr}"));
    let cfg = Config { workspace: Some(workspace), files_url: Some(format!("{}/files/{token}", public.trim_end_matches('/'))), log, web_dir };
    Ok(Launch::Http { cfg, addr, token })
}

/// The program: its arguments say which of the three ways it runs.
#[cfg(feature = "native")]
pub fn cli() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match launch(&args) {
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2)
        }
        Ok(Launch::Stdio(cfg)) => stdio(&cfg),
        Ok(Launch::Http { cfg, addr, token }) => {
            if let Err(why) = http::serve(&cfg, &addr, &token) {
                eprintln!("{why}");
                std::process::exit(1);
            }
        }
    }
}
