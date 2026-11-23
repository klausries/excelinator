//! `dump FILE`: the look of the cells A1:L40 of every sheet, one line each:
//! sheet, cell, fill, font colour, bold, italic, certain (for tools/excel_look.py).
use xl_look::{Looker, Val};

fn main() {
    let path = std::env::args().nth(1).expect("dump FILE");
    let wb = xl_cells::read_bytes(std::fs::read(path).unwrap()).unwrap();
    let mut looker = Looker::new(&wb);
    let show = |v: Option<&Val>| match v {
        Some(Val::Rgb(c)) => c.clone(),
        Some(Val::Flag(b)) => (*b as u8).to_string(),
        _ => "-".into(),
    };
    for (s, sheet) in wb.sheets.iter().enumerate() {
        let cells: Vec<(u32, u32, u32)> = (0..40).flat_map(|r| (0..12).map(move |c| (s as u32, r, c))).collect();
        for (&(_, r, c), l) in cells.iter().zip(looker.looks(&cells)) {
            let certain = ["fill_color", "font_color", "bold", "italic"].iter().all(|k| l.get(k).is_none_or(|a| a.certain));
            println!("{}\t{}{}\t{}\t{}\t{}\t{}\t{}", sheet.name, xl_formula::col_name(c + 1), r + 1, show(l.value("fill_color")), show(l.value("font_color")), show(l.value("bold")), show(l.value("italic")), certain as u8);
        }
    }
}
