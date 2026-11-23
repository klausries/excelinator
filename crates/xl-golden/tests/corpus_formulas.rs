//! Gate for the formula parser: every formula in the corpus parses and prints back
//! identically. Known non-formulas (binary garbage, `#NAME!` placeholders written by
//! broken producers) may be rejected, but never mis-printed.

use calamine::Reader;
use std::path::Path;
use xl_golden::{corpus_files, guarded, OOXML_EXT};

#[test]
fn corpus_formulas_round_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let files = corpus_files(&root);
    if files.is_empty() {
        eprintln!("corpus not present, run tools/fetch_corpus.sh");
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let (mut total, mut rejected, mut bad) = (0u64, 0u64, Vec::new());
    for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
        let data = std::fs::read(&f.path).unwrap();
        let formulas = guarded(move || {
            let mut out: Vec<String> = Vec::new();
            let Ok(mut wb) = calamine::Xlsx::new(std::io::Cursor::new(data)) else { return out };
            for name in wb.sheet_names() {
                if let Ok(range) = wb.worksheet_formula(&name) {
                    out.extend(range.cells().filter(|c| !c.2.is_empty()).map(|c| c.2.clone()));
                }
            }
            out.extend(wb.defined_names().iter().filter(|(_, f)| !f.is_empty()).map(|(_, f)| f.clone()));
            out
        })
        .unwrap_or_default();
        for text in formulas {
            total += 1;
            match xl_formula::parse(&text) {
                Ok(e) if e.to_string() == text => {}
                Ok(_) => bad.push(format!("{}: printed differently: {text}", f.rel)),
                Err(_) if text.contains("#NAME!") || text.contains("_x00") || text.contains('\u{fffd}') || text.chars().any(char::is_control) => rejected += 1,
                Err(e) => bad.push(format!("{}: {e}: {text}", f.rel)),
            }
        }
    }
    std::panic::set_hook(prev);
    assert!(bad.is_empty(), "{} formulas failed:\n{}", bad.len(), bad[..bad.len().min(20)].join("\n"));
    assert!(total > 100_000, "only {total} formulas found");
    eprintln!("formulas {total}, rejected non-formulas {rejected}");
}
