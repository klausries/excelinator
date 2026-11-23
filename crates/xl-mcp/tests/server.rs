//! The server spoken to as a client would: a workbook opened, changed, run and exported.
use serde_json::{json, Value as J};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use xl_cells::{Sheet, Value, Workbook};

mod common;
use common::http;

/// The server program: `XL_MCP` (another build of it, such as `tools/xl-mcp-wasm`, the
/// server on the WebAssembly module), else the one cargo built.
fn program() -> String {
    std::env::var("XL_MCP").unwrap_or_else(|_| env!("CARGO_BIN_EXE_xl-mcp").to_string())
}


fn sample() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
    wb.sheets.push(Sheet { name: "Model".into(), ..Default::default() });
    let c = &mut wb.cells;
    c.push(0, 0, 0, Value::Text("Orders".into()), None, 0);
    for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
        c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
    }
    for r in 0..3u32 {
        c.push(0, 2 + r, 0, Value::Text(format!("item{r}")), None, 0);
        c.push(0, 2 + r, 1, Value::Number((r + 1) as f64), None, 0);
        c.push(0, 2 + r, 2, Value::Number(2.0), None, 0);
        c.push(0, 2 + r, 3, Value::Number((r + 1) as f64 * 2.0), Some(format!("B{0}*C{0}", r + 3)), 0);
    }
    c.push(1, 0, 0, Value::Text("Factor".into()), None, 0);
    c.push(1, 0, 1, Value::Number(3.0), None, 0);
    c.push(1, 1, 0, Value::Text("Gross".into()), None, 0);
    c.push(1, 1, 1, Value::Number(12.0), Some("SUM(Sales!D3:D5)".into()), 0);
    wb
}

struct Client {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Client {
    fn send(&mut self, method: &str, params: J) -> J {
        self.next += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params });
        writeln!(self.child.stdin.as_mut().unwrap(), "{msg}").unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        let reply: J = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], self.next);
        reply
    }

    /// A tool's text, and whether it is an error.
    fn tool(&mut self, name: &str, args: J) -> (String, bool) {
        let r = self.send("tools/call", json!({ "name": name, "arguments": args }));
        (r["result"]["content"][0]["text"].as_str().unwrap().to_string(), r["result"]["isError"].as_bool().unwrap())
    }
}

#[test]
fn a_workbook_is_opened_changed_run_and_exported() {
    let dir = std::env::temp_dir().join(format!("xl-mcp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("book.xlsx");
    std::fs::write(&file, xl_cells::write_bytes(&sample()).unwrap()).unwrap();
    let mut child = Command::new(program()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client { child, reader, next: 0 };

    let init = c.send("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "xl-mcp");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    // A notification gets no answer; the next answer is that of the next request.
    writeln!(c.child.stdin.as_mut().unwrap(), "{}", json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).unwrap();
    let tools = c.send("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["open_workbook", "describe", "read_region", "change_rows", "write_region", "append_rows", "set_value", "add_sheet", "set_logic", "rename_region", "run", "recalculate", "undo", "read_range", "verify", "export_workbook", "export_region", "polars_code", "export_port", "port_guide", "show_app", "app_call"]);
    // The way to port a workbook, as a prompt for clients that offer prompts.
    assert_eq!(init["result"]["capabilities"]["prompts"], json!({}));
    let prompts = c.send("prompts/list", json!({}));
    assert_eq!(prompts["result"]["prompts"][0]["name"], "port_workbook");
    let prompt = c.send("prompts/get", json!({ "name": "port_workbook", "arguments": { "workbook": "plan.xlsx", "goal": "monthly reports" } }));
    let text = prompt["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("plan.xlsx") && text.contains("monthly reports") && text.contains("# Porting a workbook"), "{text}");
    // The app is a resource a chat can show (an MCP App): one page with all it needs,
    // named by the tool that shows it; the app's own calls are not the agent's.
    let of = |name: &str| tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap().clone();
    assert_eq!(of("show_app")["_meta"]["ui"]["resourceUri"], "ui://excelinator/app.html");
    assert_eq!(of("app_call")["_meta"]["ui"]["visibility"], json!(["app"]));
    assert!(of("describe")["_meta"].is_null());
    assert_eq!(c.send("resources/list", json!({}))["result"]["resources"][0]["mimeType"], "text/html;profile=mcp-app");
    let page = c.send("resources/read", json!({ "uri": "ui://excelinator/app.html" }));
    let html = page["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(html.starts_with("<!doctype html>") && html.contains("<style>") && html.matches("</script>").count() == 1 && !html.contains("src="), "one page, nothing to load");
    assert!(c.send("resources/read", json!({ "uri": "ui://other" }))["error"]["message"].as_str().unwrap().contains("no resource"));

    let (text, err) = c.tool("open_workbook", json!({ "path": file }));
    assert!(!err, "{text}");
    assert!(text.contains("Orders (table Sales!A2:D5): 4 columns, 1 calculated") && text.contains("Model (cells Model!A1:B2): 2 cells, 1 calculated"), "{text}");
    let store = format!("{}.store", file.display());
    // What the app asks through its host: the calls of its interface, by the store's path.
    let (text, err) = c.tool("show_app", json!({ "store": store }));
    assert!(!err && text.contains("2 tables"), "{text}");
    let (text, err) = c.tool("app_call", json!({ "op": "rows", "args": { "store": store, "region": "Orders", "limit": 1 } }));
    assert!(!err && text.contains("\"total\":3"), "{text}");
    assert_eq!(c.tool("app_call", json!({ "op": "workspace" })).0, r#"{"files":[],"stores":[],"files_url":null}"#);
    assert!(c.tool("app_call", json!({ "op": "rows", "args": { "store": store, "region": "None" } })).1);
    let (text, _) = c.tool("describe", json!({ "store": store, "region": "Orders" }));
    assert!(text.contains("amount: Int64  = [@qty]*[@price]"), "{text}");
    let (text, _) = c.tool("describe", json!({ "store": store, "region": "Model" }));
    assert!(text.contains("Gross [B2]: Int64  = SUM(Orders[amount])"), "{text}");

    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "Orders", "limit": 2 }));
    let rows: J = serde_json::from_str(&text).unwrap();
    assert_eq!(rows["total_rows"], 3);
    assert_eq!(rows["rows"], json!([{ "item": "item0", "qty": 1, "price": 2, "amount": 2 }, { "item": "item1", "qty": 2, "price": 2, "amount": 4 }]));

    // New data and new logic, then run.
    let (text, err) = c.tool("write_region", json!({ "store": store, "region": "Orders", "rows": [
        { "item": "a", "qty": 1, "price": 2.5 }, { "item": "b", "qty": 2, "price": 2.5 }, { "item": "c", "qty": 3, "price": 2.5 }, { "item": "d", "qty": 4, "price": 10 }] }));
    assert!(!err && text.contains("4 rows written"), "{text}");
    let (text, err) = c.tool("set_logic", json!({ "store": store, "region": "Orders", "name": "amount", "logic": "=[@qty]*[@price]*Factor" }));
    assert!(!err, "{text}");
    // Before the run the data differs from the workbook, afterwards not.
    let (text, _) = c.tool("verify", json!({ "store": store }));
    assert!(text.contains("differences between data and workbook") && text.contains("Orders: the data has 4 rows, the workbook 3"), "{text}");
    let (text, err) = c.tool("run", json!({ "store": store }));
    assert!(!err && text.contains("logic taken into the workbook: Orders[amount]") && text.contains("Orders: 4 rows") && !text.contains("without a value"), "{text}");
    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "Model" }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["rows"], json!([{ "name": "Factor", "cell": "B1", "value": 3 }, { "name": "Gross", "cell": "B2", "value": 165 }]));

    let (text, _) = c.tool("verify", json!({ "store": store }));
    assert_eq!(text, "Data and workbook agree.");
    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "Orders", "columns": ["item"], "limit": 1 }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["rows"], json!([{ "item": "a" }]));
    // The workbook comes back with data, formulas and its layout.
    let out = dir.join("out.xlsx");
    let (text, err) = c.tool("export_workbook", json!({ "store": store, "path": out }));
    assert!(!err, "{text}");
    let back = xl_cells::read_bytes(std::fs::read(&out).unwrap()).unwrap();
    assert!(back.cells.formula.iter().any(|f| f.as_deref() == Some("B6*C6*'Model'!$B$1")));
    assert!(back.cells.formula.iter().any(|f| f.as_deref() == Some("SUM(Sales!D3:D6)")));

    // Mistakes are answered with what would be right.
    let (text, err) = c.tool("read_region", json!({ "store": store, "region": "Order" }));
    assert!(err && text.contains("the regions are"), "{text}");
    let (text, err) = c.tool("set_logic", json!({ "store": store, "region": "Orders", "name": "amount", "logic": "[@quantity]*2" }));
    assert!(err && text.contains("its columns are"), "{text}");
    let (text, err) = c.tool("set_logic", json!({ "store": store, "region": "Orders", "name": "amount", "logic": "[@qty]*Faktor" }));
    assert!(err && text.contains("the cells that can be written so are [\"Factor\", \"Gross\"]"), "{text}");
    // Single values and rows, one at a time.
    let (text, err) = c.tool("set_value", json!({ "store": store, "region": "Model", "name": "Factor", "value": 2 }));
    assert!(!err, "{text}");
    let (text, err) = c.tool("append_rows", json!({ "store": store, "region": "Orders", "rows": [{ "item": "e", "qty": 1, "price": 0.5 }] }));
    assert!(!err && text.contains("5 rows now (1 added)"), "{text}");
    let (text, _) = c.tool("export_workbook", json!({ "store": store, "path": out }));
    // Going back to Excel runs what was changed first.
    assert!(text.contains("replaced") && text.contains("the store was run first"), "{text}");
    let (text, _) = c.tool("verify", json!({ "store": store }));
    assert!(text.starts_with("Data and workbook agree.") && !text.contains("changed since the last run"), "{text}");
    let (text, err) = c.tool("run", json!({ "store": store }));
    assert!(!err, "{text}");
    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "Model" }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["rows"], json!([{ "name": "Factor", "cell": "B1", "value": 2 }, { "name": "Gross", "cell": "B2", "value": 111 }]));
    let (text, _) = c.tool("read_range", json!({ "store": store, "sheet": "sales", "range": "C7:D7" }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["cells"], json!([{ "cell": "C7", "value": 0.5 }, { "cell": "D7", "value": 1, "formula": "=B7*C7*'Model'!$B$1" }]));
    let (text, err) = c.tool("set_value", json!({ "store": store, "region": "Orders", "name": "qty", "value": 2 }));
    assert!(err && text.contains("is a table"), "{text}");
    let (text, err) = c.tool("polars_code", json!({ "store": store }));
    assert!(!err && text.contains("import polars as pl"), "{text}");
    let unknown = c.send("nope", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);

    drop(c.child.stdin.take());
    c.child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_remote_agent_works_through_http_inside_the_workspace() {
    let dir = std::env::temp_dir().join(format!("xl-mcp-http-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let token = "test-secret-0123456789";
    let mut child = Command::new(program()).args(["--http", "127.0.0.1:0", "--workspace", dir.to_str().unwrap(), "--token", token]).stdout(Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let addr = line.trim().strip_prefix("listening on http://").expect("the address").to_string();
    let mcp = format!("/mcp/{token}");
    let mut next = 0;
    let mut tool = |name: &str, args: J| -> (String, bool) {
        next += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": next, "method": "tools/call", "params": { "name": name, "arguments": args } });
        let (status, body) = http(&addr, "POST", &mcp, msg.to_string().as_bytes());
        assert_eq!(status, 200);
        let r: J = serde_json::from_slice(&body).unwrap();
        (r["result"]["content"][0]["text"].as_str().unwrap().to_string(), r["result"]["isError"].as_bool().unwrap())
    };

    // Without the secret there is nothing; a notification is taken without an answer.
    assert_eq!(http(&addr, "POST", "/mcp/wrong-secret-0123456789", b"{}").0, 404);
    assert_eq!(http(&addr, "GET", "/files/nope", b"").0, 404);
    assert_eq!(http(&addr, "POST", &mcp, json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string().as_bytes()).0, 202);
    assert_eq!(http(&addr, "GET", &mcp, b"").0, 405);

    // A workbook comes in through the files address.
    let (text, _) = tool("list_workbooks", json!({}));
    assert!(text.contains("No Excel files") && text.contains(&format!("/files/{token}")), "{text}");
    let (status, _) = http(&addr, "PUT", &format!("/files/{token}/my%20book.xlsx"), &xl_cells::write_bytes(&sample()).unwrap());
    assert_eq!(status, 200);
    let (text, _) = tool("list_workbooks", json!({}));
    assert_eq!(text, "my book.xlsx");
    let (page_status, page) = http(&addr, "GET", &format!("/files/{token}"), b"");
    assert!(page_status == 200 && String::from_utf8_lossy(&page).contains("my book.xlsx"));

    let (text, err) = tool("open_workbook", json!({ "path": "my book.xlsx" }));
    assert!(!err && text.contains("store: my book.xlsx.store") && text.contains("Orders (table Sales!A2:D5)"), "{text}");
    let store = "my book.xlsx.store";
    let (text, err) = tool("write_region", json!({ "store": store, "region": "Model", "rows": [{ "name": "Factor", "value": 10 }] }));
    assert!(!err, "{text}");
    let (text, err) = tool("set_logic", json!({ "store": store, "region": "Orders", "name": "amount", "logic": "[@qty]*[@price]*Factor" }));
    assert!(!err, "{text}");
    let (text, err) = tool("run", json!({ "store": store }));
    assert!(!err && !text.contains("without a value"), "{text}");
    let (text, _) = tool("read_region", json!({ "store": store, "region": "Model" }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["rows"], json!([{ "name": "Factor", "cell": "B1", "value": 10 }, { "name": "Gross", "cell": "B2", "value": 120 }]));

    // The workbook goes out through a link.
    let (text, err) = tool("export_workbook", json!({ "store": store, "path": "result.xlsx" }));
    assert!(!err && text.contains(&format!("/files/{token}/result.xlsx")), "{text}");
    let (status, data) = http(&addr, "GET", &format!("/files/{token}/result.xlsx"), b"");
    assert_eq!(status, 200);
    let back = xl_cells::read_bytes(data).unwrap();
    assert!(back.cells.formula.iter().any(|f| f.as_deref() == Some("B3*C3*'Model'!$B$1")));

    // The workbook as a Python project: written in the workspace, and as a zip to download.
    let (guide, err) = tool("port_guide", json!({}));
    assert!(!err && guide.contains("# Porting a workbook") && guide.contains("export_port"), "{guide}");
    let (layout, _) = tool("port_guide", json!({ "topic": "layout" }));
    assert!(layout.starts_with("# The files of a port"), "{layout}");
    let (text, err) = tool("export_port", json!({ "store": store }));
    assert!(!err && text.contains("port written: my book-port") && text.contains("worked out by Polars alone"), "{text}");
    assert!(text.contains(&format!("/files/{token}/my%20book-port.zip")), "{text}");
    let (status, zip) = http(&addr, "GET", &format!("/files/{token}/my%20book-port.zip"), b"");
    assert!(status == 200 && zip.starts_with(b"PK"), "{status}");
    for name in ["my book-port/port/calc.py", "my book-port/check.py", "my book-port/data/Orders.parquet", "my book-port/expected/Orders.parquet", "my book-port/logic.json"] {
        assert!(zip.windows(name.len()).any(|w| w == name.as_bytes()), "{name} in the zip");
    }
    let (text, err) = tool("export_port", json!({ "store": store }));
    assert!(err && text.contains("not empty"), "a port is not overwritten unasked: {text}");
    let (text, err) = tool("export_port", json!({ "store": store, "replace": true }));
    assert!(!err, "{text}");
    let (text, err) = tool("export_port", json!({ "store": store, "path": "../elsewhere" }));
    assert!(err && text.contains("not a name of the workspace"), "{text}");

    // Nothing outside the workspace can be named.
    for bad in ["../secret.xlsx", "/etc/passwd", "a/b.xlsx", ".hidden.xlsx"] {
        let (text, err) = tool("open_workbook", json!({ "path": bad }));
        assert!(err && text.contains("not a name of the workspace"), "{bad}: {text}");
        let (text, err) = tool("export_workbook", json!({ "store": store, "path": bad }));
        assert!(err, "{bad}: {text}");
    }
    let (text, err) = tool("describe", json!({ "store": "../x.store" }));
    assert!(err && text.contains("not a name of the workspace"), "{text}");
    assert_eq!(http(&addr, "PUT", &format!("/files/{token}/..%2Fevil.xlsx"), b"x").0, 400);
    let (text, err) = tool("open_workbook", json!({ "path": "missing.xlsx" }));
    assert!(err && text.contains("there are [\"my book.xlsx\", \"result.xlsx\"]"), "{text}");

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sheets_built_alike_are_one_table_and_calls_are_logged() {
    let dir = std::env::temp_dir().join(format!("xl-mcp-stack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut wb = Workbook::default();
    for (s, month) in ["Jan", "Feb"].iter().enumerate() {
        let s = s as u32;
        wb.sheets.push(Sheet { name: month.to_string(), ..Default::default() });
        for (k, h) in ["item", "qty", "amount"].iter().enumerate() {
            wb.cells.push(s, 0, k as u32, Value::Text(h.to_string()), None, 0);
        }
        for r in 0..3u32 {
            wb.cells.push(s, 1 + r, 0, Value::Text(format!("i{r}")), None, 0);
            wb.cells.push(s, 1 + r, 1, Value::Number((r + 1 + s * 10) as f64), None, 0);
            wb.cells.push(s, 1 + r, 2, Value::Number(((r + 1 + s * 10) * 2) as f64), Some(format!("B{}*2", r + 2)), 0);
        }
    }
    let file = dir.join("months.xlsx");
    std::fs::write(&file, xl_cells::write_bytes(&wb).unwrap()).unwrap();
    let log = dir.join("calls.jsonl");
    let mut child = Command::new(program()).arg("--log").arg(&log).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client { child, reader, next: 0 };
    let (text, err) = c.tool("open_workbook", json!({ "path": file }));
    assert!(!err && text.contains("1 stacks") && text.contains("stack: sheets Jan, Feb; columns item, qty, amount"), "{text}");
    let store = dir.join("months.xlsx.store");
    let (text, _) = c.tool("describe", json!({ "store": store, "region": "stack" }));
    assert!(text.contains("sheet \"Feb\": region Feb (table Feb!A1:C4)") && text.contains("amount: Int64  = [@qty]*2"), "{text}");
    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "stack", "offset": 3, "limit": 1 }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap(), json!({ "total_rows": 6, "offset": 3, "rows": [{ "sheet": "Feb", "item": "i0", "qty": 11, "amount": 22 }] }));
    // One more sheet, its rows written through the stack.
    let (text, err) = c.tool("add_sheet", json!({ "store": store, "like": "Feb", "name": "Mar" }));
    assert!(!err && text.contains("Its regions: Mar"), "{text}");
    let (text, err) = c.tool("write_region", json!({ "store": store, "region": "stack", "rows": [{ "sheet": "Mar", "item": "x", "qty": 4 }, { "sheet": "Mar", "item": "y", "qty": 5 }, { "sheet": "Mar", "item": "z", "qty": 6 }] }));
    assert!(!err, "{text}");
    let (text, err) = c.tool("run", json!({ "store": store }));
    assert!(!err && text.contains("9 of 9 formula cells calculated"), "{text}");
    let (text, _) = c.tool("read_region", json!({ "store": store, "region": "stack", "offset": 6 }));
    assert_eq!(serde_json::from_str::<J>(&text).unwrap()["rows"], json!([{ "sheet": "Mar", "item": "x", "qty": 4, "amount": 8 }, { "sheet": "Mar", "item": "y", "qty": 5, "amount": 10 }, { "sheet": "Mar", "item": "z", "qty": 6, "amount": 12 }]));
    let (text, err) = c.tool("write_region", json!({ "store": store, "region": "stack", "rows": [{ "sheet": "Apr", "item": "x", "qty": 4 }] }));
    assert!(err && text.contains("add_sheet"), "{text}");
    drop(c.child.stdin.take());
    c.child.wait().unwrap();
    // Every call is a line of the log.
    let lines: Vec<J> = std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.iter().map(|l| l["tool"].as_str().unwrap()).collect::<Vec<_>>(), vec!["open_workbook", "describe", "read_region", "add_sheet", "write_region", "run", "read_region", "write_region"]);
    assert_eq!((lines[3]["arguments"]["name"].as_str(), lines[3]["error"].as_bool(), lines[7]["error"].as_bool()), (Some("Mar"), Some(false), Some(true)));
    assert!(lines[5]["answer"].as_str().unwrap().contains("formula cells calculated"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_agent_changes_rows_reads_a_part_takes_a_change_back_and_goes_back_to_excel() {
    let dir = std::env::temp_dir().join(format!("xl-mcp-rows-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("book.xlsx");
    std::fs::write(&file, xl_cells::write_bytes(&sample()).unwrap()).unwrap();
    let mut child = Command::new(program()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut c = Client { child, reader, next: 0 };
    let store = format!("{}.store", file.display());
    let mut ok = |name: &str, args: J| -> String {
        let (text, err) = c.tool(name, args);
        assert!(!err, "{name}: {text}");
        text
    };
    let json = |text: String| -> J { serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}")) };
    ok("open_workbook", json!({ "path": file }));

    // Some rows changed: worked out again from the data alone, the cell that sums them too.
    let done = json(ok("change_rows", json!({ "store": store, "region": "Orders", "set": [{ "row": 0, "column": "qty", "value": 10 }], "insert": { "rows": [{ "item": "new", "qty": 1, "price": 1 }] } })));
    assert_eq!((done["rows"].as_u64(), done["then"]["recalculate"]["kept"].as_array().map(Vec::len)), (Some(4), Some(0)));
    let part = json(ok("read_region", json!({ "store": store, "region": "Orders", "filter": [{ "column": "amount", "op": ">", "value": 4 }], "sort": [{ "column": "amount", "desc": true }] })));
    assert_eq!((&part["total_rows"], &part["index"], &part["rows"][0]["amount"]), (&json!(2), &json!([0, 2]), &json!(20)));
    let gross = |text: String| json(text)["rows"].as_array().unwrap().iter().find(|r| r["name"] == "Gross").unwrap()["value"].clone();
    assert_eq!(gross(ok("read_region", json!({ "store": store, "region": "Model" }))), json!(31));

    // A part of a table as CSV; the change taken back; and the whole store again.
    let out = dir.join("part.csv");
    ok("export_region", json!({ "store": store, "region": "Orders", "what": "csv", "path": out, "filter": [{ "column": "qty", "op": "<", "value": 3 }] }));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "item,qty,price,amount\nitem1,2,2,4\nnew,1,1,1\n");
    assert_eq!(json(ok("undo", json!({ "store": store })))["undone"], "rows of Orders");
    assert_eq!(gross(ok("read_region", json!({ "store": store, "region": "Model" }))), json!(12));
    assert_eq!(json(ok("recalculate", json!({ "store": store })))["recalculate"]["done"], 2);

    // Back to Excel: what changed is run first, and the workbook is built as it was.
    ok("change_rows", json!({ "store": store, "region": "Orders", "delete": [0], "then": "nothing" }));
    let back = dir.join("back.xlsx");
    assert!(ok("export_workbook", json!({ "store": store, "path": back })).contains("the store was run first"));
    let wb = xl_cells::read_bytes(std::fs::read(&back).unwrap()).unwrap();
    assert_eq!(common::cell(&wb, "Sales", "D4"), "6 =B4*C4");
    assert_eq!(common::cell(&wb, "Sales", "A5"), "");
    assert_eq!(common::cell(&wb, "Model", "B2"), "10 =SUM(Sales!D3:D4)");
    let _ = std::fs::remove_dir_all(&dir);
}
