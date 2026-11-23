//! Stores: new data and new logic, run with the built-in Polars.
use serde_json::json;
use std::fs;
use xl_cells::{Sheet, Value, Workbook};
use xl_engine::data::Frame;
use xl_polars::store::{create, read_model, read_region, read_wide, read_workbook, run, write_frame};

fn sample() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
    wb.sheets.push(Sheet { name: "Model".into(), ..Default::default() });
    let c = &mut wb.cells;
    c.push(0, 0, 0, Value::Text("Orders".into()), None, 0);
    for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
        c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
    }
    for r in 0..3u32 {
        c.push(0, 2 + r, 0, Value::Text(format!("item{r}")), None, 0);
        c.push(0, 2 + r, 1, Value::Number((r + 1) as f64), None, 0);
        c.push(0, 2 + r, 2, Value::Number(2.0), None, 0);
        c.push(0, 2 + r, 3, Value::Number((r + 1) as f64 * 2.0), Some(format!("B{0}*C{0}", r + 3)), 0);
    }
    c.push(1, 0, 0, Value::Text("Factor".into()), None, 0);
    c.push(1, 0, 1, Value::Number(3.0), None, 0);
    c.push(1, 1, 0, Value::Text("Gross".into()), None, 0);
    c.push(1, 1, 1, Value::Number(12.0), Some("SUM(Sales!D3:D5)".into()), 0);
    wb
}

fn column(df: &Frame, name: &str) -> Vec<f64> {
    df.column(name).unwrap().data.numbers().into_iter().map(|v| v.unwrap()).collect()
}

#[test]
fn a_store_runs_new_data_and_new_logic() {
    let dir = std::env::temp_dir().join(format!("xl-store-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let wb = sample();
    let bytes = xl_cells::write_bytes(&wb).unwrap();
    let made = create(&dir, &bytes, &wb, true).unwrap();
    // A table has a row per row, single cells one per cell.
    assert_eq!(made, vec![("Orders".to_string(), 3), ("Model".to_string(), 2)]);
    let m = read_model(&dir).unwrap();
    assert_eq!(m["regions"][0]["columns"][3]["logic"], "[@qty]*[@price]");
    assert_eq!(m["regions"][1]["cells"][1]["logic"], "SUM(Orders[amount])");

    // New data: one more order.
    let orders = xl_engine::frame![("item", vec!["a", "b", "c", "d"]), ("qty", vec![1i64, 2, 3, 4]), ("price", vec![2.0, 2.0, 2.0, 5.0])].unwrap();
    write_frame(&dir, "Orders", &orders).unwrap();
    let report = run(&dir).unwrap();
    assert!(report.not_computed.is_empty(), "{:?}", report.not_computed);
    assert_eq!(column(&read_region(&dir, "Orders").unwrap(), "amount"), vec![2.0, 4.0, 6.0, 20.0]);
    assert_eq!(column(&read_wide(&dir, "Model").unwrap(), "Gross"), vec![32.0]);
    assert_eq!(read_model(&dir).unwrap()["regions"][0]["ref"], "A2:D6");

    // New logic, in the structured form, and an input that becomes calculated.
    let mut m = read_model(&dir).unwrap();
    m["regions"][0]["columns"][3]["logic"] = json!("[@qty]*[@price]*Factor");
    m["regions"][0]["columns"][2]["logic"] = json!("[@qty]+1");
    fs::write(dir.join("model.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let report = run(&dir).unwrap();
    assert_eq!(report.logic_changed, vec![("Orders".to_string(), "price".to_string()), ("Orders".to_string(), "amount".to_string())]);
    assert_eq!(column(&read_region(&dir, "Orders").unwrap(), "amount"), vec![6.0, 18.0, 36.0, 60.0]);
    assert_eq!(column(&read_wide(&dir, "Model").unwrap(), "Gross"), vec![120.0]);
    // The workbook holds the same, as formulas.
    let (book, _) = read_workbook(&dir).unwrap();
    assert!(book.cells.formula.iter().any(|f| f.as_deref() == Some("B6*C6*'Model'!$B$1")));

    // What is wrong in the model is said plainly.
    m["regions"][0]["columns"][3]["logic"] = json!("[@quantity]*2");
    fs::write(dir.join("model.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let err = run(&dir).unwrap_err();
    assert!(err.contains("Orders[amount]") && err.contains("its columns are"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

/// Orders with an amount that reads a factor of the model, a total over them, a value
/// derived from the total, and a lookup (which the engine does not write as a plan).
fn noted_sample() -> Workbook {
    let mut wb = sample();
    let c = &mut wb.cells;
    for i in 0..c.len() {
        if c.sheet[i] == 0 && c.col[i] == 3 && c.row[i] >= 2 {
            c.formula[i] = Some(format!("B{0}*C{0}*Model!$B$1", c.row[i] + 1));
            c.num[i] *= 3.0;
        }
    }
    c.push(1, 1, 1, Value::Number(36.0), Some("SUM(Sales!D3:D5)".into()), 0);
    c.push(1, 2, 0, Value::Text("Half".into()), None, 0);
    c.push(1, 2, 1, Value::Number(18.0), Some("B2/2".into()), 0);
    c.push(1, 3, 0, Value::Text("Price of item1".into()), None, 0);
    c.push(1, 3, 1, Value::Number(2.0), Some("VLOOKUP(\"item1\",Sales!A3:C5,3,FALSE)".into()), 0);
    wb
}

#[test]
fn the_logic_travels_with_the_data_and_runs_without_the_workbook() {
    use xl_engine::store::Backend;
    use xl_polars::store::recalculate;
    let dir = std::env::temp_dir().join(format!("xl-store-notes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut wb = noted_sample();
    // The cell of the total as the first sample has it is there twice: keep the new one.
    let twice: Vec<usize> = (0..wb.cells.len()).filter(|&i| wb.cells.sheet[i] == 1 && wb.cells.row[i] == 1 && wb.cells.col[i] == 1).collect();
    assert_eq!(twice.len(), 2);
    wb.cells.formula[twice[0]] = wb.cells.formula[twice[1]].clone();
    wb.cells.num[twice[0]] = 36.0;
    let bytes = xl_cells::write_bytes(&wb).unwrap();
    let wb = xl_cells::read_bytes(bytes.clone()).unwrap();
    create(&dir, &bytes, &wb, true).unwrap();

    // Each Parquet file says what it is and how its calculated parts are worked out.
    let b = xl_polars::Polars::default();
    let note: serde_json::Value = serde_json::from_str(&b.read_note(&dir.join("data/Orders.parquet")).unwrap().expect("a note")).unwrap();
    assert_eq!((note["region"].as_str(), note["kind"].as_str(), note["ref"].as_str()), (Some("Orders"), Some("table"), Some("A2:D5")));
    let amount = &note["columns"][3];
    assert_eq!(amount["logic"], "[@qty]*[@price]*Factor");
    assert_eq!(amount["inputs"][0], json!({"name": "i0", "column": "qty"}));
    assert_eq!(amount["inputs"][2]["cell"], json!(["Model", "Factor"]));
    assert!(note["plans"][amount["plan"].as_u64().unwrap() as usize]["nodes"].is_array());
    let model: serde_json::Value = serde_json::from_str(&b.read_note(&dir.join("data/Model.parquet")).unwrap().unwrap()).unwrap();
    assert_eq!(model["columns"][1]["inputs"][0]["range"], json!({"columns": ["Orders", ["amount"]]}));
    assert!(model["columns"][3]["why"].is_string() && model["columns"][3]["plan"].is_null(), "{}", model["columns"][3]);

    // New data, in the files only: one more order and another factor.
    let orders = xl_engine::frame![("item", vec!["a", "b", "c", "d"]), ("qty", vec![1i64, 2, 3, 4]), ("price", vec![2.0, 2.0, 2.0, 5.0])].unwrap();
    write_frame(&dir, "Orders", &orders).unwrap();
    write_frame(&dir, "Model", &xl_engine::frame![("Factor", vec![10.0])].unwrap()).unwrap();
    let report = recalculate(&dir).unwrap();
    assert_eq!(report.done, vec![("Orders".to_string(), "amount".to_string()), ("Model".to_string(), "Gross".to_string()), ("Model".to_string(), "Half".to_string())]);
    assert_eq!(report.kept.iter().map(|k| k.1.as_str()).collect::<Vec<_>>(), vec!["Price of item1"]);
    let by_notes = (column(&read_region(&dir, "Orders").unwrap(), "amount"), column(&read_wide(&dir, "Model").unwrap(), "Gross"), column(&read_wide(&dir, "Model").unwrap(), "Half"));
    assert_eq!(by_notes, (vec![20.0, 40.0, 60.0, 200.0], vec![320.0], vec![160.0]));
    // The files keep their notes.
    assert!(b.read_note(&dir.join("data/Orders.parquet")).unwrap().is_some());

    // The workbook's own calculation of the same data gives the same.
    run(&dir).unwrap();
    let by_workbook = (column(&read_region(&dir, "Orders").unwrap(), "amount"), column(&read_wide(&dir, "Model").unwrap(), "Gross"), column(&read_wide(&dir, "Model").unwrap(), "Half"));
    assert_eq!(by_notes, by_workbook);

    // A file written by another tool has no note: the store's copy of it is taken.
    // (Single cells are rows there: name, cell, dtype, number, text.)
    let plain = read_region(&dir, "Model").unwrap();
    assert_eq!(plain.names(), vec!["name", "cell", "dtype", "number", "text"]);
    let mut changed = plain.clone();
    let xl_engine::data::Data::Float64(numbers) = &mut changed.columns[3].data else { panic!("numbers") };
    numbers[0] = Some(1.0);
    b.write_parquet(&dir.join("data/Model.parquet"), &changed, None).unwrap();
    assert!(b.read_note(&dir.join("data/Model.parquet")).unwrap().is_none());
    recalculate(&dir).unwrap();
    assert_eq!(column(&read_wide(&dir, "Model").unwrap(), "Gross"), vec![32.0]);
    assert!(b.read_note(&dir.join("data/Model.parquet")).unwrap().is_some());
    let _ = fs::remove_dir_all(&dir);
}

/// Orders with rows that are not as their column is: an amount with a formula of its
/// own, an amount that is a plain value, and a quantity that is calculated.
fn sample_with_exceptions() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
    wb.sheets.push(Sheet { name: "Model".into(), ..Default::default() });
    let c = &mut wb.cells;
    c.push(0, 0, 0, Value::Text("Orders".into()), None, 0);
    for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
        c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
    }
    for r in 0..6u32 {
        let row = r + 3;
        c.push(0, 2 + r, 0, Value::Text(format!("item{r}")), None, 0);
        c.push(0, 2 + r, 1, Value::Number((r + 1) as f64), (r == 1).then(|| "B3*2".to_string()), 0);
        c.push(0, 2 + r, 2, Value::Number(2.0), None, 0);
        match r {
            2 => c.push(0, 2 + r, 3, Value::Number(12.0), Some(format!("B{row}*C{row}*2")), 0),
            4 => c.push(0, 2 + r, 3, Value::Number(99.0), None, 0),
            _ => c.push(0, 2 + r, 3, Value::Number((r + 1) as f64 * 2.0), Some(format!("B{row}*C{row}")), 0),
        };
    }
    c.push(1, 0, 0, Value::Text("Factor".into()), None, 0);
    c.push(1, 0, 1, Value::Number(3.0), None, 0);
    c.push(1, 1, 0, Value::Text("Gross".into()), None, 0);
    c.push(1, 1, 1, Value::Number(137.0), Some("SUM(Sales!D3:D8)".into()), 0);
    wb
}

#[test]
fn rows_that_are_not_as_their_column_stay_what_they_are() {
    use xl_engine::store::Backend;
    use xl_polars::store::recalculate;
    let dir = std::env::temp_dir().join(format!("xl-store-exceptions-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let wb = sample_with_exceptions();
    let found = xl_polars::regions::regions(&wb);
    let orders = &found[0];
    assert_eq!((orders.kind.as_str(), orders.range().as_str()), ("table", "A2:D8"));
    let (qty, amount) = (&orders.columns[1], &orders.columns[3]);
    assert_eq!((qty.formula.as_deref(), qty.exceptions.len(), qty.exceptions[0].row, qty.exceptions[0].logic.as_deref()), (None, 1, 3, Some("B3*2")));
    assert_eq!(amount.formula.as_deref(), Some("B3*C3"));
    assert_eq!(amount.exceptions.iter().map(|e| (e.row, e.logic.as_deref())).collect::<Vec<_>>(), vec![(4, Some("[@qty]*[@price]*2")), (6, None)]);

    let bytes = xl_cells::write_bytes(&wb).unwrap();
    let wb = xl_cells::read_bytes(bytes.clone()).unwrap();
    create(&dir, &bytes, &wb, true).unwrap();
    let m = read_model(&dir).unwrap();
    assert_eq!(m["regions"][0]["columns"][3]["exceptions"], json!([{"row": 2, "cell": "D5", "logic": "[@qty]*[@price]*2", "formula": "B5*C5*2"}, {"row": 4, "cell": "D7"}]));
    let b = xl_polars::Polars::default();
    let note: serde_json::Value = serde_json::from_str(&b.read_note(&dir.join("data/Orders.parquet")).unwrap().expect("a note")).unwrap();
    let noted = &note["columns"][3]["exceptions"];
    assert_eq!((noted[0]["row"].as_u64(), noted[0]["plan"].is_u64(), &noted[1]), (Some(2), true, &json!({"row": 4})), "{noted}");
    assert_eq!(note["columns"][1]["exceptions"][0]["inputs"][0]["cell"], json!({"table": ["Orders", "qty", 0]}), "{}", note["columns"][1]);

    // New data, in the files only. The quantity of the second row is given, but calculated.
    let orders = xl_engine::frame![("qty", vec![10i64, 0, 30, 40, 50, 60])].unwrap();
    write_frame(&dir, "Orders", &orders).unwrap();
    let report = recalculate(&dir).unwrap();
    assert!(report.kept.is_empty(), "{:?}", report.kept);
    let done: Vec<&str> = report.done.iter().map(|d| d.1.as_str()).collect();
    assert_eq!(done, vec!["qty (row 2)", "amount", "amount (row 3)", "Gross"]);
    let data = |name: &str| column(&read_region(&dir, "Orders").unwrap(), name);
    assert_eq!(data("qty"), vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
    assert_eq!(data("amount"), vec![20.0, 40.0, 120.0, 80.0, 99.0, 120.0]);
    assert_eq!(column(&read_wide(&dir, "Model").unwrap(), "Gross"), vec![479.0]);

    // The workbook's own calculation of the same data gives the same.
    let by_notes = (data("qty"), data("amount"));
    run(&dir).unwrap();
    assert_eq!((data("qty"), data("amount")), by_notes);
    let (book, _) = read_workbook(&dir).unwrap();
    let formula = |row: u32, col: u32| (0..book.cells.len()).find(|&i| (book.cells.sheet[i], book.cells.row[i], book.cells.col[i]) == (0, row, col)).and_then(|i| book.cells.formula[i].clone());
    assert_eq!((formula(3, 1), formula(4, 3), formula(6, 3), formula(7, 3)), (Some("B3*2".into()), Some("B5*C5*2".into()), None, Some("B8*C8".into())));

    // Another logic for the column, and one more row: the exceptions stay.
    let mut m = read_model(&dir).unwrap();
    m["regions"][0]["columns"][3]["logic"] = json!("[@qty]*[@price]*Factor");
    fs::write(dir.join("model.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let more = xl_engine::frame![("item", vec!["a", "b", "c", "d", "e", "f", "g"]), ("qty", vec![10i64, 0, 30, 40, 50, 60, 70]), ("price", vec![2.0; 7]), ("amount", vec![0.0, 0.0, 0.0, 0.0, 77.0, 0.0, 0.0])].unwrap();
    write_frame(&dir, "Orders", &more).unwrap();
    run(&dir).unwrap();
    assert_eq!(data("amount"), vec![60.0, 120.0, 120.0, 240.0, 77.0, 360.0, 420.0]);
    assert_eq!(read_model(&dir).unwrap()["regions"][0]["columns"][3]["exceptions"].as_array().map(Vec::len), Some(2));
    let _ = fs::remove_dir_all(&dir);
}

/// Three sheets built alike (a table per month, a total below), and a summary.
fn months() -> Workbook {
    let mut wb = Workbook::default();
    for (s, month) in ["Jan", "Feb", "Mar"].iter().enumerate() {
        let s = s as u32;
        wb.sheets.push(Sheet { name: month.to_string(), ..Default::default() });
        let c = &mut wb.cells;
        for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
            c.push(s, 0, k as u32, Value::Text(h.to_string()), None, 0);
        }
        for r in 0..3u32 {
            let qty = (s * 3 + r + 1) as f64;
            c.push(s, 1 + r, 0, Value::Text(format!("item{r}")), None, 0);
            c.push(s, 1 + r, 1, Value::Number(qty), None, 0);
            c.push(s, 1 + r, 2, Value::Number(2.0), None, 0);
            // March rounds: the sheets need not have the same logic.
            let formula = if s == 2 { format!("ROUND(B{0}*C{0},0)", r + 2) } else { format!("B{0}*C{0}", r + 2) };
            c.push(s, 1 + r, 3, Value::Number(qty * 2.0), Some(formula), 0);
        }
    }
    wb.sheets.push(Sheet { name: "Year".into(), ..Default::default() });
    let c = &mut wb.cells;
    c.push(3, 0, 0, Value::Text("Total".into()), None, 0);
    c.push(3, 0, 1, Value::Number(90.0), Some("SUM(Jan!D2:D4)+SUM(Feb!D2:D4)+SUM(Mar!D2:D4)".into()), 0);
    wb
}

#[test]
fn sheets_built_alike_are_one_table() {
    use xl_polars::data::{Column, Data};
    use xl_polars::store::{add_sheet, stacks};
    let dir = std::env::temp_dir().join(format!("xl-store-stack-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let wb = months();
    create(&dir, &xl_cells::write_bytes(&wb).unwrap(), &wb, true).unwrap();
    let found = stacks(&dir).unwrap();
    assert_eq!(found.len(), 1);
    let stack = &found[0];
    assert_eq!(stack.name, "stack");
    assert_eq!(stack.members.iter().map(|m| (m.sheet.as_str(), m.region.as_str())).collect::<Vec<_>>(), vec![("Jan", "Jan"), ("Feb", "Feb"), ("Mar", "Mar")]);
    assert_eq!(stack.differs, vec![("amount".to_string(), vec![Some("[@qty]*[@price]".to_string()), Some("[@qty]*[@price]".to_string()), Some("ROUND([@qty]*[@price],0)".to_string())])]);
    // Read as one table, the sheet in front.
    let all = read_region(&dir, "stack").unwrap();
    assert_eq!(all.names(), vec!["sheet", "item", "qty", "price", "amount"]);
    assert_eq!(all.height, 9);
    assert_eq!(column(&all, "amount"), vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0]);
    // Written as one table: February gets a fourth row and other prices, the others stay.
    let feb = Frame::of(vec![
        Column::new("sheet", Data::String(vec![Some("Feb".into()); 4])),
        Column::new("item", Data::String((0..4).map(|k| Some(format!("f{k}"))).collect())),
        Column::new("qty", Data::Int64(vec![Some(1), Some(2), Some(3), Some(4)])),
        Column::new("price", Data::Float64(vec![Some(1.5); 4])),
    ])
    .unwrap();
    write_frame(&dir, "stack", &feb).unwrap();
    run(&dir).unwrap();
    let all = read_region(&dir, "stack").unwrap();
    assert_eq!(all.height, 10);
    assert_eq!(column(&all, "amount"), vec![2.0, 4.0, 6.0, 1.5, 3.0, 4.5, 6.0, 14.0, 16.0, 18.0]);
    // A sheet the stack does not have is refused, and says what to do.
    let apr = Frame::of(vec![Column::new("sheet", Data::String(vec![Some("Apr".into())])), Column::new("qty", Data::Int64(vec![Some(1)]))]).unwrap();
    assert!(write_frame(&dir, "stack", &apr).unwrap_err().contains("add_sheet"));
    // One more sheet: a copy of March, then its own rows.
    assert_eq!(add_sheet(&dir, "Mar", "Apr").unwrap(), vec!["Apr".to_string()]);
    assert_eq!(stacks(&dir).unwrap()[0].members.len(), 4);
    let apr = Frame::of(vec![
        Column::new("sheet", Data::String(vec![Some("Apr".into()); 3])),
        Column::new("item", Data::String((0..3).map(|k| Some(format!("a{k}"))).collect())),
        Column::new("qty", Data::Int64(vec![Some(1), Some(1), Some(1)])),
        Column::new("price", Data::Float64(vec![Some(0.4), Some(1.0), Some(2.6)])),
    ])
    .unwrap();
    write_frame(&dir, "stack", &apr).unwrap();
    run(&dir).unwrap();
    let all = read_region(&dir, "stack").unwrap();
    assert_eq!(all.height, 13);
    // April rounds as March does, whose copy it is.
    assert_eq!(column(&all, "amount")[10..], [0.0, 1.0, 3.0]);
    let (book, _) = read_workbook(&dir).unwrap();
    assert_eq!(book.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["Jan", "Feb", "Mar", "Year", "Apr"]);
    // The logic of a stack's column is set in every sheet.
    xl_polars::store::set_model_logic(&dir, "stack", "amount", Some("[@qty]*[@price]*2")).unwrap();
    run(&dir).unwrap();
    assert_eq!(column(&read_region(&dir, "stack").unwrap(), "amount")[..3], [4.0, 8.0, 12.0]);
    assert!(stacks(&dir).unwrap()[0].differs.is_empty());
    let _ = fs::remove_dir_all(&dir);
}
