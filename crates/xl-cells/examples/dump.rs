//! Print the cell table of a workbook as tab-separated text.
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.xlsx> [max-rows]");
    let max: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let wb = xl_cells::read_bytes(std::fs::read(&path).expect("read file")).expect("parse workbook");
    eprintln!("{} sheets, {} cells, {} styles", wb.sheets.len(), wb.cells.len(), wb.styles.len());
    let c = &wb.cells;
    for i in 0..c.len().min(max) {
        println!("{}\t{}\t{}\t{:?}\t{}\t{:?}\t{:?}\t{}", c.sheet[i], c.row[i], c.col[i], c.kind[i], c.num[i], c.text[i], c.formula[i], c.style[i]);
    }
}
