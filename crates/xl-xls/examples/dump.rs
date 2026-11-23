//! Print what the .xls reader makes of a file: `cargo run --release -p xl-xls --example dump -- file.xls [cells]`.
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&p).unwrap();
    match xl_xls::read_with_notes(&b) {
        Ok((wb, n)) => {
            println!("sheets {:?} cells {} notes {:?}", wb.sheets.iter().map(|s| &s.name).collect::<Vec<_>>(), wb.cells.len(), n.skipped);
            let lim: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            for i in 0..wb.cells.len().min(lim) {
                println!("{} {} {} {:?} {:?}", wb.cells.sheet[i], wb.cells.row[i], wb.cells.col[i], wb.cells.value(i), wb.cells.formula[i]);
            }
            for d in &wb.defined_names { println!("name {:?}", d); }
            for s in &wb.sheets { println!("sheet {} {:?} {:?}", s.name, s.dimension, s.visibility); }
            for e in &wb.externals { println!("ext {:?} {:?} names {:?}", e.target, e.sheets, &e.names[..e.names.len().min(6)]); }
        }
        Err(e) => println!("ERR {e}"),
    }
}
