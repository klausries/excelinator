//! The tools an agent calls, with answers written for it.

use crate::Config;
use serde_json::{json, Value as J};
use std::path::{Path, PathBuf};
use crate::engine::store;

pub(crate) const INSTRUCTIONS: &str = "Excel workbooks as stores of tables and logic. Start with open_workbook (an .xlsx or .xls file, by its path, or by its name where list_workbooks is offered); it makes a store and lists the regions found: tables (columns under a header), cells (single named cells: inputs and results of a model) and blocks (grids). describe with a region shows its columns or cells, their types and, for calculated ones, their logic; rows of a column that are not as the column is (a formula of their own, or a plain value in a calculated column) are listed below it and stay as they are. Tables that are built alike in several sheets (a sheet per month, say) are also listed as a stack: read_region and write_region with the stack's name take them as one table with a column `sheet` in front, set_logic sets a column in all of them, add_sheet makes one more sheet. read_region gives the data as rows (single cells: a row per cell with name and value); write_region replaces it, append_rows adds rows to a table, set_value sets one single cell. The logic of a calculated column or cell is a formula: [@qty] is a column of the same row, Orders[amount] a whole column of a table, and a single cell is written by its name with _ for spaces and signs (the cell `Tax rate` is Tax_rate) or by its address (Model!$B$1); change it with set_logic. read_range shows any cells of a sheet by address. change_rows changes some rows of a table (set, delete, insert) and works the calculated columns out again from the data alone; recalculate does that for the whole store, without the workbook. After changing data or logic call run: every formula is calculated exactly as Excel does (but for what run names: volatile functions such as TODAY keep the value the file had, add-in functions too), and data, model and workbook agree again; the workbook stays built as it was (what lies below a table moves with its rows, sums over it follow). undo takes the last change back. rename_region gives a region a name that says what it is. verify lists cells whose data differs from the workbook (changes not yet run). export_workbook gives the workbook back, built as the original was; export_region writes one region as CSV or as its Parquet file. export_port ports the workbook to Python: a project on Polars that runs without xlcells and is checked against the workbook, with what is not ported yet named (`rest`); read port_guide before porting, and finish the port as it says. polars_code gives the program alone (it needs the xlcells package). show_app shows a store to the person as an app inside the conversation (tables, logic, sheets, to look at and to change), where the host can show apps.";

pub(crate) fn tools(remote: bool) -> J {
    let store = json!({ "type": "string", "description": "The store, as returned by open_workbook." });
    let region = json!({ "type": "string", "description": "The name of a region (see describe)." });
    let (file, out) = if remote {
        ("Name of a workbook in the workspace (see list_workbooks).", "Name for the workbook file in the workspace, e.g. result.xlsx. The answer gives the link to download it.")
    } else {
        ("Path of the Excel file.", "Path to write the .xlsx file to.")
    };
    let mut all = json!([
        { "name": "list_workbooks", "description": "The Excel files that can be opened, and which of them have a store already.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "open_workbook", "description": "Open an Excel file (.xlsx, .xlsm, .xls) into a store and list its regions. An existing store of the file is replaced.",
          "inputSchema": { "type": "object", "properties": { "path": { "type": "string", "description": file } }, "required": ["path"] } },
        { "name": "describe", "description": "Without `region`: the regions of a store (name, kind: table, cells or block, sheet and range, number of columns or cells and how many are calculated). With `region`: every column or cell of it with its data type (`no value`: empty, not calculated yet or an error) and, if calculated, its logic; `offset` and `limit` (default 200) page through a long region.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "offset": { "type": "integer" }, "limit": { "type": "integer" } }, "required": ["store"] } },
        { "name": "read_region", "description": "Rows of a region as JSON objects. A `cells` region gives one row per cell: `name`, `cell` (its address) and `value`. Dates and times are text. `offset` and `limit` (default 100) page through rows; `columns` picks fields of a table (a table with more than 60 gives the first 60 unless told which), or cells of a `cells` region by name. A table can be read in part: `filter` is a list of {column, op, value} that must all hold (op: =, !=, <, <=, >, >=, contains, empty, not empty), `search` a text some column of the row contains, `sort` a list of {column, desc}; with any of these the answer has a list `index` beside `rows`: the number of each row in the table (what change_rows takes).",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "offset": { "type": "integer" }, "limit": { "type": "integer" }, "columns": { "type": "array", "items": { "type": "string" } }, "filter": { "type": "array", "items": { "type": "object" } }, "search": { "type": "string" }, "sort": { "type": "array", "items": { "type": "object" } } }, "required": ["store", "region"] } },
        { "name": "change_rows", "description": "Change some rows of a table without writing it whole: `set` values ([{row, column, value}], rows counted from 0), `delete` rows ([row, …]), `insert` rows ({at, rows: [{column: value}]}; before row `at`, at the end without it). All row numbers are those of the table as it is before the call (describe names a row of its own by its cell in the sheet as of the last run: the row below the header is row 0). Single cells are set by name: set [{name, value}]. Afterwards the calculated columns and cells are worked out again from the data alone (`then`: recalculate, the default; run: by the workbook, exactly as Excel does; nothing). `pending` in the answer says that the workbook has not followed yet: run or export_workbook brings it in line. A row with a plain value in a calculated column keeps it.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "set": { "type": "array", "items": { "type": "object" } }, "delete": { "type": "array", "items": { "type": "integer" } }, "insert": { "type": "object" }, "then": { "type": "string", "enum": ["recalculate", "run", "nothing"] } }, "required": ["store", "region"] } },
        { "name": "write_region", "description": "Replace the data of a region with the given rows (JSON objects by column name). A table: all its rows, not a part; it may get more or fewer rows; calculated columns need not be given. A `cells` region: rows as read_region gives them ({\"name\": …, \"value\": …}), or one row with a field per cell; cells not given keep their value. Call run afterwards.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "rows": { "type": "array", "items": { "type": "object" } } }, "required": ["store", "region", "rows"] } },
        { "name": "append_rows", "description": "Add rows to the end of a table (JSON objects by column name; calculated columns need not be given). Call run afterwards.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "rows": { "type": "array", "items": { "type": "object" } } }, "required": ["store", "region", "rows"] } },
        { "name": "set_value", "description": "Set the value of one single cell of a `cells` region (an input such as a rate). Call run afterwards.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "name": { "type": "string", "description": "The cell's name as describe shows it." }, "value": { "description": "A number, text, true/false or null." } }, "required": ["store", "region", "name", "value"] } },
        { "name": "add_sheet", "description": "Add a sheet to the workbook as a copy of an existing one (layout, formulas and data; the data are then replaced with write_region). For one more sheet of a stack, e.g. a new month. Call run afterwards.",
          "inputSchema": { "type": "object", "properties": { "store": store, "like": { "type": "string", "description": "The sheet to copy." }, "name": { "type": "string", "description": "The name of the new sheet." } }, "required": ["store", "like", "name"] } },
        { "name": "set_logic", "description": "Set the logic of a column or cell: a formula, e.g. [@qty]*[@price] (columns of the same row), SUM(Orders[amount]) (a whole column of a table), Tax_rate*2 (the single cell `Tax rate`: names are written with _ for spaces and signs), Model!$B$1 (any cell by address). The formula is checked at once. Omit `logic` to turn a calculated column or cell into plain data. Call run afterwards.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "name": { "type": "string", "description": "The column or cell." }, "logic": { "type": "string" } }, "required": ["store", "region", "name"] } },
        { "name": "rename_region", "description": "Give a region another name (found regions are often named after their sheet with a number: Sheet1_3). Its data and the logic that names it follow; the workbook itself is not changed.",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "name": { "type": "string", "description": "The new name." } }, "required": ["store", "region", "name"] } },
        { "name": "run", "description": "Bring the store up to date: data and logic go into the workbook, every formula is calculated as Excel does, and data, model and workbook are written anew. Reports what could not be calculated.",
          "inputSchema": { "type": "object", "properties": { "store": store }, "required": ["store"] } },
        { "name": "recalculate", "description": "Work out the calculated columns and cells again from the data alone, without the workbook: fast, and what a database application built on these files does. The answer counts the columns and cells worked out (`done`), lists those it cannot work out that way with the reason (`kept`: they keep their values; run works them out, but for volatile functions such as TODAY and what else it names), and counts those worked out from something kept (`from_kept`).",
          "inputSchema": { "type": "object", "properties": { "store": store }, "required": ["store"] } },
        { "name": "undo", "description": "Take back the last change of the store (rows, values, logic, a name, a run; the answer says which): data, model and workbook are as before it. Can be called again for the change before; the last 20 are kept.",
          "inputSchema": { "type": "object", "properties": { "store": store }, "required": ["store"] } },
        { "name": "read_range", "description": "The cells of a range of a sheet of the store's workbook by address (e.g. sheet `Sales`, range `A1:E10`): value and formula of every cell that holds something. For cells outside the regions, or to see a region as it lies in the sheet. At most 500 cells.",
          "inputSchema": { "type": "object", "properties": { "store": store, "sheet": { "type": "string" }, "range": { "type": "string" } }, "required": ["store", "sheet", "range"] } },
        { "name": "verify", "description": "Check the state of the store: whether data or logic changed since the last run, which cells of the data differ from the workbook, and how many formula cells have no value (not calculated yet, or an error).",
          "inputSchema": { "type": "object", "properties": { "store": store }, "required": ["store"] } },
        { "name": "export_workbook", "description": "Back to Excel: write the store's workbook as an .xlsx file, built as the original was (layout, formats, formulas, what lies around the tables), with the data and logic of now. Data or logic changed since the last run are run first.",
          "inputSchema": { "type": "object", "properties": { "store": store, "path": { "type": "string", "description": out } }, "required": ["store", "path"] } },
        { "name": "export_region", "description": "Write one region as a file: `csv` (the rows of a `filter`/`search`/`sort`, if given) or `parquet` (its file as it is, with the note that holds its logic).",
          "inputSchema": { "type": "object", "properties": { "store": store, "region": region, "what": { "type": "string", "enum": ["csv", "parquet"] }, "path": { "type": "string", "description": "Where to write the file (a path; with a workspace, a name in it). Without it the file is named after store and region." }, "filter": { "type": "array", "items": { "type": "object" } }, "search": { "type": "string" }, "sort": { "type": "array", "items": { "type": "object" } } }, "required": ["store", "region", "what"] } },
        { "name": "polars_code", "description": "A Python program (Polars, with the xlcells package) that loads the store's workbook, works out every calculated column and cell and checks the results against the workbook. accuracy: `plain` (Polars expressions wherever a translation exists, default) or `exact` (the formula engine's own translation written out as Polars code, which holds for other data and other values of the cells it reads; its values where that cannot be written out).",
          "inputSchema": { "type": "object", "properties": { "store": store, "accuracy": { "type": "string", "enum": ["plain", "exact"] } }, "required": ["store"] } },
        { "name": "export_port", "description": "Port the workbook to Python: a project on Polars (its inputs, what the workbook calculates from them, the program port/calc.py, check.py, run.py, tests) that runs with Polars alone and is checked against the workbook. Every calculated column and cell is ported as `polars` (readable), `plan` (the engine's translation written out) or `rest` (not yet: yours to port; port_guide says how). The store is run first. `path`: the project's folder (default: beside the store, named after it); a folder that is not empty is replaced only with `replace`. With a workspace the answer also gives a zip of the project to download.",
          "inputSchema": { "type": "object", "properties": { "store": store, "path": { "type": "string", "description": if remote { "Name of the project's folder in the workspace." } else { "Path of the project's folder." } }, "accuracy": { "type": "string", "enum": ["plain", "exact"] }, "replace": { "type": "boolean" } }, "required": ["store"] } },
        { "name": "port_guide", "description": "How to port a workbook to Python and Polars with export_port, and finish the port: the way, the rules, the files of the project (topic `layout`), Excel's behaviour in Polars (`excel-to-polars`), where to run it (`hosts`). Read it before porting.",
          "inputSchema": { "type": "object", "properties": { "topic": { "type": "string", "enum": ["way", "layout", "excel-to-polars", "hosts"] } } } },
        { "name": "show_app", "description": "Show a store to the person as an app inside the conversation: its tables with filters and sums, the logic of every column, the sheets of the workbook. They can change values and logic there and take changes back; what they change is in the store at once. `level`: data (default), model or book; `item`: a table or sheet to start at. A host that cannot show apps gets this answer as text only.",
          "inputSchema": { "type": "object", "properties": { "store": store, "level": { "type": "string", "enum": ["data", "model", "book"] }, "item": { "type": "string" } }, "required": ["store"] },
          "_meta": { "ui": { "resourceUri": crate::APP_URI } } },
        { "name": "app_call", "description": "A call of the app that show_app shows (the app's own; an agent uses the other tools).",
          "inputSchema": { "type": "object", "properties": { "op": { "type": "string" }, "args": { "type": "object" } }, "required": ["op"] },
          "_meta": { "ui": { "resourceUri": crate::APP_URI, "visibility": ["app"] } } }
    ]);
    // Without a workspace there is nothing to list: files are named by their paths.
    if let (false, Some(list)) = (remote, all.as_array_mut()) {
        list.retain(|t| t["name"] != "list_workbooks");
    }
    all
}

pub(crate) fn text_arg<'a>(args: &'a J, key: &str) -> Result<&'a str, String> {
    args[key].as_str().ok_or_else(|| format!("the argument `{key}` (text) is missing"))
}

/// The rows of single cells as an agent reads them: name, address and value.
pub(crate) fn cells_json(long: &crate::engine::data::Frame) -> Result<Vec<J>, String> {
    let wide = crate::engine::long::to_wide(long)?;
    let values = store::rows_json(&wide, 0, 1)?;
    let J::Array(rows) = store::rows_json(long, 0, usize::MAX)? else { return Ok(Vec::new()) };
    Ok(rows.iter().map(|r| json!({ "name": r["name"], "cell": r["cell"], "value": values[0][r["name"].as_str().unwrap_or("")] })).collect())
}

fn items_of(r: &J) -> &[J] {
    r["columns"].as_array().or(r["cells"].as_array()).map(Vec::as_slice).unwrap_or(&[])
}

/// The regions of a store; with a region, its columns or cells.
fn describe(cfg: &Config, dir: &Path, region: Option<&str>, offset: usize, limit: usize) -> Result<String, String> {
    let model = store::read_model(dir)?;
    let regions = model["regions"].as_array().cloned().unwrap_or_default();
    let head = |r: &J| format!("{} ({} {}!{})", r["name"].as_str().unwrap_or(""), r["kind"].as_str().unwrap_or(""), r["sheet"].as_str().unwrap_or(""), r["ref"].as_str().unwrap_or(""));
    let stacks = model["stacks"].as_array().cloned().unwrap_or_default();
    let texts = |v: &J| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect() };
    if let Some(stack) = region.and_then(|name| stacks.iter().find(|s| s["name"] == name)) {
        // A stack: its sheets, its columns, and where the sheets differ in logic.
        let members = stack["members"].as_array().cloned().unwrap_or_default();
        let mut out = format!("{} (stack: the same table in {} sheets; read and written as one table with the column `sheet` in front)\n", stack["name"].as_str().unwrap_or(""), members.len());
        for m in &members {
            let r = regions.iter().find(|r| r["name"] == m["region"]);
            out.push_str(&format!("  sheet {:?}: region {}\n", m["sheet"].as_str().unwrap_or(""), r.map(&head).unwrap_or_default()));
        }
        let first = members.first().and_then(|m| regions.iter().find(|r| r["name"] == m["region"]));
        out.push_str("columns:\n  sheet: String (the sheet a row is of)\n");
        for item in first.map(items_of).unwrap_or_default() {
            let column = item["name"].as_str().unwrap_or("");
            let differs = stack["differs"].as_array().into_iter().flatten().find(|d| d["column"] == column);
            match differs {
                None => out.push_str(&format!("  {column}: {}{}\n", item["dtype"].as_str().unwrap_or(""), item["logic"].as_str().map(|l| format!("  = {l}")).unwrap_or_default())),
                Some(d) => {
                    out.push_str(&format!("  {column}: {}  (not the same in every sheet)\n", item["dtype"].as_str().unwrap_or("")));
                    for (m, l) in members.iter().zip(d["logic"].as_array().into_iter().flatten()) {
                        out.push_str(&format!("      in {:?}: {}\n", m["sheet"].as_str().unwrap_or(""), l.as_str().map(|l| format!("= {l}")).unwrap_or_else(|| "plain data".into())));
                    }
                }
            }
        }
        out.push_str("Rows of their own (exceptions) are listed with each region. add_sheet makes one more sheet of the stack.\n");
        return Ok(out);
    }
    if let Some(name) = region {
        let r = regions.iter().find(|r| r["name"] == name).ok_or_else(|| format!("no region named {name:?}; the regions are {:?}", regions.iter().filter_map(|r| r["name"].as_str()).collect::<Vec<_>>()))?;
        let items = items_of(r);
        let mut out = format!("{}\n", head(r));
        for item in items.iter().skip(offset).take(limit) {
            let logic = item["logic"].as_str().map(|l| format!("  = {l}")).unwrap_or_default();
            let at = item["cell"].as_str().map(|c| format!(" [{c}]")).unwrap_or_default();
            let dtype = match item["dtype"].as_str().unwrap_or("") {
                "Null" => "no value",
                other => other,
            };
            out.push_str(&format!("  {}{at}: {dtype}{}\n", item["name"].as_str().unwrap_or(""), logic));
            // The rows that are not as the column is.
            let exceptions = item["exceptions"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            for e in exceptions.iter().take(10) {
                let what = e["logic"].as_str().map(|l| format!("= {l}")).unwrap_or_else(|| "a plain value".into());
                out.push_str(&format!("      but {}: {what}\n", e["cell"].as_str().unwrap_or("")));
            }
            if exceptions.len() > 10 {
                out.push_str(&format!("      … and {} more rows of their own\n", exceptions.len() - 10));
            }
        }
        if items.len() > offset + limit {
            out.push_str(&format!("  … {} of {} shown; the next with offset {}\n", limit.min(items.len() - offset), items.len(), offset + limit));
        }
        return Ok(out);
    }
    let mut out = format!("store: {}\n{} regions. describe with `region` shows the columns or cells of one.\n", cfg.shown(dir), regions.len());
    if store::pending(dir) {
        out.push_str("Data or logic changed since the last run: the ranges are those of the workbook as of that run.\n");
    }
    for r in regions.iter().take(80) {
        let items = items_of(r);
        let calculated = items.iter().filter(|i| i["logic"].is_string()).count();
        let what = if r["kind"] == "cells" { "cells" } else { "columns" };
        out.push_str(&format!("  {}: {} {what}, {calculated} calculated\n", head(r), items.len()));
    }
    if regions.len() > 80 {
        let rest: Vec<&str> = regions.iter().skip(80).filter_map(|r| r["name"].as_str()).collect();
        out.push_str(&format!("  … and {} more: {}\n", rest.len(), rest.join(", ")));
    }
    if !stacks.is_empty() {
        out.push_str(&format!("{} stacks (the same table in several sheets, read and written by the stack's name as one table with a column `sheet`):\n", stacks.len()));
        for s in stacks.iter().take(40) {
            let sheets: Vec<String> = s["members"].as_array().into_iter().flatten().filter_map(|m| m["sheet"].as_str().map(str::to_string)).collect();
            let shown = if sheets.len() > 8 { format!("{}, … ({} sheets)", sheets[..8].join(", "), sheets.len()) } else { sheets.join(", ") };
            out.push_str(&format!("  {}: sheets {shown}; columns {}\n", s["name"].as_str().unwrap_or(""), texts(&s["columns"]).join(", ")));
        }
    }
    Ok(out)
}

/// The view of a table an agent asks for with `filter`, `search` and `sort`.
fn view_of(args: &J) -> J {
    json!({ "filter": args["filter"], "search": args["search"], "sort": args["sort"] })
}

/// One call of a tool. What changes a store keeps the state before it (for `undo`), and
/// a change that fails leaves the store as it was.
pub(crate) fn call(cfg: &Config, name: &str, args: &J) -> Result<String, String> {
    if ["write_region", "append_rows", "set_value", "add_sheet", "set_logic", "run"].contains(&name) {
        let dir = cfg.place(text_arg(args, "store")?)?;
        return crate::api::change(&dir, name, || tool(cfg, name, args));
    }
    tool(cfg, name, args)
}

fn tool(cfg: &Config, name: &str, args: &J) -> Result<String, String> {
    let dir = || text_arg(args, "store").and_then(|s| cfg.place(s));
    let through = |op: &str, args: J| crate::api::call(cfg, op, &args).map(|v| v.to_string());
    match name {
        "change_rows" => through("patch", json!({ "store": args["store"], "region": args["region"], "patch": { "set": args["set"], "delete": args["delete"], "insert": args["insert"] }, "then": args["then"] })),
        "recalculate" | "undo" => through(name, args.clone()),
        "app_call" => through(text_arg(args, "op")?, if args["args"].is_object() { args["args"].clone() } else { json!({}) }),
        "show_app" => {
            let tables = crate::engine::store::tables(&dir()?)?;
            Ok(format!("The app shows {} to the person ({} tables). What they change there is in the store at once: read again before relying on values read earlier.", text_arg(args, "store")?, tables.len()))
        }
        "rename_region" => through("rename", args.clone()).map(|_| format!("{} is named {} now.", text_arg(args, "region").unwrap_or(""), text_arg(args, "name").unwrap_or(""))),
        "export_region" => through("export", json!({ "store": args["store"], "region": args["region"], "what": args["what"], "path": args["path"], "view": view_of(args) })),
        "list_workbooks" => {
            let books = cfg.workbooks()?;
            if books.is_empty() {
                let how = cfg.files_url.as_deref().map(|u| format!(" Upload one at {u}")).unwrap_or_default();
                return Ok(format!("No Excel files in the workspace.{how}"));
            }
            Ok(books.iter().map(|(n, has)| if *has { format!("{n} (store: {n}.store)") } else { n.clone() }).collect::<Vec<_>>().join("\n"))
        }
        "open_workbook" => {
            let arg = text_arg(args, "path")?;
            let path = cfg.place(arg)?;
            let bytes = std::fs::read(&path).map_err(|e| match cfg.workbooks() {
                Ok(books) => format!("no workbook named {arg:?} in the workspace; there are {:?}", books.iter().map(|b| b.0.as_str()).collect::<Vec<_>>()),
                Err(_) => format!("cannot read {arg}: {e}"),
            })?;
            let wb = xl_compat::read_any(bytes.clone()).map_err(|e| format!("{arg}: {e}"))?;
            let dir = PathBuf::from(format!("{}.store", path.display()));
            if dir.join("model.json").exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot replace {}: {e}", crate::engine::outbound(&dir)))?;
            }
            store::create(&dir, &bytes, &wb, !xl_compat::is_xls(&bytes))?;
            let mut out = describe(cfg, &dir, None, 0, 0)?;
            let empty = store::without_value(&wb);
            if empty > 0 {
                out.push_str(&format!("{empty} formula cells of the file hold no value (not calculated when it was saved, or errors): run calculates them, but for those it names (volatile functions such as TODAY stay as they are).\n"));
            }
            Ok(out)
        }
        "describe" => describe(cfg, &dir()?, args["region"].as_str(), args["offset"].as_u64().unwrap_or(0) as usize, args["limit"].as_u64().unwrap_or(200) as usize),
        "append_rows" | "set_value" => {
            let (dir, region) = (dir()?, text_arg(args, "region")?);
            let like = store::read_region(&dir, region)?;
            let model = store::read_model(&dir)?;
            let is_cells = model["regions"].as_array().and_then(|a| a.iter().find(|r| r["name"] == region)).is_some_and(|r| r["kind"] == "cells");
            if name == "set_value" {
                if !is_cells {
                    return Err(format!("{region:?} is a table: set_value is for single cells of a `cells` region; use write_region or append_rows"));
                }
                let cell = text_arg(args, "name")?;
                store::write_frame(&dir, region, &crate::engine::long::from_values(&[(cell.to_string(), args["value"].clone())])?)?;
                return Ok(format!("{region}[{cell}] = {}. Call run to recalculate.", args["value"]));
            }
            let J::Array(mut rows) = store::rows_json(&like, 0, usize::MAX)? else { return Err("the region has no rows".into()) };
            if is_cells {
                return Err(format!("{region:?} is a group of single cells, not a table: use set_value"));
            }
            let more = args["rows"].as_array().ok_or("rows must be a list of objects, one per row")?;
            // The rows there are, without what is calculated, and the new ones after them.
            let of = model["stacks"].as_array().and_then(|a| a.iter().find(|s| s["name"] == region)).and_then(|s| s["members"][0]["region"].as_str()).unwrap_or(region);
            let calculated: Vec<String> = model["regions"].as_array().and_then(|a| a.iter().find(|r| r["name"] == of)).map(|r| items_of(r).iter().filter(|i| i["logic"].is_string()).filter_map(|i| i["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            for row in rows.iter_mut().filter_map(J::as_object_mut) {
                row.retain(|k, _| !calculated.contains(k));
            }
            rows.extend(more.iter().cloned());
            let df = store::frame_from_rows(&J::Array(rows), &like)?;
            store::write_frame(&dir, region, &df)?;
            Ok(format!("{region}: {} rows now ({} added). Call run to recalculate.", df.height, more.len()))
        }
        "read_range" => {
            let (wb, _) = store::read_workbook(&dir()?)?;
            let sheet = text_arg(args, "sheet")?;
            let s = wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(sheet)).ok_or_else(|| format!("no sheet named {sheet:?}; the sheets are {:?}", wb.sheets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()))? as u32;
            let range = text_arg(args, "range")?;
            let (a, z) = range.split_once(':').unwrap_or((range, range));
            let (Some(a), Some(z)) = (xl_formula::parse_cell_ref(&a.replace('$', "")), xl_formula::parse_cell_ref(&z.replace('$', ""))) else {
                return Err(format!("{range:?} is not a range of cells; write it like A1:E10"));
            };
            let c = &wb.cells;
            let mut found: Vec<usize> = (0..c.len()).filter(|&i| c.sheet[i] == s && (a.row..=z.row).contains(&(c.row[i] + 1)) && (a.col..=z.col).contains(&(c.col[i] + 1)) && (c.kind[i] != xl_cells::Kind::Blank || c.formula[i].is_some())).collect();
            found.sort_by_key(|&i| (c.row[i], c.col[i]));
            let total = found.len();
            let cells: Vec<J> = found
                .iter()
                .take(500)
                .map(|&i| {
                    let value = match c.value(i) {
                        xl_cells::Value::Blank => J::Null,
                        xl_cells::Value::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => json!(n as i64),
                        xl_cells::Value::Number(n) => json!(n),
                        xl_cells::Value::Bool(b) => json!(b),
                        xl_cells::Value::Text(t) | xl_cells::Value::Error(t) => json!(t),
                    };
                    let mut o = json!({ "cell": format!("{}{}", xl_formula::col_name(c.col[i] + 1), c.row[i] + 1), "value": value });
                    if let Some(f) = &c.formula[i] {
                        o["formula"] = json!(format!("={f}"));
                    }
                    o
                })
                .collect();
            let mut out = json!({ "sheet": wb.sheets[s as usize].name, "cells": cells });
            if total > 500 {
                out["note"] = json!(format!("{total} cells hold something; the first 500 are shown. Ask for a smaller range."));
            }
            Ok(out.to_string())
        }
        "read_region" => {
            let whole = store::read_region(&dir()?, text_arg(args, "region")?)?;
            let index = crate::engine::view::rows(&whole, &view_of(args))?;
            let viewed = index.len() < whole.height || !args["sort"].is_null();
            let df = if viewed { crate::engine::view::taken(&whole, &index) } else { whole };
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(100) as usize;
            let asked: Option<Vec<String>> = args["columns"].as_array().map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
            if crate::engine::long::is_long(&df) {
                // Single cells: a row each, the ones asked for by name.
                let mut rows = cells_json(&df)?;
                if let Some(names) = &asked {
                    let have: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
                    if let Some(missing) = names.iter().find(|n| !have.contains(&n.as_str())) {
                        return Err(format!("no cell {missing:?} in this region; it has {:?}", have.iter().take(60).collect::<Vec<_>>()));
                    }
                    rows.retain(|r| r["name"].as_str().is_some_and(|n| names.iter().any(|x| x == n)));
                }
                let total = rows.len();
                let part: Vec<J> = rows.into_iter().skip(offset).take(limit).collect();
                return Ok(json!({ "total_rows": total, "offset": offset, "rows": part }).to_string());
            }
            let all: Vec<String> = df.names().into_iter().map(str::to_string).collect();
            let wanted: Vec<String> = match asked {
                Some(names) => {
                    if let Some(missing) = names.iter().find(|n| !all.contains(n)) {
                        return Err(format!("no column {missing:?} in this region; it has {:?}", all.iter().take(60).collect::<Vec<_>>()));
                    }
                    names
                }
                None => all.iter().take(60).cloned().collect(),
            };
            let part = df.select(wanted.iter().map(String::as_str))?;
            let rows = store::rows_json(&part, offset, limit)?;
            let mut out = json!({ "total_rows": df.height, "offset": offset, "rows": rows });
            if viewed {
                out["index"] = json!(index.iter().skip(offset).take(limit).collect::<Vec<_>>());
            }
            if wanted.len() < all.len() {
                out["total_columns"] = json!(all.len());
                out["note"] = json!("only some columns are shown; name others in `columns`");
            }
            Ok(out.to_string())
        }
        "write_region" => {
            let (dir, region) = (dir()?, text_arg(args, "region")?);
            let like = store::read_region(&dir, region)?;
            if crate::engine::long::is_long(&like) {
                // Single cells: rows of name and value, or one row with a field per cell.
                let rows = args["rows"].as_array().ok_or("rows must be a list of objects")?;
                let by_name = !rows.is_empty() && rows.iter().all(|r| r["name"].is_string() && r.get("value").is_some());
                let values: Vec<(String, J)> = if by_name {
                    rows.iter().map(|r| (r["name"].as_str().unwrap_or("").to_string(), r["value"].clone())).collect()
                } else if let [J::Object(one)] = rows.as_slice() {
                    one.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                } else {
                    return Err(format!("{region:?} is single cells: give rows {{\"name\": …, \"value\": …}}, or one row with a field per cell"));
                };
                store::write_frame(&dir, region, &crate::engine::long::from_values(&values)?)?;
                return Ok(format!("{region}: {} cells written. Call run to recalculate.", values.len()));
            }
            let df = store::frame_from_rows(&args["rows"], &like)?;
            store::write_frame(&dir, region, &df)?;
            Ok(format!("{region}: {} rows written. Call run to recalculate.", df.height))
        }
        "add_sheet" => {
            let (like, new) = (text_arg(args, "like")?, text_arg(args, "name")?);
            let made = store::add_sheet(&dir()?, like, new)?;
            Ok(format!("Sheet {new:?} added as a copy of {like:?}, with its data. Its regions: {}. Write its rows (in a stack: rows with sheet = {new:?}), then call run.", made.join(", ")))
        }
        "set_logic" => {
            let (region, item) = (text_arg(args, "region")?, text_arg(args, "name")?);
            let dir = dir()?;
            if let Some(logic) = args["logic"].as_str() {
                store::check_logic(&dir, region, item, logic)?;
            }
            store::set_model_logic(&dir, region, item, args["logic"].as_str())?;
            Ok(match args["logic"].as_str() {
                Some(l) => format!("{region}[{item}] = {l}. Call run to recalculate."),
                None => format!("{region}[{item}] is plain data now. Call run to carry that into the workbook."),
            })
        }
        "run" => {
            let r = store::run(&dir()?)?;
            let mut out = format!("{} of {} formula cells calculated.\n", r.computed, r.formula_cells);
            // In words an agent can pass on.
            let said = |why: &str| if why == crate::engine::lift::ERROR_RESULT { "the formula gives an error value, as it does in Excel (such as #VALUE! or #DIV/0!; which one is not worked out)".to_string() } else { why.to_string() };
            for (why, n) in &r.not_computed {
                out.push_str(&format!("{n} cells not calculated (each keeps the value the file had for it, if it had one): {}\n", said(why)));
            }
            for (at, formula, why) in r.uncomputed.iter().take(20) {
                out.push_str(&format!("  {at} ={formula}: {}\n", if why == crate::engine::lift::ERROR_RESULT { "error value" } else { why.as_str() }));
            }
            if r.uncomputed.len() > 20 {
                out.push_str("  … (the first 20 cells are listed)\n");
            }
            if !r.regions_gone.is_empty() || !r.regions_new.is_empty() {
                out.push_str(&format!("The sheet reads differently now: no region any more: {:?}; new regions: {:?}. Tables that touch after a change are read as one; describe shows what there is.\n", r.regions_gone, r.regions_new));
            }
            for (region, item) in &r.logic_changed {
                out.push_str(&format!("logic taken into the workbook: {region}[{item}]\n"));
            }
            let rows: Vec<String> = r.regions.iter().take(40).map(|(region, rows)| format!("{region}: {rows} {}", if *rows == 1 { "row" } else { "rows" })).collect();
            out.push_str(&rows.join("\n"));
            if r.regions.len() > 40 {
                out.push_str(&format!("\n… and {} more regions", r.regions.len() - 40));
            }
            Ok(out)
        }
        "verify" => {
            let dir = dir()?;
            let (wb, _) = store::read_workbook(&dir)?;
            let mut lines = Vec::new();
            let mut total = 0usize;
            let at = crate::engine::frame::cell_index(&wb);
            let known: Vec<String> = store::read_model(&dir)?["regions"].as_array().map(|a| a.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            for region in store::regions_of(&dir, &wb) {
                if !known.contains(&region.name) {
                    continue;
                }
                let Ok(df) = store::read_wide(&dir, &region.name) else { continue };
                let rows = region.data_rows().map(|(a, z)| (z - a + 1) as usize);
                if let (Some(rows), false) = (rows, region.kind == crate::engine::regions::RegionKind::Cells) {
                    if rows != df.height {
                        total += 1;
                        lines.push(format!("{}: the data has {} rows, the workbook {rows} (its cells are not compared one by one)", region.name, df.height));
                        continue;
                    }
                }
                let diff = match crate::engine::apply::verify_in(&wb, &at, &region, &df) {
                    Ok(d) => d,
                    Err(why) => {
                        total += 1;
                        lines.push(format!("{}: {why}", region.name));
                        continue;
                    }
                };
                total += diff.height;
                if lines.len() < 30 {
                    if let Ok(J::Array(rows)) = store::rows_json(&diff, 0, 30 - lines.len()) {
                        lines.extend(rows.iter().map(|r| format!("{} {}: workbook {:?}, data {:?}", r["cell"].as_str().unwrap_or(""), r["column"].as_str().unwrap_or(""), r["expected"].as_str().unwrap_or(""), r["actual"].as_str().unwrap_or(""))));
                    }
                }
            }
            let mut out = if total == 0 { "Data and workbook agree.".to_string() } else { format!("{total} differences between data and workbook (call run to bring the workbook up to date):\n{}", lines.join("\n")) };
            if store::pending(&dir) {
                out.push_str("\nData or logic was changed since the last run: call run.");
            }
            let empty = store::without_value(&wb);
            if empty > 0 {
                out.push_str(&format!("\n{empty} formula cells hold no value (not calculated yet, an error, or one of those run cannot calculate and names: volatile functions such as TODAY)."));
            }
            Ok(out)
        }
        "export_workbook" => {
            let (_, workbook, _) = store::paths(&dir()?);
            let arg = text_arg(args, "path")?;
            let path = cfg.place(arg)?;
            if cfg.workspace.is_some() && !arg.to_ascii_lowercase().ends_with(".xlsx") {
                return Err(format!("{arg:?}: the workbook is written as .xlsx; give a name that ends in .xlsx"));
            }
            let replaced = path.exists();
            let ran = store::pending(&dir()?);
            let mut left = String::new();
            if ran {
                let r = crate::api::change(&dir()?, "run", || store::run(&dir()?))?;
                let open: u64 = r.not_computed.iter().map(|x| x.1).sum();
                if open > 0 {
                    left = format!("\n{open} of {} formula cells were not calculated and keep the value the file had (run says which and why).", r.formula_cells);
                }
            }
            std::fs::copy(&workbook, &path).map_err(|e| format!("cannot write {arg}: {e}"))?;
            let mut out = format!("written: {arg}{}{}", if replaced { " (the file of that name was replaced)" } else { "" }, if ran { " (data or logic had changed: the store was run first)" } else { "" }) + &left;
            if let Some(base) = &cfg.files_url {
                out.push_str(&format!("\ndownload: {base}/{}", crate::http::encode(arg)));
            }
            Ok(out)
        }
        "polars_code" => {
            let dir = dir()?;
            let (wb, _) = store::read_workbook(&dir)?;
            let accuracy = match args["accuracy"].as_str() {
                Some("exact") => crate::engine::codegen::Accuracy::Exact,
                Some("plain") | None => crate::engine::codegen::Accuracy::Plain,
                Some(other) => return Err(format!("accuracy is `plain` or `exact`, not {other:?}")),
            };
            let (_, workbook, _) = store::paths(&dir);
            let file = if cfg.workspace.is_some() { "workbook.xlsx".to_string() } else { crate::engine::outbound(&workbook) };
            let b = crate::engine::backend();
            Ok(crate::engine::codegen::polars_program(&wb, &file, accuracy, &[], Some(b.executor())).source)
        }
        "export_port" => {
            let dir = dir()?;
            let accuracy = match args["accuracy"].as_str() {
                Some("exact") => crate::engine::codegen::Accuracy::Exact,
                Some("plain") | None => crate::engine::codegen::Accuracy::Plain,
                Some(other) => return Err(format!("accuracy is `plain` or `exact`, not {other:?}")),
            };
            let dest = match args["path"].as_str() {
                Some(p) => cfg.place(p)?,
                None => port_folder(&dir),
            };
            let replace = args["replace"].as_bool().unwrap_or(false);
            let check = crate::engine::port_check();
            let port = crate::api::change(&dir, "port", || crate::engine::port::write(&*crate::engine::backend(), &dir, &dest, accuracy, replace, check.as_ref().map(|f| f as crate::engine::port::Check)))?;
            let mut out = format!("port written: {}\n{}\n", cfg.shown(&dest), port.summary());
            for i in port.items.iter().filter(|i| i.how != "polars") {
                out.push_str(&format!("  {:6} {}[{}]  ={}  (port/calc.py:{}){}\n", i.how, i.region, i.name, i.logic.as_deref().unwrap_or(&i.formula), i.line, i.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default()));
            }
            if cfg.workspace.is_some() {
                let name = format!("{}.zip", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                zip_folder(&dest, &dest.with_file_name(&name))?;
                if let Some(base) = &cfg.files_url {
                    out.push_str(&format!("download: {base}/{}\n", crate::http::encode(&name)));
                }
            }
            out.push_str("Next: in the project, `python check.py` (Polars only) tells what is right; then port every `rest` at its line of port/calc.py (port_guide says how).");
            Ok(out)
        }
        "port_guide" => Ok(match args["topic"].as_str() {
            Some("layout") => GUIDE_LAYOUT,
            Some("excel-to-polars") => GUIDE_POLARS,
            Some("hosts") => GUIDE_HOSTS,
            Some("way") | None => GUIDE,
            Some(other) => return Err(format!("no topic {other:?}: way, layout, excel-to-polars or hosts")),
        }
        .to_string()),
        other => Err(format!("no tool named {other:?}")),
    }
}

/// The guide to porting (skills/excel-port): the same texts as the skill for Claude.
pub(crate) const GUIDE: &str = include_str!("../../../skills/excel-port/SKILL.md");
const GUIDE_LAYOUT: &str = include_str!("../../../skills/excel-port/reference/layout.md");
const GUIDE_POLARS: &str = include_str!("../../../skills/excel-port/reference/excel-to-polars.md");
const GUIDE_HOSTS: &str = include_str!("../../../skills/excel-port/reference/hosts.md");

/// A folder as a zip file (its files under its name, `/` between parts).
fn zip_folder(folder: &Path, target: &Path) -> Result<(), String> {
    use std::io::Write;
    let file = std::fs::File::create(target).map_err(|e| format!("cannot write {}: {e}", crate::engine::outbound(target)))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let top = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(folder).map_err(|e| e.to_string())?;
            let name = std::iter::once(top.clone()).chain(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned())).collect::<Vec<_>>().join("/");
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            zip.write_all(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Where the port of a store is written unless told otherwise: beside the store, named
/// after its workbook (`plan.xlsx.store` → `plan-port`).
pub(crate) fn port_folder(dir: &Path) -> PathBuf {
    let stem = dir.file_name().map(|n| n.to_string_lossy().trim_end_matches(".store").to_string()).unwrap_or_default();
    let stem = stem.rsplit_once('.').map_or(stem.as_str(), |x| x.0).to_string();
    dir.with_file_name(format!("{stem}-port"))
}
