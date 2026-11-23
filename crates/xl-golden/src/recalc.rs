//! Recalculation check: an engine recomputes a workbook, the result is compared
//! with the values the producing application cached in the file.

use calamine::{Data, Reader, Xlsx};
use std::collections::BTreeMap;
use std::io::Cursor;

#[derive(Debug, Default, Clone)]
pub struct RecalcStats {
    /// "ok", "load-error", "oracle-error", "panic"
    pub status: &'static str,
    pub detail: String,
    pub formula_cells: u64,
    /// Formula cells without a cached value, or with volatile functions: not comparable.
    pub skipped: u64,
    pub exact: u64,
    /// Numbers equal within 1e-9 relative, but not bit-identical.
    pub close: u64,
    /// Same value, different type in the file: TRUE vs 1, "12" vs 12. Depends on how
    /// the producing application stores the cached value, not on the calculation.
    pub loose: u64,
    pub diff: u64,
    /// Subset of `diff`: the engine reports an unknown or unimplemented function.
    pub unsupported: u64,
    /// Function name -> (cells compared, cells different)
    pub by_function: BTreeMap<String, (u64, u64)>,
    /// A few examples of differing cells for triage.
    pub examples: Vec<String>,
}

const VOLATILE: &[&str] = &["NOW", "TODAY", "RAND", "RANDBETWEEN", "RANDARRAY", "CELL", "INFO"];

/// Names of all functions called in a formula (upper case, without duplicates).
pub fn functions_in(formula: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut ident = String::new();
    let mut quote: Option<char> = None;
    for c in formula.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                ident.clear();
            }
            '(' => {
                if !ident.is_empty() && ident.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                    let name = ident.to_ascii_uppercase();
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
                ident.clear();
            }
            c if c.is_ascii_alphanumeric() || c == '_' || c == '.' => ident.push(c),
            _ => ident.clear(),
        }
    }
    out
}

#[derive(Debug, PartialEq)]
enum V {
    Num(f64),
    Text(String),
    Bool(bool),
    Empty,
}

fn cached(d: &Data) -> V {
    match d {
        Data::Int(i) => V::Num(*i as f64),
        Data::Float(f) => V::Num(*f),
        Data::String(s) => V::Text(s.clone()),
        Data::Bool(b) => V::Bool(*b),
        Data::DateTime(dt) => V::Num(dt.as_f64()),
        Data::DateTimeIso(s) | Data::DurationIso(s) => V::Text(s.clone()),
        Data::Error(e) => V::Text(e.to_string()),
        Data::Empty => V::Empty,
    }
}

fn engine(v: ironcalc_base::cell::CellValue) -> V {
    use ironcalc_base::cell::CellValue as C;
    match v {
        C::None => V::Empty,
        C::String(s) => V::Text(s),
        C::Number(n) => V::Num(n),
        C::Boolean(b) => V::Bool(b),
    }
}

/// Recalculate with IronCalc and compare with the cached values (read with calamine).
///
/// `legacy_oracle`: bytes of the legacy file (xls) that `bytes` was converted from. If
/// given, the expected values are the ones cached in that file instead.
pub fn recalc_ironcalc(bytes: &[u8], evaluate: bool, legacy_oracle: Option<&[u8]>) -> RecalcStats {
    let mut st = RecalcStats::default();
    let mut oracle: Xlsx<_> = match Xlsx::new(Cursor::new(bytes)) {
        Ok(x) => x,
        Err(e) => {
            st.status = "oracle-error";
            st.detail = e.to_string();
            return st;
        }
    };
    let workbook = match ironcalc::import::load_from_xlsx_bytes(bytes, "wb", "en", "UTC") {
        Ok(w) => w,
        Err(e) => {
            st.status = "load-error";
            st.detail = e.to_string();
            return st;
        }
    };
    let mut legacy = match legacy_oracle {
        Some(b) => match calamine::open_workbook_auto_from_rs(Cursor::new(b)) {
            Ok(w) => Some(w),
            Err(e) => {
                st.status = "oracle-error";
                st.detail = e.to_string();
                return st;
            }
        },
        None => None,
    };
    let mut model = match ironcalc_base::Model::from_workbook(workbook, "en") {
        Ok(m) => m,
        Err(e) => {
            st.status = "load-error";
            st.detail = e;
            return st;
        }
    };
    if evaluate {
        model.evaluate();
    }
    let sheet_index: BTreeMap<String, u32> =
        model.get_worksheets_properties().iter().enumerate().map(|(i, p)| (p.name.clone(), i as u32)).collect();

    for name in oracle.sheet_names() {
        let Some(&si) = sheet_index.get(&name) else { continue };
        let Ok(formulas) = oracle.worksheet_formula(&name) else { continue };
        let values = match &mut legacy {
            Some(l) => l.worksheet_range(&name),
            None => oracle.worksheet_range(&name).map_err(calamine::Error::from),
        };
        let Ok(values) = values else { continue };
        let Some((r0, c0)) = formulas.start() else { continue };
        for (r, c, f) in formulas.cells() {
            if f.is_empty() {
                continue;
            }
            st.formula_cells += 1;
            let (row, col) = (r0 + r as u32, c0 + c as u32);
            // Function names from the real parser; the crude scan only for text it rejects.
            let funcs = xl_formula::parse(f).map(|e| e.functions()).unwrap_or_else(|_| functions_in(f));
            let want = values.get_value((row, col)).map(cached).unwrap_or(V::Empty);
            if want == V::Empty || funcs.iter().any(|n| VOLATILE.contains(&n.as_str())) {
                st.skipped += 1;
                continue;
            }
            let got = model.get_cell_value_by_index(si, row as i32 + 1, col as i32 + 1).map(engine).unwrap_or(V::Empty);
            let verdict = match (&want, &got) {
                (V::Num(a), V::Num(b)) => {
                    if a.to_bits() == b.to_bits() || a == b {
                        0
                    } else if (a - b).abs() <= 1e-9 * a.abs().max(1.0) {
                        1
                    } else {
                        2
                    }
                }
                (a, b) if a == b => 0,
                (V::Num(n), V::Bool(b)) | (V::Bool(b), V::Num(n)) if *n == (*b as u8 as f64) => 3,
                (V::Num(n), V::Text(t)) | (V::Text(t), V::Num(n)) if t.trim().parse::<f64>().ok() == Some(*n) => 3,
                _ => 2,
            };
            for n in &funcs {
                let e = st.by_function.entry(n.clone()).or_default();
                e.0 += 1;
                if verdict == 2 {
                    e.1 += 1;
                }
            }
            match verdict {
                0 => st.exact += 1,
                1 => st.close += 1,
                3 => st.loose += 1,
                _ => {
                    st.diff += 1;
                    if let V::Text(t) = &got {
                        if t.starts_with("#N/IMPL") || (t.starts_with("#NAME") && want != got) {
                            st.unsupported += 1;
                        }
                    }
                    if st.examples.len() < 5 {
                        let f: String = f.chars().take(80).collect();
                        st.examples.push(format!("{name}!R{}C{} ={f} want {want:?} got {got:?}", row + 1, col + 1));
                    }
                }
            }
        }
    }
    st.status = "ok";
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_names() {
        assert_eq!(functions_in("SUM(A1:A3)+max(1,2)"), ["SUM", "MAX"]);
        assert_eq!(functions_in("IF(A1=\"x(y\",1,2)"), ["IF"]);
        assert_eq!(functions_in("(A1+B1)*2"), Vec::<String>::new());
        assert_eq!(functions_in("'My (sheet)'!A1+_xlfn.XLOOKUP(1,A:A,B:B)"), ["_XLFN.XLOOKUP"]);
    }
}
