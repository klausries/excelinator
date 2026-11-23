//! Conformance cases shared by all language bindings (tests/conformance/cases.txt).
//!
//! This test is the reference: it writes every case, reads it back, and compares the
//! hashes of the file and of the table with tests/conformance/expected.txt. The
//! Python, JavaScript and Dart tests must arrive at the same hashes.
//! Regenerate the expectations with XL_CONFORMANCE_WRITE=1.

use std::fmt::Write as _;
use std::path::Path;
use xl_compat::{read_table, Font, Formula, Label, Value, Workbook, XFStyle};

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn text(field: &str) -> Option<String> {
    match field {
        "-" => None,
        "=" => Some(String::new()),
        hex => Some(String::from_utf8((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()).unwrap()),
    }
}

fn run(lines: &[&str]) -> (usize, u64, u64) {
    let mut w = Workbook::new();
    let mut styles = vec![0u32];
    let mut sheet = 0usize;
    for line in lines {
        let f: Vec<&str> = line.split(' ').collect();
        let n = |i: usize| f[i].parse::<u32>().unwrap();
        match f[0] {
            "date1904" => w.set_date1904(f[1] == "1"),
            "style" => styles.push(w.add_style(&XFStyle { num_format_str: text(f[3]).unwrap(), font: Font { bold: f[1] == "1", italic: f[2] == "1" } })),
            "sheet" => sheet = w.add_sheet(&text(f[1]).unwrap()),
            "merge" => w.merge(sheet, n(1), n(2), n(3), n(4)),
            "width" => w.set_col_width(sheet, n(1), n(2)),
            "cell" => {
                let num = f64::from_bits(u64::from_str_radix(f[4], 16).unwrap());
                let t = text(f[6]).unwrap_or_default();
                let value = match f[3] {
                    "0" => Value::Blank,
                    "1" => Value::Number(num),
                    "2" => Value::Text(t),
                    "3" => Value::Bool(num != 0.0),
                    _ => Value::Error(t),
                };
                let label = match (text(f[7]), value) {
                    (Some(formula), v) => Label::FormulaWithValue(Formula::new(&formula), v),
                    (None, Value::Blank) => Label::Blank,
                    (None, Value::Number(x)) => Label::Number(x),
                    (None, Value::Text(s)) => Label::Text(s),
                    (None, Value::Bool(b)) => Label::Bool(b),
                    (None, Value::Error(e)) => Label::Error(e),
                };
                w.write_styled(sheet, n(1), n(2), label, styles[n(5) as usize]);
            }
            other => panic!("unknown line {other}"),
        }
    }
    let bytes = w.to_bytes().unwrap();
    let t = read_table(bytes.clone()).unwrap();
    let mut dump = String::new();
    for s in &t.sheet_names {
        let _ = writeln!(dump, "S\t{s}");
    }
    let _ = writeln!(dump, "D\t{}", t.date1904 as u8);
    for s in &t.num_format {
        let _ = writeln!(dump, "N\t{s}");
    }
    for i in 0..t.len() {
        let txt = if matches!(t.kind[i], 2 | 4) { t.text(i) } else { "\\N" };
        let formula = if t.formula(i).is_empty() { "\\N" } else { t.formula(i) };
        let _ = writeln!(dump, "C\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\t{}", t.sheet[i], t.row[i], t.col[i], t.kind[i], t.num[i].to_bits(), t.is_date[i], t.style[i], txt, formula);
    }
    (bytes.len(), fnv(&bytes), fnv(dump.as_bytes()))
}

#[test]
fn conformance_cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance");
    let cases = std::fs::read_to_string(dir.join("cases.txt")).unwrap();
    let mut got = String::new();
    let mut name = "";
    let mut body: Vec<&str> = Vec::new();
    for line in cases.lines() {
        if let Some(n) = line.strip_prefix("case ") {
            name = n;
            body.clear();
        } else if line == "end" {
            let (len, file, table) = run(&body);
            let _ = writeln!(got, "{name}\t{len}\t{file:016x}\t{table:016x}");
        } else {
            body.push(line);
        }
    }
    let path = dir.join("expected.txt");
    if std::env::var_os("XL_CONFORMANCE_WRITE").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(got == expected, "conformance hashes changed; if intended, regenerate with XL_CONFORMANCE_WRITE=1");
    assert!(got.lines().count() >= 45);
}
