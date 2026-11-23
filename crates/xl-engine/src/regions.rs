//! What a workbook holds, found for the one who is to move it elsewhere: tables (columns
//! under a header row), single cells (inputs and results of a model, named by their
//! labels) and blocks (grids of values without a header).
//!
//! * Excel's own tables (list objects) are taken as they are.
//! * The other cells of a sheet are cut into blocks along empty rows and columns. A block
//!   whose first row is labels (some columns may be unnamed) and whose columns are each either values or one formula
//!   filled down is a table; a grid of plain values without labels is a block; anything
//!   else is a group of single cells, each named by the defined name that points at it,
//!   else by the label left of it, else by the label above it.
//! * A column may have exceptions: rows that are not as the column is. A column is one
//!   formula filled down where that formula is in at least two rows and in two thirds of
//!   them; the other rows (another formula, or a plain value) are its exceptions. A column
//!   is values where two thirds of its rows are; the rows with a formula are its
//!   exceptions. In a table of Excel's own a column that is neither is values, with every
//!   formula an exception.
//!
//! The logic of a calculated column or cell is given twice: as the formula of the file,
//! and as that formula with its references written as Excel writes structured
//! references (`[@qty]*[@price]`, `SUM(Sales[amount])`, `Rate`), wherever a reference
//! is a column of the same row, a whole column of a table, or a named cell.

use crate::frame::{self, Area};
use crate::data::{Column, Frame};
use std::collections::{HashMap, HashSet};
use xl_cells::{Kind, Workbook};
use xl_formula::{BinaryOp, Expr, RefKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Columns under a header row.
    Table,
    /// Single cells with names.
    Cells,
    /// A grid of values without a header.
    Block,
}

/// What is known of a region of a workbook before its regions are found: its name, its
/// kind and where it lies (see [`regions_like`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub name: String,
    pub kind: RegionKind,
    pub area: Area,
}

impl From<&Region> for Known {
    fn from(r: &Region) -> Known {
        Known { name: r.name.clone(), kind: r.kind, area: r.area }
    }
}

impl RegionKind {
    pub fn of(text: &str) -> Option<RegionKind> {
        [RegionKind::Table, RegionKind::Cells, RegionKind::Block].into_iter().find(|k| k.as_str() == text)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RegionKind::Table => "table",
            RegionKind::Cells => "cells",
            RegionKind::Block => "block",
        }
    }
}

/// A column of a table or block.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionColumn {
    pub name: String,
    /// 0-based column of the sheet.
    pub col: u32,
    /// The Polars data type of the column (`Int64`, `String`, `Date`, …).
    pub dtype: String,
    /// The formula of the first data row, when one formula is filled down the column (as
    /// it would be there, where the first row is an exception).
    pub formula: Option<String>,
    /// That formula with structured references.
    pub logic: Option<String>,
    /// The rows that are not as the column is, top to bottom.
    pub exceptions: Vec<Exception>,
}

/// A row of a column that is not as the column is: a formula of its own, or (in a
/// calculated column) a plain value.
#[derive(Debug, Clone, PartialEq)]
pub struct Exception {
    /// 0-based row of the sheet.
    pub row: u32,
    /// `None`: a plain value (or nothing).
    pub formula: Option<String>,
    pub logic: Option<String>,
    /// What the row's own formula gives where the column is text of mixed kinds and the
    /// row is not: `Date`, `Datetime`, `Time` or `Duration`.
    pub dtype: Option<String>,
}

impl RegionColumn {
    /// The column, or some of its rows, are calculated.
    pub fn is_calculated(&self) -> bool {
        self.formula.is_some() || self.exceptions.iter().any(|e| e.formula.is_some())
    }

    /// The exception at a row of the sheet.
    pub fn exception(&self, row: u32) -> Option<&Exception> {
        self.exceptions.binary_search_by_key(&row, |e| e.row).ok().map(|k| &self.exceptions[k])
    }

    /// The formula of a row of the sheet as the region has it: that of the column moved
    /// to the row, or the row's own.
    pub fn formula_at(&self, row: u32, first: u32) -> Option<String> {
        match self.exception(row) {
            Some(e) => e.formula.clone(),
            None => self.formula.as_deref().map(|f| xl_formula::parse(f).map(|e| e.shifted(row as i64 - first as i64, 0).to_string()).unwrap_or_else(|_| f.to_string())),
        }
    }
}

/// A single cell with a name.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedCell {
    pub name: String,
    pub row: u32,
    pub col: u32,
    pub dtype: String,
    pub formula: Option<String>,
    pub logic: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// Unique in the workbook.
    pub name: String,
    pub kind: RegionKind,
    /// The whole region, the header row included.
    pub area: Area,
    pub has_header: bool,
    /// The rows of the header: 0, 1, or 2 where a second row of labels lies under the
    /// first (a group above, the column's own label below).
    pub header_rows: u32,
    /// `excel table` or `detected`.
    pub source: &'static str,
    /// Of a table or block.
    pub columns: Vec<RegionColumn>,
    /// Of a group of single cells.
    pub cells: Vec<NamedCell>,
}

impl Region {
    /// First and last data row (0-based) of a table or block; `None` without data rows.
    pub fn data_rows(&self) -> Option<(u32, u32)> {
        let first = self.area.first_row + self.header_rows;
        (first <= self.area.last_row).then_some((first, self.area.last_row))
    }

    /// The range in A1 notation (`A1:F40`).
    pub fn range(&self) -> String {
        let a = self.area;
        format!("{}{}:{}{}", xl_formula::col_name(a.first_col + 1), a.first_row + 1, xl_formula::col_name(a.last_col + 1), a.last_row + 1)
    }
}

fn attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let head = &xml[..xml.find('>')?];
    let at = head.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(&head[at..at + head[at..].find('"')?])
}

fn parse_range(text: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = text.split_once(':').unwrap_or((text, text));
    let (a, b) = (xl_formula::parse_cell_ref(a)?, xl_formula::parse_cell_ref(b)?);
    Some((a.row - 1, a.col - 1, b.row - 1, b.col - 1))
}

/// The cells of a workbook by place.
type Index = HashMap<(u32, u32, u32), usize>;

struct Finder<'a> {
    wb: &'a Workbook,
    at: Index,
}

impl Finder<'_> {
    fn text_at(&self, s: u32, r: u32, c: u32) -> Option<&str> {
        let i = *self.at.get(&(s, r, c))?;
        let cells = &self.wb.cells;
        (cells.kind[i] == Kind::Text && cells.formula[i].is_none()).then(|| cells.text[i].as_deref().unwrap_or("")).filter(|t| !t.trim().is_empty())
    }

    fn formula_at(&self, s: u32, r: u32, c: u32) -> Option<&str> {
        self.at.get(&(s, r, c)).and_then(|&i| self.wb.cells.formula[i].as_deref()).filter(|f| !f.is_empty())
    }

    /// The cell is a value of a model: a formula, or something that is not a label.
    fn is_value(&self, s: u32, r: u32, c: u32) -> bool {
        self.formula_at(s, r, c).is_some() || self.at.get(&(s, r, c)).is_some_and(|&i| matches!(self.wb.cells.kind[i], Kind::Number | Kind::Bool | Kind::Error))
    }

    /// What one column holds over the rows: one formula filled down (given as that of the
    /// first row), or values, and the rows that are not so (see the module). `Err` for a
    /// column that is neither. `any`: a column that is neither is values, every formula
    /// an exception.
    fn filled(&self, s: u32, c: u32, rows: (u32, u32), any: bool) -> Result<(Option<String>, Vec<Exception>), ()> {
        let form = |f: &str, r: u32| xl_formula::parse(f).map(|e| e.relative_form(r + 1, c + 1)).unwrap_or_else(|_| f.to_string());
        let forms: Vec<Option<String>> = (rows.0..=rows.1).map(|r| self.formula_at(s, r, c).map(|f| form(f, r))).collect();
        let n = forms.len();
        if forms.iter().all(|f| *f == forms[0]) {
            return Ok((self.formula_at(s, rows.0, c).map(str::to_string), Vec::new()));
        }
        let exception = |r: u32| Exception { row: r, formula: self.formula_at(s, r, c).map(str::to_string), logic: None, dtype: None };
        // The formula most rows have (the upper one of two as frequent).
        let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
        for (k, f) in forms.iter().enumerate() {
            if let Some(f) = f {
                counts.entry(f).or_insert((0, k)).0 += 1;
            }
        }
        let values = forms.iter().filter(|f| f.is_none()).count();
        let best = counts.iter().map(|(f, &(count, at))| (count, std::cmp::Reverse(at), *f)).max();
        if let Some((count, std::cmp::Reverse(at), main)) = best {
            if count >= 2 && count * 3 >= n * 2 {
                // As it would be in the first row; a reference that leaves the sheet there
                // cannot be written.
                let text = self.formula_at(s, rows.0 + at as u32, c).unwrap_or("");
                let first = if at == 0 { text.to_string() } else { xl_formula::parse(text).map_err(|_| ())?.shifted(-(at as i64), 0).to_string() };
                if at > 0 && (first.contains("#REF!") || form(&first, rows.0) != main) {
                    return Err(());
                }
                let exceptions = (rows.0..=rows.1).zip(&forms).filter(|(_, f)| f.as_deref() != Some(main)).map(|(r, _)| exception(r)).collect();
                return Ok((Some(first), exceptions));
            }
        }
        if any || values * 3 >= n * 2 {
            return Ok((None, (rows.0..=rows.1).zip(&forms).filter(|(_, f)| f.is_some()).map(|(r, _)| exception(r)).collect()));
        }
        Err(())
    }
}

/// Cut cells into blocks along empty rows and columns, in turn, until nothing splits.
fn cut(cells: Vec<(u32, u32)>, out: &mut Vec<Vec<(u32, u32)>>) {
    fn split(cells: &[(u32, u32)], key: fn(&(u32, u32)) -> u32) -> Vec<Vec<(u32, u32)>> {
        let mut lines: Vec<u32> = cells.iter().map(key).collect();
        lines.sort_unstable();
        lines.dedup();
        let mut groups: Vec<(u32, u32)> = Vec::new();
        for l in lines {
            match groups.last_mut() {
                Some(g) if g.1 + 1 == l => g.1 = l,
                _ => groups.push((l, l)),
            }
        }
        groups.iter().map(|g| cells.iter().filter(|x| (g.0..=g.1).contains(&key(x))).copied().collect()).collect()
    }
    let by_rows = split(&cells, |x| x.0);
    if by_rows.len() > 1 {
        return by_rows.into_iter().for_each(|b| cut(b, out));
    }
    let by_cols = split(&cells, |x| x.1);
    if by_cols.len() > 1 {
        return by_cols.into_iter().for_each(|b| cut(b, out));
    }
    if !cells.is_empty() {
        out.push(cells);
    }
}

/// Names in the order asked for, each unique: a name taken gets `_<n>` appended.
struct Names(HashSet<String>);

impl Names {
    fn take(&mut self, want: &str) -> String {
        let base = if want.trim().is_empty() { "region" } else { want.trim() };
        let mut name = base.to_string();
        let mut n = 1;
        while !self.0.insert(name.to_lowercase()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

/// The regions of a workbook, sheet by sheet, top to bottom.
pub fn regions(wb: &Workbook) -> Vec<Region> {
    regions_like(wb, &[])
}

/// The area a sheet's name and a range (`A2:D6`) name.
pub fn area_of(wb: &Workbook, sheet: &str, range: &str) -> Option<Area> {
    let s = wb.sheets.iter().position(|x| x.name == sheet)? as u32;
    let (first_row, first_col, last_row, last_col) = parse_range(range)?;
    Some(Area { sheet: s, first_row, first_col, last_row, last_col })
}

/// The regions of a workbook of which some are known (see [`regions`]), so that they
/// stay what they are when their rows change. A region found where a known one of its
/// kind begins (sheet, first row, first column) has its name; without that, names follow
/// the order in which regions are found, and would pass to others when one moves. A
/// table found where a known one begins, with its columns, and ending where that ends or
/// a row below (a row of totals), is that table: also with a single row of data, which
/// alone would read as single cells under their labels, with a last row that is not as
/// the others, and with columns in which many rows are not as the column is.
pub fn regions_like(wb: &Workbook, known: &[Known]) -> Vec<Region> {
    let tables: Vec<Area> = known.iter().filter(|k| k.kind == RegionKind::Table).map(|k| k.area).collect();
    let c = &wb.cells;
    let f = Finder { wb, at: (0..c.len()).map(|i| ((c.sheet[i], c.row[i], c.col[i]), i)).collect() };
    // The names of the known are theirs: a region found anew gets another.
    let mut names = Names(known.iter().map(|k| k.name.to_lowercase()).collect());
    let mut given: HashSet<usize> = HashSet::new();
    let mut name_of = |names: &mut Names, kind: RegionKind, area: Area, want: &str| {
        let same = |k: &Known| k.kind == kind && (k.area.sheet, k.area.first_row, k.area.first_col) == (area.sheet, area.first_row, area.first_col);
        match (0..known.len()).find(|&i| same(&known[i]) && !given.contains(&i)) {
            Some(i) => {
                given.insert(i);
                known[i].name.clone()
            }
            None => names.take(want),
        }
    };
    let mut out: Vec<Region> = Vec::new();
    // A defined name that points at one cell names that cell.
    let mut defined: HashMap<(u32, u32, u32), String> = HashMap::new();
    for d in wb.defined_names.iter().filter(|d| !d.hidden) {
        let Some((sheet, cell)) = d.value.trim_start_matches('=').rsplit_once('!') else { continue };
        let sheet = sheet.trim_matches('\'').replace("''", "'");
        let (Some(s), Some(r)) = (wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(&sheet)), xl_formula::parse_cell_ref(cell)) else { continue };
        defined.entry((s as u32, r.row - 1, r.col - 1)).or_insert_with(|| d.name.clone());
    }
    for (s, sheet) in wb.sheets.iter().enumerate() {
        let s = s as u32;
        let mut taken: Vec<(u32, u32, u32, u32)> = Vec::new();
        // Excel's own tables.
        for xml in &sheet.tables {
            let Some((r0, c0, r1, c1)) = attr(xml, "ref").and_then(parse_range) else { continue };
            let header = attr(xml, "headerRowCount").is_none_or(|v| v != "0");
            let totals: u32 = attr(xml, "totalsRowCount").and_then(|v| v.parse().ok()).unwrap_or(0);
            taken.push((r0, c0, r1, c1));
            let area = Area { sheet: s, first_row: r0, first_col: c0, last_row: r1.saturating_sub(totals).max(r0), last_col: c1 };
            let name = name_of(&mut names, RegionKind::Table, area, attr(xml, "displayName").or(attr(xml, "name")).unwrap_or(&sheet.name));
            out.push(table_region(&f, name, area, header as u32, "excel table"));
        }
        // The rest, cut into blocks.
        let free: Vec<(u32, u32)> = (0..c.len())
            .filter(|&i| c.sheet[i] == s && (c.formula[i].as_deref().is_some_and(|x| !x.is_empty()) || matches!(c.kind[i], Kind::Number | Kind::Bool | Kind::Error) || f.text_at(s, c.row[i], c.col[i]).is_some()))
            .map(|i| (c.row[i], c.col[i]))
            .filter(|&(r, col)| !taken.iter().any(|t| (t.0..=t.2).contains(&r) && (t.1..=t.3).contains(&col)))
            .collect();
        let mut blocks = Vec::new();
        cut(free, &mut blocks);
        blocks.sort_by_key(|b| (b.iter().map(|x| x.0).min(), b.iter().map(|x| x.1).min()));
        let mut found: Vec<Region> = Vec::new();
        for block in blocks {
            let (r0, r1) = (block.iter().map(|x| x.0).min().unwrap(), block.iter().map(|x| x.0).max().unwrap());
            let (c0, c1) = (block.iter().map(|x| x.1).min().unwrap(), block.iter().map(|x| x.1).max().unwrap());
            // Rows of a title above: a single text cell in a block wider than it.
            let mut top = r0;
            let mut title: Option<&str> = None;
            let width = (c1 - c0 + 1) as usize;
            let named_in = |r: u32| (c0..=c1).filter(|&col| f.text_at(s, r, col).is_some()).count();
            // A row of labels and nothing else; some columns may be unnamed, as long as at
            // least two and at least half of them have a name.
            let labels = |r: u32| (c0..=c1).all(|col| !f.is_value(s, r, col)) && (named_in(r) == width || (named_in(r) >= 2 && named_in(r) * 2 >= width));
            // A single label that stands over some of the columns of a header row below it
            // (merged over them, or not at the left) is a group of the header, not a title.
            let group = |r: u32, col: u32| {
                let merged = sheet.merges.iter().any(|m| (m.0..=m.2).contains(&r) && m.1 <= col && col <= m.3 && m.3 > m.1 && ((m.3 - m.1 + 1) as usize) < width);
                r < r1 && labels(r + 1) && f.text_at(s, r + 1, col).is_some() && (merged || col > c0)
            };
            while top < r1 && c1 > c0 {
                let row: Vec<u32> = (c0..=c1).filter(|&col| f.at.contains_key(&(s, top, col)) && block.contains(&(top, col))).collect();
                match row.as_slice() {
                    [only] if f.text_at(s, top, *only).is_some() && !group(top, *only) => {
                        title = title.or(f.text_at(s, top, *only));
                        top += 1;
                    }
                    _ => break,
                }
            }
            let area = Area { sheet: s, first_row: top, first_col: c0, last_row: r1, last_col: c1 };
            // Groups above a header row: labels only, fewer than a header row has.
            let groups = !labels(top) && r1 >= top + 3 && named_in(top) >= 1 && (c0..=c1).all(|col| !f.is_value(s, top, col)) && labels(top + 1);
            let header = labels(top) || groups;
            // A second header row: labels only, and under at least two of them the column
            // holds values and no text (so that it is not the first row of the data).
            let labels_below = (c0..=c1).filter(|&col| f.text_at(s, top + 1, col).is_some()).count();
            let over_values = (c0..=c1)
                .filter(|&col| f.text_at(s, top + 1, col).is_some() && r1 >= top + 3 && (top + 2..=r1).any(|r| f.is_value(s, r, col)) && (top + 2..r1).all(|r| f.text_at(s, r, col).is_none()))
                .count();
            let two = groups || (header && r1 >= top + 3 && (c0..=c1).all(|col| !f.is_value(s, top + 1, col)) && labels_below * 2 >= width && over_values >= 2);
            let h = 1 + two as u32;
            // (`any`: of a table known to be here, a column is what most of its rows are,
            // however many are not.)
            let fit = |last: u32, any: bool| -> Option<Vec<Vec<Exception>>> { (c0..=c1).map(|col| f.filled(s, col, (top + h, last), any).ok().map(|x| x.1)).collect() };
            let any_value = |rows: (u32, u32)| (rows.0..=rows.1).any(|r| (c0..=c1).any(|col| f.is_value(s, r, col)));
            let wanted = title.unwrap_or(&sheet.name);
            // A table known to begin here ends where it is known to: with the block, or a
            // row above its end (a row of totals below it).
            let known = tables.iter().find(|a| header && (a.sheet, a.first_row, a.first_col, a.last_col) == (s, top, c0, c1) && a.last_row >= top + h).map(|a| a.last_row);
            let (full, short) = match known {
                Some(last) if last == r1 => (fit(r1, true), None),
                Some(last) if last + 1 == r1 => (None, fit(last, true)),
                _ => (if header && r1 >= top + h + 1 { fit(r1, false) } else { None }, if header && r1 >= top + h + 2 { fit(r1 - 1, false) } else { None }),
            };
            // The last row is one of its own (totals) where every value of it would be an
            // exception of its column.
            let last_apart = |columns: &Vec<Vec<Exception>>| {
                let mut values = (c0..=c1).zip(columns).filter(|(col, _)| f.is_value(s, r1, *col)).map(|(_, e)| e.last().is_some_and(|e| e.row == r1)).peekable();
                values.peek().is_some() && values.all(|x| x)
            };
            if full.as_ref().is_some_and(|columns| short.is_none() || !last_apart(columns)) {
                found.push(table_region(&f, wanted.to_string(), area, h, "detected"));
            } else if short.is_some() {
                // A last row of its own (totals) below a table.
                let table = table_region(&f, wanted.to_string(), Area { last_row: r1 - 1, ..area }, h, "detected");
                let heads: HashMap<u32, String> = table.columns.iter().map(|c| (c.col, c.name.clone())).collect();
                found.push(table);
                if any_value((r1, r1)) {
                    found.push(cells_region(&f, &defined, format!("{wanted} totals"), Area { first_row: r1, ..area }, Some(&heads)));
                }
            } else if r1 > r0 && c1 > c0 && block.len() == ((r1 - r0 + 1) * (c1 - c0 + 1)) as usize && block.iter().all(|&(r, col)| f.is_value(s, r, col) && f.formula_at(s, r, col).is_none()) {
                let mut region = table_region(&f, wanted.to_string(), Area { first_row: r0, ..area }, 0, "detected");
                region.kind = RegionKind::Block;
                found.push(region);
            } else if any_value((r0, r1)) {
                found.push(cells_region(&f, &defined, wanted.to_string(), Area { first_row: r0, ..area }, None));
            }
        }
        // Names: the title, else the sheet's name; several of a sheet are numbered by `take`.
        for mut region in found {
            region.name = name_of(&mut names, region.kind, region.area, &region.name);
            out.push(region);
        }
    }
    let lookup = Lookup::new(&out, wb);
    for k in 0..out.len() {
        let region = out[k].clone();
        if let Some((first, _)) = region.data_rows() {
            for col in out[k].columns.iter_mut() {
                col.logic = col.formula.as_deref().and_then(|x| structured(x, &lookup, &region, first, wb));
                for e in col.exceptions.iter_mut() {
                    e.logic = e.formula.as_deref().and_then(|x| structured(x, &lookup, &region, e.row, wb));
                }
            }
        }
        for cell in out[k].cells.iter_mut() {
            cell.logic = cell.formula.as_deref().and_then(|x| structured(x, &lookup, &region, cell.row, wb));
        }
    }
    out
}

/// `Date`, `Datetime`, `Time` or `Duration` where a cell on its own is one of them.
pub fn moment_dtype(wb: &Workbook, at: &frame::CellIndex, sheet: u32, row: u32, col: u32) -> Option<String> {
    let own = frame::frame_in(wb, at, Area { sheet, first_row: row, first_col: col, last_row: row, last_col: col }, false).ok()?;
    own.columns.first().map(|x| x.data.type_name().to_string()).filter(|d| matches!(d.as_str(), "Date" | "Datetime" | "Time" | "Duration"))
}

fn table_region(f: &Finder, name: String, area: Area, header_rows: u32, source: &'static str) -> Region {
    let typed = frame::frame_headed(f.wb, &f.at, area, header_rows).unwrap_or_else(|_| Frame::empty());
    let rows = (area.first_row + header_rows, area.last_row);
    let columns = (area.first_col..=area.last_col)
        .zip(typed.columns.iter())
        .map(|(col, series)| {
            let (formula, mut exceptions) = if rows.0 <= rows.1 { f.filled(area.sheet, col, rows, true).unwrap_or_default() } else { Default::default() };
            if series.data.type_name() == "String" {
                for e in exceptions.iter_mut().filter(|e| e.formula.is_some()) {
                    e.dtype = moment_dtype(f.wb, &f.at, area.sheet, e.row, col);
                }
            }
            RegionColumn { name: series.name.clone(), col, dtype: series.data.type_name().to_string(), formula, logic: None, exceptions }
        })
        .collect();
    Region { name, kind: RegionKind::Table, area, has_header: header_rows > 0, header_rows, source, columns, cells: Vec::new() }
}

/// The values of an area as single cells; `header_row`: a row of labels above the area
/// (the header of the table a totals row belongs to).
fn cells_region(f: &Finder, defined: &HashMap<(u32, u32, u32), String>, name: String, area: Area, heads: Option<&HashMap<u32, String>>) -> Region {
    let s = area.sheet;
    let mut cells: Vec<NamedCell> = Vec::new();
    let mut labels: Vec<(Option<String>, Option<String>)> = Vec::new();
    for r in area.first_row..=area.last_row {
        for col in area.first_col..=area.last_col {
            if !f.is_value(s, r, col) {
                continue;
            }
            let left = (area.first_col..col).rev().find_map(|x| f.text_at(s, r, x)).map(|t| t.trim().trim_end_matches(':').to_string());
            let above = heads.and_then(|h| h.get(&col)).filter(|n| !n.starts_with("column_")).map(String::as_str).or_else(|| (area.first_row..r).rev().find_map(|x| f.text_at(s, x, col))).map(|t| t.trim().trim_end_matches(':').to_string());
            let typed = frame::frame_in(f.wb, &f.at, Area { sheet: s, first_row: r, first_col: col, last_row: r, last_col: col }, false).ok();
            let dtype = typed.as_ref().and_then(|d| d.columns.first().map(|x| x.data.type_name().to_string())).unwrap_or_else(|| "Null".into());
            cells.push(NamedCell { name: String::new(), row: r, col, dtype, formula: f.formula_at(s, r, col).map(str::to_string), logic: None });
            labels.push((left, above));
        }
    }
    // The label to the left names the cell; where it names several, the label above is added.
    let mut names = Names(HashSet::new());
    for k in 0..cells.len() {
        let (left, above) = &labels[k];
        let shared = left.is_some() && labels.iter().zip(&cells).filter(|(l, c)| l.0 == *left && c.row == cells[k].row).count() > 1;
        let address = format!("{}{}", xl_formula::col_name(cells[k].col + 1), cells[k].row + 1);
        let want = match (defined.get(&(s, cells[k].row, cells[k].col)), left, above) {
            (Some(n), _, _) => n.clone(),
            (None, Some(l), Some(a)) if shared => format!("{l} {a}"),
            (None, Some(l), _) => l.clone(),
            (None, None, Some(a)) => a.clone(),
            (None, None, None) => address,
        };
        cells[k].name = names.take(&want);
    }
    Region { name, kind: RegionKind::Cells, area, has_header: false, header_rows: 0, source: "detected", columns: Vec::new(), cells }
}

/// Where the cells of regions lie, to write references to them by name.
struct Lookup {
    /// (sheet, column) → (region index, column name, first data row, last data row).
    columns: HashMap<(u32, u32), Vec<(usize, String, u32, u32)>>,
    /// The cells that can be written by name, each with that spelling.
    cells: HashMap<(u32, u32, u32), String>,
    tables: Vec<String>,
}

/// The name of a cell as it is written in logic: the name itself if it is a plain
/// name, else with `_` for everything that is not a letter or digit (`Tax rate` is
/// written `Tax_rate`).
pub fn written_name(name: &str) -> String {
    if plain_name(name) {
        return name.to_string();
    }
    let mut out = String::new();
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() || ch == '.' {
            out.push(ch);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.chars().next().is_some_and(|c| c.is_alphabetic()) { out } else { format!("_{out}") }
}

/// The cells of all regions that can be written by name: (sheet, row, column, spelling).
/// A spelling that would stand for two cells, a table or a defined name is left out
/// (such a cell is written by its address).
pub fn written_cells(all: &[Region], wb: &Workbook) -> Vec<(u32, u32, u32, String)> {
    let mut count: HashMap<String, usize> = HashMap::new();
    let cells: Vec<(u32, u32, u32, String)> = all.iter().flat_map(|r| r.cells.iter().map(|c| (r.area.sheet, c.row, c.col, written_name(&c.name)))).collect();
    for c in &cells {
        *count.entry(c.3.to_lowercase()).or_default() += 1;
    }
    let taken = |n: &str| all.iter().any(|r| r.kind == RegionKind::Table && r.name.eq_ignore_ascii_case(n)) || wb.defined_names.iter().any(|d| d.name.eq_ignore_ascii_case(n));
    cells.into_iter().filter(|c| plain_name(&c.3) && count[&c.3.to_lowercase()] == 1 && !taken(&c.3)).collect()
}

impl Lookup {
    fn new(regions: &[Region], wb: &Workbook) -> Lookup {
        let mut l = Lookup { columns: HashMap::new(), cells: HashMap::new(), tables: regions.iter().map(|r| r.name.clone()).collect() };
        for (k, r) in regions.iter().enumerate() {
            if let (RegionKind::Table, Some((first, last))) = (r.kind, r.data_rows()) {
                for col in &r.columns {
                    l.columns.entry((r.area.sheet, col.col)).or_default().push((k, col.name.clone(), first, last));
                }
            }
        }
        l.cells = written_cells(regions, wb).into_iter().map(|(s, r, c, name)| ((s, r, c), name)).collect();
        l
    }
}

/// A name as Excel accepts it without quoting: letters, digits, `_` and `.`, not a cell.
fn plain_name(n: &str) -> bool {
    let mut chars = n.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') && xl_formula::parse_cell_ref(n).is_none()
}

fn column_spec(name: &str) -> String {
    // In a structured reference `[`, `]`, `#` and `'` are escaped by an apostrophe.
    let mut out = String::new();
    for ch in name.chars() {
        if matches!(ch, '[' | ']' | '#' | '\'') {
            out.push('\'');
        }
        out.push(ch);
    }
    out
}

/// A formula of the cell (`row` 0-based) of `home` with its references written by name
/// where they are a column of the same row, a whole column of a table, or a named cell.
/// `None` if the formula cannot be parsed.
fn structured(formula: &str, l: &Lookup, home: &Region, row: u32, wb: &Workbook) -> Option<String> {
    let e = xl_formula::parse(formula).ok()?;
    let sheet_of = |prefix: &Option<String>| -> Option<u32> {
        match prefix {
            None => Some(home.area.sheet),
            Some(p) => {
                let n = p.trim_end_matches('!').trim_matches('\'').replace("''", "'");
                wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(&n)).map(|i| i as u32)
            }
        }
    };
    fn go(e: &Expr, l: &Lookup, home: &Region, row: u32, sheet_of: &dyn Fn(&Option<String>) -> Option<u32>) -> Expr {
        let b = |x: &Expr| Box::new(go(x, l, home, row, sheet_of));
        match e {
            // A range: whole columns of one table.
            Expr::Binary { op: BinaryOp::Range, lhs, rhs, .. } => {
                if let (Expr::Ref { prefix, kind: RefKind::Cell, text: a }, Expr::Ref { prefix: p2, kind: RefKind::Cell, text: z }) = (lhs.unspaced(), rhs.unspaced()) {
                    if let (Some(s), None, Some(a), Some(z)) = (sheet_of(prefix), p2, xl_formula::parse_cell_ref(a), xl_formula::parse_cell_ref(z)) {
                        let (first, last) = (l.columns.get(&(s, a.col - 1)), l.columns.get(&(s, z.col - 1)));
                        if let (Some(first), Some(last)) = (first, last) {
                            for (k, name, r0, r1) in first {
                                let Some((_, end, _, _)) = last.iter().find(|x| x.0 == *k) else { continue };
                                if a.row - 1 == *r0 && z.row - 1 == *r1 {
                                    let table = Some(l.tables[*k].clone()).filter(|t| plain_name(t));
                                    let Some(table) = table else { continue };
                                    let spec = if name == end { format!("[{}]", column_spec(name)) } else { format!("[[{}]:[{}]]", column_spec(name), column_spec(end)) };
                                    return Expr::Structured { table: Some(table), spec };
                                }
                            }
                        }
                    }
                }
                e.clone()
            }
            Expr::Ref { prefix, kind: RefKind::Cell, text } => {
                let (Some(s), Some(r)) = (sheet_of(prefix), xl_formula::parse_cell_ref(text)) else { return e.clone() };
                // A column of the own row of the own table.
                if s == home.area.sheet && r.row - 1 == row && !r.abs_row && home.kind == RegionKind::Table {
                    if let Some(col) = home.columns.iter().find(|c| c.col == r.col - 1) {
                        return Expr::Structured { table: None, spec: format!("[@{}]", if plain_name(&col.name) { col.name.clone() } else { format!("[{}]", column_spec(&col.name)) }) };
                    }
                }
                match l.cells.get(&(s, r.row - 1, r.col - 1)) {
                    Some(name) => Expr::Name { prefix: None, text: name.clone() },
                    _ => e.clone(),
                }
            }
            Expr::Call { name, args } => Expr::Call { name: name.clone(), args: args.iter().map(|a| go(a, l, home, row, sheet_of)).collect() },
            Expr::Apply { callee, args } => Expr::Apply { callee: b(callee), args: args.iter().map(|a| go(a, l, home, row, sheet_of)).collect() },
            Expr::Unary { op, expr } => Expr::Unary { op: *op, expr: b(expr) },
            Expr::Postfix { expr, op } => Expr::Postfix { expr: b(expr), op: *op },
            Expr::Binary { op, op_text, lhs, rhs } => Expr::Binary { op: *op, op_text: op_text.clone(), lhs: b(lhs), rhs: b(rhs) },
            Expr::Paren(x) => Expr::Paren(b(x)),
            Expr::Spaced { before, expr, after } => Expr::Spaced { before: before.clone(), expr: b(expr), after: after.clone() },
            Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|x| go(x, l, home, row, sheet_of)).collect()).collect()),
            other => other.clone(),
        }
    }
    Some(go(&e, l, home, row, &sheet_of).to_string())
}

/// The inverse of the structured form: a formula written with `[@column]`, `Table[column]`
/// and the names of single cells, as the formula of the cell (`row` 0-based) of `home`
/// in A1 notation. Names that are neither a table nor a named cell stay as they are
/// (defined names of the workbook).
pub fn unstructured(logic: &str, all: &[Region], home: &Region, row: u32, wb: &Workbook) -> Result<String, String> {
    let e = xl_formula::parse(logic.trim().trim_start_matches('=')).map_err(|e| format!("the formula {logic:?} is not understood (at character {}: {})", e.pos + 1, e.message))?;
    let sheet_prefix = |s: u32| format!("'{}'!", wb.sheets[s as usize].name.replace('\'', "''"));
    let strip = |s: &str| s.trim().strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(s.trim()).replace("''", "\u{1}").replace('\'', "").replace('\u{1}', "'");
    let column = |t: &Region, name: &str| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(name)).map(|c| c.col).ok_or_else(|| format!("{:?} has no column {name:?}; its columns are {:?}", t.name, t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()));
    let mut problem: Option<String> = None;
    fn go(e: &Expr, f: &mut dyn FnMut(&Expr) -> Option<Expr>) -> Expr {
        if let Some(new) = f(e) {
            return new;
        }
        let mut b = |x: &Expr| Box::new(go(x, f));
        match e {
            Expr::Call { name, args } => Expr::Call { name: name.clone(), args: args.iter().map(|a| go(a, f)).collect() },
            Expr::Apply { callee, args } => Expr::Apply { callee: b(callee), args: args.iter().map(|a| go(a, f)).collect() },
            Expr::Unary { op, expr } => Expr::Unary { op: *op, expr: b(expr) },
            Expr::Postfix { expr, op } => Expr::Postfix { expr: b(expr), op: *op },
            Expr::Binary { op, op_text, lhs, rhs } => Expr::Binary { op: *op, op_text: op_text.clone(), lhs: b(lhs), rhs: b(rhs) },
            Expr::Paren(x) => Expr::Paren(b(x)),
            Expr::Spaced { before, expr, after } => Expr::Spaced { before: before.clone(), expr: b(expr), after: after.clone() },
            Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|x| go(x, f)).collect()).collect()),
            other => other.clone(),
        }
    }
    let out = go(&e, &mut |x| match x {
        Expr::Structured { table, spec } => {
            let inner = spec.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(spec).trim();
            let result: Result<Expr, String> = (|| {
                if let Some(name) = inner.strip_prefix('@') {
                    if table.is_some() || home.kind != RegionKind::Table {
                        return Err(format!("{spec} stands for the own row of a table: it can only be used in a column of that table"));
                    }
                    let col = column(home, &strip(name))?;
                    return Ok(Expr::Ref { prefix: None, kind: RefKind::Cell, text: format!("{}{}", xl_formula::col_name(col + 1), row + 1) });
                }
                let name = table.as_deref().ok_or_else(|| format!("{spec} needs the name of its table (Table{spec})"))?;
                let t = all.iter().find(|r| r.kind == RegionKind::Table && r.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("there is no table {name:?}; the tables are {:?}", all.iter().filter(|r| r.kind == RegionKind::Table).map(|r| r.name.as_str()).collect::<Vec<_>>()))?;
                let (first, last) = t.data_rows().ok_or_else(|| format!("the table {name:?} has no rows"))?;
                let (c0, c1) = if inner.is_empty() {
                    (t.area.first_col, t.area.last_col)
                } else if let Some((a, z)) = inner.split_once("]:[") {
                    (column(t, &strip(a.trim_start_matches('[')))?, column(t, &strip(z.trim_end_matches(']')))?)
                } else {
                    let c = column(t, &strip(inner))?;
                    (c, c)
                };
                let cell = |r: u32, c: u32| format!("${}${}", xl_formula::col_name(c + 1), r + 1);
                Ok(Expr::Binary {
                    op: BinaryOp::Range,
                    op_text: ":".into(),
                    lhs: Box::new(Expr::Ref { prefix: Some(sheet_prefix(t.area.sheet)), kind: RefKind::Cell, text: cell(first, c0.min(c1)) }),
                    rhs: Box::new(Expr::Ref { prefix: None, kind: RefKind::Cell, text: cell(last, c0.max(c1)) }),
                })
            })();
            match result {
                Ok(new) => Some(new),
                Err(why) => {
                    problem.get_or_insert(why);
                    None
                }
            }
        }
        Expr::Name { prefix: None, text } if !wb.defined_names.iter().any(|d| d.name.eq_ignore_ascii_case(text)) => {
            let known = written_cells(all, wb);
            match known.iter().find(|c| c.3.eq_ignore_ascii_case(text)) {
                Some((s, r, c, _)) => Some(Expr::Ref { prefix: Some(sheet_prefix(*s)), kind: RefKind::Cell, text: format!("${}${}", xl_formula::col_name(c + 1), r + 1) }),
                None => {
                    let names: Vec<&str> = known.iter().map(|c| c.3.as_str()).take(40).collect();
                    let more = if known.len() > 40 { format!(" and {} more", known.len() - 40) } else { String::new() };
                    problem.get_or_insert(format!("{text:?} is neither a named cell nor a defined name. Cells are written by their name with _ for spaces and signs; the cells that can be written so are {names:?}{more}. Any cell can be written by its address (Sheet!$B$1)."));
                    None
                }
            }
        }
        _ => None,
    });
    match problem {
        Some(why) => Err(why),
        None => Ok(out.to_string()),
    }
}

/// A region as a frame: a table or block as its columns; single cells as one row with
/// a column per cell.
pub fn region_frame(wb: &Workbook, region: &Region) -> Result<Frame, String> {
    region_frame_in(wb, &frame::cell_index(wb), region)
}

/// As [`region_frame`], with an index of the workbook's cells (for many regions).
pub fn region_frame_in(wb: &Workbook, at: &frame::CellIndex, region: &Region) -> Result<Frame, String> {
    match region.kind {
        RegionKind::Table | RegionKind::Block => frame::frame_headed(wb, at, region.area, region.header_rows),
        RegionKind::Cells => {
            let mut columns: Vec<Column> = Vec::with_capacity(region.cells.len());
            for cell in &region.cells {
                let one = frame::frame_in(wb, at, Area { sheet: region.area.sheet, first_row: cell.row, first_col: cell.col, last_row: cell.row, last_col: cell.col }, false)?;
                let data = one.columns.into_iter().next().map(|c| c.data).ok_or("no column")?;
                columns.push(Column::new(cell.name.clone(), data));
            }
            Frame::new(1, columns)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xl_cells::{Sheet, Value};

    pub(super) fn sample_for_other_tests() -> Workbook {
        sample()
    }

    fn sample() -> Workbook {
        let mut wb = Workbook::default();
        wb.sheets.push(Sheet { name: "Sales".into(), ..Default::default() });
        wb.sheets.push(Sheet { name: "Model".into(), ..Default::default() });
        let c = &mut wb.cells;
        // A table with a title above, a calculated column and a totals row.
        c.push(0, 0, 0, Value::Text("Orders 2026".into()), None, 0);
        for (k, h) in ["item", "qty", "price", "amount"].iter().enumerate() {
            c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
        }
        for r in 0..4u32 {
            c.push(0, 2 + r, 0, Value::Text(format!("item{r}")), None, 0);
            c.push(0, 2 + r, 1, Value::Number((r + 1) as f64), None, 0);
            c.push(0, 2 + r, 2, Value::Number(2.5), None, 0);
            c.push(0, 2 + r, 3, Value::Number((r + 1) as f64 * 2.5 * 1.19), Some(format!("B{0}*C{0}*(1+Model!$B$1)", r + 3)), 0);
        }
        c.push(0, 6, 0, Value::Text("Total".into()), None, 0);
        c.push(0, 6, 3, Value::Number(29.75), Some("SUM(D3:D6)".into()), 0);
        // A grid of plain numbers, apart from the table.
        for r in 0..3u32 {
            for k in 0..2u32 {
                c.push(0, 10 + r, 6 + k, Value::Number((r * 2 + k) as f64), None, 0);
            }
        }
        // A model: labelled inputs and a result.
        c.push(1, 0, 0, Value::Text("Tax rate:".into()), None, 0);
        c.push(1, 0, 1, Value::Number(0.19), None, 0);
        c.push(1, 1, 0, Value::Text("Orders".into()), None, 0);
        c.push(1, 1, 1, Value::Number(4.0), Some("COUNTA(Sales!A3:A6)".into()), 0);
        c.push(1, 2, 0, Value::Text("Gross".into()), None, 0);
        c.push(1, 2, 1, Value::Number(29.75), Some("Sales!D7".into()), 0);
        wb
    }

    #[test]
    fn a_second_row_of_labels_is_part_of_the_header() {
        let mut wb = Workbook::default();
        wb.sheets.push(Sheet { name: "Volumes".into(), merges: vec![(0, 1, 0, 2)], ..Default::default() });
        let c = &mut wb.cells;
        // A group label merged over two columns, the columns' own labels below.
        c.push(0, 0, 1, Value::Text("Volume".into()), None, 0);
        c.push(0, 0, 3, Value::Text("Note".into()), None, 0);
        for (k, h) in ["Day", "Plan", "Actual", "Difference"].iter().enumerate() {
            c.push(0, 1, k as u32, Value::Text(h.to_string()), None, 0);
        }
        for r in 0..4u32 {
            c.push(0, 2 + r, 0, Value::Text(format!("d{r}")), None, 0);
            c.push(0, 2 + r, 1, Value::Number(10.0 + r as f64), None, 0);
            c.push(0, 2 + r, 2, Value::Number(9.0), None, 0);
            c.push(0, 2 + r, 3, Value::Number(1.0 + r as f64), Some(format!("B{0}-C{0}", r + 3)), 0);
        }
        let found = regions(&wb);
        assert_eq!(found.len(), 1);
        let t = &found[0];
        assert_eq!((t.header_rows, t.data_rows(), t.range()), (2, Some((2, 5)), "A1:D6".to_string()));
        assert_eq!(t.columns.iter().map(|c| (c.name.as_str(), c.dtype.as_str())).collect::<Vec<_>>(), vec![("Day", "String"), ("Volume Plan", "Int64"), ("Volume Actual", "Int64"), ("Note Difference", "Int64")]);
        assert_eq!(t.columns[3].logic.as_deref(), Some("[@[Volume Plan]]-[@[Volume Actual]]"));
        assert_eq!(region_frame(&wb, t).unwrap().height, 4);
        // A first row of texts in the data is not a header: the column goes on with text.
        let mut wb = Workbook::default();
        wb.sheets.push(Sheet { name: "S".into(), ..Default::default() });
        let c = &mut wb.cells;
        for (r, row) in [["name", "kind"], ["a", "x"], ["b", "y"], ["c", "z"]].iter().enumerate() {
            c.push(0, r as u32, 0, Value::Text(row[0].into()), None, 0);
            c.push(0, r as u32, 1, Value::Text(row[1].into()), None, 0);
        }
        assert_eq!(regions(&wb)[0].header_rows, 1);
    }

    #[test]
    fn tables_cells_and_blocks_are_found() {
        let wb = sample();
        let found = regions(&wb);
        let names: Vec<(&str, &str, String)> = found.iter().map(|r| (r.name.as_str(), r.kind.as_str(), r.range())).collect();
        assert_eq!(names, vec![("Orders 2026", "table", "A2:D6".to_string()), ("Orders 2026 totals", "cells", "A7:D7".to_string()), ("Sales", "block", "G11:H13".to_string()), ("Model", "cells", "A1:B3".to_string())]);
        let table = &found[0];
        assert_eq!(table.columns.iter().map(|c| (c.name.as_str(), c.dtype.as_str())).collect::<Vec<_>>(), vec![("item", "String"), ("qty", "Int64"), ("price", "Float64"), ("amount", "Float64")]);
        assert_eq!(table.columns[3].formula.as_deref(), Some("B3*C3*(1+Model!$B$1)"));
        assert_eq!(table.columns[3].logic.as_deref(), Some("[@qty]*[@price]*(1+Tax_rate)"));
        assert!(table.columns[..3].iter().all(|c| c.formula.is_none()));
        // The totals row: named by the header above it.
        assert_eq!(found[1].cells.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Total"]);
        // Single cells are named by their labels; their logic names what it reads.
        let model = &found[3];
        assert_eq!(model.cells.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Tax rate", "Orders", "Gross"]);
        assert_eq!(model.cells[2].logic.as_deref(), Some("Total"));
        let row = region_frame(&wb, model).unwrap();
        assert_eq!(row.shape(), (1, 3));
        assert_eq!(region_frame(&wb, table).unwrap().shape(), (4, 4));
    }

    #[test]
    fn a_whole_column_of_a_table_is_named() {
        let mut wb = sample();
        // A table whose name can stand in a formula.
        wb.cells.text[0] = Some(std::sync::Arc::from("Orders"));
        let found = regions(&wb);
        assert_eq!(found[0].name, "Orders");
        assert_eq!(found[1].cells[0].logic.as_deref(), Some("SUM(Orders[amount])"));
        assert_eq!(found[3].cells[1].logic.as_deref(), Some("COUNTA(Orders[item])"));
    }
}

#[cfg(test)]
mod unstructured_tests {
    use super::*;

    #[test]
    fn structured_logic_goes_back_to_cells() {
        let mut wb = super::tests::sample_for_other_tests();
        wb.cells.text[0] = Some(std::sync::Arc::from("Orders"));
        let found = regions(&wb);
        let table = &found[0];
        assert_eq!(unstructured("[@qty]*[@price]*2", &found, table, 2, &wb).unwrap(), "B3*C3*2");
        assert_eq!(unstructured("=SUM(Orders[amount])/Gross", &found, &found[3], 0, &wb).unwrap(), "SUM('Sales'!$D$3:$D$6)/'Model'!$B$3");
        assert_eq!(unstructured("VLOOKUP(1,Orders[[qty]:[amount]],3,FALSE)", &found, &found[3], 0, &wb).unwrap(), "VLOOKUP(1,'Sales'!$B$3:$D$6,3,FALSE)");
        assert!(unstructured("[@nope]", &found, table, 2, &wb).unwrap_err().contains("its columns are"));
        assert!(unstructured("Nope+1", &found, table, 2, &wb).unwrap_err().contains("the cells that can be written so are"));
        // A name with a space is written with an underscore, both ways.
        assert_eq!(unstructured("Tax_rate*2", &found, table, 2, &wb).unwrap(), "'Model'!$B$1*2");
        assert_eq!(found[0].columns[3].logic.as_deref(), Some("[@qty]*[@price]*(1+Tax_rate)"));
        assert!(unstructured("1+", &found, table, 2, &wb).unwrap_err().contains("at character"));
    }

    #[test]
    fn rows_that_differ_are_exceptions_of_their_column() {
        let mut wb = super::tests::sample_for_other_tests();
        let at = |wb: &Workbook, row: u32, col: u32| (0..wb.cells.len()).find(|&i| (wb.cells.sheet[i], wb.cells.row[i], wb.cells.col[i]) == (0, row, col)).unwrap();
        // An amount of its own in the last row of the data, one that is a plain value, and
        // a quantity that is calculated.
        let (own, plain, qty) = (at(&wb, 5, 3), at(&wb, 3, 3), at(&wb, 4, 1));
        wb.cells.formula[own] = Some("B6*C6*2".into());
        wb.cells.formula[qty] = Some("B4+1".into());
        let found = regions(&wb);
        assert_eq!((found[0].range(), found[1].kind), ("A2:D6".to_string(), RegionKind::Cells));
        let (q, amount) = (&found[0].columns[1], &found[0].columns[3]);
        assert_eq!((q.formula.as_deref(), q.is_calculated()), (None, true));
        assert_eq!(q.exceptions, vec![Exception { row: 4, formula: Some("B4+1".into()), logic: Some("B4+1".into()), dtype: None }]);
        assert_eq!(amount.formula.as_deref(), Some("B3*C3*(1+Model!$B$1)"));
        assert_eq!(amount.exceptions, vec![Exception { row: 5, formula: Some("B6*C6*2".into()), logic: Some("[@qty]*[@price]*2".into()), dtype: None }]);
        assert_eq!(amount.formula_at(4, 2).as_deref(), Some("B5*C5*(1+Model!$B$1)"));
        // Too many rows of their own: the column is neither one formula nor values, and
        // the block is single cells.
        wb.cells.formula[plain] = None;
        let first = at(&wb, 2, 3);
        wb.cells.formula[first] = Some("B3".into());
        assert!(regions(&wb).iter().all(|r| r.kind != RegionKind::Table));
    }
}
