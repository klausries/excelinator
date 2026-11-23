//! `shape_check DIR STEP [DIR STEP ...]`: does a workbook stay built as it was when a
//! table of it gets more rows, and is it the workbook it was when they go again?
//!
//! For every STEP-th workbook under DIR a store is made and run. Then, for up to three
//! of its tables: two rows are added (copies of its last row) and the store is run; the
//! table must be two rows longer and the regions the same by name. The rows are taken
//! away again and the store is run: the workbook must hold what it held, cell by cell.
//! Last, a row is taken away from the table (the regions must stay the same by name).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xl_cells::{Value, Workbook};
use xl_engine::data::{Column, Frame};
use xl_polars::store;

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("xls" | "xlsx" | "xlsm")) {
            out.push(p);
        }
    }
}

/// What the cells of a workbook hold, and its merged ranges.
fn held(wb: &Workbook) -> Vec<String> {
    let c = &wb.cells;
    let mut out: Vec<String> = (0..c.len())
        .filter(|&i| c.kind[i] != xl_cells::Kind::Blank || c.formula[i].is_some())
        .map(|i| {
            let value = match c.value(i) {
                Value::Number(n) => format!("{n:.9e}"),
                Value::Text(t) | Value::Error(t) => t,
                Value::Bool(b) => b.to_string(),
                Value::Blank => String::new(),
            };
            format!("{:03}!{:07}:{:05} {value} ={}", c.sheet[i], c.row[i], c.col[i], c.formula[i].as_deref().unwrap_or(""))
        })
        .collect();
    out.extend(wb.sheets.iter().enumerate().flat_map(|(s, sheet)| sheet.merges.iter().map(move |m| format!("{s:03} merge {m:?}"))));
    out.sort();
    out
}

fn rows(df: &Frame, n: usize) -> Frame {
    let take: Vec<usize> = (0..n).map(|k| k.min(df.height - 1)).collect();
    Frame { height: n, columns: df.columns.iter().map(|c| Column::new(c.name.clone(), c.data.take(&take))).collect() }
}

/// (name, kind, range) of the regions of a store.
fn regions(dir: &Path) -> Vec<(String, String, String)> {
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    store::read_model(dir).map(|m| m["regions"].as_array().into_iter().flatten().map(|r| (text(&r["name"]), text(&r["kind"]), text(&r["ref"]))).collect()).unwrap_or_default()
}

fn names(regions: &[(String, String, String)]) -> Vec<&str> {
    let mut names: Vec<&str> = regions.iter().map(|r| r.0.as_str()).collect();
    names.sort_unstable();
    names
}

/// The regions that are not the same in two lists.
fn other(before: &[(String, String, String)], after: &[(String, String, String)]) -> String {
    let gone: Vec<String> = before.iter().filter(|r| !after.contains(r)).map(|r| format!("{} {} {}", r.0, r.1, r.2)).collect();
    let new: Vec<String> = after.iter().filter(|r| !before.contains(r)).map(|r| format!("{} {} {}", r.0, r.1, r.2)).collect();
    format!("was [{}] is [{}]", gone.join("; "), new.join("; "))
}

fn longer(before: &str, after: &str, by: i64) -> bool {
    let last = |r: &str| r.rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<i64>().ok());
    matches!((last(before), last(after)), (Some(a), Some(b)) if b - a == by)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut count: BTreeMap<String, u64> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    let mut add = |what: &str, example: Option<String>| {
        *count.entry(what.to_string()).or_default() += 1;
        if let Some(e) = example.filter(|_| examples.len() < 60) {
            examples.push(format!("{what}: {e}"));
        }
    };
    for pair in args.chunks(2) {
        let (mut list, step) = (Vec::new(), pair.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1));
        files(Path::new(&pair[0]), &mut list);
        list.sort();
        let only = std::env::var("ONLY").ok();
        for (k, path) in list.iter().enumerate().filter(|(k, p)| only.as_ref().map_or(k % step == 0, |o| p.to_string_lossy().contains(o.as_str()))) {
            let Ok(bytes) = std::fs::read(path) else { continue };
            let xls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls"));
            let Some(wb) = (if xls { xl_xls::read_bytes(&bytes).ok() } else { xl_cells::read_bytes(bytes.clone()).ok() }) else { continue };
            if wb.cells.len() > 20_000 || wb.externals.iter().any(|e| !e.cells.is_empty() || !e.names.is_empty()) {
                continue;
            }
            let dir = std::env::temp_dir().join(format!("xl-shape-check-{}-{k}", std::process::id()));
            let fresh = || -> Result<(), String> {
                let _ = std::fs::remove_dir_all(&dir);
                store::create(&dir, &bytes, &wb, !xls)?;
                store::run(&dir).map(|_| ())
            };
            if fresh().is_err() {
                add("workbooks left out (no store, or not run)", None);
                continue;
            }
            add("workbooks", None);
            let tables: Vec<(String, String, String)> = regions(&dir).into_iter().filter(|r| r.1 == "table").collect();
            let picked: Vec<usize> = if tables.len() <= 3 { (0..tables.len()).collect() } else { vec![0, tables.len() / 2, tables.len() - 1] };
            for (name, _, at) in picked.iter().map(|&i| &tables[i]) {
                let said = || Some(format!("{} {name}", path.display()));
                let (Ok(start), Ok(df)) = (store::read_workbook(&dir).map(|x| held(&x.0)), store::read_region(&dir, name)) else { continue };
                let before = regions(&dir);
                add("tables", None);
                // Two rows more.
                let grown = store::write_frame(&dir, name, &rows(&df, df.height + 2)).and_then(|_| store::run(&dir));
                if let Err(why) = grown {
                    add("more rows: not written", Some(format!("{} {name}: {why}", path.display())));
                    let _ = fresh();
                    continue;
                }
                let now = regions(&dir);
                let ok = names(&now) == names(&before) && now.iter().find(|r| r.0 == *name).is_some_and(|r| longer(at, &r.2, 2));
                add(if ok { "more rows: the regions are the same, the table two rows longer" } else { "more rows: OTHER REGIONS afterwards" }, if ok { None } else { said().map(|s| format!("{s} ({at}): {}", other(&before, &now))) });
                // And back.
                let back = store::write_frame(&dir, name, &df).and_then(|_| store::run(&dir)).and_then(|_| store::read_workbook(&dir)).map(|x| held(&x.0));
                match back {
                    Ok(end) if end == start => add("and back: the workbook as it was", None),
                    Ok(end) => {
                        let first = end.iter().zip(&start).find(|(a, b)| a != b).map(|(a, b)| format!("{a:?} was {b:?}")).unwrap_or_else(|| format!("{} cells, were {}", end.len(), start.len()));
                        add("and back: NOT the workbook it was", Some(format!("{} {name}: {first}", path.display())));
                        let _ = fresh();
                    }
                    Err(why) => {
                        add("and back: not written", Some(format!("{} {name}: {why}", path.display())));
                        let _ = fresh();
                    }
                }
                // A row less.
                if df.height >= 2 {
                    let before = regions(&dir);
                    match store::write_frame(&dir, name, &rows(&df, df.height - 1)).and_then(|_| store::run(&dir)) {
                        Err(why) => add("a row less: not written", Some(format!("{} {name}: {why}", path.display()))),
                        Ok(_) => {
                            let now = regions(&dir);
                            let ok = names(&now) == names(&before);
                            add(if ok { "a row less: the regions are the same" } else { "a row less: OTHER REGIONS afterwards" }, if ok { None } else { said().map(|s| format!("{s} ({at}): {}", other(&before, &now))) });
                        }
                    }
                    let _ = fresh();
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    for (what, n) in &count {
        println!("{n:>7}  {what}");
    }
    for e in &examples {
        println!("{e}");
    }
    println!("done");
}
