//! `baseline OUT DIR STEP [DIR STEP ...]`: every value the engine calculates, cell by
//! cell, for every STEP-th .xls/.xlsx/.xlsm under each DIR, written to OUT (one line
//! per formula cell, files in sorted order). Two runs of the same files compare with
//! `diff`: a change of the engine that should not change behaviour must give the same file.
//! The formula values are removed first, as in the golden tests; cells on circular
//! references keep theirs (1 % off). Also the lift report and the generated program.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use xl_cells::Workbook;
use xl_polars::codegen::{polars_program, Accuracy};

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

fn dump(path: &Path) -> String {
    let Some(stored) = read(path) else { return format!("{}\tunreadable\n", path.display()) };
    if stored.cells.len() > 60_000 {
        return format!("{}\ttoo large\n", path.display());
    }
    let name = path.display().to_string();
    let ex = xl_polars::builtin::PolarsExecutor::default();
    let mut out: String = xl_polars::lift::recalculation_opts(&ex, &stored, xl_polars::lift::default_options()).lines().map(|l| format!("{name}\t{l}\n")).collect();
    let report = xl_polars::lift::lift(&stored);
    out += &format!("{name}\tlift\truns {} cells {} lifted {} {} mismatch {} {} unsupported {:?}\n", report.runs, report.run_cells, report.lifted_runs, report.lifted_cells, report.mismatch_runs, report.mismatch_cells, report.unsupported);
    let program = polars_program(&stored, "book.xlsx", Accuracy::Plain, &[], Some(&ex));
    let mut h: u64 = 0xcbf29ce484222325;
    for b in program.source.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    out += &format!("{name}\tprogram\t{h:016x} {} items\n", program.items.len());
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out_path = PathBuf::from(&args[0]);
    let mut chosen = Vec::new();
    for pair in args[1..].chunks(2) {
        let mut all = Vec::new();
        files(Path::new(&pair[0]), &mut all);
        all.sort();
        let step: usize = pair[1].parse().unwrap();
        chosen.extend(all.into_iter().step_by(step));
    }
    let total = chosen.len();
    let queue = Arc::new(Mutex::new(chosen.into_iter().enumerate().collect::<Vec<_>>()));
    let results = Arc::new(Mutex::new(vec![String::new(); total]));
    let threads: Vec<_> = (0..std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .map(|_| {
            let (queue, results) = (queue.clone(), results.clone());
            std::thread::spawn(move || loop {
                let Some((k, path)) = queue.lock().unwrap().pop() else { break };
                let text = std::panic::catch_unwind(|| dump(&path)).unwrap_or_else(|_| format!("{}\tpanic\n", path.display()));
                results.lock().unwrap()[k] = text;
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let all = results.lock().unwrap().concat();
    std::fs::write(&out_path, &all).unwrap();
    eprintln!("{total} files, {} lines -> {}", all.lines().count(), out_path.display());
}
