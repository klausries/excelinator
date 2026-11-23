//! The server over HTTP: MCP messages, the files of the workspace, the app and its
//! interface.

use crate::{plain_name, Config};
use serde_json::{json, Value as J};
use std::path::Path;

/// Percent-encoding of a file name for a link.
pub(crate) fn encode(name: &str) -> String {
    name.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn decode(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The page of the workspace: its files, and a way to add one.
fn files_page(cfg: &Config, w: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(w).into_iter().flatten().flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with(['.', '~'])).collect();
    names.sort();
    let base = cfg.files_url.clone().unwrap_or_default();
    let list: String = if names.is_empty() { "<li>(none yet)</li>".into() } else { names.iter().map(|n| format!("<li><a href=\"{base}/{}\">{}</a></li>", encode(n), html(n))).collect() };
    format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>xl-mcp workspace</title>\
<style>body{{font:16px system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem}}li{{margin:.3rem 0}}</style>\
<h1>Workbooks</h1><p>Excel files here can be opened by the agent by name. Files it exports appear here too.</p>\
<p><input type=\"file\" id=\"f\" accept=\".xlsx,.xlsm,.xls\"> <button id=\"b\">Upload</button> <span id=\"s\"></span></p><ul>{list}</ul>\
<script>document.getElementById('b').onclick=async()=>{{const f=document.getElementById('f').files[0];const s=document.getElementById('s');if(!f){{s.textContent='Choose a file first.';return}}\
s.textContent='Uploading…';const r=await fetch(location.pathname.replace(/\\/$/,'')+'/'+encodeURIComponent(f.name),{{method:'PUT',body:f}});s.textContent=r.ok?'Done.':'Failed: '+await r.text();if(r.ok)location.reload()}}</script>"
    )
}

const MAX_UPLOAD: u64 = 300 << 20;

/// The token of a server that runs for its own machine only (`--app`).
pub(crate) const LOCAL: &str = "local";

/// A file of the app: its type and content. The app is built from `app/web`
/// (`tools/build_app.sh`); its files are part of the program. With `--web-dir DIR` they
/// are read from there instead (for work on the app: `app/web/build/jaspr`).
fn page(cfg: &Config, name: &str) -> Option<(&'static str, String)> {
    let (file, kind, built_in) = match name {
        "" | "index.html" => ("index.html", "text/html; charset=utf-8", include_str!("../web/index.html")),
        "main.client.dart.js" => ("main.client.dart.js", "text/javascript; charset=utf-8", include_str!("../web/main.client.dart.js")),
        // A page that shows the app as a chat does (see `app_page`), to try that out.
        "chat.html" => ("chat.html", "text/html; charset=utf-8", include_str!("../web/chat.html")),
        "app.css" => ("app.css", "text/css; charset=utf-8", include_str!("../web/app.css")),
        _ => return None,
    };
    let from_disk = cfg.web_dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(file)).ok());
    Some((kind, from_disk.unwrap_or_else(|| built_in.to_string())))
}

/// The app as one page with its style and program inside: what a chat shows in a
/// frame that may load nothing else.
pub(crate) fn app_page(cfg: &Config) -> String {
    let part = |name: &str| page(cfg, name).map(|p| p.1).unwrap_or_default();
    format!("<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>excelinator</title><style>{}</style></head><body><script>{}</script></body></html>", part("app.css"), part("main.client.dart.js").replace("</script", "<\\/script"))
}

/// An answer to a request: its status, headers and body.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    fn new(status: u16, kind: &str, body: impl Into<Vec<u8>>) -> Reply {
        Reply { status, headers: vec![("Content-Type".into(), kind.into())], body: body.into() }
    }

    fn text(status: u16, body: impl Into<String>) -> Reply {
        Reply::new(status, "text/plain; charset=utf-8", body.into())
    }

    fn json(status: u16, body: &J) -> Reply {
        Reply::new(status, "application/json", body.to_string())
    }

    fn with(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// The largest body a request may have.
pub const MAX_BODY: u64 = MAX_UPLOAD;

/// One request of the server (`token` is part of every address; [`LOCAL`] for the app
/// of this machine): its method, its address, a header by name, its body. The answer
/// is worked out here; who listens on the network and sends it is the caller's business
/// (the program: `tiny_http`; built for WebAssembly: the host's Python).
pub fn handle(cfg: &Config, token: &str, method: &str, url: &str, header: &dyn Fn(&str) -> Option<String>, body: &[u8]) -> Reply {
    let workspace = cfg.workspace.clone().expect("the HTTP transport has a workspace");
    let url = url.split('?').next().unwrap_or("");
    let parts: Vec<String> = url.trim_matches('/').split('/').map(decode).collect();
    // The secret is part of the address; anything else is not found.
    let known = parts.len() >= 2 && parts[1] == token && matches!(parts[0].as_str(), "mcp" | "files" | "app" | "api");
    // The app has no secret: it answers this machine's own pages only, not a page
    // of another site that a browser on this machine has open.
    let host = header("Host").unwrap_or_default();
    let foreign = token == LOCAL && (!(host.starts_with("127.0.0.1") || host.starts_with("localhost")) || header("Origin").is_some_and(|o| o != format!("http://{host}")));
    let text_body = || String::from_utf8_lossy(body).into_owned();
    if foreign {
        Reply::text(403, "this server answers its own pages only")
    } else if url == "/" && token == LOCAL {
        Reply::text(302, "").with("Location", &format!("/app/{LOCAL}/"))
    } else if !known {
        Reply::text(404, "not found")
    } else if parts[0] == "app" {
        match page(cfg, parts.get(2).map_or("", String::as_str)) {
            Some((kind, page)) => Reply::new(200, kind, page).with("Cache-Control", "no-store"),
            None => Reply::text(404, "not found"),
        }
    } else if parts[0] == "api" {
        // One call of the app: its name in the address, its arguments as JSON.
        let body = text_body();
        let args = if body.trim().is_empty() { Ok(json!({})) } else { serde_json::from_str::<J>(&body).map_err(|e| format!("not JSON: {e}")) };
        let op = parts.get(2).map_or("", String::as_str);
        let start = std::time::Instant::now();
        let done = args.and_then(|args| {
            let done = crate::guarded(|| crate::api::call(cfg, op, &args));
            crate::log_call(cfg, op, &args, &done.as_ref().map(|v| v.to_string()).map_err(String::clone), start.elapsed());
            done
        });
        match done {
            Ok(v) => Reply::json(200, &v),
            Err(why) => Reply::json(400, &json!({ "error": why })),
        }
    } else if parts[0] == "mcp" {
        if method != "POST" {
            return Reply::text(405, "POST one JSON-RPC message").with("Allow", "POST");
        }
        match serde_json::from_str::<J>(&text_body()) {
            Err(e) => Reply::json(400, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("not JSON: {e}") } })),
            Ok(J::Array(batch)) => {
                let replies: Vec<J> = batch.iter().filter_map(|m| crate::answer(cfg, m)).collect();
                if replies.is_empty() { Reply::text(202, "") } else { Reply::json(200, &J::Array(replies)) }
            }
            Ok(msg) => match crate::answer(cfg, &msg) {
                Some(reply) => Reply::json(200, &reply),
                None => Reply::text(202, ""),
            },
        }
    } else if parts.len() == 2 {
        Reply::new(200, "text/html; charset=utf-8", files_page(cfg, &workspace))
    } else {
        match (plain_name(&parts[2..].join("/")), method) {
            (Err(why), _) => Reply::text(400, why),
            (Ok(_), "PUT") if body.len() as u64 > MAX_UPLOAD => Reply::text(413, "the file is larger than 300 MB"),
            (Ok(name), "PUT") => match std::fs::write(workspace.join(name), body) {
                Ok(()) => Reply::text(200, format!("stored: {name}")),
                Err(e) => Reply::text(500, format!("cannot store {name}: {e}")),
            },
            (Ok(name), "GET") => match std::fs::read(workspace.join(name)) {
                Ok(data) if workspace.join(name).is_file() => {
                    let kind = if name.to_ascii_lowercase().ends_with(".xlsx") { "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" } else { "application/octet-stream" };
                    Reply::new(200, kind, data).with("Content-Disposition", &format!("attachment; filename*=UTF-8''{}", encode(name)))
                }
                _ => Reply::text(404, "not found"),
            },
            _ => Reply::text(405, "GET or PUT"),
        }
    }
}

/// What a server says when it starts to listen.
pub fn banner(addr: &str, token: &str) -> String {
    let mut out = format!("listening on http://{addr}");
    if token == LOCAL {
        out.push_str(&format!("\nthe app: http://{addr}/"));
    }
    out
}

#[cfg(feature = "native")]
pub(crate) fn serve(cfg: &Config, addr: &str, token: &str) -> Result<(), String> {
        use std::io::Read;
    use tiny_http::{Header, Response, Server};
    let server = Server::http(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    // Who started the server may stop reading after the first line: that is no reason to stop.
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", banner(&server.server_addr().to_string(), token)).and_then(|_| out.flush());
    }
    for mut request in server.incoming_requests() {
        let mut body = Vec::new();
        let _ = request.as_reader().take(MAX_UPLOAD + 1).read_to_end(&mut body);
        let said = |name: &str| request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string());
        let reply = handle(cfg, token, request.method().as_str(), request.url(), &said, &body);
        let mut response = Response::from_data(reply.body).with_status_code(reply.status);
        for (k, v) in &reply.headers {
            response.add_header(Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("a valid header"));
        }
        let _ = request.respond(response);
    }
    Ok(())
}
