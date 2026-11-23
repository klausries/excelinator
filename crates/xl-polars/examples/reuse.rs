//! `reuse DIR STEP [DIR STEP ...]`: do the engine's translations hold for other data?
//!
//! For every STEP-th workbook under DIR: its numbers that are not formulas (dates left
//! alone) are multiplied by 1.37, the workbook is calculated again, and the formulas are
//! translated once on the original and run on the inputs of the changed workbook, with
//! fixed cells baked into the expressions (`baked`) and as inputs (`param`). What the
//! changed workbook has is the reference. Where a translation's assumptions are violated
//! by the change (a value it was shaped by changed) a difference is expected.
use std::path::{Path, PathBuf};
use xl_cells::{Kind, Workbook};
use xl_polars::builtin::PolarsExecutor;
use xl_polars::lift::{calculate_opts, reuse_check, Options, Reuse};

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

fn read(path: &Path) -> Option<Workbook> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls")) {
        xl_xls::read_bytes(&bytes).ok()
    } else {
        xl_cells::read_bytes(bytes).ok()
    }
}

fn one(path: &Path) -> Option<(Reuse, Reuse, bool)> {
    let old = read(path)?;
    if old.cells.len() > 40_000 || old.externals.iter().any(|e| !e.cells.is_empty() || !e.names.is_empty()) {
        return None;
    }
    let ex = PolarsExecutor::default();
    let mut changed = old.clone();
    for i in 0..changed.cells.len() {
        let date = changed.styles.get(changed.cells.style[i] as usize).is_some_and(|s| s.is_date());
        if changed.cells.formula[i].is_none() && changed.cells.kind[i] == Kind::Number && !date {
            changed.cells.num[i] *= 1.37;
        }
    }
    let (data, calc) = calculate_opts(&ex, &changed, Options::default());
    let settled = calc.settled && calc.not_computed.keys().all(|k| !k.starts_with("circular reference"));
    if !settled {
        return None;
    }
    let left: std::collections::HashSet<usize> = calc.uncomputed.iter().map(|(i, _)| *i).collect();
    Some((reuse_check(&ex, &old, &data, &left, Options { param: false }), reuse_check(&ex, &old, &data, &left, Options { param: true }), true))
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
    let total = std::sync::Mutex::new((Reuse::default(), Reuse::default(), 0u64, 0u64));
    std::thread::scope(|s| {
        for _ in 0..std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) {
            s.spawn(|| loop {
                let Some(path) = queue.lock().unwrap().pop() else { break };
                let r = std::panic::catch_unwind(|| one(&path));
                let mut t = total.lock().unwrap();
                match r {
                    Ok(Some((a, b, _))) => {
                        t.0.add(&a);
                        t.1.add(&b);
                        t.2 += 1;
                    }
                    _ => t.3 += 1,
                }
            });
        }
    });
    let t = total.lock().unwrap();
    println!("workbooks {} (left out: {})", t.2, t.3);
    for (name, r) in [("fixed cells baked in", &t.0), ("fixed cells as inputs", &t.1)] {
        let pct = |n: u64| 100.0 * n as f64 / r.cells.max(1) as f64;
        println!("\n{name}: {} translated cells", r.cells);
        println!("  right on the changed data      {:>9} {:5.1} %", r.ok, pct(r.ok));
        println!("  assumption violated (expected) {:>9} {:5.1} %", r.violated, pct(r.violated));
        println!("  WRONG, assumptions hold        {:>9} {:5.1} %", r.wrong, pct(r.wrong));
        println!("  could not be evaluated         {:>9} {:5.1} %", r.failed, pct(r.failed));
        println!("  (cells the calculation left alone, not compared: {})", r.left);
        for (why, n) in &r.why_violated {
            println!("      violated: {why}: {n}");
        }
        for e in r.examples.iter().take(std::env::var("EXAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(5)) {
            println!("      {e}");
        }
    }
}
