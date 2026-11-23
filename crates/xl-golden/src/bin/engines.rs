//! `engines DIR [STEP] [MAX_CELLS]`: three formula engines over the same workbooks, each
//! counted against the values Excel stored: ours (`xl_polars::lift::calculate`), IronCalc
//! and Formualizer. For every STEP-th .xls under DIR the workbook is read by `xl-xls`
//! (its values are Excel's); IronCalc loads it as xlsx (written by `xl-cells`),
//! Formualizer is fed cell by cell from our cell table, the way an engine behind our
//! own model would be.
//!
//! A formula cell counts as *right* (the engine's value is Excel's, to 9 digits; an error
//! where Excel has one), *wrong* (another value), or *none* (no value: not computed, or an
//! error where Excel has a value). Cells with TODAY, NOW, RAND and the like are left out.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use xl_cells::{Kind, Workbook};

#[derive(Debug, Clone, PartialEq)]
enum V {
    Num(f64),
    Text(String),
    Bool(bool),
    Error,
    Empty,
}

#[derive(Default, Clone)]
struct Tally {
    right: u64,
    wrong: u64,
    /// Wrong values in workbooks that read no other workbook.
    wrong_clean: u64,
    none: u64,
    failed_books: u64,
    failures: BTreeMap<String, u64>,
    seconds: f64,
    wrong_functions: BTreeMap<String, u64>,
    none_functions: BTreeMap<String, u64>,
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("xls")) {
            out.push(p);
        }
    }
}

fn excel(wb: &Workbook, i: usize) -> V {
    let c = &wb.cells;
    match c.kind[i] {
        Kind::Blank => V::Empty,
        Kind::Number => V::Num(c.num[i]),
        Kind::Bool => V::Bool(c.num[i] != 0.0),
        Kind::Text => V::Text(c.text[i].as_deref().unwrap_or("").to_string()),
        Kind::Error => V::Error,
    }
}

fn same(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs()) || (x - y).abs() < 1e-9,
        // An empty result and the empty text, a number and a boolean of the same value.
        (V::Empty, V::Text(t)) | (V::Text(t), V::Empty) => t.is_empty(),
        (V::Empty, V::Num(n)) | (V::Num(n), V::Empty) => *n == 0.0,
        (V::Bool(b), V::Num(n)) | (V::Num(n), V::Bool(b)) => (*n != 0.0) == *b && (*n == 0.0 || *n == 1.0),
        _ => a == b,
    }
}

fn count(t: &mut Tally, expected: &V, got: Option<V>, functions: &[String]) {
    let mut note = |m: &mut BTreeMap<String, u64>| {
        for f in functions {
            *m.entry(f.clone()).or_default() += 1;
        }
    };
    match got {
        Some(v) if same(expected, &v) => t.right += 1,
        None | Some(V::Error) => {
            t.none += 1;
            note(&mut t.none_functions);
        }
        Some(_) => {
            t.wrong += 1;
            note(&mut t.wrong_functions);
        }
    }
}

fn ours(wb: &Workbook) -> HashMap<usize, Option<V>> {
    let (done, calc) = xl_polars::lift::calculate(wb);
    let bad: HashSet<usize> = calc.uncomputed.iter().map(|x| x.0).collect();
    (0..wb.cells.len()).filter(|&i| wb.cells.formula[i].is_some()).map(|i| (i, if bad.contains(&i) { None } else { Some(excel(&done, i)) })).collect()
}

fn ironcalc(wb: &Workbook) -> Result<HashMap<usize, Option<V>>, String> {
    let bytes = xl_cells::write_bytes(wb).map_err(|e| e.to_string())?;
    let loaded = ironcalc::import::load_from_xlsx_bytes(&bytes, "wb", "en", "UTC").map_err(|e| e.to_string())?;
    let mut model = ironcalc_base::Model::from_workbook(loaded, "en")?;
    model.evaluate();
    let index: HashMap<String, u32> = model.get_worksheets_properties().iter().enumerate().map(|(i, p)| (p.name.clone(), i as u32)).collect();
    let c = &wb.cells;
    Ok((0..c.len())
        .filter(|&i| c.formula[i].is_some())
        .map(|i| {
            use ironcalc_base::cell::CellValue as C;
            let v = index.get(&wb.sheets[c.sheet[i] as usize].name).and_then(|&s| model.get_cell_value_by_index(s, c.row[i] as i32 + 1, c.col[i] as i32 + 1).ok()).map(|v| match v {
                C::None => V::Empty,
                C::Number(n) => V::Num(n),
                C::Boolean(b) => V::Bool(b),
                C::String(s) if s.starts_with('#') && s.len() <= 16 && s.chars().skip(1).all(|ch| ch.is_ascii_uppercase() || matches!(ch, '/' | '!' | '?' | '0')) => V::Error,
                C::String(s) => V::Text(s),
            });
            (i, v)
        })
        .collect())
}

fn formualizer(wb: &Workbook) -> Result<HashMap<usize, Option<V>>, String> {
    use formualizer_common::LiteralValue as L;
    let mut book = formualizer_workbook::Workbook::new();
    for s in &wb.sheets {
        book.add_sheet(&s.name).map_err(|e| format!("{e:?}"))?;
    }
    let c = &wb.cells;
    let name_of = |i: usize| wb.sheets[c.sheet[i] as usize].name.as_str();
    // Values first, then names, then formulas.
    for i in (0..c.len()).filter(|&i| c.formula[i].is_none()) {
        let v = match c.kind[i] {
            Kind::Blank => continue,
            Kind::Number => L::Number(c.num[i]),
            Kind::Bool => L::Boolean(c.num[i] != 0.0),
            Kind::Text => L::Text(c.text[i].as_deref().unwrap_or("").to_string()),
            Kind::Error => continue,
        };
        let _ = book.set_value(name_of(i), c.row[i] + 1, c.col[i] + 1, v);
    }
    for d in wb.defined_names.iter().filter(|d| d.local_sheet.is_none()) {
        let Some((sheet, range)) = d.value.trim_start_matches('=').rsplit_once('!') else { continue };
        let sheet = sheet.trim_matches('\'').replace("''", "'");
        let (a, z) = range.split_once(':').unwrap_or((range, range));
        let (Some(a), Some(z)) = (xl_formula::parse_cell_ref(a), xl_formula::parse_cell_ref(z)) else { continue };
        if let Ok(address) = formualizer_common::RangeAddress::new(sheet, a.row, a.col, z.row, z.col) {
            let _ = book.define_named_range(&d.name, &address, formualizer_workbook::traits::NamedRangeScope::Workbook);
        }
    }
    let mut refused: HashSet<usize> = HashSet::new();
    for i in (0..c.len()).filter(|&i| c.formula[i].is_some()) {
        let f = format!("={}", c.formula[i].as_deref().unwrap_or(""));
        // A formula that reads another workbook is left out: the engine would refuse the
        // whole workbook for it.
        if f.contains('[') {
            refused.insert(i);
            continue;
        }
        if book.set_formula(name_of(i), c.row[i] + 1, c.col[i] + 1, &f).is_err() {
            refused.insert(i);
        }
    }
    book.evaluate_all().map_err(|e| format!("{e:?}"))?;
    Ok((0..c.len())
        .filter(|&i| c.formula[i].is_some())
        .map(|i| {
            let v = if refused.contains(&i) {
                None
            } else {
                book.get_value(name_of(i), c.row[i] + 1, c.col[i] + 1).and_then(|v| match v {
                    L::Int(n) => Some(V::Num(n as f64)),
                    L::Number(n) => Some(V::Num(n)),
                    L::Text(t) => Some(V::Text(t)),
                    L::Boolean(b) => Some(V::Bool(b)),
                    L::Empty => Some(V::Empty),
                    L::Error(_) => Some(V::Error),
                    L::Pending => None,
                    L::Array(_) => None,
                    // Dates and times are serials in a cell: compared through the engine's own conversion.
                    other => other.as_serial_number().map(V::Num),
                })
            };
            (i, v)
        })
        .collect())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("engines DIR [STEP] [MAX_CELLS]");
    let step: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let max_cells: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(40_000);
    let mut all = Vec::new();
    files(Path::new(&dir), &mut all);
    all.sort();
    std::panic::set_hook(Box::new(|_| {}));
    let names = ["ours", "IronCalc", "Formualizer"];
    let mut tallies = vec![Tally::default(); 3];
    let (mut books, mut cells, mut left_out, mut closed) = (0u64, 0u64, 0u64, [0u64; 2]);
    let mut open_cells = 0u64;
    let (mut external_books, mut clean_cells) = (0u64, 0u64);
    for path in all.iter().step_by(step) {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(wb) = xl_xls::read_bytes(&bytes) else { continue };
        let formulas = wb.cells.formula.iter().filter(|f| f.is_some()).count();
        if wb.cells.len() > max_cells || formulas == 0 {
            continue;
        }
        books += 1;
        let c = &wb.cells;
        // Cells that change by themselves, and what Excel itself has no value for.
        let volatile = |f: &str| {
            let u = f.to_ascii_uppercase();
            ["TODAY(", "NOW(", "RAND(", "RANDBETWEEN(", "CELL(", "INFO("].iter().any(|n| u.contains(n))
        };
        let counted: Vec<usize> = (0..c.len()).filter(|&i| c.formula[i].as_deref().is_some_and(|f| !f.is_empty() && !volatile(f))).collect();
        left_out += (formulas - counted.len()) as u64;
        cells += counted.len() as u64;
        let functions: HashMap<usize, Vec<String>> = counted.iter().map(|&i| (i, xl_formula::parse(c.formula[i].as_deref().unwrap_or("")).map(|e| e.functions()).unwrap_or_default())).collect();
        let external = c.formula.iter().flatten().any(|f| f.contains('['));
        if external {
            external_books += 1;
        } else {
            clean_cells += counted.len() as u64;
        }
        let mut results: Vec<Option<HashMap<usize, Option<V>>>> = Vec::new();
        for (k, t) in tallies.iter_mut().enumerate() {
            let start = std::time::Instant::now();
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match k {
                0 => Ok(ours(&wb)),
                1 => ironcalc(&wb),
                _ => formualizer(&wb),
            }));
            t.seconds += start.elapsed().as_secs_f64();
            let values = match run {
                Ok(Ok(v)) => Some(v),
                other => {
                    t.failed_books += 1;
                    let why = match other {
                        Ok(Err(e)) => e.chars().filter(|ch| !ch.is_ascii_digit()).take(90).collect(),
                        _ => "panic".to_string(),
                    };
                    *t.failures.entry(why).or_default() += 1;
                    None
                }
            };
            let before = t.wrong;
            for &i in &counted {
                let got = values.as_ref().and_then(|v| v.get(&i).cloned().flatten());
                count(t, &excel(&wb, i), got, &functions[&i]);
            }
            if !external {
                t.wrong_clean += t.wrong - before;
            }
            results.push(values);
        }
        // Of the cells ours has no right value for: how many each other engine gets right.
        for &i in &counted {
            let expected = excel(&wb, i);
            let right = |k: usize| results[k].as_ref().and_then(|v| v.get(&i).cloned().flatten()).is_some_and(|v| same(&expected, &v) && v != V::Error);
            if !right(0) && expected != V::Error {
                open_cells += 1;
                for k in 0..2 {
                    if right(k + 1) {
                        closed[k] += 1;
                    }
                }
            }
        }
    }
    println!("{dir}, every {step}. .xls up to {max_cells} cells: {books} workbooks, {cells} formula cells ({left_out} volatile left out)");
    for (name, t) in names.iter().zip(&tallies) {
        let pct = |n: u64| 100.0 * n as f64 / cells.max(1) as f64;
        println!("{name:<12} right {:>8} ({:.1} %)  wrong {:>7} ({:.2} %)  none {:>7} ({:.1} %)  workbooks failed {:>3}  {:.0} s", t.right, pct(t.right), t.wrong, pct(t.wrong), t.none, pct(t.none), t.failed_books, t.seconds);
        println!("             wrong in workbooks that read no other workbook: {} of {clean_cells} formula cells ({:.2} %)", t.wrong_clean, 100.0 * t.wrong_clean as f64 / clean_cells.max(1) as f64);
        let top = |m: &BTreeMap<String, u64>| {
            let mut v: Vec<_> = m.iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(*x.1));
            v.iter().take(12).map(|(f, n)| format!("{f} {n}")).collect::<Vec<_>>().join(", ")
        };
        println!("             wrong, by function in the formula: {}", top(&t.wrong_functions));
        println!("             none, by function in the formula:  {}", top(&t.none_functions));
        if !t.failures.is_empty() {
            println!("             workbooks failed: {}", top(&t.failures));
        }
    }
    println!("workbooks that read other workbooks: {external_books} of {books}");
    println!("cells ours has no right value for (Excel has a value): {open_cells}; of these right in IronCalc {}, in Formualizer {}", closed[0], closed[1]);
}
