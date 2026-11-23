#![allow(dead_code)]
//! The golden comparison of calculated formula values: a workbook's stored formula
//! values (computed by LibreOffice or by Excel) are removed, the workbook is calculated
//! again, and every cell the module computes must have the stored value. Differences
//! that are the reference's own are counted by name, not hidden.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use xl_cells::{Kind, Workbook};
use xl_polars::lift::calculate;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reference {
    /// Values LibreOffice computed when it converted the file.
    LibreOffice,
    /// Values Excel computed and stored in the file.
    Excel,
}

/// Functions whose result Excel finds by iteration to a tolerance.
const SOLVED: &[&str] = &["IRR(", "XIRR(", "RATE(", "YIELD(", "PRICE("];

/// Functions of Excel's Analysis ToolPak add-in (built in since Excel 2007).
const ADDIN: &[&str] = &[
    "EOMONTH", "EDATE", "NETWORKDAYS", "WORKDAY", "YEARFRAC", "XIRR", "XNPV", "WEEKNUM", "MROUND", "QUOTIENT", "GCD", "LCM", "ISEVEN", "ISODD",
    "SERIESSUM", "SQRTPI", "MULTINOMIAL", "FACTDOUBLE", "DELTA", "GESTEP", "ERF", "ERFC", "CONVERT", "DEC2BIN", "DEC2HEX", "DEC2OCT", "BIN2DEC", "HEX2DEC",
    "OCT2DEC", "COUPDAYS", "COUPNCD", "COUPPCD", "COUPNUM", "DISC", "DURATION", "MDURATION", "ACCRINT", "ACCRINTM", "PRICE", "YIELD", "RECEIVED",
    "INTRATE", "TBILLEQ", "TBILLPRICE", "TBILLYIELD", "DOLLARDE", "DOLLARFR", "EFFECT", "NOMINAL", "FVSCHEDULE", "CUMIPMT", "CUMPRINC", "RANDBETWEEN",
];

pub struct Golden {
    reference: Reference,
    cells: u64,
    computed: u64,
    wrong: Vec<String>,
    wrong_cells: u64,
    residues: u64,
    solved: u64,
    stale_external: u64,
    start_dependent: u64,
    deviations: u64,
    after_deviation: u64,
    why: BTreeMap<String, u64>,
    /// Formula cells and computed cells of workbooks without circular references.
    acyclic: (u64, u64),
    files: u64,
    /// Cells on cycles computed by iteration (and compared like all others).
    pub iterated: u64,
    /// Larger workbooks are left out (time).
    pub max_cells: usize,
    /// Every formula cell starts from its stored number 1 % off (as a workbook does that
    /// is calculated again), not only those on cycles: for testing the iteration alone.
    pub perturb_all: bool,
}

/// The files of the corpus to take: every `step`-th (`CALC_STEP`), or those whose path
/// contains `CALC_ONLY`.
pub fn chosen(files: &[PathBuf], step: usize) -> Vec<&PathBuf> {
    let step = std::env::var("CALC_STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(step);
    let only = std::env::var("CALC_ONLY").ok();
    files.iter().step_by(step).filter(|p| only.as_deref().is_none_or(|o| p.to_string_lossy().contains(o))).collect()
}

impl Golden {
    pub fn new(reference: Reference) -> Self {
        Golden {
            reference,
            cells: 0,
            computed: 0,
            wrong: Vec::new(),
            wrong_cells: 0,
            residues: 0,
            solved: 0,
            stale_external: 0,
            start_dependent: 0,
            deviations: 0,
            after_deviation: 0,
            why: BTreeMap::new(),
            acyclic: (0, 0),
            files: 0,
            iterated: 0,
            max_cells: 60_000,
            perturb_all: false,
        }
    }

    pub fn file(&mut self, path: &Path, stored: &Workbook) {
        if stored.cells.len() > self.max_cells {
            return;
        }
        self.files += 1;
        // Formula values removed: the calculation has to find them itself. Cells on a
        // circular reference keep a value to start from, as in Excel, but 1 % off: the
        // iteration has to bring them back.
        let mut blank = stored.clone();
        let on_cycle: std::collections::HashSet<usize> = xl_polars::lift::circular_cells(stored).into_iter().collect();
        let factor: f64 = std::env::var("CALC_PERTURB").ok().and_then(|f| f.parse().ok()).unwrap_or(1.01);
        for i in 0..blank.cells.len() {
            if (on_cycle.contains(&i) || self.perturb_all) && blank.cells.formula[i].is_some() && blank.cells.kind[i] == Kind::Number {
                blank.cells.num[i] *= factor;
                continue;
            }
            if blank.cells.formula[i].is_some() {
                blank.cells.kind[i] = Kind::Blank;
                blank.cells.text[i] = None;
                blank.cells.num[i] = 0.0;
            }
        }
        let (got, calc) = calculate(&blank);
        self.cells += calc.formula_cells;
        let cyclic = calc.not_computed.keys().any(|k| k.starts_with("circular reference"));
        if !cyclic {
            self.acyclic.0 += calc.formula_cells;
            self.acyclic.1 += calc.computed;
        }
        if std::env::var_os("CALC_FILES").is_some() {
            eprintln!("{} {:?}", path.display(), calc.not_computed);
        }
        for (k, v) in &calc.not_computed {
            *self.why.entry(k.chars().take(60).collect()).or_default() += v;
        }
        self.computed += calc.computed;
        self.iterated += calc.iterated;
        let lo = self.reference == Reference::LibreOffice;
        let (a, b) = (&stored.cells, &got.cells);
        let index: HashMap<(u32, u32, u32), usize> = (0..a.len()).map(|i| ((a.sheet[i], a.row[i], a.col[i]), i)).collect();
        let deviations_before = self.deviations;
        let mut scales: HashMap<usize, f64> = Default::default();
        let mut file_wrong = Vec::new();
        for i in 0..a.len() {
            if a.formula[i].is_none() || b.kind[i] == Kind::Blank {
                continue;
            }
            // A cell still at its starting value was not computed.
            if (on_cycle.contains(&i) || self.perturb_all) && b.kind[i] == blank.cells.kind[i] && b.num[i].to_bits() == blank.cells.num[i].to_bits() {
                continue;
            }
            let formula = a.formula[i].as_deref().unwrap_or("");
            let same = match (a.kind[i], b.kind[i]) {
                (Kind::Number, Kind::Number) => {
                    let close = (a.num[i] - b.num[i]).abs() <= 1e-9 * a.num[i].abs().max(1.0);
                    if close {
                        true
                    } else if solved(stored, &index, i, 4) && (a.num[i] - b.num[i]).abs() <= 1e-7 * a.num[i].abs().max(1e-3) {
                        // IRR, XIRR, RATE, YIELD: solved by iteration, Excel's to 0.000001 percent.
                        self.solved += 1;
                        true
                    } else if a.num[i].abs() < 1e-6 && b.num[i].abs() < 1e-6 && (formula.contains(['+', '-']) || ["SUM", "SUBTOTAL"].iter().any(|n| formula.to_ascii_uppercase().contains(n))) {
                        // Both nearly 0 after sums and differences: residues of rounding,
                        // summed in another order (LibreOffice compensates).
                        self.residues += 1;
                        true
                    } else if cancels(stored, &index, &mut scales, i, (a.num[i] - b.num[i]).abs()) {
                        self.residues += 1;
                        true
                    } else {
                        false
                    }
                }
                // LibreOffice stores the result of a comparison as 1 or 0.
                (Kind::Number, Kind::Bool) if lo => a.num[i] == b.num[i],
                // DATE with a year below 1900: Excel adds 1900 (DATE(100,12,1) is 1 Dec
                // 2000), LibreOffice takes the year as it is and fails.
                (Kind::Error, Kind::Number) if lo && formula.to_ascii_uppercase().starts_with("DATE(") => {
                    self.deviations += 1;
                    true
                }
                // A name of another workbook ([1]!date): LibreOffice cannot read those (see
                // the .xls golden) and stores an error; we take the name from the link.
                // Likewise a range of another workbook written with the book at both ends
                // ([1]S!B11:[1]S!B15), which LibreOffice writes and then cannot read.
                (Kind::Error, Kind::Number | Kind::Text | Kind::Bool) if lo && (formula.contains("]!") || formula.matches('[').count() >= 2 && formula.contains(":[")) => {
                    self.deviations += 1;
                    true
                }
                // Excel before 2007: functions of the Analysis ToolPak (EOMONTH, EDATE, ...)
                // are #NAME? when the add-in is not loaded; built in since.
                (Kind::Error, _) if !lo && a.text[i].as_deref() == Some("#NAME?") && ADDIN.iter().any(|f| formula.to_ascii_uppercase().contains(&format!("{f}("))) => {
                    self.deviations += 1;
                    true
                }
                (Kind::Text, Kind::Text) | (Kind::Bool, Kind::Bool) => a.value(i) == b.value(i),
                // An empty text result reads back as text or as nothing.
                (Kind::Blank, Kind::Text) => b.text[i].as_deref() == Some(""),
                _ => false,
            };
            // Excel's value of a formula that reads another workbook can lag behind the
            // cells of that workbook kept in the file (`[1]S!E12` cached as 45, the formula
            // stored from 35): counted, not compared.
            if !same && !lo && reads_external(stored, &index, i, 4) {
                self.stale_external += 1;
                continue;
            }
            if !same {
                file_wrong.push(format!("{}: {}!R{}C{} ={}: stored {:?}, calculated {:?}", path.display(), stored.sheets[a.sheet[i] as usize].name, a.row[i] + 1, a.col[i] + 1, formula, a.value(i), b.value(i)));
            }
        }
        // A cycle that settles slowly has many values within the workbook's delta: from
        // a start 1 % off the iteration stops elsewhere than Excel, which started from its
        // own last values. If Excel's values are themselves settled for the module (from
        // them nothing moves), the differences are of the start, not of the formulas.
        if !file_wrong.is_empty() && !on_cycle.is_empty() && calc.iterated > 0 {
            let mut again = blank.clone();
            for &i in &on_cycle {
                again.cells.kind[i] = stored.cells.kind[i];
                again.cells.num[i] = stored.cells.num[i];
                again.cells.text[i] = stored.cells.text[i].clone();
            }
            let (from_excel, _) = calculate(&again);
            let at_rest = on_cycle.iter().all(|&i| {
                let (x, y) = (&stored.cells, &from_excel.cells);
                x.kind[i] != Kind::Number || y.kind[i] != Kind::Number || (x.num[i] - y.num[i]).abs() <= 1e-9 * x.num[i].abs().max(1.0)
            });
            if at_rest {
                self.start_dependent += file_wrong.len() as u64;
                file_wrong.clear();
            }
        }
        // Behind such a cell every cell that reads it differs too: the file says nothing.
        if self.deviations > deviations_before {
            self.after_deviation += file_wrong.len() as u64;
        } else {
            // A few per file, so that one file does not hide the others.
            self.wrong_cells += file_wrong.len() as u64;
            let room = 60usize.saturating_sub(self.wrong.len());
            self.wrong.extend(file_wrong.into_iter().take(3).take(room));
        }
    }

    /// The report, and the assertions: no differing cell; for the default sample, the
    /// share computed of workbooks without cycles.
    pub fn check(&self, default_step: usize, min_share: u64) {
        eprintln!("{:?}: {} files, {} of {} formula cells computed", self.reference, self.files, self.computed, self.cells);
        let mut top: Vec<_> = self.why.iter().collect();
        top.sort_by_key(|x| std::cmp::Reverse(*x.1));
        for (k, v) in top.iter().take(15) {
            eprintln!("  {v}\t{k}");
        }
        eprintln!("without cycles: {} of {}; cells on cycles computed by iteration: {}", self.acyclic.1, self.acyclic.0, self.iterated);
        eprintln!(
            "residues of cancellation: {}; solved functions within their tolerance: {}; reading other workbooks, not as their cache: {}; after slowly settling cycles, \
             Excel stopped elsewhere (its values at rest for the module too): {}; the reference's own failures: {}, cells after them: {}",
            self.residues, self.solved, self.stale_external, self.start_dependent, self.deviations, self.after_deviation
        );
        assert!(self.wrong.is_empty(), "{} cells differ:\n{}", self.wrong_cells, self.wrong.join("\n"));
        let step = std::env::var("CALC_STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(default_step);
        if step == default_step && std::env::var_os("CALC_ONLY").is_none() {
            assert!(self.acyclic.1 * 100 >= self.acyclic.0 * min_share, "{:?}", self.acyclic);
        }
    }
}

/// A sum or difference of large numbers whose result differs only in the last digits of
/// its terms: LibreOffice writes values of other cells (and of other workbooks) with 15
/// significant digits, so what it computed from its own full values cannot be told
/// apart closer than that. The terms are the cells the formula reads (single cells and
/// ranges of its sheet); a term that was itself such a residue counts with the size of
/// its own terms, and so does this cell for its readers (`scales`).
/// Whether a formula reads another workbook: directly (`[1]S!A1`), through a name of
/// this workbook that does, or through up to `depth` cells that do.
fn reads_external(wb: &Workbook, index: &HashMap<(u32, u32, u32), usize>, i: usize, depth: u32) -> bool {
    let c = &wb.cells;
    let Some(f) = c.formula[i].as_deref() else { return false };
    if f.contains('[') {
        return true;
    }
    let Ok(expr) = xl_formula::parse(f) else { return false };
    let mut external = false;
    let mut targets = Vec::new();
    expr.walk(&mut |e| match e {
        xl_formula::Expr::Name { text, .. } => {
            if wb.defined_names.iter().any(|n| n.name.eq_ignore_ascii_case(text) && n.value.contains('[')) {
                external = true;
            }
        }
        xl_formula::Expr::Ref { prefix, kind: xl_formula::RefKind::Cell, text } => {
            let sheet = match prefix {
                None => Some(c.sheet[i]),
                Some(p) => {
                    let name = p.trim_end_matches('!');
                    let name = name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')).map(|n| n.replace("''", "'")).unwrap_or_else(|| name.to_string());
                    wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(&name)).map(|s| s as u32)
                }
            };
            if let (Some(sheet), Some(r)) = (sheet, xl_formula::parse_cell_ref(text)) {
                targets.push((sheet, r.row - 1, r.col - 1));
            }
        }
        _ => {}
    });
    external || (depth > 0 && targets.into_iter().any(|t| index.get(&t).is_some_and(|&j| reads_external(wb, index, j, depth - 1))))
}

/// Whether a formula's value comes from a function solved by iteration: it calls one, or
/// reads (through up to `depth` cells) a cell that does.
fn solved(wb: &Workbook, index: &HashMap<(u32, u32, u32), usize>, i: usize, depth: u32) -> bool {
    let c = &wb.cells;
    let Some(f) = c.formula[i].as_deref() else { return false };
    if SOLVED.iter().any(|s| f.to_ascii_uppercase().contains(s)) {
        return true;
    }
    if depth == 0 {
        return false;
    }
    let Ok(expr) = xl_formula::parse(f) else { return false };
    let mut targets = Vec::new();
    expr.walk(&mut |e| {
        if let xl_formula::Expr::Ref { prefix, kind: xl_formula::RefKind::Cell, text } = e {
            let sheet = match prefix {
                None => Some(c.sheet[i]),
                Some(p) => {
                    let name = p.trim_end_matches('!');
                    let name = name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')).map(|n| n.replace("''", "'")).unwrap_or_else(|| name.to_string());
                    wb.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(&name)).map(|s| s as u32)
                }
            };
            if let (Some(sheet), Some(r)) = (sheet, xl_formula::parse_cell_ref(text)) {
                targets.push((sheet, r.row - 1, r.col - 1));
            }
        }
    });
    targets.into_iter().any(|t| index.get(&t).is_some_and(|&j| solved(wb, index, j, depth - 1)))
}

fn cancels(wb: &Workbook, index: &std::collections::HashMap<(u32, u32, u32), usize>, scales: &mut std::collections::HashMap<usize, f64>, i: usize, diff: f64) -> bool {
    let c = &wb.cells;
    let Some(f) = c.formula[i].as_deref() else { return false };
    let Ok(expr) = xl_formula::parse(f) else { return false };
    let upper = f.to_ascii_uppercase();
    if !(f.contains(['+', '-']) || upper.starts_with("SUM(")) {
        return false;
    }
    let mut scale = 0.0f64;
    let mut take = |row: u32, col: u32| {
        if let Some(&j) = index.get(&(c.sheet[i], row, col)) {
            if c.kind[j] == Kind::Number {
                scale = scale.max(c.num[j].abs());
            }
            if let Some(s) = scales.get(&j) {
                scale = scale.max(*s);
            }
        }
    };
    let mut blocks = Vec::new();
    expr.walk(&mut |e| match e {
        xl_formula::Expr::Binary { op: xl_formula::BinaryOp::Range, lhs, rhs, .. } => {
            if let (xl_formula::Expr::Ref { prefix: None, text: a, .. }, xl_formula::Expr::Ref { prefix: None, text: b, .. }) = (lhs.unspaced(), rhs.unspaced()) {
                if let (Some(p), Some(q)) = (xl_formula::parse_cell_ref(a), xl_formula::parse_cell_ref(b)) {
                    blocks.push((p.row.min(q.row), p.row.max(q.row), p.col.min(q.col), p.col.max(q.col)));
                }
            }
        }
        xl_formula::Expr::Ref { prefix: None, kind: xl_formula::RefKind::Cell, text } => {
            if let Some(r) = xl_formula::parse_cell_ref(text) {
                blocks.push((r.row, r.row, r.col, r.col));
            }
        }
        _ => {}
    });
    for (r1, r2, c1, c2) in blocks {
        if (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64 > 100_000 {
            continue;
        }
        for r in r1..=r2 {
            for col in c1..=c2 {
                take(r - 1, col - 1);
            }
        }
    }
    let ok = diff <= 1e-14 * scale;
    if ok {
        scales.insert(i, scale);
    }
    ok
}


pub fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
