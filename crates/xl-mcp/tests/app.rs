//! The app's interface, used as the app uses it: an Excel tool is moved into data, seen
//! and changed there on all three levels, and given back as the workbook it was.
use serde_json::{json, Value as J};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use xl_cells::Workbook;

mod common;
use common::{cell, http, sheet};

/// The server program: `XL_MCP` (another build of it, such as `tools/xl-mcp-wasm`, the
/// server on the WebAssembly module), else the one cargo built.
fn program() -> String {
    std::env::var("XL_MCP").unwrap_or_else(|_| env!("CARGO_BIN_EXE_xl-mcp").to_string())
}


/// A report: orders with an amount and a total below them, a note, shares that read
/// the orders, and a second sheet that reads the first.
fn report() -> Workbook {
    let mut wb = Workbook::default();
    sheet(
        &mut wb,
        "Report",
        &[
            "Orders",
            "item | qty | price | amount",
            "a | 1 | 2 | =B3*C3*Rates!$B$1",
            "b | 2 | 2 | =B4*C4*Rates!$B$1",
            "c | 3 | 2 | =B5*C5*Rates!$B$1",
            "Total | - | - | =SUM(D3:D5)",
            "",
            "Prices in EUR",
        ],
    );
    sheet(&mut wb, "Rates", &["Factor | 1", "Gross | =Report!D6*2", "Stamp | =NOW()"]);
    wb
}

struct App {
    child: std::process::Child,
    addr: String,
    dir: std::path::PathBuf,
}

impl App {
    fn start(name: &str) -> App {
        let dir = std::env::temp_dir().join(format!("xl-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = Command::new(program()).args(["--app", dir.to_str().unwrap(), "--port", "0"]).stdout(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        App { child, addr: line.trim().strip_prefix("listening on http://").expect("the address").to_string(), dir }
    }

    /// A call that is expected to work.
    fn call(&self, op: &str, args: J) -> J {
        let (status, answer) = self.ask(op, args);
        assert_eq!(status, 200, "{op}: {answer}");
        answer
    }

    fn ask(&self, op: &str, args: J) -> (u16, J) {
        let (status, body) = http(&self.addr, "POST", &format!("/api/local/{op}"), args.to_string().as_bytes());
        (status, serde_json::from_slice(&body).unwrap_or_else(|_| panic!("{op}: {}", String::from_utf8_lossy(&body))))
    }

    fn rows(&self, store: &str, region: &str) -> J {
        self.call("rows", json!({ "store": store, "region": region }))["rows"].clone()
    }

    /// A single cell of a region, by name.
    fn value(&self, store: &str, region: &str, name: &str) -> J {
        self.rows(store, region).as_array().unwrap().iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("no cell {name}"))["value"].clone()
    }

    fn column(&self, store: &str, region: &str, name: &str) -> Vec<J> {
        self.rows(store, region).as_array().unwrap().iter().map(|r| r[name].clone()).collect()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn an_excel_tool_is_moved_into_data_worked_on_and_given_back() {
    let app = App::start("flow");
    std::fs::write(app.dir.join("report.xlsx"), xl_cells::write_bytes(&report()).unwrap()).unwrap();

    // The app is served, and the workspace lists the file.
    assert_eq!(http(&app.addr, "GET", "/", b"").0, 302);
    let (status, page) = http(&app.addr, "GET", "/app/local/", b"");
    assert!(status == 200 && String::from_utf8_lossy(&page).contains("main.client.dart.js"));
    assert_eq!(http(&app.addr, "GET", "/app/local/main.client.dart.js", b"").0, 200);
    assert_eq!(http(&app.addr, "GET", "/app/local/nope.js", b"").0, 404);
    // A page of another site, open in a browser of this machine, gets no answer.
    let foreign = format!("POST /api/local/workspace HTTP/1.1\r\nHost: {}\r\nOrigin: http://example.com\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}", app.addr);
    assert!(common::raw(&app.addr, &foreign).starts_with("HTTP/1.1 403"));
    assert_eq!(app.call("workspace", json!({}))["files"], json!([{ "name": "report.xlsx", "store": null }]));
    let store = app.call("open", json!({ "file": "report.xlsx" }))["store"].as_str().unwrap().to_string();
    assert_eq!(app.call("workspace", json!({}))["stores"], json!([{ "name": store, "workbook": true }]));
    let s = store.as_str();
    // The file was never calculated: the first run gives its formulas their values.
    assert_eq!(app.call("run", json!({ "store": s }))["run"]["computed"], 5);

    // The data level: tables with their logic, as the notes of the files have it.
    let t = app.call("tables", json!({ "store": s }));
    let names: Vec<&str> = t["tables"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Orders", "Orders totals", "Rates"]);
    let amount = &t["tables"][0]["columns"][3];
    assert_eq!((amount["logic"].as_str(), amount["state"].as_str()), (Some("[@qty]*[@price]*Factor"), Some("calculated")));
    assert_eq!(amount["reads"], json!(["qty", "price", "Rates.Factor"]));
    let stamp = t["tables"][2]["columns"].as_array().unwrap().iter().find(|c| c["name"] == "Stamp").unwrap();
    assert!(stamp["state"] == "kept" && stamp["why"].as_str().unwrap().starts_with("volatile function"), "{stamp}");
    assert_eq!((t["workbook"].as_bool(), t["pending"].as_bool(), &t["undo"]), (Some(true), Some(false), &json!(["run"])));

    // A view: filtered, sorted, summed.
    let view = json!({ "filter": [{ "column": "qty", "op": ">", "value": 1 }], "sort": [{ "column": "qty", "desc": true }] });
    let v = app.call("rows", json!({ "store": s, "region": "Orders", "view": view, "summary": true }));
    assert_eq!((v["total"].as_u64(), v["all"].as_u64(), &v["index"]), (Some(2), Some(3), &json!([2, 1])));
    assert_eq!(v["rows"][0], json!({ "item": "c", "qty": 3, "price": 2, "amount": 6 }));
    assert_eq!(v["summary"]["amount"]["sum"], 10.0);

    // Rows changed in the data: calculated again from the notes alone, the total too.
    let patch = json!({ "set": [{ "row": 0, "column": "qty", "value": 10 }], "insert": { "rows": [{ "item": "d", "qty": 4, "price": 5 }] } });
    let done = app.call("patch", json!({ "store": s, "region": "Orders", "patch": patch }));
    assert_eq!(done["rows"], 4);
    assert_eq!(app.column(s, "Orders", "amount"), vec![json!(20), json!(4), json!(6), json!(20)]);
    assert_eq!(app.value(s, "Orders totals", "Total"), json!(50));
    assert_eq!(app.value(s, "Rates", "Gross"), json!(100));
    assert_eq!(app.call("tables", json!({ "store": s }))["pending"], true);
    // A single cell, by name.
    app.call("patch", json!({ "store": s, "region": "Rates", "patch": { "set": [{ "name": "Factor", "value": 2 }] } }));
    assert_eq!(app.value(s, "Orders totals", "Total"), json!(100));

    // Undone step by step; a change that cannot be made leaves everything as it is.
    assert_eq!(app.call("tables", json!({ "store": s }))["undo"], json!(["rows of Rates", "rows of Orders", "run"]));
    assert_eq!(app.call("undo", json!({ "store": s }))["undone"], "rows of Rates");
    assert_eq!(app.value(s, "Orders totals", "Total"), json!(50));
    let (status, why) = app.ask("patch", json!({ "store": s, "region": "Orders", "patch": { "set": [{ "row": 0, "column": "nope", "value": 1 }] } }));
    assert!(status == 400 && why["error"].as_str().unwrap().contains("the columns are"), "{why}");
    assert_eq!(app.call("tables", json!({ "store": s }))["undo"], json!(["rows of Orders", "run"]));

    // Back to Excel: the workbook as it was built, with the new rows in it.
    let out = app.call("export", json!({ "store": s }));
    assert_eq!((out["file"].as_str(), out["url"].as_str().map(|u| u.ends_with("/files/local/report.new.xlsx"))), (Some("report.new.xlsx"), Some(true)));
    let back = xl_cells::read_bytes(std::fs::read(app.dir.join("report.new.xlsx")).unwrap()).unwrap();
    assert_eq!(cell(&back, "Report", "D6"), "20 =B6*C6*Rates!$B$1");
    assert_eq!(cell(&back, "Report", "D7"), "50 =SUM(D3:D6)");
    assert_eq!(cell(&back, "Report", "A9"), "Prices in EUR");
    assert_eq!(cell(&back, "Rates", "B2"), "100 =Report!D7*2");
    assert_eq!(app.call("tables", json!({ "store": s }))["pending"], false);

    // The model level: new logic, checked, and worked out by the workbook's engine.
    let (status, why) = app.ask("set_logic", json!({ "store": s, "region": "Orders", "name": "amount", "logic": "[@quantity]*2" }));
    assert!(status == 400 && why["error"].as_str().unwrap().contains("its columns are"), "{why}");
    let done = app.call("set_logic", json!({ "store": s, "region": "Orders", "name": "amount", "logic": "[@qty]*[@price]*Factor*2" }));
    assert_eq!(done["run"]["logic_changed"], json!([["Orders", "amount"]]));
    assert_eq!(app.column(s, "Orders", "amount"), vec![json!(40), json!(8), json!(12), json!(40)]);
    assert!(app.call("verify", json!({ "store": s }))["text"].as_str().unwrap().starts_with("Data and workbook agree."));

    // A region gets a name that says what it is; what reads it says the new name.
    let (status, why) = app.ask("rename", json!({ "store": s, "region": "Orders totals", "name": "rates" }));
    assert!(status == 400 && why["error"].as_str().unwrap().contains("already"), "{why}");
    app.call("rename", json!({ "store": s, "region": "Orders totals", "name": "Sums" }));
    let t = app.call("tables", json!({ "store": s }));
    assert_eq!(t["tables"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["Orders", "Sums", "Rates"]);
    let gross = t["tables"][2]["columns"].as_array().unwrap().iter().find(|c| c["name"] == "Gross").unwrap().clone();
    assert_eq!(gross["reads"], json!(["Sums.Total"]));
    assert_eq!(app.value(s, "Sums", "Total"), json!(100));
    assert!(!app.dir.join(s).join("data/Orders totals.parquet").exists());
    assert_eq!(app.call("undo", json!({ "store": s }))["undone"], "name of Orders totals");
    assert_eq!(app.value(s, "Orders totals", "Total"), json!(100));
    // A table that others name in their logic: they name it by its new name.
    app.call("rename", json!({ "store": s, "region": "Orders", "name": "Positions" }));
    let t = app.call("tables", json!({ "store": s }));
    assert_eq!(t["tables"][1]["columns"][0]["logic"], "SUM(Positions[amount])");
    assert_eq!(app.column(s, "Positions", "amount"), vec![json!(40), json!(8), json!(12), json!(40)]);
    assert_eq!(app.call("undo", json!({ "store": s }))["undone"], "name of Orders");

    // The Excel level: the sheet as it lies, and a cell typed into.
    let sheets = app.call("sheets", json!({ "store": s }));
    assert_eq!(sheets["sheets"][0]["regions"][0], json!({ "name": "Orders", "kind": "table", "ref": "A2:D6" }));
    let part = app.call("cells", json!({ "store": s, "sheet": "Report", "top": 5, "rows": 2 }));
    assert_eq!(part["cells"], json!([[5, 0, "d", null, 0], [5, 1, "4", null, 0], [5, 2, "5", null, 0], [5, 3, "40", "=B6*C6*'Rates'!$B$1*2", 0], [6, 0, "Total", null, 0], [6, 3, "100", "=SUM(D3:D6)", 0]]));
    app.call("set_cells", json!({ "store": s, "sheet": "Report", "cells": [{ "cell": "B3", "value": 1 }, { "cell": "D4", "value": "=B4*100" }] }));
    assert_eq!(app.column(s, "Orders", "amount"), vec![json!(4), json!(200), json!(12), json!(40)]);
    // The row with a formula of its own is an exception of its column.
    let t = app.call("tables", json!({ "store": s }));
    assert_eq!(t["tables"][0]["columns"][3]["exceptions"], json!([{ "row": 1, "logic": "[@qty]*100" }]));

    // Cells typed in beside the table are a region of their own; emptied again, it is gone
    // with its file.
    app.call("set_cells", json!({ "store": s, "sheet": "Report", "cells": [{ "cell": "H2", "value": "Limit" }, { "cell": "I2", "value": 500 }] }));
    let names = |t: J| t["tables"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(app.call("tables", json!({ "store": s }))), vec!["Orders", "Orders totals", "Report", "Rates"]);
    assert_eq!(app.value(s, "Report", "Limit"), json!(500));
    let done = app.call("set_cells", json!({ "store": s, "sheet": "Report", "cells": [{ "cell": "H2", "value": null }, { "cell": "I2", "value": null }] }));
    assert_eq!(done["run"]["regions_gone"], json!(["Report"]));
    assert!(!app.dir.join(s).join("data/Report.parquet").exists());
    assert_eq!(names(app.call("tables", json!({ "store": s }))), vec!["Orders", "Orders totals", "Rates"]);

    // Views are kept by name; a region leaves as CSV (a view of it) or as its Parquet file.
    app.call("save_view", json!({ "store": s, "name": "big", "region": "Orders", "view": { "filter": [{ "column": "amount", "op": ">=", "value": 40 }] } }));
    assert_eq!(app.call("tables", json!({ "store": s }))["views"]["big"]["region"], "Orders");
    // Taking a change back leaves the views.
    app.call("patch", json!({ "store": s, "region": "Orders", "patch": { "delete": [0] }, "then": "nothing" }));
    app.call("undo", json!({ "store": s }));
    assert_eq!(app.call("tables", json!({ "store": s }))["views"]["big"]["region"], "Orders");
    let out = app.call("export", json!({ "store": s, "what": "csv", "region": "Orders", "view": { "filter": [{ "column": "amount", "op": ">=", "value": 40 }] } }));
    assert_eq!(std::fs::read_to_string(app.dir.join(out["file"].as_str().unwrap())).unwrap(), "item,qty,price,amount\nb,2,2,200\nd,4,5,40\n");
    let out = app.call("export", json!({ "store": s, "what": "parquet", "region": "Orders" }));
    assert!(std::fs::metadata(app.dir.join(out["file"].as_str().unwrap())).unwrap().len() > 0);
}

#[test]
fn parquet_files_with_their_notes_are_data_to_work_on_without_a_workbook() {
    let app = App::start("plain");
    std::fs::write(app.dir.join("report.xlsx"), xl_cells::write_bytes(&report()).unwrap()).unwrap();
    let store = app.call("open", json!({ "file": "report.xlsx" }))["store"].as_str().unwrap().to_string();
    app.call("run", json!({ "store": store }));
    // The files alone, somewhere else.
    std::fs::create_dir_all(app.dir.join("plain")).unwrap();
    for e in std::fs::read_dir(app.dir.join(&store).join("data")).unwrap().flatten() {
        std::fs::copy(e.path(), app.dir.join("plain").join(e.file_name())).unwrap();
    }
    let stores = app.call("workspace", json!({}))["stores"].clone();
    assert_eq!(stores, json!([{ "name": "plain", "workbook": false }, { "name": store, "workbook": true }]));
    let t = app.call("tables", json!({ "store": "plain" }));
    assert_eq!((t["workbook"].as_bool(), t["tables"].as_array().map(Vec::len)), (Some(false), Some(3)));

    // Changed and calculated again, as in a store.
    app.call("patch", json!({ "store": "plain", "region": "Orders", "patch": { "delete": [0], "set": [{ "row": 2, "column": "price", "value": 10 }] } }));
    assert_eq!(app.column("plain", "Orders", "amount"), vec![json!(4), json!(30)]);
    assert_eq!(app.value("plain", "Orders totals", "Total"), json!(34));
    assert_eq!(app.call("undo", json!({ "store": "plain" }))["undone"], "rows of Orders");
    assert_eq!(app.value("plain", "Orders totals", "Total"), json!(12));
    // What needs the workbook says so.
    for (op, args) in [("sheets", json!({})), ("set_logic", json!({ "region": "Orders", "name": "amount", "logic": "1" })), ("export", json!({}))] {
        let mut args = args;
        args["store"] = json!("plain");
        let (status, why) = app.ask(op, args);
        assert!(status == 400 && why["error"].as_str().unwrap().contains("data alone"), "{op}: {why}");
    }
    // The store it came from is as it was.
    assert_eq!(app.value(&store, "Orders totals", "Total"), json!(12));
}
