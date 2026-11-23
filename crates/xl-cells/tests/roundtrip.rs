use xl_cells::*;

fn sample() -> Workbook {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "Daten & <Werte>".into(), merges: vec![(5, 0, 6, 2)], columns: vec![Column::width(0, 0, 20.5), Column::width(2, 3, 8.0)], rows: vec![Row::height(0, 30.0), Row::height(2, 12.75), Row::height(40, 5.0), Row::height(2_000_000 / 2, 18.0)], freeze: Some((1, 2)), ..Sheet::default() });
    wb.sheets.push(Sheet { name: "Zwei".into(), visibility: Visibility::Hidden, ..Sheet::default() });
    let date = wb.intern_style(&Style { num_format: "yyyy-mm-dd".into(), ..Style::default() });
    let bold = wb.intern_style(&Style {
        bold: true,
        italic: true,
        num_format: "0.00".into(),
        underline: Some("double".into()),
        strike: true,
        font_name: Some("Arial".into()),
        font_size: Some(9.5),
        font_color: Some("FFFF0000".into()),
        fill: Some("FFFFFF00".into()),
        borders: [Some(Border { style: "thin".into(), color: Some("FF000000".into()) }), None, Some(Border { style: "double".into(), color: None }), None],
        h_align: Some("center".into()),
        v_align: Some("top".into()),
        wrap: true,
        rotation: 45,
        indent: 2,
        ..Style::default()
    });
    let c = &mut wb.cells;
    c.push(0, 0, 0, Value::Text("Größe".into()), None, bold);
    c.push(0, 0, 1, Value::Number(1.0 / 3.0), None, 0);
    c.push(0, 1, 0, Value::Text("  leading and trailing  ".into()), None, 0);
    c.push(0, 1, 1, Value::Number(-1.5e-300), None, 0);
    c.push(0, 2, 0, Value::Bool(true), None, 0);
    c.push(0, 2, 1, Value::Bool(false), None, 0);
    c.push(0, 3, 0, Value::Error("#DIV/0!".into()), Some("1/0".into()), 0);
    c.push(0, 3, 1, Value::Number(3.0), Some("SUM(B1:B2)&\"<\"".into()), 0);
    c.push(0, 3, 2, Value::Text("a<b".into()), Some("\"a\"&\"<b\"".into()), 0);
    c.push(0, 3, 3, Value::Blank, Some("NOW()".into()), date);
    c.push(0, 4, 0, Value::Text("line1\r\nline2\ttab _x000D_ \u{1} 😀".into()), None, 0);
    c.push(0, 4, 1, Value::Number(46000.5), None, date);
    c.push(0, 4, 2, Value::Blank, None, bold);
    c.push(0, 1_048_575, 16_383, Value::Text("last".into()), None, 0);
    c.push(1, 0, 0, Value::Text("Größe".into()), None, 0);
    wb.defined_names.push(DefinedName { name: "Alles".into(), local_sheet: None, value: "'Daten & <Werte>'!$A$1:$B$5".into(), hidden: false });
    wb.defined_names.push(DefinedName { name: "Lokal".into(), local_sheet: Some(1), value: "Zwei!$A$1".into(), hidden: true });
    wb.sheets[0].auto_filter = Some("A1:B4".into());
    wb.sheets[0].validations.push(Validation { attrs: vec![("type".into(), "list".into()), ("allowBlank".into(), "1".into()), ("error".into(), "nur <a> & b".into()), ("sqref".into(), "D1:D5 F1".into())], formula1: Some("\"a,b\"".into()), formula2: None });
    wb.sheets[0].validations.push(Validation { attrs: vec![("type".into(), "whole".into()), ("operator".into(), "between".into()), ("sqref".into(), "E1".into())], formula1: Some("1".into()), formula2: Some("10".into()) });
    wb.sheets[0].cond_formats.push(CondFormat {
        sqref: "B1:B4 D1".into(),
        rules: vec![
            CondRule { attrs: vec![("type".into(), "cellIs".into()), ("priority".into(), "1".into()), ("operator".into(), "greaterThan".into())], inner: "<formula>5 &amp; 1</formula>".into(), dxf: Some("<font><b/><color rgb=\"FFFF0000\"/></font>".into()) },
            CondRule { attrs: vec![("type".into(), "colorScale".into()), ("priority".into(), "2".into())], inner: "<colorScale><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FFFFFFFF\"/><color rgb=\"FF00FF00\"/></colorScale>".into(), dxf: None },
        ],
    });
    let png: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"\x89PNG\r\n\x1a\nnot really a picture"[..]);
    wb.sheets[0].drawings.push(Drawing { anchor: Anchor { from: (1, 0, 2, 9525), to: None, ext: Some((952500, 476250)) }, image: Some((png.clone(), "png".into())), chart: None });
    wb.sheets[0].drawings.push(Drawing { anchor: Anchor { from: (5, 0, 0, 0), to: Some((9, 100, 14, 200)), ext: None }, image: None, chart: Some(std::sync::Arc::from("<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart/></c:chartSpace>")) });
    wb.sheets[1].drawings.push(Drawing { anchor: Anchor { from: (0, 0, 0, 0), to: None, ext: Some((100, 100)) }, image: Some((png, "png".into())), chart: None });
    wb.sheets[0].comments.push(Comment { cell: "A1".into(), author: "Klaus & Co".into(), text: "Erste Zeile\n  zweite <Zeile>".into() });
    wb.sheets[0].comments.push(Comment { cell: "C3".into(), author: "".into(), text: "ohne Autor".into() });
    wb.sheets[1].comments.push(Comment { cell: "B2".into(), author: "Klaus & Co".into(), text: "auf Blatt zwei".into() });
    wb.sheets[0].hyperlinks.push(Hyperlink { cell: "A1".into(), target: Some("https://example.com/?a=1&b=2".into()), location: None, tooltip: Some("Tipp".into()), display: None });
    wb.sheets[0].hyperlinks.push(Hyperlink { cell: "B2".into(), target: None, location: Some("Zwei!A1".into()), tooltip: None, display: Some("intern".into()) });
    wb
}

#[test]
fn write_then_read_is_identity() {
    let wb = sample();
    let bytes = write_bytes(&wb).unwrap();
    let mut back = read_bytes(bytes).unwrap();
    let mut wb = wb;
    for (i, s) in wb.sheets.iter_mut().enumerate() {
        s.origin = Some(i as u32);
    }
    // The used range is read from the file (written from the cells), not taken from the model.
    let dims: Vec<_> = back.sheets.iter_mut().map(|s| s.dimension.take()).collect();
    assert_eq!(dims, vec![Some((0, 0, 1_048_575, 16_383)), Some((0, 0, 0, 0))]);
    assert_eq!(back, wb);
}

#[test]
fn later_write_to_the_same_cell_wins_and_order_does_not_matter() {
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Sheet::default() });
    wb.cells.push(0, 5, 5, Value::Number(1.0), None, 0);
    wb.cells.push(0, 0, 0, Value::Number(2.0), None, 0);
    wb.cells.push(0, 5, 5, Value::Text("x".into()), None, 0);
    let back = read_bytes(write_bytes(&wb).unwrap()).unwrap();
    assert_eq!(back.cells.len(), 2);
    assert_eq!((back.cells.row[0], back.cells.value(0)), (0, Value::Number(2.0)));
    assert_eq!(back.cells.value(1), Value::Text("x".into()));
}

#[test]
fn invalid_workbooks_are_rejected() {
    let sheet = |name: &str| Sheet { name: name.into(), ..Sheet::default() };
    let with = |sheets: Vec<Sheet>| Workbook { sheets, ..Workbook::default() };
    assert!(write_bytes(&Workbook::default()).is_err());
    for bad in ["", "a/b", "a[b]", "x:y", "'x", "this name is longer than thirty-one chars"] {
        assert!(write_bytes(&with(vec![sheet(bad)])).is_err(), "{bad}");
    }
    assert!(write_bytes(&with(vec![sheet("A"), sheet("a")])).is_err());
    assert!(write_bytes(&with(vec![sheet(&"ä".repeat(31))])).is_ok());
    assert!(write_bytes(&with(vec![sheet(&"😀".repeat(15))])).is_ok());
    assert!(write_bytes(&with(vec![sheet(&"😀".repeat(16))])).is_err(), "32 UTF-16 units");
    let mut wb = with(vec![sheet("A")]);
    wb.cells.push(0, 1_048_576, 0, Value::Number(1.0), None, 0);
    assert!(write_bytes(&wb).is_err());
    let mut wb = with(vec![sheet("A")]);
    wb.cells.push(1, 0, 0, Value::Number(1.0), None, 0);
    assert!(write_bytes(&wb).is_err());
    let mut wb = with(vec![sheet("A")]);
    wb.cells.push(0, 0, 0, Value::Number(1.0), None, 7);
    assert!(write_bytes(&wb).is_err());
}

#[test]
fn date_formats() {
    for (code, date) in [("General", false), ("0.00", false), ("yyyy-mm-dd", true), ("[h]:mm:ss", true), ("[Red]0.0", false), ("\"day\" 0", false), ("0.0\\d", false), ("[$-407]d. mmm yy", true), ("#,##0_d", false), ("mm:ss.0", true), ("@", false)] {
        assert_eq!(is_date_format(code), date, "{code}");
    }
}

#[test]
fn excel_limits_are_enforced() {
    let sheet = || Sheet { name: "A".into(), ..Sheet::default() };
    let mut wb = Workbook { sheets: vec![sheet()], ..Workbook::default() };
    wb.cells.push(0, 0, 0, Value::Text("x".repeat(32_767)), None, 0);
    wb.cells.push(0, 0, 1, Value::Text("ä".repeat(32_767)), None, 0);
    assert!(write_bytes(&wb).is_ok());
    wb.cells.push(0, 0, 2, Value::Text("x".repeat(32_768)), None, 0);
    assert!(write_bytes(&wb).is_err());
    let mut wb = Workbook { sheets: vec![sheet()], ..Workbook::default() };
    wb.sheets[0].merges = vec![(0, 0, 1, 1), (1, 1, 2, 2)];
    assert!(write_bytes(&wb).is_err());
    wb.sheets[0].merges = vec![(0, 0, 1, 1), (2, 0, 3, 1)];
    assert!(write_bytes(&wb).is_ok());
}

#[test]
fn style_specs_round_trip() {
    let wb = sample();
    for s in &wb.styles {
        assert_eq!(&Style::from_spec(&s.to_spec()), s);
    }
}


/// The cached cells of other workbooks (xl/externalLinks) are read; a broken part costs
/// only its values.
#[test]
fn external_link_caches_are_read() {
    let mut pkg = xl_opc::Package::open(xl_cells::write_bytes(&sample()).unwrap()).unwrap();
    let text = |pkg: &xl_opc::Package, name: &str| String::from_utf8(pkg.read(name).unwrap()).unwrap();
    let workbook = text(&pkg, "xl/workbook.xml").replace("<definedNames", "<externalReferences><externalReference r:id=\"rIdX1\"/><externalReference r:id=\"rIdX2\"/></externalReferences><definedNames");
    pkg.set("xl/workbook.xml", workbook.into_bytes());
    let kind = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink";
    let rels = text(&pkg, "xl/_rels/workbook.xml.rels").replace(
        "</Relationships>",
        &format!("<Relationship Id=\"rIdX1\" Type=\"{kind}\" Target=\"externalLinks/externalLink1.xml\"/><Relationship Id=\"rIdX2\" Type=\"{kind}\" Target=\"externalLinks/externalLink2.xml\"/></Relationships>"),
    );
    pkg.set("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let good = r#"<?xml version="1.0"?><externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><externalBook><sheetNames><sheetName val="A"/><sheetName val="B &amp; C"/></sheetNames><sheetDataSet><sheetData sheetId="0"><row r="2"><cell r="B2"><v>1.5</v></cell><cell r="C2" t="str"><v>x</v></cell></row></sheetData><sheetData sheetId="1"><row r="1"><cell r="A1" t="e"><v>#N/A</v></cell><cell r="A2" t="b"><v>1</v></cell></row></sheetData></sheetDataSet></externalBook></externalLink>"#;
    pkg.set("xl/externalLinks/externalLink1.xml", good.as_bytes().to_vec());
    pkg.set("xl/externalLinks/externalLink2.xml", b"<externalLink><sheetNames><sheetName val=\"x&\"/>".to_vec());
    let wb = xl_cells::read_bytes(pkg.to_bytes().unwrap()).unwrap();
    assert_eq!(wb.externals.len(), 2);
    assert_eq!(wb.externals[0].sheets, ["A", "B & C"]);
    assert_eq!(
        wb.externals[0].cells,
        [(0, 1, 1, xl_cells::Value::Number(1.5)), (0, 1, 2, xl_cells::Value::Text("x".into())), (1, 0, 0, xl_cells::Value::Error("#N/A".into())), (1, 1, 0, xl_cells::Value::Bool(true))]
    );
    // The broken part gives an empty entry; the workbook still reads.
    assert!(wb.externals[1].cells.is_empty());
}

#[test]
fn elements_kept_as_xml_go_through_in_schema_order() {
    let mut wb = sample();
    let extra = vec![
        ("pageSetup".to_string(), "<pageSetup paperSize=\"9\" orientation=\"landscape\"/>".to_string()),
        ("sheetPr".to_string(), "<sheetPr><tabColor rgb=\"FFFF0000\"/></sheetPr>".to_string()),
        ("pageMargins".to_string(), "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>".to_string()),
        ("sheetProtection".to_string(), "<sheetProtection sheet=\"1\" objects=\"1\"/>".to_string()),
        ("headerFooter".to_string(), "<headerFooter><oddHeader>&amp;C&amp;\"Arial\"Seite &amp;P</oddHeader></headerFooter>".to_string()),
    ];
    wb.sheets[0].extra = extra.clone();
    let bytes = write_bytes(&wb).unwrap();
    let back = read_bytes(bytes.clone()).unwrap();
    let mut got = back.sheets[0].extra.clone();
    let mut want = extra;
    got.sort();
    want.sort();
    assert_eq!(got, want);
    // The order in the part follows the schema.
    let pkg = xl_opc::Package::open(bytes).unwrap();
    let xml = String::from_utf8(pkg.read("xl/worksheets/sheet1.xml").unwrap()).unwrap();
    let at = |n: &str| xml.find(&format!("<{n}")).unwrap();
    assert!(at("sheetPr") < at("sheetViews") && at("sheetViews") < at("sheetData") && at("sheetData") < at("sheetProtection"));
    assert!(at("sheetProtection") < at("autoFilter") && at("hyperlinks") < at("pageMargins") && at("pageMargins") < at("pageSetup") && at("pageSetup") < at("headerFooter"));
}

#[test]
fn rich_styles_and_named_styles_go_through() {
    let mut wb = sample();
    wb.named_styles = vec![
        NamedStyle { name: "Normal".into(), builtin_id: Some(0), ..NamedStyle::default() },
        NamedStyle {
            name: "Gut".into(),
            builtin_id: Some(26),
            // The fields hold what they can of the XML (as reading gives them).
            style: Style { font_xml: Some("<font><color rgb=\"FF006100\"/><sz val=\"11\"/><name val=\"Calibri\"/></font>".into()), font_color: Some("FF006100".into()), ..Style::default() },
            ..NamedStyle::default()
        },
        NamedStyle { name: "Mein Stil".into(), style: Style { bold: true, ..Style::default() }, ..NamedStyle::default() },
    ];
    let rich = wb.intern_style(&Style {
        font_xml: Some("<font><b/><color theme=\"1\" tint=\"-0.25\"/><sz val=\"11\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font>".into()),
        fill_xml: Some("<fill><gradientFill degree=\"90\"><stop position=\"0\"><color rgb=\"FFFF0000\"/></stop><stop position=\"1\"><color theme=\"4\"/></stop></gradientFill></fill>".into()),
        border_xml: Some("<border diagonalUp=\"1\"><left style=\"thin\"><color indexed=\"64\"/></left><right/><top/><bottom/><diagonal style=\"dashed\"/></border>".into()),
        alignment_xml: Some("<alignment horizontal=\"center\" shrinkToFit=\"1\" readingOrder=\"2\"/>".into()),
        protection_xml: Some("<protection locked=\"0\" hidden=\"1\"/>".into()),
        xf_attrs: vec![("quotePrefix".into(), "1".into())],
        named: Some("Gut".into()),
        bold: true,
        h_align: Some("center".into()),
        borders: [Some(Border { style: "thin".into(), color: None }), None, None, None],
        ..Style::default()
    });
    let own = wb.intern_style(&Style { bold: true, named: Some("Mein Stil".into()), ..Style::default() });
    wb.cells.push(0, 50, 0, Value::Text("reich".into()), None, rich);
    wb.cells.push(0, 51, 0, Value::Text("eigen".into()), None, own);
    let back = read_bytes(write_bytes(&wb).unwrap()).unwrap();
    for (a, b) in back.named_styles.iter().zip(&wb.named_styles) {
        assert_eq!(a, b);
    }
    for (i, (a, b)) in back.styles.iter().zip(&wb.styles).enumerate() {
        assert_eq!(a, b, "style {i}");
    }
    assert_eq!(back.styles.len(), wb.styles.len());
    let at = |row: u32| (0..back.cells.len()).find(|&i| back.cells.sheet[i] == 0 && back.cells.row[i] == row).map(|i| back.cells.style[i]);
    assert_eq!((at(50), at(51)), (Some(rich), Some(own)));
}

#[test]
fn formatted_runs_go_through_shared_and_inline() {
    let runs = "<r><rPr><b/><sz val=\"11\"/><color rgb=\"FFFF0000\"/><rFont val=\"Calibri\"/></rPr><t>rot</t></r><r><t xml:space=\"preserve\"> und schlicht</t></r>";
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "S".into(), ..Default::default() });
    for r in 0..3 {
        wb.cells.push(0, r, 0, Value::Text("rot und schlicht".into()), None, 0);
        *wb.cells.rich.last_mut().unwrap() = Some(runs.into());
    }
    wb.cells.push(0, 3, 0, Value::Text("rot und schlicht".into()), None, 0);
    assert_eq!(rich_text(runs).unwrap(), "rot und schlicht");
    let bytes = write_bytes(&wb).unwrap();
    let back = read_bytes(bytes.clone()).unwrap();
    assert_eq!(back.cells.rich[..3], [Some(runs.into()), Some(runs.into()), Some(runs.into())]);
    assert_eq!(back.cells.rich[3], None, "the same text without runs is its own string");
    let sst = String::from_utf8(xl_opc::Package::open(bytes).unwrap().read("xl/sharedStrings.xml").unwrap()).unwrap();
    assert!(sst.contains("uniqueCount=\"2\""), "{sst}");
    // Saved onto the original as a new cell, the runs go inline.
    let mut changed = back.clone();
    changed.cells.push(0, 5, 1, Value::Text("rot und schlicht".into()), None, 0);
    *changed.cells.rich.last_mut().unwrap() = Some(runs.into());
    let updated = read_bytes(update_bytes(write_bytes(&back).unwrap(), &changed).unwrap()).unwrap();
    let i = (0..updated.cells.len()).find(|&i| updated.cells.row[i] == 5).unwrap();
    assert_eq!(updated.cells.rich[i].as_deref(), Some(runs));
    assert_eq!(updated.cells.text[i].as_deref(), Some("rot und schlicht"));
}

#[test]
fn tables_and_row_column_attributes_go_through() {
    let table = |id: u32, name: &str, r: &str| {
        format!("<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{r}\" totalsRowShown=\"0\"><autoFilter ref=\"{r}\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"a\"/><tableColumn id=\"2\" name=\"b\"/></tableColumns><tableStyleInfo name=\"TableStyleMedium9\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>")
    };
    let mut wb = Workbook::default();
    wb.styles = vec![Style::default(), Style { bold: true, ..Style::default() }];
    let mut s = Sheet { name: "S".into(), tables: vec![table(7, "Erste", "A1:B3")], ..Default::default() };
    s.rows = vec![Row { row: 1, hidden: true, outline_level: 2, ..Row::default() }, Row { row: 4, collapsed: true, style: Some(1), custom_format: true, thick_bot: true, ..Row::default() }, Row::height(9, 30.0)];
    s.columns = vec![Column { first: 1, last: 3, hidden: true, outline_level: 1, ..Column::default() }, Column { first: 5, last: 5, style: Some(1), best_fit: true, width: Some(4.5), ..Column::default() }];
    wb.sheets.push(s);
    wb.sheets.push(Sheet { name: "T".into(), tables: vec![table(7, "Zweite", "C1:D2")], ..Default::default() });
    for r in 0..3 {
        wb.cells.push(0, r, 0, Value::Text(format!("a{r}")), None, 0);
        wb.cells.push(0, r, 1, Value::Text(format!("b{r}")), None, 0);
    }
    let back = read_bytes(write_bytes(&wb).unwrap()).unwrap();
    assert_eq!(back.sheets[0].rows, wb.sheets[0].rows);
    assert_eq!(back.sheets[0].columns, wb.sheets[0].columns);
    // Ids clash (7 twice): numbered across the workbook; the rest stays as given.
    assert_eq!(back.sheets[0].tables, vec![table(1, "Erste", "A1:B3")]);
    assert_eq!(back.sheets[1].tables, vec![table(2, "Zweite", "C1:D2")]);
    // Unique ids are kept.
    let mut own = wb.clone();
    own.sheets[1].tables = vec![table(9, "Zweite", "C1:D2")];
    let back = read_bytes(write_bytes(&own).unwrap()).unwrap();
    assert_eq!((back.sheets[0].tables.clone(), back.sheets[1].tables.clone()), (vec![table(7, "Erste", "A1:B3")], vec![table(9, "Zweite", "C1:D2")]));
    // Display names are unique in the workbook, ignoring case.
    let mut twice = wb.clone();
    twice.sheets[1].tables = vec![table(1, "ERSTE", "C1:D2")];
    assert!(write_bytes(&twice).unwrap_err().to_string().contains("used twice"));
    // Overlapping column ranges are refused.
    let mut overlap = wb.clone();
    overlap.sheets[0].columns.push(Column::width(2, 2, 3.0));
    assert!(write_bytes(&overlap).is_err());
}

#[test]
fn workbook_settings_and_document_properties_go_through() {
    let core = "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Plan</dc:title><dc:creator>Klaus</dc:creator></cp:coreProperties>";
    let mut wb = Workbook::default();
    wb.sheets.push(Sheet { name: "A".into(), ..Default::default() });
    wb.sheets.push(Sheet { name: "B".into(), ..Default::default() });
    wb.date1904 = true;
    wb.extra = vec![
        ("workbookPr".into(), "<workbookPr codeName=\"DieseArbeitsmappe\" defaultThemeVersion=\"166925\"/>".into()),
        ("workbookProtection".into(), "<workbookProtection lockStructure=\"1\" workbookPassword=\"CBEB\"/>".into()),
        ("bookViews".into(), "<bookViews><workbookView activeTab=\"1\" firstSheet=\"0\"/></bookViews>".into()),
        ("calcPr".into(), "<calcPr calcId=\"191029\"/>".into()),
    ];
    wb.properties = vec![("core".into(), core.into())];
    wb.cells.push(0, 0, 0, Value::Blank, Some("1+1".into()), 0);
    let bytes = write_bytes(&wb).unwrap();
    let back = read_bytes(bytes.clone()).unwrap();
    assert!(back.date1904);
    let get = |w: &Workbook, n: &str| w.extra.iter().find(|e| e.0 == n).map(|e| e.1.clone());
    // date1904 written into the given workbookPr; fullCalcOnLoad added for the formula without value.
    assert_eq!(get(&back, "workbookPr").unwrap(), "<workbookPr codeName=\"DieseArbeitsmappe\" defaultThemeVersion=\"166925\" date1904=\"1\"/>");
    assert_eq!(get(&back, "workbookProtection"), get(&wb, "workbookProtection"));
    assert_eq!(get(&back, "bookViews"), get(&wb, "bookViews"));
    let wb_xml = String::from_utf8(xl_opc::Package::open(bytes.clone()).unwrap().read("xl/workbook.xml").unwrap()).unwrap();
    assert!(wb_xml.contains("<calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/>"), "{wb_xml}");
    // Read back, the request the writer adds anyway is not part of the model.
    assert_eq!(get(&back, "calcPr").unwrap(), "<calcPr calcId=\"191029\"/>");
    assert_eq!(back.properties, vec![("core".to_string(), core.to_string())]);
    // A sheet goes: the active tab no longer points past the end.
    let mut one = back.clone();
    one.sheets.pop();
    let w = read_bytes(write_bytes(&one).unwrap()).unwrap();
    assert_eq!(get(&w, "bookViews").unwrap(), "<bookViews><workbookView activeTab=\"0\" firstSheet=\"0\"/></bookViews>");
    // Saved onto the original: protection removed, title changed, the rest as it was.
    let mut changed = back.clone();
    changed.extra.retain(|e| e.0 != "workbookProtection");
    changed.properties[0].1 = core.replace("Plan", "Neu");
    changed.properties.push(("custom".into(), "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\"/>".into()));
    let (out, kept) = update_bytes_report(bytes, &changed).unwrap();
    assert!(kept);
    let after = read_bytes(out).unwrap();
    assert_eq!(after.extra.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), vec!["workbookPr", "bookViews", "calcPr"]);
    assert_eq!(after.properties, changed.properties);
}

#[test]
fn iso_date_cells_are_numbers_and_stay_iso() {
    for date1904 in [false, true] {
        let mut wb = Workbook { date1904, ..Workbook::default() };
        wb.sheets.push(Sheet { name: "S".into(), ..Default::default() });
        let cases = [("2021-01-01", IsoDate::Date), ("2021-01-01T10:10:10", IsoDate::DateTime), ("10:10:10", IsoDate::Time), ("1999-12-31T23:59:59.250", IsoDate::DateTime)];
        for (r, (text, form)) in cases.iter().enumerate() {
            let (n, f) = IsoDate::parse(text, date1904).unwrap();
            assert_eq!(f, *form);
            assert_eq!(f.format(n, date1904).unwrap(), *text);
            wb.cells.push(0, r as u32, 0, Value::Number(n), None, 0);
            *wb.cells.iso.last_mut().unwrap() = f;
        }
        let (n, _) = IsoDate::parse("2021-01-01", date1904).unwrap();
        assert_eq!(n, if date1904 { 42735.0 } else { 44197.0 });
        let back = read_bytes(write_bytes(&wb).unwrap()).unwrap();
        assert_eq!(back.cells, wb.cells);
        assert_eq!(IsoDate::parse("2021-02-30", false), None);
        assert_eq!(IsoDate::parse("25:00:00", false), None);
    }
}
