//! `baked_stats DIR STEP [DIR STEP ...]`: how the values of a workbook go into the
//! engine's expressions, in formula cells of every STEP-th workbook under DIR. An
//! expression that reads only the columns of its row holds for changed data; one with a
//! value of the workbook in it holds for the workbook as it is.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xl_polars::lift::{BAKED_BLOCK, BAKED_CELL, BAKED_DECISION, BAKED_RANGE};

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut chosen = Vec::new();
    for pair in args.chunks(2) {
        let mut all = Vec::new();
        files(Path::new(&pair[0]), &mut all);
        all.sort();
        chosen.extend(all.into_iter().step_by(pair[1].parse().unwrap()));
    }
    let queue = std::sync::Mutex::new(chosen);
    let total = std::sync::Mutex::new((BTreeMap::<u32, u64>::new(), 0u64, 0u64, 0u64));
    std::thread::scope(|s| {
        for _ in 0..std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) {
            s.spawn(|| loop {
                let Some(path) = queue.lock().unwrap().pop() else { break };
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let wb = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls")) { xl_xls::read_bytes(&bytes).ok() } else { xl_cells::read_bytes(bytes).ok() };
                let Some(wb) = wb.filter(|w| w.cells.len() <= 60_000) else { continue };
                let Ok(r) = std::panic::catch_unwind(|| xl_polars::lift::lift_from(&wb, 1)) else { continue };
                let mut t = total.lock().unwrap();
                for (k, v) in &r.baked {
                    *t.0.entry(*k).or_default() += v;
                }
                t.1 += 1;
                t.2 += r.run_cells;
                t.3 += r.lifted_cells;
            });
        }
    });
    let t = total.lock().unwrap();
    let translated: u64 = t.0.values().sum();
    println!("workbooks {}, formula cells {}, translated {}, of those matching the stored value {}", t.1, t.2, translated, t.3);
    let name = |k: u32| {
        let parts: Vec<&str> = [(BAKED_CELL, "fixed cell"), (BAKED_RANGE, "aggregate of a fixed range"), (BAKED_DECISION, "value decides the translation"), (BAKED_BLOCK, "block of values")].iter().filter(|(b, _)| k & b != 0).map(|(_, n)| *n).collect();
        if parts.is_empty() { "only the row's columns".to_string() } else { parts.join(" + ") }
    };
    let mut rows: Vec<(&u32, &u64)> = t.0.iter().collect();
    rows.sort_by_key(|(_, v)| std::cmp::Reverse(**v));
    for (k, v) in rows {
        println!("{:>9} {:5.1} %  {}", v, 100.0 * *v as f64 / translated.max(1) as f64, name(*k));
    }
    for (bit, n) in [(BAKED_CELL, "fixed cell"), (BAKED_RANGE, "aggregate of a fixed range"), (BAKED_DECISION, "value decides the translation"), (BAKED_BLOCK, "block of values")] {
        let v: u64 = t.0.iter().filter(|(k, _)| *k & bit != 0).map(|(_, v)| *v).sum();
        println!("any use of: {n}: {v} ({:.1} %)", 100.0 * v as f64 / translated.max(1) as f64);
    }
}
