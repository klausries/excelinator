use xl_cells::{Sheet, Value, Workbook};
use xl_polars::cells_frame;
use xl_polars::lift::lift;

/// A sheet with columns A (quantity), B (price), and formulas filled down C..G.
fn sample() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Daten".into(), ..Sheet::default() });
    wb.sheets.push(Sheet { name: "Zweites Blatt".into(), ..Sheet::default() });
    wb.cells.push(1, 0, 1, Value::Number(100.0), None, 0);
    let c = &mut wb.cells;
    c.push(0, 0, 8, Value::Number(0.19), None, 0); // I1: tax rate
    let rows: [(f64, Option<f64>); 5] = [(2.0, Some(10.0)), (3.0, Some(2.5)), (0.0, None), (-4.0, Some(1.005)), (7.0, Some(0.0))];
    for (r, (qty, price)) in rows.iter().enumerate() {
        let r = r as u32 + 1;
        let p = price.unwrap_or(0.0);
        c.push(0, r, 0, Value::Number(*qty), None, 0);
        if let Some(p) = price {
            c.push(0, r, 1, Value::Number(*p), None, 0);
        }
        let n = r + 1;
        c.push(0, r, 2, Value::Number(qty * p), Some(format!("A{n}*B{n}")), 0);
        c.push(0, r, 3, Value::Number(qty * p * 1.19), Some(format!("C{n}*(1+$I$1)")), 0);
        c.push(0, r, 4, Value::Bool(qty * p > 5.0), Some(format!("C{n}>5")), 0);
        c.push(0, r, 5, Value::Number(if *qty > 0.0 { qty + p } else { 0.0 }), Some(format!("IF(A{n}>0,SUM(A{n}:B{n}),0)")), 0);
        let quotient = if p == 0.0 { Value::Error("#DIV/0!".into()) } else { Value::Number(qty / p) };
        c.push(0, r, 6, quotient, Some(format!("A{n}/B{n}")), 0);
        let rounded = ((qty * p).abs() * 10.0 + 0.5).floor() / 10.0 * if qty * p < 0.0 { -1.0 } else { 1.0 };
        c.push(0, r, 7, Value::Number(rounded), Some(format!("ROUND(C{n},1)")), 0);
        c.push(0, r, 9, Value::Text(format!("{qty}x").into()), Some(format!("A{n}&\"x\"")), 0);
        // Reads the row above (row 1 holds no number in column A, so it counts as 0).
        let above = if r == 1 { 0.0 } else { rows[r as usize - 2].0 };
        c.push(0, r, 10, Value::Number(qty + above), Some(format!("A{n}+A{}", n - 1)), 0);
        // Reads the same row of another sheet.
        c.push(0, r, 11, Value::Number(qty * 2.0 + 100.0), Some(format!("'Zweites Blatt'!A{n}*2+'Zweites Blatt'!$B$1")), 0);
        c.push(1, r, 0, Value::Number(*qty), None, 0);
        // A running total reads its own column.
        let total: f64 = rows[..r as usize].iter().map(|x| x.0).sum();
        c.push(0, r, 12, Value::Number(total), Some(format!("M{}+A{n}", n - 1)), 0);
        // 8.255 must round to 8.26 as in Excel, and a difference of noise must be 0.
        c.push(0, r, 13, Value::Number(8.26), Some("ROUND(8.255,2)".to_string()), 0);
        c.push(0, r, 14, Value::Number(0.0), Some(format!("(0.1+0.2)-0.3+A{n}*0")), 0);
    }
    wb
}

#[test]
fn column_formulas_become_polars_expressions() {
    let wb = sample();
    let report = lift(&wb);
    assert_eq!(report.runs, 12, "{report:?}");
    assert_eq!(report.mismatch_runs, 0, "{report:?}");
    assert_eq!(report.lifted_runs, 12, "{report:?}");
}

/// One formula filled down rows 2..=6 of column C over the given columns A and B;
/// `stored` are the values Excel shows for it.
fn filled(formula: &str, a: &[Value], b: &[Value], stored: &[Value]) -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "T".into(), ..Sheet::default() });
    let c = &mut wb.cells;
    c.push(0, 0, 0, Value::Text("Kopf".into()), None, 0);
    c.push(0, 0, 4, Value::Number(10.0), None, 0); // E1
    c.push(0, 0, 5, Value::Text("ja".into()), None, 0); // F1
    for r in 0..stored.len() {
        let row = r as u32 + 1;
        for (col, values) in [(0, a), (1, b)] {
            if !matches!(values[r], Value::Blank) {
                c.push(0, row, col, values[r].clone(), None, 0);
            }
        }
        c.push(0, row, 2, stored[r].clone(), Some(formula.replace('#', &(row + 1).to_string()).replace('^', &row.to_string())), 0);
    }
    wb
}

fn n(v: f64) -> Value {
    Value::Number(v)
}

fn t(v: &str) -> Value {
    Value::Text(v.into())
}

fn e() -> Value {
    Value::Error("#VALUE!".into())
}

fn check(formula: &str, a: &[Value], b: &[Value], stored: &[Value]) {
    let report = lift(&filled(formula, a, b, stored));
    assert_eq!((report.lifted_runs, report.mismatch_runs), (1, 0), "{formula}: {report:?}");
}

#[test]
fn text_and_numbers_in_one_column() {
    let a = [n(2.0), t("x"), Value::Blank, t("7"), n(-1.5)];
    let b = [n(1.0), n(1.0), n(1.0), n(1.0), n(1.0)];
    // Arithmetic: text that reads as a number counts, other text is an error, empty is 0.
    check("A#+B#", &a, &b, &[n(3.0), e(), n(1.0), n(8.0), n(-0.5)]);
    // Aggregates pass over text.
    check("SUM(A#:B#)", &a, &b, &[n(3.0), n(1.0), n(1.0), n(1.0), n(-0.5)]);
    // Joining: whole numbers without a fraction, an empty cell as nothing.
    check("A#&\"-\"&B#", &a, &b, &[t("2-1"), t("x-1"), t("-1"), t("7-1"), t("-1.5-1")]);
    check("CONCATENATE(A#,B#)", &a, &b, &[t("21"), t("x1"), t("1"), t("71"), t("-1.51")]);
    // Comparison: without regard to case, numbers before text, empty equals "" and 0.
    check("A#=\"X\"", &a, &b, &[Value::Bool(false), Value::Bool(true), Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
    check("A#=\"\"", &a, &b, &[Value::Bool(false), Value::Bool(false), Value::Bool(true), Value::Bool(false), Value::Bool(false)]);
    check("A#>B#", &a, &b, &[Value::Bool(true), Value::Bool(true), Value::Bool(false), Value::Bool(true), Value::Bool(false)]);
    check("A#=0", &a, &b, &[Value::Bool(false), Value::Bool(false), Value::Bool(true), Value::Bool(false), Value::Bool(false)]);
    // A result that is a number in some rows and text in others.
    check("IF(A#=\"\",\"leer\",A#)", &a, &b, &[n(2.0), t("x"), t("leer"), t("7"), n(-1.5)]);
    check("IF(ISNUMBER(A#),A#*2,\"\")", &a, &b, &[n(4.0), t(""), t(""), t(""), n(-3.0)]);
    check("IF(ISTEXT(A#),LEN(A#),0)", &a, &b, &[n(0.0), n(1.0), n(0.0), n(1.0), n(0.0)]);
    check("IF(ISBLANK(A#),1,0)", &a, &b, &[n(0.0), n(0.0), n(1.0), n(0.0), n(0.0)]);
    check("UPPER(LEFT(A#&\"abc\",2))", &a, &b, &[t("2A"), t("XA"), t("AB"), t("7A"), t("-1")]);
    check("MID(\"abcdef\",B#+1,3)&RIGHT(\"xyz\")", &a, &b, &vec![t("bcdz"); 5]);
    // Fixed cells may hold text.
    check("IF($F$1=\"JA\",$E$1,0)+B#", &a, &b, &vec![n(11.0); 5]);
}

#[test]
fn errors_are_values() {
    let a = [n(6.0), Value::Error("#DIV/0!".into()), Value::Blank, n(3.0), n(1.0)];
    let b = [n(2.0), n(1.0), n(0.0), n(0.0), n(4.0)];
    check("A#/B#", &a, &b, &[n(3.0), e(), e(), e(), n(0.25)]);
    check("IF(ISERROR(A#/B#),-1,A#/B#)", &a, &b, &[n(3.0), n(-1.0), n(-1.0), n(-1.0), n(0.25)]);
    check("IFERROR(A#/B#,\"-\")", &a, &b, &[n(3.0), t("-"), t("-"), t("-"), n(0.25)]);
    check("IF(A#>1,1,0)", &a, &b, &[n(1.0), e(), n(0.0), n(1.0), n(0.0)]);
    check("SUM(A#:B#)", &a, &b, &[n(8.0), e(), n(0.0), n(3.0), n(5.0)]);
    check("COUNT(A#:B#)", &a, &b, &[n(2.0), n(1.0), n(1.0), n(2.0), n(2.0)]);
    check("AND(B#>0,A#>0)", &a, &b, &[Value::Bool(true), e(), Value::Bool(false), Value::Bool(false), Value::Bool(true)]);
    check("CHOOSE(B#+1,\"null\",A#,\"zwei\")", &a, &b, &[t("zwei"), e(), t("null"), t("null"), e()]);
}

#[test]
fn running_totals_and_growing_ranges() {
    let a = [n(1.0), n(2.0), Value::Blank, n(4.0), n(-0.5)];
    let b = [n(2.0), n(0.5), n(3.0), n(1.0), n(2.0)];
    // ^ is the row above, # the formula's own row.
    check("C^+A#", &a, &b, &[n(1.0), n(3.0), n(3.0), n(7.0), n(6.5)]);
    check("C^+A#-B#", &a, &b, &[n(-1.0), n(0.5), n(-2.5), n(0.5), n(-2.0)]);
    check("A#+C^", &a, &b, &[n(1.0), n(3.0), n(3.0), n(7.0), n(6.5)]);
    check("C^+1", &a, &b, &[n(1.0), n(2.0), n(3.0), n(4.0), n(5.0)]);
    check("SUM(A$2:A#)", &a, &b, &[n(1.0), n(3.0), n(3.0), n(7.0), n(6.5)]);
    check("AVERAGE(A$2:A#)", &a, &b, &[n(1.0), n(1.5), n(1.5), n(7.0 / 3.0), n(6.5 / 4.0)]);
    check("SUM(A$2:B#)-COUNT(B$2:B#)", &a, &b, &[n(2.0), n(3.5), n(5.5), n(9.5), n(10.0)]);
    // A block that moves with the row.
    check("SUM(A^:B#)", &a, &b, &[n(3.0), n(5.5), n(5.5), n(8.0), n(6.5)]);
    // A running product starts from the cell above the run; here that is empty.
    let mut wb = filled("C^*B#", &a, &b, &[n(6.0), n(3.0), n(9.0), n(9.0), n(18.0)]);
    let top = (0..wb.cells.len()).find(|&i| wb.cells.row[i] == 0 && wb.cells.col[i] == 0).unwrap();
    wb.cells.col[top] = 2;
    wb.cells.kind[top] = xl_cells::Kind::Number;
    wb.cells.num[top] = 3.0;
    wb.cells.text[top] = None;
    let report = lift(&wb);
    assert_eq!((report.lifted_runs, report.mismatch_runs), (1, 0), "{report:?}");
}

#[test]
fn dates_as_excel_counts_them() {
    // 36526 = 1 Jan 2000 (Saturday), 60 = the 29 Feb 1900 that never was, 61 = 1 Mar 1900.
    let a = [n(36526.0), n(60.0), n(61.0), Value::Blank, n(45657.75)];
    let b = vec![n(1.0); 5];
    check("YEAR(A#)", &a, &b, &[n(2000.0), n(1900.0), n(1900.0), n(1900.0), n(2024.0)]);
    check("MONTH(A#)", &a, &b, &[n(1.0), n(2.0), n(3.0), n(1.0), n(12.0)]);
    check("DAY(A#)", &a, &b, &[n(1.0), n(29.0), n(1.0), n(0.0), n(31.0)]);
    check("WEEKDAY(A#)", &a, &b, &[n(7.0), n(4.0), n(5.0), n(7.0), n(3.0)]);
    check("WEEKDAY(A#,2)", &a, &b, &[n(6.0), n(3.0), n(4.0), n(6.0), n(2.0)]);
    check("DATE(YEAR(A#),MONTH(A#)+1,B#)", &a, &b, &[n(36557.0), n(61.0), n(92.0), n(32.0), n(45658.0)]);
    check("DATE(2000,14,0)+B#", &a, &b, &vec![n(36923.0); 5]);
}

#[test]
fn defined_names_stand_for_fixed_cells() {
    let a = [n(1.0), n(2.0), n(3.0), n(4.0), n(5.0)];
    let mut wb = filled("A#*Satz+SUM(Bereich)", &a, &a, &[n(13.0), n(23.0), n(33.0), n(43.0), n(53.0)]);
    wb.cells.push(0, 0, 6, Value::Number(1.0), None, 0);
    wb.cells.push(0, 0, 7, Value::Number(2.0), None, 0);
    wb.defined_names.push(xl_cells::DefinedName { hidden: false, name: "Satz".into(), local_sheet: None, value: "T!$E$1".into() });
    wb.defined_names.push(xl_cells::DefinedName { hidden: false, name: "Bereich".into(), local_sheet: None, value: "T!$G$1:$H$1".into() });
    let report = lift(&wb);
    assert_eq!((report.lifted_runs, report.mismatch_runs), (1, 0), "{report:?}");
}

#[test]
fn a_wrong_stored_value_is_noticed() {
    let mut wb = sample();
    let i = (0..wb.cells.len()).find(|&i| wb.cells.col[i] == 2 && wb.cells.row[i] == 2).unwrap();
    wb.cells.num[i] += 1.0;
    let report = lift(&wb);
    // The product itself no longer matches, and neither do the two columns computed from it.
    assert_eq!(report.mismatch_runs, 3, "{:?}", report.examples);
    assert_eq!(report.lifted_runs, 9);
}

#[test]
fn the_cell_table_is_a_frame() {
    let wb = sample();
    let df = cells_frame(&wb).unwrap();
    assert_eq!(df.height(), wb.cells.len());
    let names: Vec<String> = df.get_column_names().iter().map(|n| n.to_string()).collect();
    assert_eq!(names, ["sheet", "row", "col", "kind", "num", "text", "formula", "style"]);
}

/// Formulas computed by LibreOffice (tools/make_lift_golden.py): every column formula
/// of the file must be lifted and reproduce the stored values.
#[test]
fn golden_workbook_from_libreoffice() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/lift-golden.xlsx");
    let wb = xl_cells::read_bytes(std::fs::read(path).unwrap()).unwrap();
    let report = lift(&wb);
    assert_eq!(report.runs, 428, "{report:?}");
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    assert_eq!(report.mismatch_runs, 0, "{:#?}", report.examples);
    assert_eq!(report.lifted_cells, report.run_cells);
}

/// ISNA and ISERR tell #N/A from other errors; where an expression can give both,
/// the column formula is not translated rather than translated wrongly.
#[test]
fn error_kinds_are_not_confused() {
    let na = || Value::Error("#N/A".into());
    let div = || Value::Error("#DIV/0!".into());
    let a = [na(), div(), n(1.0), na(), div()];
    let b = vec![n(1.0); 5];
    let report = lift(&filled("ISNA(A#)", &a, &b, &[Value::Bool(true), Value::Bool(false), Value::Bool(false), Value::Bool(true), Value::Bool(false)]));
    // As a column it is refused; cell by cell each error's kind is known.
    assert_eq!((report.lifted_cells, report.mismatch_runs), (5, 0), "{report:?}");
    assert!(report.unsupported.is_empty(), "{report:?}");
    // Only one kind in the cells read: translated, and checked against the stored values.
    let only_na = [na(), n(1.0), n(2.0), na(), n(0.0)];
    check("ISNA(A#)", &only_na, &b, &[Value::Bool(true), Value::Bool(false), Value::Bool(false), Value::Bool(true), Value::Bool(false)]);
    check("ISERR(A#)", &only_na, &b, &vec![Value::Bool(false); 5]);
    let only_div = [div(), n(1.0), n(2.0), div(), n(0.0)];
    check("ISNA(A#)", &only_div, &b, &vec![Value::Bool(false); 5]);
    check("ISERR(A#)", &only_div, &b, &[Value::Bool(true), Value::Bool(false), Value::Bool(false), Value::Bool(true), Value::Bool(false)]);
}

/// Text that is a date or a number reads as one in arithmetic, as in Excel with US settings.
#[test]
fn dates_written_as_text() {
    let a = [t("01-JAN-2001"), t("12/31/01"), t("Jan 5, 2001"), t(" 7 "), t("2001-03-04")];
    let b = vec![n(1.0); 5];
    check("A#+0", &a, &b, &[n(36892.0), n(37256.0), n(36896.0), n(7.0), n(36954.0)]);
    check("MONTH(A#)*100+DAY(A#)", &a, &b, &[n(101.0), n(1231.0), n(105.0), n(107.0), n(304.0)]);
    // Not a date: an error, as before.
    let bad = [t("31-FEB-2001"), t("x"), t("13/45/2001"), t("5%"), t("1e2")];
    check("A#+0", &bad, &b, &[e(), e(), e(), n(0.05), n(100.0)]);
}

/// Where LibreOffice differs from Excel (documented behaviour of Excel): FIND of the
/// empty text gives the start position.
#[test]
fn find_of_empty_text_is_the_start() {
    let a = [t("ab"), t("qq"), t("xyz"), t("é"), n(5.0)];
    let b = vec![n(1.0); 5];
    check("FIND(\"\",A#)", &a, &b, &vec![n(1.0); 5]);
    check("FIND(\"\",A#,2)", &a, &b, &[n(2.0), n(2.0), n(2.0), e(), e()]);
}

/// Text written into a formula reads as a number or date in arithmetic.
#[test]
fn text_literals_in_arithmetic() {
    let a = [n(36892.0), n(36893.0), n(0.0), n(1.0), n(-5.0)];
    let b = vec![n(1.0); 5];
    check("A#-\"1/1/2001\"", &a, &b, &[n(0.0), n(1.0), n(-36892.0), n(-36891.0), n(-36897.0)]);
    check("A#*\"2\"+\"x\"", &a, &b, &vec![e(); 5]);
    check("\"50%\"*A#", &a, &b, &[n(18446.0), n(18446.5), n(0.0), n(0.5), n(-2.5)]);
}

/// SUMIF over a large fixed range with a criterion per row: the sums per key are worked
/// out once and looked up (no expression per cell).
#[test]
fn sumif_over_a_large_range() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "T".into(), ..Sheet::default() });
    let c = &mut wb.cells;
    // Keys in E1:E600 cycle through a, b, 1, 2; values in F are the row numbers.
    let mut sums = std::collections::HashMap::new();
    for r in 0..600u32 {
        let key = match r % 4 {
            0 => Value::Text("a".into()),
            1 => Value::Text("B".into()),
            2 => Value::Number(1.0),
            _ => Value::Number(2.0),
        };
        *sums.entry(r % 4).or_insert(0.0) += r as f64 + 1.0;
        c.push(0, r, 4, key, None, 0);
        c.push(0, r, 5, Value::Number(r as f64 + 1.0), None, 0);
    }
    let keys = [Value::Text("b".into()), Value::Number(1.0), Value::Text("x".into()), Value::Number(2.0), Value::Text("A".into())];
    let expected = [sums[&1], sums[&2], 0.0, sums[&3], sums[&0]];
    for (r, (k, e)) in keys.iter().zip(expected).enumerate() {
        c.push(0, r as u32 + 1, 0, k.clone(), None, 0);
        c.push(0, r as u32 + 1, 1, Value::Number(e), Some(format!("SUMIF($E$1:$E$600,A{},$F$1:$F$600)", r + 2)), 0);
        c.push(0, r as u32 + 1, 2, Value::Number(if matches!(k, Value::Text(t) if &**t == "x") { 0.0 } else { 150.0 }), Some(format!("COUNTIF($E$1:$E$600,A{})", r + 2)), 0);
    }
    let report = lift(&wb);
    assert_eq!((report.lifted_runs, report.mismatch_runs), (2, 0), "{report:?}");
}
