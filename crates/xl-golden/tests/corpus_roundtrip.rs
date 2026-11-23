//! Gate for phase 1: every readable package in the corpus survives read -> write
//! unchanged, entry by entry and byte by byte. Skipped when the corpus is not fetched.

use std::path::Path;
use xl_golden::{corpus_files, guarded, roundtrip, OOXML_EXT};

#[test]
fn corpus_round_trips_losslessly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let files = corpus_files(&root);
    if files.is_empty() {
        eprintln!("corpus not present, run tools/fetch_corpus.sh");
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let (mut ok, mut rejected, mut bad) = (0, 0, Vec::new());
    for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
        let data = std::fs::read(&f.path).unwrap();
        match guarded(|| roundtrip(&data)) {
            Ok(r) if r.status == "ok" && r.model_mismatch == 0 => ok += 1,
            Ok(r) if matches!(r.status, "ole" | "not-zip" | "open-error") => rejected += 1,
            Ok(r) => bad.push(format!("{}: {} {}", f.rel, r.status, r.detail)),
            Err(p) => bad.push(format!("{}: panic {p}", f.rel)),
        }
    }
    std::panic::set_hook(prev);
    assert!(bad.is_empty(), "{} files failed:\n{}", bad.len(), bad.join("\n"));
    assert!(ok > 1000, "only {ok} files round-tripped");
    eprintln!("round-trip ok {ok}, rejected as unreadable {rejected}");
}
