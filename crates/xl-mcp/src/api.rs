//! What the app does with a store, and agents through some of the tools: JSON in, JSON
//! out. A store is named as `open_workbook` names it; a plain directory of Parquet files
//! with notes is taken as one too (data alone: what needs the workbook says so).
//!
//! Three levels: the workbook (`sheets`, `cells`, `set_cells`), the model (`tables`,
//! `set_logic`, `rename`, `run`, `verify`, `add_sheet`) and the data (`rows`, `patch`,
//! `recalculate`, `save_view`, `export`). Every change keeps the state before it
//! (`undo`), and a change that fails leaves the store as it was.

use crate::tools::text_arg;
use crate::Config;
use serde_json::{json, Value as J};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use xl_cells::{Kind, Workbook};
use crate::engine::{store, view};

const VIEWS: &str = "views.json";

type Held = (PathBuf, std::time::SystemTime, Arc<Workbook>);
static BOOK: Mutex<Option<Held>> = Mutex::new(None);

/// The workbook of a store, read once for as long as its file stays as it is.
fn workbook(dir: &Path) -> Result<Arc<Workbook>, String> {
    let path = store::paths(dir).1;
    let time = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|_| "this is data alone, without a workbook: what shows or changes the workbook cannot be done here".to_string())?;
    let mut held = BOOK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, _, wb)) = held.as_ref().filter(|(p, t, _)| *p == path && *t == time) {
        return Ok(wb.clone());
    }
    let wb = Arc::new(store::read_workbook(dir)?.0);
    *held = Some((path, time, wb.clone()));
    Ok(wb)
}

/// A change of a store: the state before it is kept (for `undo`) and comes back if the
/// change fails.
pub(crate) fn change<T>(dir: &Path, what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    store::remember(dir, what)?;
    f().inspect_err(|_| {
        let _ = store::undo(dir);
    })
}

fn run_report(r: &store::RunReport) -> J {
    json!({
        "formula_cells": r.formula_cells,
        "computed": r.computed,
        "not_computed": r.not_computed.iter().map(|(why, cells)| json!({ "why": why, "cells": cells })).collect::<Vec<_>>(),
        "uncomputed": r.uncomputed.iter().map(|(cell, formula, why)| json!({ "cell": cell, "formula": formula, "why": why })).collect::<Vec<_>>(),
        "logic_changed": r.logic_changed,
        "regions_gone": r.regions_gone,
        "regions_new": r.regions_new,
    })
}

fn recalculated(r: &crate::engine::plans::Recalculation) -> J {
    json!({
        "done": r.done.len(),
        "kept": r.kept.iter().map(|(region, name, formula, why)| json!({ "region": region, "name": name, "formula": formula, "why": why })).collect::<Vec<_>>(),
        "from_kept": r.from_kept.len(),
    })
}

/// What follows a change of data: the calculation from the notes (`recalculate`, the
/// default), the workbook's (`run`), or `nothing`.
fn follow(dir: &Path, then: Option<&str>) -> Result<J, String> {
    Ok(match then.unwrap_or("recalculate") {
        "nothing" => J::Null,
        "run" => json!({ "run": run_report(&store::run(dir)?) }),
        "recalculate" => json!({ "recalculate": recalculated(&store::recalculate(dir)?) }),
        other => return Err(format!("`then` is recalculate, run or nothing, not {other:?}")),
    })
}

fn views(dir: &Path) -> J {
    std::fs::read_to_string(dir.join(VIEWS)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

/// The cells of a region as an agent or a person reads them: name, address and value.
fn cells_of(long: &crate::engine::data::Frame) -> Result<J, String> {
    crate::tools::cells_json(long).map(J::Array)
}

/// Rows of a region, or of a view of it: `view` filters and sorts (see
/// `xl_engine::view::rows`), `offset` and `limit` page, `columns` picks. `index` gives
/// the number of each row in the region (what `patch` takes).
pub(crate) fn rows(dir: &Path, args: &J) -> Result<J, String> {
    let df = store::read_region(dir, text_arg(args, "region")?)?;
    let keep = view::rows(&df, &args["view"])?;
    let (offset, limit) = (args["offset"].as_u64().unwrap_or(0) as usize, args["limit"].as_u64().unwrap_or(200) as usize);
    let page: Vec<usize> = keep.iter().skip(offset).take(limit).copied().collect();
    let mut part = view::taken(&df, &page);
    let rows = if crate::engine::long::is_long(&df) {
        cells_of(&part)?
    } else {
        if let Some(names) = args["columns"].as_array() {
            part = part.select(names.iter().filter_map(J::as_str)).map_err(|why| format!("{why}; the columns are {:?}", df.names()))?;
        }
        store::rows_json(&part, 0, usize::MAX)?
    };
    let mut out = json!({ "total": keep.len(), "all": df.height, "offset": offset, "index": page, "rows": rows });
    if args["summary"] == true {
        out["summary"] = view::summary(&df, &keep);
    }
    Ok(out)
}

/// A number of a cell as its format shows it, roughly: dates, percent, fixed decimals
/// and thousands; anything else as short as it can be written.
fn shown(wb: &Workbook, i: usize) -> String {
    let c = &wb.cells;
    let n = c.num[i];
    match c.kind[i] {
        Kind::Blank => String::new(),
        Kind::Text | Kind::Error => c.text[i].as_deref().unwrap_or("").to_string(),
        Kind::Bool => (if n != 0.0 { "TRUE" } else { "FALSE" }).to_string(),
        Kind::Number => {
            let format = wb.styles.get(c.style[i] as usize).map_or("General", |s| s.num_format.as_str());
            if xl_cells::is_date_format(format) {
                let form = if n < 1.0 { xl_cells::IsoDate::Time } else if n.fract() == 0.0 { xl_cells::IsoDate::Date } else { xl_cells::IsoDate::DateTime };
                return form.format(n, wb.date1904).map_or_else(|| n.to_string(), |t| t.replace('T', " "));
            }
            let first = format.split(';').next().unwrap_or("");
            let Some(dot) = first.find(['0', '#']).map(|_| first.find('.')) else {
                return format!("{}", (n * 1e10).round() / 1e10);
            };
            let decimals = dot.map_or(0, |d| first[d + 1..].chars().take_while(|ch| matches!(ch, '0' | '#')).count());
            let percent = first.contains('%');
            let text = format!("{:.*}", decimals, if percent { n * 100.0 } else { n });
            let (whole, rest) = text.split_once('.').map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
            let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |d| ("-", d));
            let grouped = if first.contains(",0") || first.contains(",#") {
                digits.as_bytes().rchunks(3).rev().map(|g| std::str::from_utf8(g).unwrap_or("")).collect::<Vec<_>>().join(",")
            } else {
                digits.to_string()
            };
            format!("{sign}{grouped}{}{}", rest.map(|r| format!(".{r}")).unwrap_or_default(), if percent { "%" } else { "" })
        }
    }
}

/// A colour of a style as CSS, where it is written as RGB.
fn css(colour: &Option<String>) -> Option<String> {
    let c = colour.as_deref()?;
    let hex = if c.len() == 8 { &c[2..] } else { c };
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("#{hex}"))
}

/// A part of a sheet as the workbook has it: the cells that hold something or have a
/// look of their own, as `[row, column, text shown, formula or null, style]`, the styles
/// they name, and the merged ranges and column widths of the part.
fn cells(dir: &Path, args: &J) -> Result<J, String> {
    let wb = workbook(dir)?;
    let name = text_arg(args, "sheet")?;
    let s = wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("no sheet named {name:?}; the sheets are {:?}", wb.sheets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()))?;
    let number = |key: &str, default: u64| args[key].as_u64().unwrap_or(default) as u32;
    let (top, left) = (number("top", 0), number("left", 0));
    let (bottom, right) = (top + number("rows", 200), left + number("cols", 60));
    let c = &wb.cells;
    let mut cells = Vec::new();
    let mut styles = serde_json::Map::new();
    for i in (0..c.len()).filter(|&i| c.sheet[i] == s as u32 && (top..bottom).contains(&c.row[i]) && (left..right).contains(&c.col[i])) {
        let style = c.style[i];
        if c.kind[i] == Kind::Blank && c.formula[i].is_none() && style == 0 {
            continue;
        }
        if let (Some(st), false) = (wb.styles.get(style as usize), styles.contains_key(&style.to_string())) {
            styles.insert(style.to_string(), json!({ "bold": st.bold, "italic": st.italic, "fill": css(&st.fill), "color": css(&st.font_color), "align": st.h_align, "format": st.num_format }));
        }
        cells.push(json!([c.row[i], c.col[i], shown(&wb, i), c.formula[i].as_ref().map(|f| format!("={f}")), style]));
    }
    let sheet = &wb.sheets[s];
    let merges: Vec<_> = sheet.merges.iter().filter(|m| m.0 < bottom && m.2 >= top && m.1 < right && m.3 >= left).collect();
    let widths: Vec<J> = sheet.columns.iter().filter(|col| col.first < right && col.width.is_some()).map(|col| json!([col.first, col.last.min(right), col.width])).collect();
    Ok(json!({ "sheet": sheet.name, "top": top, "left": left, "cells": cells, "styles": styles, "merges": merges, "widths": widths }))
}

/// A table as text with commas.
fn csv(df: &crate::engine::data::Frame) -> String {
    let field = |t: &str| if t.contains([',', '"', '\n', '\r']) { format!("\"{}\"", t.replace('"', "\"\"")) } else { t.to_string() };
    let mut out = df.names().iter().map(|n| field(n)).collect::<Vec<_>>().join(",") + "\n";
    for k in 0..df.height {
        out += &df.columns.iter().map(|c| field(&c.data.text(k).unwrap_or_default())).collect::<Vec<_>>().join(",");
        out.push('\n');
    }
    out
}

/// Write the workbook of a store (run first, if data or logic changed), a region as CSV
/// (the rows of a view, if one is given) or its Parquet file with its note.
fn export(cfg: &Config, dir: &Path, args: &J) -> Result<J, String> {
    let stem = cfg.shown(dir).trim_end_matches(".store").rsplit(['/', '\\']).next().unwrap_or("data").to_string();
    let what = args["what"].as_str().unwrap_or("workbook");
    let region = || text_arg(args, "region");
    let default = match what {
        "workbook" => format!("{}.new.xlsx", stem.rsplit_once('.').map_or(stem.as_str(), |x| x.0)),
        "csv" | "parquet" => format!("{stem}.{}.{what}", region()?),
        other => return Err(format!("`what` is workbook, csv or parquet, not {other:?}")),
    };
    let name = args["path"].as_str().map_or(default, str::to_string);
    let path = cfg.place(&name)?;
    let write = |data: &[u8]| std::fs::write(&path, data).map_err(|e| format!("cannot write {name}: {e}"));
    let mut out = json!({ "file": name });
    match what {
        "workbook" => {
            workbook(dir)?;
            if store::pending(dir) {
                out["run"] = run_report(&store::run(dir)?);
            }
            write(&std::fs::read(store::paths(dir).1).map_err(|e| e.to_string())?)?;
        }
        "csv" => {
            let df = store::read_region(dir, region()?)?;
            write(csv(&view::taken(&df, &view::rows(&df, &args["view"])?)).as_bytes())?;
        }
        _ => {
            let file = store::tables(dir)?.iter().find(|t| t["name"] == region().unwrap_or("")).and_then(|t| t["file"].as_str().map(str::to_string)).ok_or_else(|| format!("no table named {:?} (a stack has no file of its own: export its sheets, or CSV)", region().unwrap_or("")))?;
            write(&std::fs::read(dir.join(&file)).map_err(|e| format!("cannot read {file}: {e}"))?)?;
        }
    }
    if let Some(base) = &cfg.files_url {
        out["url"] = json!(format!("{base}/{}", crate::http::encode(&name)));
    }
    Ok(out)
}

/// One call of the interface.
pub(crate) fn call(cfg: &Config, op: &str, args: &J) -> Result<J, String> {
    let dir = || text_arg(args, "store").and_then(|s| cfg.place(s));
    let then = || args["then"].as_str();
    match op {
        // The workspace: its Excel files, and what there is of stores and plain data.
        "workspace" => {
            // A server that works on paths of its machine has no workspace to list.
            let Some(w) = cfg.workspace.as_ref() else { return Ok(json!({ "files": [], "stores": [], "files_url": null })) };
            let files: Vec<J> = cfg.workbooks()?.iter().map(|(name, has)| json!({ "name": name, "store": has.then(|| format!("{name}.store")) })).collect();
            let mut data: Vec<J> = Vec::new();
            for e in std::fs::read_dir(w).map_err(|e| format!("cannot read the workspace: {e}"))?.flatten().filter(|e| e.path().is_dir()) {
                let (name, path) = (e.file_name().to_string_lossy().into_owned(), e.path());
                let plain = std::fs::read_dir(&path).into_iter().flatten().flatten().any(|f| f.file_name().to_string_lossy().ends_with(".parquet"));
                if path.join("model.json").exists() || plain {
                    data.push(json!({ "name": name, "workbook": store::paths(&path).1.exists() }));
                }
            }
            data.sort_by_key(|d| d["name"].as_str().map(str::to_lowercase));
            Ok(json!({ "files": files, "stores": data, "files_url": cfg.files_url }))
        }
        "open" => {
            let text = crate::tools::call(cfg, "open_workbook", &json!({ "path": text_arg(args, "file")? }))?;
            Ok(json!({ "store": format!("{}.store", text_arg(args, "file")?), "text": text }))
        }
        "tables" => {
            let dir = dir()?;
            let model = store::read_model(&dir).ok();
            Ok(json!({
                "tables": store::tables(&dir)?,
                "stacks": model.as_ref().map_or(json!([]), |m| m["stacks"].clone()),
                "workbook": store::paths(&dir).1.exists(),
                "pending": store::pending(&dir),
                "undo": store::undoable(&dir),
                "views": views(&dir),
            }))
        }
        "rows" => rows(&dir()?, args),
        "patch" => {
            let (dir, region) = (dir()?, text_arg(args, "region")?);
            change(&dir, &format!("rows of {region}"), || {
                let rows = store::patch(&dir, region, &args["patch"])?;
                let then = follow(&dir, then())?;
                Ok(json!({ "rows": rows, "then": then, "pending": store::pending(&dir) }))
            })
        }
        "recalculate" | "run" => {
            let dir = dir()?;
            change(&dir, op, || follow(&dir, Some(op)))
        }
        "set_logic" => {
            let (dir, region, name) = (dir()?, text_arg(args, "region")?, text_arg(args, "name")?);
            workbook(&dir)?;
            change(&dir, &format!("logic of {region}[{name}]"), || {
                if let Some(logic) = args["logic"].as_str() {
                    store::check_logic(&dir, region, name, logic)?;
                }
                store::set_model_logic(&dir, region, name, args["logic"].as_str())?;
                Ok(json!({ "run": run_report(&store::run(&dir)?) }))
            })
        }
        "rename" => {
            let (dir, region, name) = (dir()?, text_arg(args, "region")?, text_arg(args, "name")?);
            workbook(&dir)?;
            let done = change(&dir, &format!("name of {region}"), || Ok(json!({ "run": run_report(&store::rename_region(&dir, region, name)?) })))?;
            // The views of the region are its views still.
            let mut all = views(&dir);
            for v in all.as_object_mut().into_iter().flat_map(|o| o.values_mut()).filter(|v| v["region"] == region) {
                v["region"] = json!(name);
            }
            let _ = std::fs::write(dir.join(VIEWS), serde_json::to_string_pretty(&all).unwrap_or_default());
            Ok(done)
        }
        "add_sheet" => {
            let (dir, like, name) = (dir()?, text_arg(args, "like")?, text_arg(args, "name")?);
            change(&dir, &format!("sheet {name}"), || {
                let regions = store::add_sheet(&dir, like, name)?;
                Ok(json!({ "regions": regions, "run": run_report(&store::run(&dir)?) }))
            })
        }
        "undo" => {
            // The views are not part of what is taken back.
            let (dir, kept) = (dir()?, std::fs::read(dir()?.join(VIEWS)).ok());
            let undone = store::undo(&dir)?;
            if let Some(views) = kept.filter(|_| undone.is_some()) {
                let _ = std::fs::write(dir.join(VIEWS), views);
            }
            Ok(json!({ "undone": undone }))
        }
        "verify" => Ok(json!({ "text": crate::tools::call(cfg, "verify", args)? })),
        "sheets" => {
            let dir = dir()?;
            let wb = workbook(&dir)?;
            let model = store::read_model(&dir)?;
            let sheets: Vec<J> = (wb.sheets.iter().enumerate())
                .map(|(i, s)| {
                    let area = crate::engine::frame::used_area(&wb, i as u32);
                    let regions: Vec<J> = model["regions"].as_array().into_iter().flatten().filter(|r| r["sheet"] == s.name.as_str()).map(|r| json!({ "name": r["name"], "kind": r["kind"], "ref": r["ref"] })).collect();
                    json!({ "name": s.name, "rows": area.map_or(0, |a| a.last_row + 1), "cols": area.map_or(0, |a| a.last_col + 1), "regions": regions })
                })
                .collect();
            Ok(json!({ "sheets": sheets }))
        }
        "cells" => cells(&dir()?, args),
        "set_cells" => {
            let (dir, sheet) = (dir()?, text_arg(args, "sheet")?);
            let typed: Vec<(String, String, J)> = args["cells"].as_array().into_iter().flatten().map(|c| text_arg(c, "cell").map(|at| (sheet.to_string(), at.to_string(), c["value"].clone()))).collect::<Result<_, _>>()?;
            workbook(&dir)?;
            change(&dir, &format!("cells of {sheet}"), || Ok(json!({ "run": run_report(&store::set_cells(&dir, &typed)?) })))
        }
        "save_view" => {
            let (dir, name) = (dir()?, text_arg(args, "name")?);
            let mut all = views(&dir);
            match args["view"].is_null() {
                true => all.as_object_mut().and_then(|o| o.remove(name)).map(|_| ()).unwrap_or(()),
                false => all[name] = json!({ "region": text_arg(args, "region")?, "view": args["view"] }),
            }
            std::fs::write(dir.join(VIEWS), serde_json::to_string_pretty(&all).unwrap_or_default()).map_err(|e| format!("cannot write the views: {e}"))?;
            Ok(all)
        }
        "export" => export(cfg, &dir()?, args),
        // The program (a port of the workbook, see xl_engine::port): its state, made or
        // made anew, checked where Python can run it.
        "port" => port_state(cfg, &dir()?),
        "export_port" => {
            let text = crate::tools::call(cfg, "export_port", &json!({ "store": args["store"], "replace": args["replace"] }))?;
            let mut state = port_state(cfg, &dir()?)?;
            state["text"] = json!(text);
            Ok(state)
        }
        "check_port" => {
            let dir = dir()?;
            let check = crate::engine::port_check().ok_or("checking the program needs Python: the server xlcells-mcp (of the Python package) can, the program xl-mcp cannot; run check.py in the program's folder")?;
            check(&crate::tools::port_folder(&dir))?;
            port_state(cfg, &dir)
        }
        other => Err(format!("no call named {other:?}")),
    }
}

/// The program of a store as the app shows it: every calculated column and cell with how
/// it is ported, whether the last check found it right, and its lines of port/calc.py.
fn port_state(cfg: &Config, dir: &Path) -> Result<J, String> {
    let folder = crate::tools::port_folder(dir);
    let can_check = crate::engine::port_check().is_some();
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).ok();
    let Some(logic) = read("logic.json").and_then(|t| serde_json::from_str::<J>(&t).ok()) else {
        return Ok(json!({ "exists": false, "folder": cfg.shown(&folder), "can_check": can_check }));
    };
    let status = read("status.json").and_then(|t| serde_json::from_str::<J>(&t).ok());
    let calc = read("port/calc.py").unwrap_or_default();
    let lines: Vec<&str> = calc.lines().collect();
    let mut items = logic["items"].as_array().cloned().unwrap_or_default();
    // Each item's line is that of its comment in the program as it is now (lines move as
    // it is changed); logic.json's where the comment is gone.
    for i in items.iter_mut() {
        let (region, name) = (i["region"].as_str().unwrap_or(""), i["name"].as_str().unwrap_or(""));
        let marks = [format!("# {region}[{name}]"), format!("# {region}: {name}")];
        if let Some(k) = lines.iter().position(|l| marks.iter().any(|m| l.trim_start().starts_with(m.as_str()))) {
            i["line"] = json!(k + 1);
        }
    }
    let mut starts: Vec<usize> = items.iter().filter_map(|i| i["line"].as_u64().map(|l| l as usize)).collect();
    starts.sort();
    let results = lines.iter().position(|l| l.trim_start().starts_with("results = {")).map_or(lines.len(), |k| k + 1);
    let out: Vec<J> = items
        .iter()
        .map(|i| {
            let line = i["line"].as_u64().unwrap_or(0) as usize;
            let end = starts.iter().copied().find(|&s| s > line).unwrap_or(results).min(lines.len() + 1);
            let code = if line >= 1 && line < end { lines[line - 1..end - 1].join("\n") } else { String::new() };
            let state = status.as_ref().and_then(|s| s["items"].as_array()).and_then(|list| list.iter().find(|x| x["region"] == i["region"] && x["name"] == i["name"])).map(|x| x["state"].clone()).unwrap_or(J::Null);
            json!({ "region": i["region"], "name": i["name"], "formula": i["formula"], "logic": i["logic"], "how": i["how"], "reason": i["reason"], "line": i["line"], "state": state, "code": code })
        })
        .collect();
    let zip = folder.with_file_name(format!("{}.zip", folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let url = match (&cfg.files_url, zip.exists()) {
        (Some(base), true) => json!(format!("{base}/{}", crate::http::encode(&zip.file_name().unwrap().to_string_lossy()))),
        _ => J::Null,
    };
    Ok(json!({ "exists": true, "folder": cfg.shown(&folder), "items": out, "checked": status.is_some(), "can_check": can_check, "url": url, "file": cfg.shown(&folder) }))
}
