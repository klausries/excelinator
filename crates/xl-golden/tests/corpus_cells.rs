//! Gate: the cell reader agrees with calamine on every corpus file, except for
//! documented calamine deviations, and read → write → read is the identity.

use std::path::Path;
use xl_golden::{cells::compare, corpus_files, guarded, OOXML_EXT};

/// Files where calamine is wrong (checked by hand against the XML and openpyxl):
/// shared formulas whose master is not the first cell of the range, and a
/// shared-string index with trailing whitespace.
const CALAMINE_DEVIATES: &[&str] = &["51710.xlsx", "FormatConditionTests.xlsx", "bug69769.xlsx"];

/// Files only we reject, each for a reason we keep: checksum errors, duplicate part
/// names (invalid per OPC), and the Excel 2007 beta format.
const REJECTED_BY_US: &[&str] = &[
    "clusterfuzz-testcase-minimized-POIXSSFFuzzer-5265527465181184.xlsx",
    "clusterfuzz-testcase-minimized-POIXSSFFuzzer-5937385319563264.xlsx",
    "duplicate-filename-case-insensitive.xlsx",
    "sample-beta.xlsx",
];

/// Files we read but refuse to write back: LibreOffice stored a formula result longer
/// than the 32,767 characters a cell can hold in Excel.
const NOT_WRITABLE: &[&str] = &["ReptFunctionTestCaseData.xls.xlsx"];

#[test]
fn cells_agree_with_calamine_and_round_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let files = corpus_files(&root);
    if files.is_empty() {
        eprintln!("corpus not fetched; run tools/fetch_corpus.sh");
        return;
    }
    let mut failures = Vec::new();
    let mut checked = 0;
    for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
        let data = std::fs::read(&f.path).unwrap();
        let name = f.rel.rsplit('/').next().unwrap();
        let st = match guarded(move || compare(&data)) {
            Ok(st) => st,
            Err(e) => {
                failures.push(format!("{}: panic {e}", f.rel));
                continue;
            }
        };
        checked += 1;
        let diffs = st.value_diff + st.only_ours + st.only_calamine + st.formula_diff + st.formula_only_ours + st.formula_only_calamine;
        let bad = match st.status.as_str() {
            "rejected-ours" => !REJECTED_BY_US.contains(&name),
            "ok" => (diffs > 0 && !CALAMINE_DEVIATES.contains(&name)) || (st.roundtrip == Some(false) && !NOT_WRITABLE.contains(&name)),
            _ => false,
        };
        if bad {
            failures.push(format!("{}: {} {:?}", f.rel, st.status, st.examples));
        }
    }
    assert!(failures.is_empty(), "{} of {checked} files:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn saving_onto_the_original_keeps_everything_else() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let files = corpus_files(&root);
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
        let data = std::fs::read(&f.path).unwrap();
        if data.len() > 4_000_000 {
            continue;
        }
        match guarded(move || xl_golden::cells::update_check(&data)) {
            Ok(Ok(kind)) => *counts.entry(kind).or_insert(0u32) += 1,
            Ok(Err(e)) => failures.push(format!("{}: {e}", f.rel)),
            Err(e) => failures.push(format!("{}: panic {e}", f.rel)),
        }
    }
    eprintln!("update: {counts:?}");
    assert!(failures.is_empty(), "{} files:\n{}", failures.len(), failures.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn renaming_adding_and_removing_sheets_keeps_everything_else() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let files = corpus_files(&root);
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
        let data = std::fs::read(&f.path).unwrap();
        if data.len() > 4_000_000 {
            continue;
        }
        match guarded(move || xl_golden::cells::structure_check(&data)) {
            Ok(Ok(kind)) => *counts.entry(kind).or_insert(0u32) += 1,
            Ok(Err(e)) => failures.push(format!("{}: {e}", f.rel)),
            Err(e) => failures.push(format!("{}: panic {e}", f.rel)),
        }
    }
    eprintln!("structure: {counts:?}");
    assert!(failures.is_empty(), "{} files:\n{}", failures.len(), failures.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}
