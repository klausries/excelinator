//! A table gets more or fewer rows, and the sheet stays built as it was: what lies below
//! moves, what reads the table follows, and going back gives the workbook there was.
use std::fs;
use xl_cells::{Sheet, Value, Workbook};
use xl_engine::data::Frame;
use xl_polars::store::{create, read_model, read_region, read_workbook, run, write_frame};

/// A sheet from lines of cells: `=…` is a formula, a number a number, `-` nothing.
fn sheet(wb: &mut Workbook, name: &str, lines: &[&str]) {
    let s = wb.sheets.len() as u32;
    wb.sheets.push(Sheet { name: name.into(), ..Default::default() });
    for (r, line) in lines.iter().enumerate() {
        for (c, text) in line.split('|').map(str::trim).enumerate() {
            let (r, c) = (r as u32, c as u32);
            match (text, text.strip_prefix('='), text.parse::<f64>()) {
                ("-" | "", _, _) => {}
                (_, Some(f), _) => wb.cells.push(s, r, c, Value::Blank, Some(f.to_string()), 0),
                (_, _, Ok(n)) => wb.cells.push(s, r, c, Value::Number(n), None, 0),
                _ => wb.cells.push(s, r, c, Value::Text(text.to_string()), None, 0),
            }
        }
    }
}

/// What the cells of a workbook hold, as lines `Sheet!A1 value =formula`.
fn cells(wb: &Workbook) -> Vec<String> {
    let c = &wb.cells;
    let mut out: Vec<(u32, u32, u32, String)> = (0..c.len())
        .filter(|&i| c.kind[i] != xl_cells::Kind::Blank || c.formula[i].is_some())
        .map(|i| {
            let value = match c.value(i) {
                Value::Number(n) => format!("{}", (n * 1e9).round() / 1e9),
                Value::Text(t) | Value::Error(t) => t,
                Value::Bool(b) => b.to_string(),
                Value::Blank => String::new(),
            };
            let formula = c.formula[i].as_ref().map(|f| format!(" ={f}")).unwrap_or_default();
            (c.sheet[i], c.row[i], c.col[i], format!("{}!{}{} {value}{formula}", wb.sheets[c.sheet[i] as usize].name, xl_formula::col_name(c.col[i] + 1), c.row[i] + 1))
        })
        .collect();
    out.sort();
    out.into_iter().map(|x| x.3).collect()
}

fn cell(wb: &Workbook, at: &str) -> String {
    cells(wb).into_iter().find_map(|line| line.strip_prefix(&format!("{at} ")).map(str::to_string)).unwrap_or_default()
}

fn store(name: &str, wb: &Workbook) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xl-shape-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let bytes = xl_cells::write_bytes(wb).unwrap();
    create(&dir, &bytes, &xl_cells::read_bytes(bytes.clone()).unwrap(), true).unwrap();
    run(&dir).unwrap();
    dir
}

/// The region that has a column of this name, and its range.
fn region_with(dir: &std::path::Path, column: &str) -> (String, String) {
    let model = read_model(dir).unwrap();
    let r = model["regions"].as_array().unwrap().iter().find(|r| r["columns"].as_array().is_some_and(|c| c.iter().any(|c| c["name"] == column))).unwrap_or_else(|| panic!("no region with a column {column}: {model}")).clone();
    (r["name"].as_str().unwrap().to_string(), r["ref"].as_str().unwrap().to_string())
}

fn names(dir: &std::path::Path) -> Vec<String> {
    read_model(dir).unwrap()["regions"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

fn rows(df: &Frame, n: usize) -> Frame {
    let take: Vec<usize> = (0..n).map(|k| k.min(df.height - 1)).collect();
    Frame::new(n, df.columns.iter().map(|c| xl_engine::data::Column::new(c.name.clone(), c.data.take(&take))).collect()).unwrap()
}

#[test]
fn what_lies_below_a_table_moves_with_its_rows_and_comes_back() {
    let mut wb = Workbook::default();
    sheet(
        &mut wb,
        "Report",
        &[
            "Sales report",
            "",
            "item | qty | price | amount",
            "a | 1 | 2 | =B4*C4",
            "b | 2 | 2 | =B5*C5",
            "c | 3 | 2 | =B6*C6",
            "Total | - | - | =SUM(D4:D6)",
            "",
            "Prices in EUR",
            "",
            "place | share",
            "north | =D4/$D$7",
            "south | =D5/$D$7",
        ],
    );
    sheet(&mut wb, "Other", &["Total again | =Report!D7", "First two | =SUM(Report!D4:D5)", "All | =SUM(Report!D4:D6)"]);
    let dir = store("below", &wb);
    let (before, _) = read_workbook(&dir).unwrap();
    let (orders, at) = region_with(&dir, "qty");
    assert_eq!(at, "A3:D6");
    let regions = names(&dir);

    // Two more rows: the total, the note and the second table are two rows further down.
    let three = read_region(&dir, &orders).unwrap();
    write_frame(&dir, &orders, &rows(&three, 5).select(["item", "qty", "price"]).unwrap()).unwrap();
    let report = run(&dir).unwrap();
    assert!(report.not_computed.is_empty(), "{:?}", report.not_computed);
    let (wb, _) = read_workbook(&dir).unwrap();
    assert_eq!(region_with(&dir, "qty").1, "A3:D8");
    assert_eq!(names(&dir), regions);
    assert_eq!(cell(&wb, "Report!D8"), "6 =B8*C8");
    assert_eq!(cell(&wb, "Report!A9"), "Total");
    assert_eq!(cell(&wb, "Report!D9"), "24 =SUM(D4:D8)");
    assert_eq!(cell(&wb, "Report!A11"), "Prices in EUR");
    assert_eq!(cell(&wb, "Report!B14"), format!("{} =D4/$D$9", (2.0f64 / 24.0 * 1e9).round() / 1e9));
    assert_eq!(region_with(&dir, "share").1, "A13:B15");
    // Other sheets follow: the cell that moved, the range that grew; a part of the rows stays.
    assert_eq!(cell(&wb, "Other!B1"), "24 =Report!D9");
    assert_eq!(cell(&wb, "Other!B2"), "6 =SUM(Report!D4:D5)");
    assert_eq!(cell(&wb, "Other!B3"), "24 =SUM(Report!D4:D8)");

    // The three rows again: the workbook is the one there was.
    write_frame(&dir, &orders, &three.select(["item", "qty", "price"]).unwrap()).unwrap();
    run(&dir).unwrap();
    assert_eq!(cells(&read_workbook(&dir).unwrap().0), cells(&before));
    assert_eq!(names(&dir), regions);

    // One row only: it is a table still. What read a row that is no more says so, as in
    // Excel; a range keeps the rows that are left.
    write_frame(&dir, &orders, &rows(&three, 1).select(["item", "qty", "price"]).unwrap()).unwrap();
    run(&dir).unwrap();
    let (wb, _) = read_workbook(&dir).unwrap();
    assert_eq!(names(&dir), regions);
    assert_eq!(region_with(&dir, "qty").1, "A3:D4");
    assert_eq!(cell(&wb, "Report!D5"), "2 =SUM(D4:D4)");
    assert_eq!(cell(&wb, "Report!B10"), "1 =D4/$D$5");
    assert!(cell(&wb, "Report!B11").ends_with(" =#REF!/$D$5"));
    assert_eq!(cell(&wb, "Other!B1"), "2 =Report!D5");
    assert_eq!(cell(&wb, "Other!B2"), "2 =SUM(Report!D4:D4)");
    // Three rows again: all as it was, but for what read the rows that went.
    write_frame(&dir, &orders, &three.select(["item", "qty", "price"]).unwrap()).unwrap();
    run(&dir).unwrap();
    let (wb, _) = read_workbook(&dir).unwrap();
    let lost = ["Report!B13 ", "Other!B2 "];
    let rest = |wb: &Workbook| cells(wb).into_iter().filter(|line| !lost.iter().any(|at| line.starts_with(at))).collect::<Vec<_>>();
    assert_eq!(rest(&wb), rest(&before));
    assert!(cell(&wb, "Report!B13").ends_with(" =#REF!/$D$7"));
    assert_eq!(cell(&wb, "Other!B2"), "12 =SUM(Report!D4:D6)");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn of_two_tables_side_by_side_each_moves_what_is_below_it() {
    let mut wb = Workbook::default();
    sheet(
        &mut wb,
        "Days",
        &[
            "day | bought | - | date | sold",
            "1 | 10 | - | 1 | 7",
            "2 | 20 | - | 2 | 8",
            "3 | 30 | - | 3 | 9",
            "Sum | =SUM(B2:B4) | - | Sum | =SUM(E2:E4)",
        ],
    );
    let dir = store("beside", &wb);
    let (before, _) = read_workbook(&dir).unwrap();
    let (left, right) = (region_with(&dir, "bought").0, region_with(&dir, "sold").0);
    let (l3, r3) = (read_region(&dir, &left).unwrap(), read_region(&dir, &right).unwrap());

    // One more row on the left: its sum moves down, the right table and its sum stay.
    write_frame(&dir, &left, &rows(&l3, 4)).unwrap();
    run(&dir).unwrap();
    let (wb, _) = read_workbook(&dir).unwrap();
    assert_eq!(cell(&wb, "Days!B6"), "90 =SUM(B2:B5)");
    assert_eq!(cell(&wb, "Days!E5"), "24 =SUM(E2:E4)");
    // And on the right: both sums are in one row again.
    write_frame(&dir, &right, &rows(&r3, 4)).unwrap();
    run(&dir).unwrap();
    let (wb, _) = read_workbook(&dir).unwrap();
    assert_eq!(cell(&wb, "Days!E6"), "33 =SUM(E2:E5)");
    assert_eq!(cell(&wb, "Days!D6"), "Sum");
    // Both back in one go.
    write_frame(&dir, &left, &l3).unwrap();
    write_frame(&dir, &right, &r3).unwrap();
    run(&dir).unwrap();
    assert_eq!(cells(&read_workbook(&dir).unwrap().0), cells(&before));
    let _ = fs::remove_dir_all(&dir);
}
