//! Reference geometry: A1 text to coordinates, and formulas in relative (R1C1) form.
//!
//! Two formulas that were produced by filling one cell down or across have the same
//! relative form. That is what makes a block of cells one column-wise computation.

use crate::{Expr, RefKind};

/// A single cell reference. Columns and rows are 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    pub col: u32,
    pub row: u32,
    pub abs_col: bool,
    pub abs_row: bool,
}

fn col_of(letters: &str) -> Option<u32> {
    if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(letters.bytes().fold(0u32, |n, b| n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32))
}

/// Split `$A`, `A`, `$12`, `12` into (absolute, body).
fn piece(s: &str) -> (bool, &str) {
    match s.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, s),
    }
}

/// Parse the text of a `RefKind::Cell` reference such as `$B$12`.
pub fn parse_cell_ref(text: &str) -> Option<CellRef> {
    let (abs_col, rest) = piece(text);
    let split = rest.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, rest) = rest.split_at(split);
    let (abs_row, digits) = piece(rest);
    Some(CellRef { col: col_of(letters)?, row: digits.parse().ok()?, abs_col, abs_row })
}

fn rel(axis: char, abs: bool, target: u32, origin: u32) -> String {
    if abs {
        format!("{axis}{target}")
    } else {
        match target as i64 - origin as i64 {
            0 => axis.to_string(),
            d => format!("{axis}[{d}]"),
        }
    }
}

/// Text of a reference in R1C1 notation relative to the cell (row, col), 1-based.
pub fn to_r1c1(kind: RefKind, text: &str, row: u32, col: u32) -> Option<String> {
    match kind {
        RefKind::Cell => {
            let r = parse_cell_ref(text)?;
            Some(format!("{}{}", rel('R', r.abs_row, r.row, row), rel('C', r.abs_col, r.col, col)))
        }
        RefKind::Columns => {
            let (a, b) = text.split_once(':')?;
            let ((aa, al), (ba, bl)) = (piece(a), piece(b));
            Some(format!("{}:{}", rel('C', aa, col_of(al)?, col), rel('C', ba, col_of(bl)?, col)))
        }
        RefKind::Rows => {
            let (a, b) = text.split_once(':')?;
            let ((aa, ad), (ba, bd)) = (piece(a), piece(b));
            Some(format!("{}:{}", rel('R', aa, ad.parse().ok()?, row), rel('R', ba, bd.parse().ok()?, row)))
        }
    }
}

/// Column letters of a 1-based column number.
pub fn col_name(mut n: u32) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

pub const MAX_ROW: u32 = 1_048_576;
pub const MAX_COL: u32 = 16_384;

fn moved(abs: bool, v: u32, d: i64, max: u32) -> Option<u32> {
    if abs {
        return Some(v);
    }
    let n = v as i64 + d;
    (n >= 1 && n <= max as i64).then_some(n as u32)
}

fn dollar(abs: bool) -> &'static str {
    if abs { "$" } else { "" }
}

/// The reference moved by (drow, dcol); absolute parts stay. `None` when a relative
/// part leaves the grid.
pub fn shift_ref(kind: RefKind, text: &str, drow: i64, dcol: i64) -> Option<String> {
    match kind {
        RefKind::Cell => {
            let r = parse_cell_ref(text)?;
            let col = moved(r.abs_col, r.col, dcol, MAX_COL)?;
            let row = moved(r.abs_row, r.row, drow, MAX_ROW)?;
            Some(format!("{}{}{}{}", dollar(r.abs_col), col_name(col), dollar(r.abs_row), row))
        }
        RefKind::Columns => {
            let (a, b) = text.split_once(':')?;
            let ((aa, al), (ba, bl)) = (piece(a), piece(b));
            let (c1, c2) = (moved(aa, col_of(al)?, dcol, MAX_COL)?, moved(ba, col_of(bl)?, dcol, MAX_COL)?);
            Some(format!("{}{}:{}{}", dollar(aa), col_name(c1), dollar(ba), col_name(c2)))
        }
        RefKind::Rows => {
            let (a, b) = text.split_once(':')?;
            let ((aa, ad), (ba, bd)) = (piece(a), piece(b));
            let (r1, r2) = (moved(aa, ad.parse().ok()?, drow, MAX_ROW)?, moved(ba, bd.parse().ok()?, drow, MAX_ROW)?);
            Some(format!("{}{}:{}{}", dollar(aa), r1, dollar(ba), r2))
        }
    }
}

impl Expr {
    /// The formula as it reads after being filled (drow, dcol) cells away: relative
    /// references move, absolute ones stay, references leaving the grid become `#REF!`.
    pub fn shifted(&self, drow: i64, dcol: i64) -> Expr {
        collapse_ref_ranges(self.map_refs(&mut |kind, text| shift_ref(kind, text, drow, dcol).unwrap_or_else(|| "#REF!".to_string())))
    }
}

/// A range one of whose ends has left the grid is `#REF!` as a whole (as Excel writes it).
fn collapse_ref_ranges(e: Expr) -> Expr {
    let lost = |x: &Expr| matches!(x.unspaced(), Expr::Ref { text, .. } if text == "#REF!");
    let go = |x: Box<Expr>| Box::new(collapse_ref_ranges(*x));
    match e {
        Expr::Binary { op: crate::BinaryOp::Range, op_text, lhs, rhs } => {
            if lost(&lhs) || lost(&rhs) {
                let prefix = match lhs.unspaced() {
                    Expr::Ref { prefix, .. } => prefix.clone(),
                    _ => None,
                };
                return Expr::Ref { prefix, kind: crate::RefKind::Cell, text: "#REF!".into() };
            }
            Expr::Binary { op: crate::BinaryOp::Range, op_text, lhs: go(lhs), rhs: go(rhs) }
        }
        Expr::Binary { op, op_text, lhs, rhs } => Expr::Binary { op, op_text, lhs: go(lhs), rhs: go(rhs) },
        Expr::Call { name, args } => Expr::Call { name, args: args.into_iter().map(collapse_ref_ranges).collect() },
        Expr::Apply { callee, args } => Expr::Apply { callee: go(callee), args: args.into_iter().map(collapse_ref_ranges).collect() },
        Expr::Unary { op, expr } => Expr::Unary { op, expr: go(expr) },
        Expr::Postfix { expr, op } => Expr::Postfix { expr: go(expr), op },
        Expr::Paren(x) => Expr::Paren(go(x)),
        Expr::Spaced { before, expr, after } => Expr::Spaced { before, expr: go(expr), after },
        Expr::Array(rows) => Expr::Array(rows.into_iter().map(|r| r.into_iter().map(collapse_ref_ranges).collect()).collect()),
        other => other,
    }
}

impl Expr {
    /// A copy of the tree in which every part `f` gives another expression for is that
    /// one; the parts of the others are asked in turn (from the outside in).
    pub fn rebuilt(&self, f: &mut dyn FnMut(&Expr) -> Option<Expr>) -> Expr {
        if let Some(other) = f(self) {
            return other;
        }
        let b = |e: &Expr, f: &mut dyn FnMut(&Expr) -> Option<Expr>| Box::new(e.rebuilt(f));
        match self {
            Expr::Call { name, args } => Expr::Call { name: name.clone(), args: args.iter().map(|a| a.rebuilt(f)).collect() },
            Expr::Apply { callee, args } => Expr::Apply { callee: b(callee, f), args: args.iter().map(|a| a.rebuilt(f)).collect() },
            Expr::Unary { op, expr } => Expr::Unary { op: *op, expr: b(expr, f) },
            Expr::Postfix { expr, op } => Expr::Postfix { expr: b(expr, f), op: *op },
            Expr::Binary { op, op_text, lhs, rhs } => Expr::Binary { op: *op, op_text: op_text.clone(), lhs: b(lhs, f), rhs: b(rhs, f) },
            Expr::Paren(e) => Expr::Paren(b(e, f)),
            Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|e| e.rebuilt(f)).collect()).collect()),
            Expr::Spaced { before, expr, after } => Expr::Spaced { before: before.clone(), expr: b(expr, f), after: after.clone() },
            other => other.clone(),
        }
    }

    /// A copy of the tree in which the text of every reference is replaced by `f(kind, text)`.
    pub fn map_refs(&self, f: &mut dyn FnMut(RefKind, &str) -> String) -> Expr {
        self.rebuilt(&mut |e| match e {
            Expr::Ref { prefix, kind, text } => Some(Expr::Ref { prefix: prefix.clone(), kind: *kind, text: f(*kind, text) }),
            _ => None,
        })
    }

    /// A copy of the tree with other cells referred to: `f(prefix, first, last)` is asked
    /// for every reference to a cell (`last` is `None`) and every range of two cells
    /// (`A1:B9`), and gives their new texts, or `None` to leave them.
    pub fn map_cells(&self, f: &mut dyn FnMut(&Option<String>, &str, Option<&str>) -> Option<(String, Option<String>)>) -> Expr {
        let cell = |prefix: &Option<String>, text: String| Expr::Ref { prefix: prefix.clone(), kind: RefKind::Cell, text };
        self.rebuilt(&mut |e| match e {
            Expr::Binary { op: crate::BinaryOp::Range, op_text, lhs, rhs } => match (lhs.unspaced(), rhs.unspaced()) {
                (Expr::Ref { prefix, kind: RefKind::Cell, text: a }, Expr::Ref { prefix: None, kind: RefKind::Cell, text: z }) => Some(match f(prefix, a, Some(z)) {
                    Some((a, Some(z))) => Expr::Binary { op: crate::BinaryOp::Range, op_text: op_text.clone(), lhs: Box::new(cell(prefix, a)), rhs: Box::new(cell(&None, z)) },
                    _ => e.clone(),
                }),
                _ => None,
            },
            Expr::Ref { prefix, kind: RefKind::Cell, text } => Some(f(prefix, text, None).map_or_else(|| e.clone(), |(a, _)| cell(prefix, a))),
            _ => None,
        })
    }

    /// The formula with all references in R1C1 notation relative to the cell (row, col).
    /// Cells filled from one another have equal relative forms.
    pub fn relative_form(&self, row: u32, col: u32) -> String {
        self.map_refs(&mut |kind, text| to_r1c1(kind, text, row, col).unwrap_or_else(|| text.to_string())).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn cell_refs() {
        assert_eq!(parse_cell_ref("B12"), Some(CellRef { col: 2, row: 12, abs_col: false, abs_row: false }));
        assert_eq!(parse_cell_ref("$AA$1"), Some(CellRef { col: 27, row: 1, abs_col: true, abs_row: true }));
        assert_eq!(parse_cell_ref("XFD1048576").map(|r| (r.col, r.row)), Some((16_384, 1_048_576)));
    }

    #[test]
    fn filled_down_formulas_share_a_relative_form() {
        let a = parse("B2*C2+$F$1").unwrap().relative_form(2, 4);
        let b = parse("B3*C3+$F$1").unwrap().relative_form(3, 4);
        assert_eq!(a, "RC[-2]*RC[-1]+R1C6");
        assert_eq!(a, b);
        assert_ne!(a, parse("B3*C3+$F$2").unwrap().relative_form(3, 4));
    }

    #[test]
    fn ranges_and_whole_columns() {
        assert_eq!(parse("SUM($A$1:A5)").unwrap().relative_form(5, 2), "SUM(R1C1:RC[-1])");
        assert_eq!(parse("SUM(A:A)+SUM($1:3)").unwrap().relative_form(2, 3), "SUM(C[-2]:C[-2])+SUM(R1:R[1])");
        assert_eq!(parse("Sheet2!A1").unwrap().relative_form(1, 1), "Sheet2!RC");
    }

    #[test]
    fn shifting() {
        let e = parse("SUM($A$1:A5)+B$2*$C3+Sheet2!D4+A:B+$1:2").unwrap();
        assert_eq!(e.shifted(2, 1).to_string(), "SUM($A$1:B7)+C$2*$C5+Sheet2!E6+B:C+$1:4");
        assert_eq!(parse("A1+B2").unwrap().shifted(-1, 0).to_string(), "#REF!+B1");
        assert_eq!(col_name(1), "A");
        assert_eq!(col_name(27), "AA");
        assert_eq!(col_name(16_384), "XFD");
    }
}
