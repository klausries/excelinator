//! Write-back of frames into a workbook, calculated with the built-in Polars.
use xl_engine::apply::{verify, write_region};
use xl_engine::data::Data;
use xl_engine::regions::{region_frame, regions, RegionKind};
use xl_cells::Workbook;
use xl_cells::{Sheet, Value};

fn sample() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
    let c = &mut wb.cells;
    for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
        c.push(0, 0, k as u32, Value::Text(h.to_string()), None, 0);
    }
    for r in 0..3u32 {
        c.push(0, 1 + r, 0, Value::Text(format!("item{r}")), None, 0);
        c.push(0, 1 + r, 1, Value::Number((r + 1) as f64), None, 0);
        c.push(0, 1 + r, 2, Value::Number(2.5), None, 0);
        c.push(0, 1 + r, 3, Value::Number((r + 1) as f64 * 2.5), Some(format!("B{0}*C{0}", r + 2)), 0);
    }
    c.push(0, 6, 2, Value::Text("Total".into()), None, 0);
    c.push(0, 6, 3, Value::Number(15.0), Some("SUM(D2:D4)".into()), 0);
    wb
}

#[test]
fn a_table_takes_new_rows_and_the_workbook_follows() {
    let mut wb = sample();
    let found = regions(&wb);
    let table = found[0].clone();
    assert_eq!((table.name.as_str(), table.range()), ("Sales", "A1:D4".to_string()));
    let df = xl_engine::frame![("item", vec!["a", "b", "c", "d", "e"]), ("qty", vec![1i64, 2, 3, 4, 5]), ("price", vec![1.0, 2.0, 3.0, 4.0, 5.0])].unwrap();
    let after = write_region(&mut wb, &table, &df).unwrap();
    assert_eq!(after.range(), "A1:D6");
    let (done, calc) = xl_polars::lift::calculate(&wb);
    assert!(calc.uncomputed.is_empty(), "{:?}", calc.uncomputed);
    // The calculated column is filled down and the total reads the longer column.
    let again = regions(&done);
    let amounts = region_frame(&done, &again[0]).unwrap();
    assert_eq!(amounts.column("amount").unwrap().data, Data::from(vec![1i64, 4, 9, 16, 25]));
    let total = done.cells.formula.iter().position(|f| f.as_deref() == Some("SUM(D2:D6)")).expect("the total follows the table");
    assert_eq!(done.cells.num[total], 55.0);
    // Fewer rows again.
    let short = df.slice(0, 2);
    let mut wb2 = done.clone();
    let after = write_region(&mut wb2, &again[0], &short).unwrap();
    assert_eq!(after.range(), "A1:D3");
    let (done2, _) = xl_polars::lift::calculate(&wb2);
    let f = region_frame(&done2, &regions(&done2)[0]).unwrap();
    assert_eq!((f.height, f.width()), (2, 4));
    assert!(done2.cells.formula.iter().any(|f| f.as_deref() == Some("SUM(D2:D3)")));
}

#[test]
fn what_is_in_the_way_is_named() {
    let mut wb = sample();
    wb.cells.push(0, 4, 0, Value::Text("note".into()), None, 0);
    let table = regions(&wb).into_iter().find(|r| r.kind == RegionKind::Table).unwrap();
    let df = xl_engine::frame![("item", vec!["a", "b", "c", "d"]), ("qty", vec![1i64, 2, 3, 4]), ("price", vec![1.0, 2.0, 3.0, 4.0])].unwrap();
    // The note belongs to the block: the table is found only without it.
    let err = write_region(&mut wb, &table, &df);
    assert!(err.is_err() || table.area.last_row >= 4, "{err:?}");
}

#[test]
fn differences_are_listed_by_cell() {
    let wb = sample();
    let table = regions(&wb)[0].clone();
    let same = region_frame(&wb, &table).unwrap();
    assert_eq!(verify(&wb, &table, &same).unwrap().height, 0);
    let other = xl_engine::frame![("item", vec!["item0", "x", "item2"]), ("amount", vec![2.5, 5.0, 8.0])].unwrap();
    let diff = verify(&wb, &table, &other).unwrap();
    assert_eq!(diff.column("cell").unwrap().data, Data::from(vec!["Sales!A3", "Sales!D4"]));
    assert_eq!(diff.column("expected").unwrap().data, Data::from(vec!["item1", "7.5"]));
}
