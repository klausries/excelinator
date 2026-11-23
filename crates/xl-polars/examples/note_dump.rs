//! `note_dump FILE`: the notes of a workbook's regions, and how large their parts are.
fn main() {
    let path = std::env::args().nth(1).expect("a workbook");
    let bytes = std::fs::read(&path).unwrap();
    let wb = if path.to_lowercase().ends_with(".xls") { xl_xls::read_bytes(&bytes).unwrap() } else { xl_cells::read_bytes(bytes).unwrap() };
    let start = std::time::Instant::now();
    let notes = xl_engine::store::notes(&xl_polars::builtin::PolarsExecutor::default(), &wb);
    let took = start.elapsed();
    let (mut plans, mut inputs, mut items, mut all) = (0, 0, 0, 0);
    for n in &notes {
        all += n.to_string().len();
        for c in n["columns"].as_array().unwrap() {
            if !c["plan"].is_null() {
                items += 1;
                plans = n["plans"].to_string().len();
                inputs += c["inputs"].to_string().len();
            }
        }
    }
    eprintln!("{} regions, {items} plans, {all} bytes: plans {plans}, inputs {inputs}; {took:?}", notes.len());
    if std::env::args().nth(2).is_some() {
        println!("{}", serde_json::to_string_pretty(&notes).unwrap());
    }
}
