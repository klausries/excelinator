//! The Enron .xls files against Excel itself: the formula values Excel computed and
//! stored in them, removed and calculated again. `CALC_STEP=1` takes every file
//! (20,000; the default every 40th).

use std::path::Path;

mod golden;

#[test]
fn calculated_values_match_what_excel_stored() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/src-enron");
    if !root.exists() {
        return;
    }
    let mut files: Vec<_> = golden::walk(&root).into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls"))).collect();
    files.sort();
    let mut g = golden::Golden::new(golden::Reference::Excel);
    let mut manual = 0;
    for path in golden::chosen(&files, 40) {
        let Ok(stored) = xl_xls::read_bytes(&std::fs::read(path).unwrap()) else { continue };
        // Saved with manual calculation: the stored values need not be current.
        if stored.extra.iter().any(|(n, x)| n == "calcPr" && x.contains("calcMode=\"manual\"")) {
            manual += 1;
            continue;
        }
        g.file(path, &stored);
    }
    eprintln!("left out, saved with manual calculation: {manual}");
    g.check(40, 90);
}

/// The Enron .xls files saved with iterative (and automatic) calculation that have
/// circular references: worked out by iteration, they must give what Excel stored. Every
/// formula cell starts 1 % off its stored value, so the iteration has to find it.
#[test]
fn iterated_cycles_match_what_excel_stored() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/src-enron");
    if !root.exists() {
        return;
    }
    let mut files: Vec<_> = golden::walk(&root).into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xls"))).collect();
    files.sort();
    let mut g = golden::Golden::new(golden::Reference::Excel);
    // Larger workbooks with `CALC_MAX_CELLS` (the largest take many minutes each).
    g.max_cells = std::env::var("CALC_MAX_CELLS").ok().and_then(|s| s.parse().ok()).unwrap_or(60_000);
    g.perturb_all = true;
    let step = std::env::var("CALC_STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let only = std::env::var("CALC_ONLY").ok();
    let mut iterating = 0;
    let mut cyclic = 0;
    for path in &files {
        if only.as_deref().is_some_and(|o| !path.to_string_lossy().contains(o)) {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        let Ok(Some(calc)) = xl_xls::calc_settings(&bytes) else { continue };
        if !calc.contains("iterate=\"1\"") || calc.contains("calcMode=\"manual\"") {
            continue;
        }
        let Ok(stored) = xl_xls::read_bytes(&bytes) else { continue };
        iterating += 1;
        // Only those that have circular references (the rest the test above covers).
        if xl_polars::lift::circular_cells(&stored).is_empty() {
            continue;
        }
        cyclic += 1;
        if cyclic % step != 0 {
            continue;
        }
        g.file(path, &stored);
    }
    eprintln!("files with iterative calculation: {iterating}, with circular references: {cyclic}");
    g.check(0, 0);
    if only.is_none() {
        assert!(g.iterated > 0);
    }
}
