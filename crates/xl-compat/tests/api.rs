use xl_compat::*;

#[test]
fn pyexcelerator_shaped_round_trip() {
    let mut w = Workbook::new();
    let ws = w.add_sheet("Hey, Dude");
    let mut st = XFStyle::default();
    st.num_format_str = "D-MMM-YY".into();
    st.font.bold = true;
    let date = w.add_style(&st);
    w.write(ws, 0, 0, "text");
    w.write(ws, 0, 1, 42);
    w.write(ws, 0, 2, 3.25);
    w.write(ws, 1, 0, true);
    w.write(ws, 1, 1, "");
    w.write(ws, 1, 2, Formula::new("SUM(B1;C1)"));
    w.write_styled(ws, 2, 0, date_serial(2005, 7, 20, 12, 0, 0, 0).unwrap(), date);
    w.write_merge(ws, 4, 5, 0, 2, "merged", 0);
    w.set_col_width(ws, 0, 0x0d00);
    let ws2 = w.add_sheet("Zwei");
    w.write(ws2, 9, 9, Label::FormulaWithValue(Formula::new("IF(A1>A2;\"a;b\";{1;2})"), Value::Text("a;b".into())));

    let bytes = w.to_bytes().unwrap();
    let sheets = parse_xls(bytes.clone()).unwrap();
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[0].0, "Hey, Dude");
    let v = &sheets[0].1;
    assert_eq!(v[&(0, 0)], Value::Text("text".into()));
    assert_eq!(v[&(0, 1)], Value::Number(42.0));
    assert_eq!(v[&(0, 2)], Value::Number(3.25));
    assert_eq!(v[&(1, 0)], Value::Bool(true));
    assert!(!v.contains_key(&(1, 1)));
    assert!(!v.contains_key(&(1, 2)), "a formula without a cached value has no value yet");
    assert_eq!(v[&(2, 0)], Value::Number(38553.5));
    assert_eq!(v[&(4, 0)], Value::Text("merged".into()));
    assert_eq!(sheets[1].1[&(9, 9)], Value::Text("a;b".into()));

    let t = read_table(bytes).unwrap();
    let at = |r: u32, c: u32, s: u32| (0..t.len()).find(|&i| (t.sheet[i], t.row[i], t.col[i]) == (s, r, c)).unwrap();
    assert_eq!(t.formula(at(1, 2, 0)), "SUM(B1,C1)");
    assert_eq!(t.formula(at(9, 9, 1)), "IF(A1>A2,\"a;b\",{1;2})");
    assert_eq!(t.is_date[at(2, 0, 0)], 1);
    assert_eq!(t.num_format[t.style[at(2, 0, 0)] as usize], "D-MMM-YY");
    assert_eq!(t.is_date[at(0, 1, 0)], 0);
    assert_eq!(t.text(at(0, 0, 0)), "text");
    let wb = xl_cells::read_bytes(w.to_bytes().unwrap()).unwrap();
    assert_eq!(wb.sheets[0].merges, vec![(4, 0, 5, 2)]);
    assert_eq!(wb.sheets[0].columns, vec![xl_cells::Column::width(0, 0, 13.0)]);
}

#[test]
fn serials() {
    assert_eq!(date_serial(1900, 1, 1, 0, 0, 0, 0), Some(1.0));
    assert_eq!(date_serial(2026, 11, 12, 6, 0, 0, 0), Some(46338.25));
    assert_eq!(time_serial(12, 0, 0, 0), 0.5);
    assert_eq!(date_serial(1899, 12, 30, 0, 0, 0, 0), None);
}

#[test]
fn shared_text_beyond_4_gib_is_refused_before_allocating() {
    // Shaped like POI's poc-shared-strings.xlsx: one long shared string in many cells. The file
    // stays small, the cells together would be 4.6 GB of text.
    let mut wb = xl_cells::Workbook::default();
    wb.sheets.push(xl_cells::Sheet { name: "S".into(), ..Default::default() });
    let long: std::sync::Arc<str> = "x".repeat(32767).into();
    for r in 0..140_000 {
        wb.cells.push_raw(0, r, 0, xl_cells::Kind::Text, 0.0, Some(long.clone()), None, 0);
    }
    let bytes = xl_cells::write_bytes(&wb).unwrap();
    assert!(bytes.len() < 10 << 20, "{}", bytes.len());
    let err = read_table(bytes).unwrap_err();
    assert!(err.to_string().contains("4 GiB"), "{err}");
}
