//! .xls files of the corpus against calamine (values and formulas), xlrd's view through
//! LibreOffice's conversions (lo-enron: the same files saved as .xlsx by LibreOffice).

use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use calamine::{Data, Reader, Xls};
use xl_cells::{Kind, Value, Workbook};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn xls_files() -> Vec<PathBuf> {
    let step: usize = std::env::var("XLS_STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut out = Vec::new();
    let mut walk = vec![corpus()];
    while let Some(d) = walk.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xls")) {
                out.push(p);
            }
        }
    }
    out.sort();
    // All test files of the libraries; every n-th of the Enron mail attachments.
    let (enron, other): (Vec<_>, Vec<_>) = out.into_iter().partition(|p| p.to_string_lossy().contains("src-enron"));
    other.into_iter().chain(enron.into_iter().step_by(step)).collect()
}

/// Strings calamine misreads: some 16-bit strings as bytes (with NULs), and long strings
/// over CONTINUE records cut short. xlrd reads them as we do.
fn calamine_string_bug(ours: &Value, theirs: &Data) -> bool {
    match (ours, theirs) {
        (Value::Text(a), Data::String(b)) => b.replace('\0', "") == *a || (b.len() < a.len() && a.starts_with(b.as_str())),
        _ => false,
    }
}

fn same(ours: &Value, theirs: &Data) -> bool {
    match (ours, theirs) {
        (Value::Number(a), Data::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
        (Value::Number(a), Data::Int(b)) => *a == *b as f64,
        (Value::Number(a), Data::DateTime(d)) => *a == d.as_f64(),
        (Value::Text(a), Data::String(b)) => a == b,
        (Value::Bool(a), Data::Bool(b)) => a == b,
        (Value::Error(a), Data::Error(e)) => *a == e.to_string(),
        _ => false,
    }
}

fn cells(wb: &Workbook, sheet: usize) -> (BTreeMap<(u32, u32), Value>, BTreeMap<(u32, u32), String>) {
    let c = &wb.cells;
    let mut v = BTreeMap::new();
    let mut f = BTreeMap::new();
    for i in c.sorted_unique() {
        if c.sheet[i] as usize != sheet {
            continue;
        }
        if c.kind[i] != Kind::Blank {
            v.insert((c.row[i], c.col[i]), c.value(i));
        }
        if let Some(t) = &c.formula[i] {
            f.insert((c.row[i], c.col[i]), t.clone());
        }
    }
    (v, f)
}

#[derive(Default, Debug)]
struct Stats {
    files: usize,
    read: usize,
    errors: BTreeMap<String, usize>,
    calamine_only_reads: usize,
    cells: usize,
    value_diff: usize,
    only_ours: usize,
    only_theirs: usize,
    formulas: usize,
    formula_diff: usize,
    formula_only_theirs: usize,
    formula_only_ours: usize,
    skipped: HashMap<String, usize>,
    examples: BTreeMap<&'static str, Vec<String>>,
}

impl Stats {
    fn note(&mut self, kind: &'static str, s: String) {
        let v = self.examples.entry(kind).or_default();
        // At most two examples per file.
        let file = s.split(' ').next().unwrap_or("").to_string();
        if v.iter().filter(|e| e.starts_with(&file)).count() >= 2 {
            return;
        }
        if v.len() < if kind == "value" || kind == "formula" { 40 } else { 6 } {
            v.push(s);
        }
    }
}

#[test]
fn xls_against_calamine() {
    let mut st = Stats::default();
    for path in xls_files() {
        let bytes = std::fs::read(&path).unwrap();
        st.files += 1;
        let name = path.strip_prefix(corpus()).unwrap().display().to_string();
        let ours = std::panic::catch_unwind(|| xl_xls::read_with_notes(&bytes));
        let theirs: Result<Xls<_>, _> = Xls::new(Cursor::new(bytes.clone()));
        let (wb, notes) = match ours {
            Err(_) => panic!("{name}: panic"),
            Ok(Err(e)) => {
                let key = e.to_string();
                let key: String = key.chars().take(70).collect();
                *st.errors.entry(key).or_default() += 1;
                if theirs.is_ok() {
                    st.calamine_only_reads += 1;
                    st.note("fails", format!("{name}: ours fails ({e}), calamine reads"));
                }
                continue;
            }
            Ok(Ok(x)) => x,
        };
        st.read += 1;
        for (k, n) in notes.skipped {
            *st.skipped.entry(k).or_default() += n;
        }
        let Ok(mut cal) = theirs else { continue };
        for (si, sheet) in wb.sheets.iter().enumerate() {
            let (mut values, mut formulas) = cells(&wb, si);
            if let Ok(range) = cal.worksheet_range(&sheet.name) {
                let (r0, c0) = range.start().unwrap_or((0, 0));
                for (r, c, d) in range.used_cells() {
                    if matches!(d, Data::Empty) {
                        continue;
                    }
                    let at = (r as u32 + r0, c as u32 + c0);
                    match values.remove(&at) {
                        Some(v) => {
                            st.cells += 1;
                            if calamine_string_bug(&v, d) {
                                st.note("calamine string bug", format!("{name} {}!{at:?}", sheet.name));
                            } else if !same(&v, d) {
                                st.value_diff += 1;
                                st.note("value", format!("{name} {}!{at:?}: ours {v:?}, calamine {d:?}", sheet.name));
                            }
                        }
                        None => {
                            st.only_theirs += 1;
                            st.note("only calamine", format!("{name} {}!{at:?}: only calamine {d:?}", sheet.name));
                        }
                    }
                }
                for (at, v) in values {
                    // calamine drops empty strings.
                    if !matches!(&v, Value::Text(t) if t.is_empty()) {
                        st.only_ours += 1;
                        st.note("only ours", format!("{name} {}!{at:?}: only ours {v:?}", sheet.name));
                    }
                }
            }
            if let Ok(range) = cal.worksheet_formula(&sheet.name) {
                let (r0, c0) = range.start().unwrap_or((0, 0));
                for (r, c, f) in range.used_cells() {
                    if f.is_empty() {
                        continue;
                    }
                    let at = (r as u32 + r0, c as u32 + c0);
                    match formulas.remove(&at) {
                        Some(o) => {
                            st.formulas += 1;
                            if &o != f {
                                st.formula_diff += 1;
                                st.note("formula", format!("{name} {}!{at:?}: formula ours {o:?}, calamine {f:?}", sheet.name));
                            }
                        }
                        None => {
                            st.formula_only_theirs += 1;
                            st.note("formula only calamine", format!("{name} {}!{at:?}: formula only calamine {f:?}", sheet.name));
                        }
                    }
                }
                st.formula_only_ours += formulas.len();
            }
        }
    }
    let examples = std::mem::take(&mut st.examples);
    for (k, v) in &examples {
        eprintln!("{k}:");
        for e in v {
            eprintln!("  {e}");
        }
    }
    eprintln!("{st:#?}");
    // Every value calamine reads, we read the same (calamine's own string bugs aside); its
    // formulas are not compared (it drops sheet quotes and misreads 3D columns).
    assert_eq!((st.value_diff, st.only_ours, st.only_theirs), (0, 0, 0));
    assert!(st.cells > 0);
}

/// `Sheet!A1:A1` as `Sheet!A1`: LibreOffice writes single cells of 3D references as ranges.
fn single_cell_ranges(f: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"!(\$?[A-Z]{1,3}\$?\d+):(\$?[A-Z]{1,3}\$?\d+)").unwrap());
    re.replace_all(f, |c: &regex::Captures| if c[1] == c[2] { format!("!{}", &c[1]) } else { c[0].to_string() }).into_owned()
}

fn without_sheet_of_ref_errors(f: &str) -> String {
    let mut out = f.to_string();
    while let Some(at) = out.find("!#REF!") {
        let head = &out[..at];
        let start = if head.ends_with('\'') {
            // The opening quote, past doubled quotes inside the name.
            let b = head.as_bytes();
            let mut i = head.len() - 1;
            loop {
                match head[..i].rfind('\'') {
                    Some(j) if j > 0 && b[j - 1] == b'\'' => i = j - 1,
                    Some(j) => break j,
                    None => break 0,
                }
            }
        } else {
            head.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '[' || c == ']')).map(|i| i + 1).unwrap_or(0)
        };
        out.replace_range(start..at + 1, "");
    }
    out
}

/// The Enron .xls files against LibreOffice's .xlsx conversions of them: every value and
/// every formula that both have.
#[test]
fn xls_against_libreoffice() {
    let mut st = Stats::default();
    let lo = corpus().join("lo-enron");
    let unq = regex::Regex::new(r"'((?:\[<[^>]*>\])?[A-Za-z0-9_][A-Za-z0-9_.]*)'!").unwrap();
    let num = regex::Regex::new(r"\b\d+\.\d{12,}(?:E[-+]\d+)?").unwrap();
    let whole = regex::Regex::new(r"(\$?[A-Z]{1,3})\$?1:(\$?[A-Z]{1,3})\$?1048576\b").unwrap();
    // Whole rows: LibreOffice writes $A$2:$IV9 (or :$XFD9 in other workbooks) for $2:9.
    let rows = regex::Regex::new(r"\$?A(\$?\d+):\$?(?:XFD|IV)(\$?\d+)\b").unwrap();
    for path in xls_files() {
        let rel = path.strip_prefix(corpus().join("src-enron")).ok().map(|p| p.to_path_buf());
        let Some(rel) = rel else { continue };
        let conv = lo.join(format!("{}.xlsx", rel.display()));
        let Ok(conv) = std::fs::read(&conv) else { continue };
        let Ok(theirs) = xl_cells::read_bytes(conv) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        st.files += 1;
        let name = rel.display().to_string();
        let wb = match xl_xls::read_with_notes(&bytes) {
            Ok((wb, notes)) => {
                for (k, n) in notes.skipped {
                    *st.skipped.entry(k).or_default() += n;
                }
                wb
            }
            Err(e) => {
                *st.errors.entry(e.to_string()).or_default() += 1;
                continue;
            }
        };
        st.read += 1;
        for (si, sheet) in wb.sheets.iter().enumerate() {
            let Some(ti) = theirs.sheets.iter().position(|s| s.name == sheet.name) else {
                // LibreOffice leaves out very hidden sheets of old macro workbooks (`***00`).
                if sheet.visibility == xl_cells::Visibility::VeryHidden {
                    continue;
                }
                st.note("sheet", format!("{name}: sheet {} not in LibreOffice's file", sheet.name));
                continue;
            };
            let (values, formulas) = cells(&wb, si);
            let (mut tv, tf) = cells(&theirs, ti);
            // External workbooks by file name: LibreOffice numbers them in its own order.
            let book = |wb: &Workbook, f: &str| {
                let mut f = f.to_string();
                for (k, e) in wb.externals.iter().enumerate() {
                    let t = e.target.clone().unwrap_or_default().replace("%20", " ").replace("%23", "#");
                    let base = t.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
                    f = f.replace(&format!("[{}]", k + 1), &format!("[<{base}>]"));
                }
                f
            };
            // A reference to deleted cells of another sheet: Excel keeps the sheet
            // (`Sheet2!#REF!`), LibreOffice drops it.
            let formulas: BTreeMap<_, _> = formulas.into_iter().map(|(k, f)| (k, without_sheet_of_ref_errors(&book(&wb, &f)).replace("TRUE()", "TRUE").replace("FALSE()", "FALSE"))).collect();
            // LibreOffice writes the constants TRUE and FALSE as functions, and whole
            // columns of other workbooks as ranges ($A$1:$B$1048576 for $A:$B).
            let mut tf: BTreeMap<_, _> = tf
                .into_iter()
                .map(|(k, f)| {
                    let f = whole.replace_all(&book(&theirs, &f).replace("TRUE()", "TRUE").replace("FALSE()", "FALSE"), "$1:$2").into_owned();
                    (k, rows.replace_all(&f, "$1:$2").into_owned())
                })
                .collect();
            // What LibreOffice holds for cells it keeps no formula of.
            let their_values = tv.clone();
            // LibreOffice recalculates on loading: only values of cells without formula compare.
            for at in formulas.keys().chain(tf.keys()) {
                tv.remove(at);
            }
            for (at, v) in values {
                if formulas.contains_key(&at) || tf.contains_key(&at) {
                    continue;
                }
                match tv.remove(&at) {
                    Some(t) => {
                        st.cells += 1;
                        let same = match (&v, &t) {
                            // LibreOffice writes 15 significant digits.
                            (Value::Number(a), Value::Number(b)) => a == b || (a - b).abs() <= 1e-14 * a.abs().max(b.abs()),
                            _ => v == t,
                        };
                        if !same {
                            st.value_diff += 1;
                            st.note("value", format!("{name} {}!{at:?}: ours {v:?}, LibreOffice {t:?}", sheet.name));
                        }
                    }
                    None => {
                        st.only_ours += 1;
                        st.note("only ours", format!("{name} {}!{at:?}: only ours {v:?}", sheet.name));
                    }
                }
            }
            st.only_theirs += tv.len();
            for (at, t) in tv.into_iter().take(1) {
                st.note("only LibreOffice", format!("{name} {}!{at:?}: only LibreOffice {t:?}", sheet.name));
            }
            for (at, f) in formulas {
                match tf.remove(&at) {
                    Some(t) => {
                        st.formulas += 1;
                        // LibreOffice cannot read names of other workbooks ([1]!Name), and
                        // reads references to deleted cells (#REF!) as the cells they were.
                        // Quotes around plain sheet names are optional; we also quote names
                        // that read as cells (NP15), as Excel does since 16384 columns.
                        // Numbers: LibreOffice writes 15 significant digits, we the shortest
                        // spelling that reads back as the same number.
                        let norm = |x: &str| {
                            let x = single_cell_ranges(&unq.replace_all(x, "$1!"));
                            let x = x.as_str();
                            num.replace_all(&x, |c: &regex::Captures| format!("{:.14e}", c[0].parse::<f64>().unwrap_or(0.0))).into_owned()
                        };
                        let (f, t) = (norm(&f), norm(&t));
                        // Names of other workbooks ([1]!Name): LibreOffice assigns them to
                        // another of the external workbooks than the record order says.
                        if f.contains("]!") && t.contains("]!") && f != t {
                            st.note("external name", format!("{name} {}!{at:?}", sheet.name));
                        } else if f != t && f.replace(",)", ",0)").replace("(,", "(0,").replace(",,", ",0,") == t {
                            // A missing argument (IF(a,b,)): LibreOffice writes 0.
                            st.note("missing argument", format!("{name} {}!{at:?}", sheet.name));
                        } else if f != t && !t.contains("#NAME") && !(f.contains("#REF!") && !t.contains("#REF!")) {
                            st.formula_diff += 1;
                            let p = f.chars().zip(t.chars()).take_while(|(a, b)| a == b).count().saturating_sub(20);
                            let (fs, ts): (String, String) = (f.chars().skip(p).collect(), t.chars().skip(p).collect());
                            st.note("formula", format!("{name} {}!{at:?}: formula ours …{fs:?}, LibreOffice …{ts:?}", sheet.name));
                        }
                    }
                    // A function LibreOffice does not know (a macro function): LibreOffice 26
                    // stores the cell as #NAME? without its formula (24 kept the formula).
                    None if matches!(their_values.get(&at), Some(Value::Error(e)) if e == "#NAME?") => {
                        st.note("unknown function (LibreOffice stores #NAME? without the formula)", format!("{name} {}!{at:?}: {f:?}", sheet.name));
                    }
                    None => {
                        st.formula_only_ours += 1;
                        st.note("formula only ours", format!("{name} {}!{at:?}: formula only ours {f:?}", sheet.name));
                    }
                }
            }
            // LibreOffice writes boolean and error cells as formulas (TRUE(), NA()) and keeps
            // DDE links as formulas.
            tf.retain(|_, t| !matches!(t.as_str(), "TRUE" | "FALSE" | "NA()") && !t.starts_with("DDE(") && !t.starts_with('#'));
            st.formula_only_theirs += tf.len();
            for (at, t) in tf.into_iter().take(1) {
                st.note("formula only LibreOffice", format!("{name} {}!{at:?}: formula only LibreOffice {t:?}", sheet.name));
            }
        }
    }
    let examples = std::mem::take(&mut st.examples);
    for (k, v) in &examples {
        eprintln!("{k}:");
        for e in v {
            eprintln!("  {e}");
        }
    }
    eprintln!("{st:#?}");
    // Formulas: what differs is LibreOffice's (see above) or below one in 2,000.
    assert!(st.formulas > 0 && st.formula_diff * 2000 <= st.formulas, "{} of {}", st.formula_diff, st.formulas);
    assert_eq!(st.formula_only_ours, 0);
    // Values of cells without formula: below one in 5,000 (LibreOffice leaves out the
    // cells of some long sheets, and writes 15 digits).
    assert!(st.value_diff * 5000 <= st.cells, "{} of {}", st.value_diff, st.cells);
}
