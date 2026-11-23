//! `migrate_stats DIR [STEP]`: what the migration path makes of real workbooks. For every
//! STEP-th .xls/.xlsx/.xlsm under DIR: the regions found, how the calculated columns and
//! cells are written in the Polars program (and why not, where the engine has to), and
//! whether the regions' own frames go back into the workbook without changing a cell.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xl_polars::codegen::{polars_program, Accuracy};
use xl_polars::regions::regions;

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
struct Stats {
    files: u64,
    unreadable: u64,
    too_large: u64,
    panics: Vec<String>,
    regions: BTreeMap<&'static str, u64>,
    formula_cells: u64,
    in_columns: u64,
    exceptions: u64,
    columns_with_exceptions: u64,
    in_cells: u64,
    items: u64,
    polars: u64,
    reasons: BTreeMap<String, u64>,
    frames_failed: u64,
    written_back: u64,
    write_refused: BTreeMap<String, u64>,
    cells_changed: u64,
    files_changed: Vec<String>,
    seconds: f64,
}

fn one(path: &Path, s: &mut Stats) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let wb = if xl_xls::is_cfb(&bytes) { xl_xls::read_bytes(&bytes).map_err(|e| e.to_string()) } else { xl_cells::read_bytes(bytes).map_err(|e| e.to_string()) };
    let Ok(wb) = wb else {
        s.unreadable += 1;
        return;
    };
    if wb.cells.len() > 200_000 {
        s.too_large += 1;
        return;
    }
    s.files += 1;
    let found = regions(&wb);
    s.formula_cells += wb.cells.formula.iter().filter(|f| f.as_deref().is_some_and(|x| !x.is_empty())).count() as u64;
    for r in &found {
        *s.regions.entry(r.kind.as_str()).or_default() += 1;
        if let Some((a, z)) = r.data_rows() {
            for c in &r.columns {
                let own = c.exceptions.iter().filter(|e| e.formula.is_some()).count() as u64;
                s.in_columns += own + if c.formula.is_some() { (z - a + 1) as u64 - c.exceptions.len() as u64 } else { 0 };
                s.exceptions += c.exceptions.len() as u64;
                s.columns_with_exceptions += !c.exceptions.is_empty() as u64;
            }
        }
        s.in_cells += r.cells.iter().filter(|c| c.formula.is_some()).count() as u64;
    }
    let program = polars_program(&wb, "book.xlsx", Accuracy::Plain, &[], Some(&xl_polars::builtin::PolarsExecutor::default()));
    for item in &program.items {
        s.items += 1;
        if item.how == "polars" {
            s.polars += 1;
        } else {
            let why = item.reason.clone().unwrap_or_default();
            // Reasons that name a cell or a function are counted by their kind.
            let key = if let Some(f) = why.strip_prefix("the function ") { format!("function {f}") } else if why.contains(" is not a cell of a region") { "a cell outside the regions".into() } else if why.contains("another row of a table") { "a cell of another row of a table".into() } else if why.starts_with("the name ") { "a defined name (not a reference, or unknown)".into() } else if why.starts_with("reference to ") { "another workbook or unknown sheet".into() } else if why.starts_with("the whole rows or columns") { "whole rows or columns".into() } else { why };
            *s.reasons.entry(key).or_default() += 1;
        }
    }
    // The regions' own frames back into the workbook: no cell may change.
    let mut back = wb.clone();
    let mut items = Vec::new();
    let at = xl_polars::frame::cell_index(&wb);
    for r in &found {
        let Ok(df) = xl_polars::regions::region_frame_in(&wb, &at, r) else {
            s.frames_failed += 1;
            continue;
        };
        items.push((r.clone(), df));
    }
    // One at a time only where the whole does not go through, to count what is refused.
    if xl_polars::apply::write_regions(&mut back, &items).is_ok() {
        s.written_back += items.len() as u64;
    } else {
        back = wb.clone();
        for (r, df) in &items {
            match xl_polars::apply::write_region(&mut back, r, df) {
                Ok(_) => s.written_back += 1,
                Err(why) => *s.write_refused.entry(why.split(':').next().unwrap_or("").chars().filter(|c| !c.is_ascii_digit()).take(60).collect()).or_default() += 1,
            }
        }
    }
    let (a, b) = (&wb.cells, &back.cells);
    let mut changed = 0u64;
    for i in 0..a.len() {
        let same_number = a.num[i] == b.num[i] || (a.num[i] - b.num[i]).abs() <= 1e-9 * a.num[i].abs().max(1.0);
        // An error value and an empty text are null in a frame: they come back empty.
        let lossy = matches!(a.kind[i], xl_cells::Kind::Error) || (a.kind[i] == xl_cells::Kind::Text && a.text[i].as_deref().is_none_or(str::is_empty));
        if a.formula[i] != b.formula[i] || (!lossy && (a.kind[i] != b.kind[i] || !same_number || a.text[i] != b.text[i])) {
            changed += 1;
        }
    }
    if changed > 0 {
        s.cells_changed += changed;
        if s.files_changed.len() < 12 {
            s.files_changed.push(format!("{} ({changed})", path.display()));
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("migrate_stats DIR [STEP]");
    let step: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(14);
    let mut all = Vec::new();
    files(Path::new(&dir), &mut all);
    all.sort();
    let mut s = Stats::default();
    let start = std::time::Instant::now();
    std::panic::set_hook(Box::new(|_| {}));
    for path in all.iter().step_by(step) {
        let mut local = Stats::default();
        std::mem::swap(&mut local, &mut s);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut st = local;
            one(path, &mut st);
            st
        }));
        match result {
            Ok(st) => s = st,
            Err(_) => {
                // What was counted before this file is lost with the panic: say so.
                eprintln!("PANIC {}", path.display());
                s.panics.push(path.display().to_string());
            }
        }
    }
    s.seconds = start.elapsed().as_secs_f64();
    println!("files of {dir} (every {step}.): {} read, {} unreadable, {} over 200,000 cells, {} panics, {:.0} s", s.files, s.unreadable, s.too_large, s.panics.len(), s.seconds);
    println!("regions: {:?}", s.regions);
    println!("formula cells: {} (in columns {}, single cells {}); {} rows are exceptions, in {} columns", s.formula_cells, s.in_columns, s.in_cells, s.exceptions, s.columns_with_exceptions);
    println!("calculated columns and cells: {} ({} as Polars, {:.1} %)", s.items, s.polars, 100.0 * s.polars as f64 / s.items.max(1) as f64);
    let mut reasons: Vec<_> = s.reasons.iter().collect();
    reasons.sort_by_key(|x| std::cmp::Reverse(*x.1));
    for (why, n) in reasons.iter().take(25) {
        println!("  engine {n:>7}  {why}");
    }
    println!("write-back of the regions' own frames: {} written, {} frames failed, refused: {:?}", s.written_back, s.frames_failed, s.write_refused);
    println!("cells changed by it: {} {:?}", s.cells_changed, s.files_changed);
    for p in s.panics.iter().take(10) {
        println!("panic: {p}");
    }
}
