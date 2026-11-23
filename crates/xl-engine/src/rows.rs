//! Rows of a sheet moved: rows inserted or removed, in the whole sheet or in some of its
//! columns, with everything that names cells following (formulas, names, merged cells,
//! the ranges of filters, validations, conditional formats, tables), and ranges that
//! follow the last row of a table.

use std::collections::HashMap;
use xl_cells::Workbook;

/// The cells a reference names: columns and rows 0-based; `single` for one cell.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub c1: u32,
    pub c2: u32,
    pub a: u32,
    pub z: u32,
    pub single: bool,
}

/// The new first and last row of what a reference names, or `None` to leave it;
/// [`GONE`] as the first row for a cell that is no more.
type Rows<'a> = &'a dyn Fn(Span) -> Option<(u32, u32)>;

const GONE: u32 = u32::MAX;
const NO_CELL: &str = "#REF!";

fn pair(a: &str, z: Option<&str>, f: Rows) -> Option<(String, Option<String>)> {
    let first = xl_formula::parse_cell_ref(a)?;
    let last = match z {
        Some(z) => xl_formula::parse_cell_ref(z)?,
        None => first,
    };
    if last.row < first.row {
        return None;
    }
    let (a, b) = f(Span { c1: first.col.min(last.col) - 1, c2: first.col.max(last.col) - 1, a: first.row - 1, z: last.row - 1, single: z.is_none() })?;
    if a == GONE {
        return Some((NO_CELL.to_string(), None));
    }
    let text = |r: xl_formula::CellRef, row: u32| format!("{}{}{}{}", if r.abs_col { "$" } else { "" }, xl_formula::col_name(r.col), if r.abs_row { "$" } else { "" }, row + 1);
    Some((text(first, a), z.map(|_| text(last, b))))
}

/// A list of cells and ranges (`A1 C3:D9`); a cell that is no more is left out.
fn list(text: &str, f: Rows) -> String {
    let one = |t: &str| {
        let (a, z) = t.split_once(':').map_or((t, None), |(a, z)| (a, Some(z)));
        match pair(a, z, f) {
            Some((a, Some(z))) => format!("{a}:{z}"),
            Some((a, None)) => a,
            None => t.to_string(),
        }
    };
    text.split(' ').map(one).filter(|t| t != NO_CELL).collect::<Vec<_>>().join(" ")
}

/// The `ref` attributes of a piece of XML (a table, a filter).
fn refs(xml: &str, f: Rows) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(k) = rest.find(" ref=\"") {
        let (head, tail) = rest.split_at(k + 6);
        let end = tail.find('"').unwrap_or(tail.len());
        out.push_str(head);
        out.push_str(&list(&tail[..end], f));
        rest = &tail[end..];
    }
    out + rest
}

/// Everything that names cells of a sheet, with `f` applied: the formulas of the
/// workbook (not those of the cells `skip` says yes to, by row and column), its names,
/// and the sheet's array formulas, filter, validations, links, conditional formats,
/// comments and tables.
fn rewrite(wb: &mut Workbook, sheet: u32, skip: &dyn Fn(u32, u32) -> bool, f: Rows) {
    let name = wb.sheets[sheet as usize].name.clone();
    let formula = |text: &str, own: bool| -> Option<String> {
        let e = xl_formula::parse(text).ok()?;
        let here = |prefix: &Option<String>| match prefix {
            None => own,
            Some(p) => p.trim_end_matches('!').trim_matches('\'').replace("''", "'").eq_ignore_ascii_case(&name),
        };
        let new = e.map_cells(&mut |prefix, a, z| if here(prefix) { pair(a, z, f) } else { None }).to_string();
        (new != text).then_some(new)
    };
    let c = &mut wb.cells;
    for i in 0..c.len() {
        let own = c.sheet[i] == sheet;
        if own && skip(c.row[i], c.col[i]) {
            continue;
        }
        if let Some(new) = c.formula[i].as_deref().filter(|t| own || t.contains('!')).and_then(|t| formula(t, own)) {
            c.formula[i] = Some(new);
        }
        if own {
            if let Some(r) = &c.array_ref[i] {
                c.array_ref[i] = Some(list(r, f));
            }
        }
    }
    for n in wb.defined_names.iter_mut() {
        if let Some(new) = formula(&n.value, n.local_sheet == Some(sheet)) {
            n.value = new;
        }
    }
    let s = &mut wb.sheets[sheet as usize];
    s.auto_filter = s.auto_filter.as_deref().map(|a| list(a, f));
    for v in s.validations.iter_mut().flat_map(|v| v.attrs.iter_mut()).filter(|(k, _)| k == "sqref") {
        v.1 = list(&v.1, f);
    }
    s.hyperlinks.iter_mut().for_each(|h| h.cell = list(&h.cell, f));
    s.cond_formats.iter_mut().for_each(|c| c.sqref = list(&c.sqref, f));
    s.comments.iter_mut().for_each(|c| c.cell = list(&c.cell, f));
    // What was on a cell that is no more goes with it.
    s.validations.retain(|v| !v.attrs.iter().any(|(k, v)| k == "sqref" && v.is_empty()));
    s.hyperlinks.retain(|h| !h.cell.is_empty());
    s.cond_formats.retain(|c| !c.sqref.is_empty());
    s.comments.retain(|c| !c.cell.is_empty());
    s.tables.iter_mut().for_each(|xml| *xml = refs(xml, f));
    for (_, xml) in s.extra.iter_mut().filter(|(element, _)| element == "autoFilter") {
        *xml = refs(xml, f);
    }
}

/// Move the rows of a sheet from the row `from` on by `by` rows: down (`by` empty rows
/// come in before `from`) or up (the rows before `from` go; they should be empty). With
/// `cols`, only what lies within these columns moves, as when cells are inserted in
/// Excel; else the whole rows do, with their heights and what is drawn on them.
pub fn move_rows(wb: &mut Workbook, sheet: u32, cols: Option<(u32, u32)>, from: u32, by: i64) {
    if by == 0 {
        return;
    }
    let gone = (from as i64 + by.min(0)).max(0) as u32..from;
    let within = |c1: u32, c2: u32| cols.is_none_or(|(a, z)| a <= c1 && c2 <= z);
    // Where a row is afterwards. Of a range that reaches into the rows that go, what is
    // left; a single cell there is no more (`#REF!`, as in Excel).
    let place = |r: u32, end: bool| {
        if gone.contains(&r) {
            if end { gone.start.saturating_sub(1) } else { gone.start }
        } else if r >= from {
            (r as i64 + by) as u32
        } else {
            r
        }
    };
    rewrite(wb, sheet, &|_, _| false, &|s: Span| {
        let lost = if s.single { gone.contains(&s.a) } else { gone.contains(&s.a) && gone.contains(&s.z) };
        within(s.c1, s.c2).then(|| if lost { (GONE, GONE) } else { (place(s.a, false), place(s.z, true).max(place(s.a, false))) })
    });
    let c = &mut wb.cells;
    let moves = |i: usize| c.sheet[i] == sheet && within(c.col[i], c.col[i]);
    if !gone.is_empty() {
        let keep: Vec<bool> = (0..c.len()).map(|i| !(moves(i) && gone.contains(&c.row[i]))).collect();
        c.retain(&keep);
    }
    for i in 0..c.len() {
        if c.sheet[i] == sheet && within(c.col[i], c.col[i]) {
            c.row[i] = place(c.row[i], false);
        }
    }
    let s = &mut wb.sheets[sheet as usize];
    s.merges.retain(|m| !(within(m.1, m.3) && gone.contains(&m.0)));
    for m in s.merges.iter_mut().filter(|m| within(m.1, m.3)) {
        (m.0, m.2) = (place(m.0, false), place(m.2, true));
    }
    if cols.is_none() {
        s.rows.retain(|r| !gone.contains(&r.row));
        s.rows.iter_mut().for_each(|r| r.row = place(r.row, false));
        // Rows that come in are as the row above them.
        if let Some(above) = s.rows.iter().find(|r| by > 0 && r.row + 1 == from).cloned() {
            s.rows.extend((from..from + by as u32).map(|row| xl_cells::Row { row, ..above.clone() }));
            s.rows.sort_by_key(|r| r.row);
        }
        for d in s.drawings.iter_mut() {
            d.anchor.from.2 = place(d.anchor.from.2, false);
            if let Some(to) = d.anchor.to.as_mut() {
                to.2 = place(to.2, false);
            }
        }
    }
}

/// Ranges that end at the row `old` of columns `cols` of a sheet and begin above it (or at
/// the row `first`) end at the row `new`: what reads the rows of a table follows its
/// length. The formulas in the rows `first` to `old` of these columns stay as they are.
pub fn end_ranges(wb: &mut Workbook, sheet: u32, cols: (u32, u32), first: u32, old: u32, new: u32) {
    let inside = |c1: u32, c2: u32| cols.0 <= c1 && c2 <= cols.1;
    rewrite(wb, sheet, &|row, col| inside(col, col) && (first..=old).contains(&row), &|s: Span| (!s.single && inside(s.c1, s.c2) && s.z == old && (s.a < old || s.a == first)).then_some((s.a, new)));
}

/// The cells of a sheet that hold something (a value or a formula), by row: their columns.
pub fn used(wb: &Workbook, sheet: u32, rows: std::ops::RangeInclusive<u32>) -> HashMap<u32, Vec<u32>> {
    let c = &wb.cells;
    let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
    for i in (0..c.len()).filter(|&i| c.sheet[i] == sheet && rows.contains(&c.row[i])) {
        if c.kind[i] != xl_cells::Kind::Blank || c.formula[i].as_deref().is_some_and(|f| !f.is_empty()) {
            out.entry(c.row[i]).or_default().push(c.col[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use xl_cells::{Sheet, Value};

    fn book() -> Workbook {
        let mut wb = Workbook::default();
        wb.sheets.push(Sheet { name: "S".into(), merges: vec![(5, 0, 5, 1)], ..Default::default() });
        wb.sheets.push(Sheet { name: "T".into(), ..Default::default() });
        for r in 0..4 {
            wb.cells.push(0, r, 0, Value::Number(r as f64), None, 0);
            wb.cells.push(0, r, 3, Value::Number(1.0), None, 0);
        }
        wb.cells.push(0, 5, 0, Value::Number(6.0), Some("SUM(A1:A4)+$A$4+D5".into()), 0);
        wb.cells.push(1, 0, 0, Value::Number(6.0), Some("S!A6+SUM(S!A2:A9)+A6".into()), 0);
        wb
    }

    fn formulas(wb: &Workbook) -> Vec<(u32, u32, String)> {
        (0..wb.cells.len()).filter_map(|i| wb.cells.formula[i].clone().map(|f| (wb.cells.sheet[i], wb.cells.row[i], f))).collect()
    }

    #[test]
    fn rows_come_in_and_go_and_what_names_them_follows() {
        let mut wb = book();
        move_rows(&mut wb, 0, None, 4, 2);
        // The total moved down; what it reads above the new rows stays, what is below follows.
        assert_eq!(formulas(&wb), vec![(0, 7, "SUM(A1:A4)+$A$4+D7".to_string()), (1, 0, "S!A8+SUM(S!A2:A11)+A6".to_string())]);
        assert_eq!(wb.sheets[0].merges, vec![(7, 0, 7, 1)]);
        end_ranges(&mut wb, 0, (0, 0), 0, 3, 5);
        assert_eq!(formulas(&wb)[0].2, "SUM(A1:A6)+$A$4+D7");
        // And back.
        end_ranges(&mut wb, 0, (0, 0), 0, 5, 3);
        move_rows(&mut wb, 0, None, 6, -2);
        let mut back = book();
        back.cells.retain(&vec![true; back.cells.len()]);
        assert_eq!(formulas(&wb), formulas(&back));
        assert_eq!(wb.sheets[0].merges, back.sheets[0].merges);
        assert_eq!(wb.cells.row, back.cells.row);
        // A cell that goes is no more to what names it; a range keeps what is left of it.
        move_rows(&mut wb, 0, None, 4, -2);
        assert_eq!(formulas(&wb)[0].2, "SUM(A1:A2)+#REF!+D3");
        assert_eq!(formulas(&wb)[1].2, "S!A4+SUM(S!A2:A7)+A6");
    }

    #[test]
    fn rows_of_some_columns_move_alone() {
        let mut wb = book();
        move_rows(&mut wb, 0, Some((0, 1)), 4, 1);
        // Column D stays where it is, and so does the reference to it.
        assert_eq!(formulas(&wb)[0], (0, 6, "SUM(A1:A4)+$A$4+D5".to_string()));
        assert_eq!(used(&wb, 0, 0..=9).get(&3), Some(&vec![0, 3]));
    }
}
