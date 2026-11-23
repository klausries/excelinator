//! Language contract: a formula typed in German is stored in English and shown
//! again in German. German and English are the first two supported languages;
//! further languages only need the engine's language table.

use calamine::Reader;
use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;

fn model(lang: &'static str) -> Model<'static> {
    Model::new_empty("test", lang, "UTC", lang).unwrap()
}

#[test]
fn german_input_is_stored_in_english_and_shown_in_german() {
    let mut m = model("de");
    m.set_user_input(0, 1, 1, "10,5".into()).unwrap();
    m.set_user_input(0, 2, 1, "=SUMME(A1;1,5)".into()).unwrap();
    m.set_user_input(0, 3, 1, "=WENN(A2>10;\"groß\";\"klein\")".into()).unwrap();
    m.evaluate();
    assert_eq!(m.get_cell_value_by_index(0, 1, 1), Ok(CellValue::Number(10.5)));
    assert_eq!(m.get_cell_value_by_index(0, 2, 1), Ok(CellValue::Number(12.0)));
    assert_eq!(m.get_cell_value_by_index(0, 3, 1), Ok(CellValue::String("groß".into())));
    // What the user sees: German names, semicolon, decimal comma.
    assert_eq!(m.get_localized_cell_content(0, 2, 1).unwrap(), "=SUMME(A1;1,5)");
    // What goes into the file: English names, comma, decimal point.
    let bytes = ironcalc::export::save_xlsx_to_writer(&m, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
    let mut wb = calamine::Xlsx::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let name = wb.sheet_names()[0].clone();
    let formulas = wb.worksheet_formula(&name).unwrap();
    let stored: Vec<String> = formulas.cells().map(|c| c.2.clone()).filter(|f| !f.is_empty()).collect();
    assert_eq!(stored, ["SUM(A1,1.5)", "IF(A2>10,\"groß\",\"klein\")"]);
    // The stored form parses with our parser, and the written file is a package we round-trip.
    assert_eq!(xl_formula::parse(&stored[1]).unwrap().functions(), ["IF"]);
    assert_eq!(xl_golden::roundtrip(&bytes).status, "ok");
}

#[test]
fn the_same_workbook_reads_in_english() {
    let mut m = model("de");
    m.set_user_input(0, 1, 1, "=SUMME(1;2,5)".into()).unwrap();
    m.evaluate();
    m.set_language("en").unwrap();
    m.set_locale("en").unwrap();
    assert_eq!(m.get_localized_cell_content(0, 1, 1).unwrap(), "=SUM(1,2.5)");
    assert_eq!(m.get_cell_value_by_index(0, 1, 1), Ok(CellValue::Number(3.5)));
}
