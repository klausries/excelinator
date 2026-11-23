//! `calculate` against values Excel and LibreOffice stored, and against hand-checked
//! changes of inputs.

use std::path::Path;
use xl_cells::{Sheet, Value, Workbook};
use xl_polars::lift::calculate;

mod golden;

/// A chain across sheets and columns: C = A*B, D = C*(1+rate), E = running total of D,
/// on sheet 2 the sum of E.
fn chain() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Daten".into(), ..Sheet::default() });
    wb.sheets.push(Sheet { name: "Summe".into(), ..Sheet::default() });
    let c = &mut wb.cells;
    c.push(0, 0, 8, Value::Number(0.19), None, 0);
    for r in 1..=5u32 {
        let n = r + 1;
        c.push(0, r, 0, Value::Number(r as f64), None, 0);
        c.push(0, r, 1, Value::Number(10.0), None, 0);
        c.push(0, r, 2, Value::Blank, Some(format!("A{n}*B{n}")), 0);
        c.push(0, r, 3, Value::Blank, Some(format!("C{n}*(1+$I$1)")), 0);
        c.push(0, r, 4, Value::Blank, Some(if r == 1 { format!("D{n}") } else { format!("E{}+D{n}", n - 1) }), 0);
        c.push(0, r, 5, Value::Blank, Some(format!("IF(E{n}>100,\"viel\",\"wenig\")")), 0);
    }
    c.push(1, 0, 0, Value::Blank, Some("Daten!E6".into()), 0);
    wb
}

fn value(wb: &Workbook, sheet: u32, row: u32, col: u32) -> Value {
    let c = &wb.cells;
    (0..c.len()).find(|&i| (c.sheet[i], c.row[i], c.col[i]) == (sheet, row, col)).map(|i| c.value(i)).unwrap()
}

#[test]
fn values_follow_a_chain_of_formulas_across_sheets() {
    let (wb, calc) = calculate(&chain());
    assert!(calc.settled && calc.not_computed.is_empty(), "{calc:?}");
    assert_eq!(calc.formula_cells, 21);
    assert_eq!(calc.computed, 21);
    // 10+20+30+40+50 = 150, times 1.19
    match value(&wb, 1, 0, 0) {
        Value::Number(n) => assert!((n - 178.5).abs() < 1e-9, "{n}"),
        v => panic!("{v:?}"),
    }
    assert_eq!(value(&wb, 0, 1, 5), Value::Text("wenig".into()));
    assert_eq!(value(&wb, 0, 5, 5), Value::Text("viel".into()));
    // Change an input: everything that depends on it follows, nothing else.
    let mut changed = wb.clone();
    let c = &mut changed.cells;
    let i = (0..c.len()).find(|&i| (c.sheet[i], c.row[i], c.col[i]) == (0, 0, 8)).unwrap();
    c.num[i] = 0.0;
    let (again, calc) = calculate(&changed);
    // D and E of all five rows, the total, and F in row 5 (E there: 119 before, 100 now).
    assert_eq!(calc.changed, 12, "{calc:?}");
    assert_eq!(value(&again, 0, 4, 5), Value::Text("wenig".into()));
    match value(&again, 1, 0, 0) {
        Value::Number(n) => assert!((n - 150.0).abs() < 1e-9, "{n}"),
        v => panic!("{v:?}"),
    }
}

#[test]
fn a_cycle_without_iterative_calculation_is_not_worked_out() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.cells.push(0, 0, 0, Value::Number(0.0), Some("B1+1".into()), 0);
    wb.cells.push(0, 0, 1, Value::Number(0.0), Some("A1+1".into()), 0);
    let (_, calc) = calculate(&wb);
    // Without iterative calculation the cycle is not worked out at all.
    assert!(calc.settled);
    assert_eq!(calc.not_computed.get("circular reference"), Some(&2));
}

/// The formula values of LibreOffice-converted Enron workbooks (values computed by
/// LibreOffice, from files Excel computed): blanked and calculated again, every cell
/// the module computes must have the stored value. `CALC_STEP=1` takes every file.
#[test]
fn calculated_values_match_the_stored_ones_of_the_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/lo-enron");
    if !root.exists() {
        return;
    }
    let mut files: Vec<_> = golden::walk(&root).into_iter().filter(|p| p.extension().is_some_and(|e| e == "xlsx")).collect();
    files.sort();
    let mut g = golden::Golden::new(golden::Reference::LibreOffice);
    for path in golden::chosen(&files, 15) {
        let Ok(stored) = xl_cells::read_bytes(std::fs::read(path).unwrap()) else { continue };
        g.file(path, &stored);
    }
    g.check(15, 97);
}


#[test]
fn an_error_in_a_row_range_is_the_result_of_max() {
    // A1 = MAX(B1:D1) where C1 is an error result: A1 is an error, not the max of the rest.
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.cells.push(0, 0, 0, Value::Blank, Some("MAX(B1:D1)".into()), 0);
    wb.cells.push(0, 0, 1, Value::Number(2.0), None, 0);
    wb.cells.push(0, 0, 2, Value::Blank, Some("1/E1".into()), 0);
    wb.cells.push(0, 0, 3, Value::Number(3.0), None, 0);
    wb.cells.push(0, 0, 4, Value::Number(0.0), None, 0);
    let (out, calc) = calculate(&wb);
    eprintln!("{calc:?} {:?}", out.cells.value(0));
    assert_ne!(out.cells.value(0), Value::Number(3.0));
}

#[test]
fn max_over_nothing_but_errors_is_an_error() {
    // A1 = MAX(B1:C1) with B1 an error result and C1 empty: an error, not 0.
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.cells.push(0, 0, 0, Value::Blank, Some("MAX(B1:C1)".into()), 0);
    wb.cells.push(0, 0, 1, Value::Blank, Some("1/D1".into()), 0);
    wb.cells.push(0, 0, 3, Value::Number(0.0), None, 0);
    let (out, _) = calculate(&wb);
    assert_ne!(out.cells.value(0), Value::Number(0.0));
}

#[test]
fn numbers_equal_to_15_digits_compare_equal() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.cells.push(0, 0, 0, Value::Blank, Some("0.1+0.2=0.3".into()), 0);
    wb.cells.push(0, 0, 1, Value::Blank, Some("IF(0.1+0.2<0.3,1,2)".into()), 0);
    let (out, _) = calculate(&wb);
    assert_eq!(out.cells.value(0), Value::Bool(true));
    assert_eq!(out.cells.value(1), Value::Number(2.0));
}

#[test]
fn a_lookup_with_an_error_as_key_is_an_error() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    for r in 0..3u32 {
        wb.cells.push(0, r, 0, Value::Number(r as f64 + 1.0), None, 0);
        wb.cells.push(0, r, 1, Value::Number(10.0 * (r as f64 + 1.0)), None, 0);
    }
    wb.cells.push(0, 0, 3, Value::Blank, Some("1/E1".into()), 0);
    wb.cells.push(0, 0, 5, Value::Blank, Some("VLOOKUP(D1,$A$1:$B$3,2)".into()), 0);
    let (out, _) = calculate(&wb);
    let i = (0..out.cells.len()).find(|&i| out.cells.col[i] == 5).unwrap();
    assert!(!matches!(out.cells.value(i), Value::Number(_)), "{:?}", out.cells.value(i));
}

/// Interest on the average balance: a cycle that converges. With iteration it is
/// computed; without it is left alone.
fn interest_model(iterate: bool) -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    if iterate {
        wb.extra.push(("calcPr".into(), "<calcPr iterate=\"1\" iterateCount=\"100\" iterateDelta=\"0.000001\"/>".into()));
    }
    // A1 start, B1 interest = 5% of the average of A1 and C1, C1 end = A1 + B1.
    wb.cells.push(0, 0, 0, Value::Number(1000.0), None, 0);
    wb.cells.push(0, 0, 1, Value::Blank, Some("0.05*(A1+C1)/2".into()), 0);
    wb.cells.push(0, 0, 2, Value::Blank, Some("A1+B1".into()), 0);
    wb.cells.push(0, 0, 3, Value::Blank, Some("C1*2".into()), 0);
    wb
}

#[test]
fn iterative_calculation_settles_a_converging_cycle() {
    // Fixed point: B = 0.025 (2000 + B) / ... : B = 0.05 (1000 + 1000 + B) / 2 → B = 50 / 0.975.
    let b = 50.0 / 0.975;
    let (out, calc) = calculate(&interest_model(true));
    assert!(calc.settled, "{calc:?}");
    assert!(calc.not_computed.is_empty(), "{calc:?}");
    match (out.cells.value(1), out.cells.value(3)) {
        (Value::Number(x), Value::Number(d)) => {
            assert!((x - b).abs() < 1e-5, "{x} vs {b}");
            assert!((d - 2.0 * (1000.0 + b)).abs() < 1e-4);
        }
        other => panic!("{other:?}"),
    }
    let (_, without) = calculate(&interest_model(false));
    assert!(without.not_computed.contains_key("circular reference"), "{without:?}");
    assert_eq!(without.computed, 0, "{without:?}");
}

#[test]
fn a_diverging_cycle_is_not_computed_even_with_iteration() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.extra.push(("calcPr".into(), "<calcPr iterate=\"1\" iterateCount=\"50\"/>".into()));
    wb.cells.push(0, 0, 0, Value::Blank, Some("B1+1".into()), 0);
    wb.cells.push(0, 0, 1, Value::Blank, Some("A1*2".into()), 0);
    let (_, calc) = calculate(&wb);
    assert!(!calc.settled);
    assert!(calc.not_computed.keys().any(|k| k.contains("does not settle")), "{calc:?}");
}
