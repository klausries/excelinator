//! Shape analysis: how much of a workbook's formulas is column-shaped, i.e. could be
//! expressed as one expression per column in a dataframe engine instead of one
//! formula per cell.

use calamine::{Reader, Xlsx};
use std::collections::BTreeMap;
use std::io::Cursor;
use xl_formula::{parse, parse_cell_ref, BinaryOp, Expr, RefKind};

/// What a formula needs from a dataframe engine, most demanding first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Reads another workbook.
    External,
    /// OFFSET / INDIRECT: the cells read are only known while calculating.
    Dynamic,
    /// Reads its own column in another row (running balance, carry-forward).
    SelfRecursive,
    /// VLOOKUP, INDEX/MATCH and similar: a join.
    Lookup,
    /// A range that moves with the row and spans several rows: rolling or cumulative.
    Window,
    /// A cell in another row of another column: shift.
    Shifted,
    /// Aggregates a fixed range or a whole column: aggregate, then broadcast.
    Aggregate,
    /// Reads only its own row, fixed cells and constants: a plain column expression.
    RowWise,
    /// The parser rejected the text.
    Unparsed,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::External => "external",
            Kind::Dynamic => "dynamic",
            Kind::SelfRecursive => "self-recursive",
            Kind::Lookup => "lookup",
            Kind::Window => "window",
            Kind::Shifted => "shifted",
            Kind::Aggregate => "aggregate",
            Kind::RowWise => "row-wise",
            Kind::Unparsed => "unparsed",
        }
    }
}

const LOOKUPS: &[&str] = &["VLOOKUP", "HLOOKUP", "LOOKUP", "INDEX", "MATCH", "XLOOKUP", "XMATCH", "_XLFN.XLOOKUP", "_XLFN.XMATCH", "GETPIVOTDATA"];
const DYNAMIC: &[&str] = &["OFFSET", "INDIRECT"];

/// Classify one formula sitting in (row, col), 1-based.
pub fn classify(e: &Expr, row: u32, col: u32) -> Kind {
    let mut kind = Kind::RowWise;
    let mut note = |k: Kind| {
        if k < kind {
            kind = k;
        }
    };
    fn cell(e: &Expr) -> Option<(xl_formula::CellRef, bool)> {
        match e.unspaced() {
            Expr::Ref { kind: RefKind::Cell, text, prefix } => parse_cell_ref(text).map(|r| (r, prefix.is_some())),
            _ => None,
        }
    }
    // Ranges first, so that their end points are not counted again as single cells.
    let mut in_range: Vec<*const Expr> = Vec::new();
    e.walk(&mut |n| match n {
        Expr::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
            if let (Some((a, _)), Some((b, _))) = (cell(lhs), cell(rhs)) {
                in_range.push(lhs.unspaced() as *const Expr);
                in_range.push(rhs.unspaced() as *const Expr);
                let fixed = a.abs_row && b.abs_row;
                let same_row = !a.abs_row && !b.abs_row && a.row == row && b.row == row;
                note(if fixed {
                    Kind::Aggregate
                } else if same_row {
                    Kind::RowWise
                } else {
                    Kind::Window
                });
            } else {
                note(Kind::Dynamic); // range built from computed end points
            }
        }
        Expr::Ref { kind: RefKind::Columns | RefKind::Rows, .. } => note(Kind::Aggregate),
        Expr::Ref { prefix: Some(p), .. } | Expr::Name { prefix: Some(p), .. } if p.contains('[') => note(Kind::External),
        Expr::Call { name, .. } => {
            let n = name.to_ascii_uppercase();
            if DYNAMIC.contains(&n.as_str()) {
                note(Kind::Dynamic);
            } else if LOOKUPS.contains(&n.as_str()) {
                note(Kind::Lookup);
            }
        }
        _ => {}
    });
    e.walk(&mut |n| {
        if in_range.contains(&(n as *const Expr)) {
            return;
        }
        if let Some((r, other_sheet)) = cell(n) {
            if !r.abs_row && r.row != row {
                note(if !other_sheet && r.col == col { Kind::SelfRecursive } else { Kind::Shifted });
            }
        }
    });
    kind
}

#[derive(Debug, Default, Clone)]
pub struct ShapeStats {
    pub formula_cells: u64,
    /// (in a vertical run, in a horizontal run only, single) per kind
    pub by_kind: BTreeMap<Kind, (u64, u64, u64)>,
    /// Number of vertical runs and of distinct column formulas they stand for.
    pub vertical_runs: u64,
    pub sheets: u64,
    /// Sheets in which every formula cell is part of a vertical run.
    pub sheets_fully_columnar: u64,
}

/// Minimum number of consecutive cells with the same relative formula to count as a run.
pub const MIN_RUN: usize = 3;

pub fn analyse(bytes: &[u8]) -> Option<ShapeStats> {
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).ok()?;
    let mut st = ShapeStats::default();
    for name in wb.sheet_names() {
        let Ok(range) = wb.worksheet_formula(&name) else { continue };
        let Some((r0, c0)) = range.start() else { continue };
        // (row, col, relative form, kind)
        let mut cells: Vec<(u32, u32, String, Kind)> = Vec::new();
        for (r, c, f) in range.cells().filter(|c| !c.2.is_empty()) {
            let (row, col) = (r0 + r as u32 + 1, c0 + c as u32 + 1);
            match parse(f) {
                Ok(e) => cells.push((row, col, e.relative_form(row, col), classify(&e, row, col))),
                Err(_) => cells.push((row, col, f.clone(), Kind::Unparsed)),
            }
        }
        if cells.is_empty() {
            continue;
        }
        st.sheets += 1;
        st.formula_cells += cells.len() as u64;
        let mut state = vec![0u8; cells.len()]; // 0 single, 1 vertical, 2 horizontal
        // Vertical runs: same column, consecutive rows, same relative form.
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by_key(|&i| (cells[i].1, cells[i].0));
        let mark = |order: &[usize], state: &mut [u8], tag: u8, vertical: bool, runs: &mut u64| {
            let mut start = 0;
            while start < order.len() {
                let mut end = start + 1;
                while end < order.len() {
                    let (a, b) = (&cells[order[end - 1]], &cells[order[end]]);
                    let adjacent = if vertical { a.1 == b.1 && a.0 + 1 == b.0 } else { a.0 == b.0 && a.1 + 1 == b.1 };
                    if !adjacent || a.2 != b.2 || a.3 == Kind::Unparsed {
                        break;
                    }
                    end += 1;
                }
                if end - start >= MIN_RUN {
                    *runs += 1;
                    for &i in &order[start..end] {
                        if state[i] == 0 {
                            state[i] = tag;
                        }
                    }
                }
                start = end;
            }
        };
        mark(&order, &mut state, 1, true, &mut st.vertical_runs);
        order.sort_by_key(|&i| (cells[i].0, cells[i].1));
        let mut ignored = 0;
        mark(&order, &mut state, 2, false, &mut ignored);
        for (i, c) in cells.iter().enumerate() {
            let e = st.by_kind.entry(c.3).or_default();
            match state[i] {
                1 => e.0 += 1,
                2 => e.1 += 1,
                _ => e.2 += 1,
            }
        }
        st.sheets_fully_columnar += state.iter().all(|&s| s == 1) as u64;
    }
    Some(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(f: &str, row: u32, col: u32) -> Kind {
        classify(&parse(f).unwrap(), row, col)
    }

    #[test]
    fn classification() {
        assert_eq!(k("B2*C2", 2, 4), Kind::RowWise);
        assert_eq!(k("B2*$F$1", 2, 4), Kind::RowWise);
        assert_eq!(k("SUM(B2:F2)", 2, 7), Kind::RowWise);
        assert_eq!(k("B2/SUM($B$2:$B$50)", 2, 4), Kind::Aggregate);
        assert_eq!(k("B2/SUM(B:B)", 2, 4), Kind::Aggregate);
        assert_eq!(k("B2-B1", 2, 4), Kind::Shifted);
        assert_eq!(k("D1+B2", 2, 4), Kind::SelfRecursive);
        assert_eq!(k("SUM($B$2:B2)", 2, 4), Kind::Window);
        assert_eq!(k("AVERAGE(B1:B3)", 2, 4), Kind::Window);
        assert_eq!(k("VLOOKUP(A2,Prices!$A$1:$B$9,2,FALSE)", 2, 4), Kind::Lookup);
        assert_eq!(k("OFFSET(A1,1,1)", 2, 4), Kind::Dynamic);
        assert_eq!(k("[1]Sheet1!A2*2", 2, 4), Kind::External);
        assert_eq!(k("Sheet2!B2*2", 2, 4), Kind::RowWise);
    }
}
