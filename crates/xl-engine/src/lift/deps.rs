//! Which formula cells a formula reads, for the order of trust in `calculate`: cells on a
//! cycle, and cells that read (directly or through others) a cell whose value could not
//! be computed, must not take a computed value.

use std::collections::HashMap;
use xl_cells::Workbook;
use xl_formula::{parse, parse_cell_ref, BinaryOp, Expr, RefKind};

/// A block of cells: sheet, first row, first column, last row, last column (0-based).
type Rect = (u32, u32, u32, u32, u32);

/// What a formula reads.
#[derive(Default)]
pub(super) struct Reads {
    pub rects: Vec<Rect>,
    /// It reads cells that are only known when it is evaluated (INDIRECT, OFFSET,
    /// structured references, 3D references, names this module cannot resolve).
    pub dynamic: bool,
}

/// Functions whose result can be a reference not written in the formula.
const DYNAMIC_FUNCTIONS: &[&str] = &["INDIRECT", "OFFSET", "CELL", "INFO", "EVALUATE", "GETPIVOTDATA"];

fn sheet_of(wb: &Workbook, prefix: Option<&str>, own: u32) -> Option<u32> {
    let Some(p) = prefix else { return Some(own) };
    let p = p.strip_suffix('!')?;
    let p = p.strip_prefix('\'').and_then(|q| q.strip_suffix('\'')).map(|q| q.replace("''", "'")).unwrap_or_else(|| p.to_string());
    if p.contains(':') || p.starts_with('[') {
        return None; // 3D or another workbook
    }
    wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(&p)).map(|i| i as u32)
}

/// The block a reference (or two references joined by `:`) stands for.
fn block(wb: &Workbook, e: &Expr, own: u32) -> Option<Rect> {
    match e.unspaced() {
        Expr::Ref { prefix, kind, text } => {
            let sheet = sheet_of(wb, prefix.as_deref(), own)?;
            let max_r = xl_formula::MAX_ROW - 1;
            let max_c = xl_formula::MAX_COL - 1;
            match kind {
                RefKind::Cell => {
                    let r = parse_cell_ref(text)?;
                    Some((sheet, r.row - 1, r.col - 1, r.row - 1, r.col - 1))
                }
                RefKind::Columns => {
                    let (a, b) = text.split_once(':')?;
                    let col = |s: &str| parse_cell_ref(&format!("{}1", s.trim_start_matches('$'))).map(|r| r.col - 1);
                    Some((sheet, 0, col(a)?, max_r, col(b)?))
                }
                RefKind::Rows => {
                    let (a, b) = text.split_once(':')?;
                    let row = |s: &str| s.trim_start_matches('$').parse::<u32>().ok().map(|r| r - 1);
                    Some((sheet, row(a)?, 0, row(b)?, max_c))
                }
            }
        }
        Expr::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
            // `Sheet1!A1:B2`: the sheet is written once, before the first end.
            let a = block(wb, lhs, own)?;
            let b = block(wb, rhs, a.0)?;
            if a.0 != b.0 {
                return None;
            }
            Some((a.0, a.1.min(b.1), a.2.min(b.2), a.3.max(b.3), a.4.max(b.4)))
        }
        Expr::Paren(inner) => block(wb, inner, own),
        _ => None,
    }
}

/// What the formula of a cell on sheet `own` reads; names are followed (to depth 8).
pub(super) fn reads(wb: &Workbook, formula: &str, own: u32) -> Reads {
    let mut out = Reads::default();
    collect(wb, formula, own, &mut out, 0);
    out
}

fn collect(wb: &Workbook, formula: &str, own: u32, out: &mut Reads, depth: u32) {
    let Ok(expr) = parse(formula) else {
        out.dynamic = true;
        return;
    };
    let mut ranges: Vec<&Expr> = Vec::new();
    expr.walk(&mut |e| {
        if let Expr::Binary { op: BinaryOp::Range, .. } = e {
            ranges.push(e);
        }
    });
    // References inside a range are part of its block.
    let mut inside: std::collections::HashSet<*const Expr> = std::collections::HashSet::new();
    for r in &ranges {
        if let Expr::Binary { lhs, rhs, .. } = r {
            lhs.walk(&mut |e| {
                inside.insert(e as *const Expr);
            });
            rhs.walk(&mut |e| {
                inside.insert(e as *const Expr);
            });
        }
    }
    let mut names: Vec<(Option<String>, String)> = Vec::new();
    expr.walk(&mut |e| match e {
        Expr::Binary { op: BinaryOp::Range, .. } if !inside.contains(&(e as *const Expr)) => match block(wb, e, own) {
            Some(b) => out.rects.push(b),
            // A range with a name or function at one end, or over several sheets: what it
            // spans is known when evaluated.
            None => out.dynamic = true,
        },
        Expr::Ref { .. } if !inside.contains(&(e as *const Expr)) => match block(wb, e, own) {
            Some(b) => out.rects.push(b),
            None => {
                // Another workbook or a 3D reference: no formula cells of this workbook,
                // or (3D) several sheets.
                if let Expr::Ref { prefix: Some(p), .. } = e {
                    if !p.starts_with('[') && !p.starts_with("'[") {
                        out.dynamic = true;
                    }
                }
            }
        },
        Expr::Name { prefix, text } => names.push((prefix.clone(), text.clone())),
        Expr::Structured { .. } => out.dynamic = true,
        Expr::Call { name, .. } if DYNAMIC_FUNCTIONS.contains(&name.to_ascii_uppercase().trim_start_matches("_XLFN.")) => out.dynamic = true,
        _ => {}
    });
    for (prefix, name) in names {
        if depth >= 8 {
            out.dynamic = true;
            continue;
        }
        // A name of another workbook ([1]!Rate) is also followed into a name of this one
        // with the same text: more edges only make fewer cells trusted, and LibreOffice
        // writes names of the own workbook that way when it converts .xls files.
        let local = prefix.as_deref().and_then(|p| sheet_of(wb, Some(p), own)).unwrap_or(own);
        let found = wb
            .defined_names
            .iter()
            .find(|n| n.local_sheet == Some(local) && n.name.eq_ignore_ascii_case(&name))
            .or_else(|| wb.defined_names.iter().find(|n| n.local_sheet.is_none() && n.name.eq_ignore_ascii_case(&name)));
        match found {
            Some(n) => collect(wb, &n.value, own, out, depth + 1),
            // TRUE/FALSE-like words, LAMBDA parameters, ...: nothing to read; an unknown
            // name is an error in Excel, so reading nothing is right.
            None => {}
        }
    }
}

/// For each formula cell (by cell index): the formula cells it reads, and whether it
/// reads dynamically.
pub(super) fn graph(wb: &Workbook) -> (HashMap<usize, Vec<usize>>, Vec<usize>) {
    let c = &wb.cells;
    // Formula cells by sheet and column, sorted by row, for looking up blocks.
    let mut by_col: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
    let mut formulas = Vec::new();
    for i in 0..c.len() {
        if c.formula[i].as_deref().is_some_and(|f| !f.is_empty()) {
            by_col.entry((c.sheet[i], c.col[i])).or_default().push((c.row[i], i));
            formulas.push(i);
        }
    }
    for v in by_col.values_mut() {
        v.sort();
    }
    let mut cols_of_sheet: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(s, col) in by_col.keys() {
        cols_of_sheet.entry(s).or_default().push(col);
    }
    let mut edges: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut dynamic = Vec::new();
    for &i in &formulas {
        let r = reads(wb, c.formula[i].as_deref().unwrap_or(""), c.sheet[i]);
        if r.dynamic {
            dynamic.push(i);
        }
        let mut targets = Vec::new();
        for (s, r1, c1, r2, c2) in r.rects {
            for &col in cols_of_sheet.get(&s).map(|v| v.as_slice()).unwrap_or(&[]) {
                if col < c1 || col > c2 {
                    continue;
                }
                let cells = &by_col[&(s, col)];
                let from = cells.partition_point(|x| x.0 < r1);
                for &(row, j) in &cells[from..] {
                    if row > r2 {
                        break;
                    }
                    targets.push(j);
                }
            }
        }
        targets.sort_unstable();
        targets.dedup();
        edges.insert(i, targets);
    }
    (edges, dynamic)
}

/// Cells on a cycle (a strongly connected group of more than one, or reading itself).
pub(super) fn cyclic(edges: &HashMap<usize, Vec<usize>>) -> Vec<usize> {
    // Tarjan, without recursion.
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut low: HashMap<usize, usize> = HashMap::new();
    let mut on_stack: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let mut next = 0usize;
    let mut nodes: Vec<usize> = edges.keys().copied().collect();
    nodes.sort_unstable();
    let empty: Vec<usize> = Vec::new();
    for &start in &nodes {
        if index.contains_key(&start) {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(start, 0)];
        index.insert(start, next);
        low.insert(start, next);
        next += 1;
        stack.push(start);
        on_stack.insert(start);
        while let Some(&mut (v, ref mut k)) = work.last_mut() {
            let succ = edges.get(&v).unwrap_or(&empty);
            if *k < succ.len() {
                let w = succ[*k];
                *k += 1;
                if !edges.contains_key(&w) {
                    continue;
                }
                if !index.contains_key(&w) {
                    index.insert(w, next);
                    low.insert(w, next);
                    next += 1;
                    stack.push(w);
                    on_stack.insert(w);
                    work.push((w, 0));
                } else if on_stack.contains(&w) {
                    let lw = index[&w];
                    let lv = low[&v];
                    low.insert(v, lv.min(lw));
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    let lv = low[&v];
                    let lp = low[&parent];
                    low.insert(parent, lp.min(lv));
                }
                if low[&v] == index[&v] {
                    let mut group = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack.remove(&w);
                        group.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if group.len() > 1 || edges.get(&v).is_some_and(|s| s.contains(&v)) {
                        out.extend(group);
                    }
                }
            }
        }
    }
    out
}
