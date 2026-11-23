//! `notes_check DIR STEP [DIR STEP ...]`: does the logic kept with the data calculate as
//! the workbook does?
//!
//! For every STEP-th workbook under DIR a store is made; the numbers of its Parquet
//! files that are not calculated are multiplied by 1.37 (dates left alone); the files
//! are calculated again from their notes alone (`store::recalculate`). The reference is
//! the workbook with the same data, calculated by the engine. Counted in cells: what the
//! notes calculated and whether it is the same, and what they left as it was, by reason.
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use xl_cells::Workbook;
use xl_engine::data::{Data, Frame};
use xl_engine::plans::{value_at, V};
use xl_engine::regions::{region_frame_in, regions, RegionKind};
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

#[derive(Default)]
struct Count {
    workbooks: u64,
    left_out: u64,
    /// Calculated cells of regions.
    cells: u64,
    /// Of them: rows of a column with a formula of their own.
    exception_cells: u64,
    done: u64,
    same: u64,
    differ: u64,
    /// Calculated from something kept: the same as the workbook, or not (as expected).
    stale_same: u64,
    stale_differ: u64,
    /// Cells the engine left as they were: nothing to compare with.
    no_reference: u64,
    kept: BTreeMap<String, u64>,
    from_kept: u64,
    note_bytes: u64,
    data_bytes: u64,
    examples: Vec<String>,
}

fn same(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1e-300) || (x - y).abs() < 1e-12,
        // Numbers in a column of mixed kinds are text.
        (V::Text(x), V::Text(y)) => x == y || matches!((x.parse::<f64>(), y.parse::<f64>()), (Ok(x), Ok(y)) if same(&V::Num(x), &V::Num(y))),
        (V::Text(t), V::Null) | (V::Null, V::Text(t)) => t.is_empty(),
        _ => a == b || matches!((a.num(), b.num()), (Some(x), Some(y)) if x == y),
    }
}

fn one(path: &Path, k: usize) -> Option<Count> {
    let bytes = std::fs::read(path).ok()?;
    let xls = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls"));
    let wb: Workbook = if xls { xl_xls::read_bytes(&bytes).ok()? } else { xl_cells::read_bytes(bytes.clone()).ok()? };
    if wb.cells.len() > 20_000 || wb.externals.iter().any(|e| !e.cells.is_empty() || !e.names.is_empty()) {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("xl-notes-check-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = (|| -> Result<Count, String> {
        let mut c = Count { workbooks: 1, ..Default::default() };
        store::create(&dir, &bytes, &wb, !xls)?;
        let b = xl_polars::Polars::default();
        let noted = xl_engine::store::noted_files(&b, &dir)?;
        for (path, note) in &noted {
            c.note_bytes += note.to_string().len() as u64;
            c.data_bytes += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }
        // Other data: every number that is not calculated.
        for (_, note) in &noted {
            let region = note["region"].as_str().unwrap_or("");
            let mut frame = store::read_frame(&dir, region)?;
            let calculated: HashSet<&str> = note["columns"].as_array().into_iter().flatten().filter(|x| x["formula"].is_string()).filter_map(|x| x["name"].as_str()).collect();
            let mut changed = false;
            if xl_polars::long::is_long(&frame) {
                // Single cells, a row each: the numbers of those that are not calculated.
                let names: Vec<Option<String>> = match &frame.column("name").unwrap().data {
                    Data::String(v) => v.clone(),
                    _ => Vec::new(),
                };
                if let Some(Data::Float64(v)) = frame.columns.iter_mut().find(|c| c.name == "number").map(|c| &mut c.data) {
                    for (x, name) in v.iter_mut().zip(&names) {
                        if !name.as_deref().is_some_and(|n| calculated.contains(n)) {
                            *x = x.map(|x| x * 1.37);
                        }
                    }
                }
                // (A whole number times 1.37 is none any more.)
                if let Some(Data::String(v)) = frame.columns.iter_mut().find(|c| c.name == "dtype").map(|c| &mut c.data) {
                    for (d, name) in v.iter_mut().zip(&names) {
                        if d.as_deref() == Some("Int64") && !name.as_deref().is_some_and(|n| calculated.contains(n)) {
                            *d = Some("Float64".into());
                        }
                    }
                }
                store::write_frame(&dir, region, &frame)?;
                continue;
            }
            for col in frame.columns.iter_mut().filter(|col| !calculated.contains(col.name.as_str())) {
                let new = match &col.data {
                    Data::Int64(v) => Some(v.iter().map(|x| x.map(|x| x as f64 * 1.37)).collect()),
                    Data::Float64(v) => Some(v.iter().map(|x| x.map(|x| x * 1.37)).collect()),
                    _ => None,
                };
                if let Some(v) = new {
                    col.data = Data::Float64(v);
                    changed = true;
                }
            }
            if changed {
                store::write_frame(&dir, region, &frame)?;
            }
        }
        let report = store::recalculate(&dir)?;
        // The reference: the workbook with the same data, calculated by the engine.
        let (mut book, _) = store::read_workbook(&dir)?;
        let found = regions(&book);
        let mine: Vec<Frame> = found.iter().map(|r| store::read_wide(&dir, &r.name)).collect::<Result<_, _>>()?;
        let items: Vec<_> = found.iter().cloned().zip(mine.iter().cloned()).collect();
        xl_engine::apply::write_regions(&mut book, &items)?;
        let (done, calc) = xl_polars::lift::calculate(&book);
        let left: HashSet<(u32, u32, u32)> = calc.uncomputed.iter().map(|(i, _)| (done.cells.sheet[*i], done.cells.row[*i], done.cells.col[*i])).collect();
        let at = xl_engine::frame::cell_index(&done);
        let after = regions(&done);
        let was_done: HashSet<(&str, &str)> = report.done.iter().map(|(r, n)| (r.as_str(), n.as_str())).collect();
        let stale: HashSet<(&str, &str)> = report.from_kept.iter().map(|(r, n, _)| (r.as_str(), n.as_str())).collect();
        for (r, a) in found.iter().zip(&mine) {
            let Some(r2) = after.iter().find(|x| x.name == r.name) else { continue };
            let reference = region_frame_in(&done, &at, r2)?;
            // Every calculated cell: (name in the data, name in the report, sheet column, sheet row, row of the data).
            let mut entries: Vec<(&str, String, u32, u32, usize)> = Vec::new();
            match r.kind {
                RegionKind::Cells => entries.extend(r.cells.iter().filter(|x| x.formula.is_some()).map(|x| (x.name.as_str(), x.name.clone(), x.col, x.row, 0))),
                _ => {
                    let first = r2.data_rows().map_or(0, |d| d.0);
                    for x in r.columns.iter().filter(|x| x.is_calculated()) {
                        for k in 0..a.height {
                            match x.exception(first + k as u32) {
                                Some(e) if e.formula.is_some() => {
                                    c.exception_cells += 1;
                                    entries.push((x.name.as_str(), format!("{} (row {})", x.name, k + 1), x.col, first + k as u32, k));
                                }
                                Some(_) => {}
                                // A row of the column: worked out with the column, or on its own.
                                None if x.formula.is_some() => {
                                    let alone = format!("{} (row {})", x.name, k + 1);
                                    let said = if was_done.contains(&(r.name.as_str(), alone.as_str())) { alone } else { x.name.clone() };
                                    entries.push((x.name.as_str(), said, x.col, first + k as u32, k));
                                }
                                None => {}
                            }
                        }
                    }
                }
            }
            for (name, said, col, row, k) in entries {
                c.cells += 1;
                if !was_done.contains(&(r.name.as_str(), said.as_str())) {
                    continue;
                }
                c.done += 1;
                let (Some(x), Some(y)) = (a.column(name), reference.column(name)) else { continue };
                if left.contains(&(r.area.sheet, row, col)) {
                    c.no_reference += 1;
                    continue;
                }
                let (va, vb) = (value_at(&x.data, k, done.date1904), value_at(&y.data, k, done.date1904));
                if stale.contains(&(r.name.as_str(), said.as_str())) {
                    if same(&va, &vb) {
                        c.stale_same += 1;
                    } else {
                        c.stale_differ += 1;
                    }
                } else if same(&va, &vb) {
                    c.same += 1;
                } else {
                    c.differ += 1;
                    if c.examples.len() < 3 {
                        c.examples.push(format!("{} {}[{said}] row {k}: notes {va:?}, workbook {vb:?}", path.display(), r.name));
                    }
                }
            }
        }
        let heights: std::collections::HashMap<&str, usize> = found.iter().zip(&mine).map(|(r, f)| (r.name.as_str(), if r.kind == RegionKind::Cells { 1 } else { f.height })).collect();
        // `WHY=text` prints what was kept for a reason that holds the text.
        let asked = std::env::var("WHY").ok();
        let mut shown = 0;
        for (region, name, formula, why) in &report.kept {
            if asked.as_deref().is_some_and(|a| why.contains(a)) && shown < std::env::var("SHOW").ok().and_then(|v| v.parse().ok()).unwrap_or(3) {
                shown += 1;
                eprintln!("WHY {} | {region} | {name} | ={formula} | {why}", path.display());
            }
            // Reasons that name a cell, a column or a region are counted by their kind.
            let why: String = why.split(':').next().unwrap_or(why).chars().take(70).collect();
            let why = if why.starts_with("it depends on the value of ") {
                "it depends on the value of a cell".to_string()
            } else if why.ends_with("is not a column of a table over these rows") {
                "a column that is not one of a table over these rows".to_string()
            } else if let Some(k) = why.find(" held ").filter(|_| why.contains(" and holds ")) {
                format!("data{}", &why[k..]).replace("a cell it reads", "a cell")
            } else {
                why
            };
            *c.kept.entry(why).or_default() += if name.contains(" (row ") { 1 } else { heights.get(region.as_str()).copied().unwrap_or(1) as u64 };
        }
        c.from_kept = report.from_kept.len() as u64;
        Ok(c)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    match out {
        Ok(c) => Some(c),
        Err(why) => {
            eprintln!("{}: {why}", path.display());
            None
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut chosen = Vec::new();
    for pair in args.chunks(2) {
        let mut all = Vec::new();
        files(Path::new(&pair[0]), &mut all);
        all.sort();
        chosen.extend(all.into_iter().step_by(pair[1].parse().unwrap()));
    }
    let queue = std::sync::Mutex::new(chosen.into_iter().enumerate().collect::<Vec<_>>());
    let total = std::sync::Mutex::new(Count::default());
    std::thread::scope(|s| {
        for _ in 0..std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) {
            s.spawn(|| loop {
                let Some((k, path)) = queue.lock().unwrap().pop() else { break };
                let r = std::panic::catch_unwind(|| one(&path, k));
                let mut t = total.lock().unwrap();
                match r {
                    Ok(Some(c)) => {
                        t.workbooks += 1;
                        t.cells += c.cells;
                        t.exception_cells += c.exception_cells;
                        t.done += c.done;
                        t.same += c.same;
                        t.differ += c.differ;
                        t.stale_same += c.stale_same;
                        t.stale_differ += c.stale_differ;
                        t.no_reference += c.no_reference;
                        t.from_kept += c.from_kept;
                        t.note_bytes += c.note_bytes;
                        t.data_bytes += c.data_bytes;
                        for (why, n) in c.kept {
                            *t.kept.entry(why).or_default() += n;
                        }
                        t.examples.extend(c.examples);
                    }
                    _ => t.left_out += 1,
                }
            });
        }
    });
    let t = total.lock().unwrap();
    let pct = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;
    println!("workbooks {} (left out: {})", t.workbooks, t.left_out);
    println!("calculated cells of regions       {:>9}", t.cells);
    println!("  (rows with a formula of their own: {})", t.exception_cells);
    println!("  calculated again from the notes {:>9} {:5.1} %", t.done, pct(t.done, t.cells));
    println!("    as the workbook               {:>9} {:5.1} %", t.same, pct(t.same, t.same + t.differ));
    println!("    DIFFERENT                     {:>9} {:5.1} %", t.differ, pct(t.differ, t.same + t.differ));
    println!("    from something kept: {} as the workbook, {} not (expected)", t.stale_same, t.stale_differ);
    println!("    (the workbook's engine left the cell alone, not compared: {})", t.no_reference);
    println!("  kept as they were               {:>9} {:5.1} %", t.cells - t.done, pct(t.cells - t.done, t.cells));
    let mut kept: Vec<_> = t.kept.iter().collect();
    kept.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (why, n) in kept.iter().take(std::env::var("REASONS").ok().and_then(|v| v.parse().ok()).unwrap_or(25)) {
        println!("      {n:>8}  {why}");
    }
    println!("columns and cells calculated from something kept: {}", t.from_kept);
    println!("notes: {} bytes beside {} bytes of Parquet files", t.note_bytes, t.data_bytes);
    for e in t.examples.iter().take(std::env::var("EXAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(12)) {
        println!("      {e}");
    }
}
