//! Frames back into a workbook, and frames compared with what a workbook holds.
//!
//! [`write_region`] puts a frame into the cells of a region: the values of the columns
//! that are not calculated, row by row. A row that is an exception of its column stays
//! what it is: a formula of its own is kept, a plain value in a calculated column is
//! written (where the frame has one: no value there leaves the cell as it is).
//!
//! A table may get more or fewer rows, and the sheet stays built as it was: its
//! calculated columns are filled down; what lies below it (a row of totals, other
//! tables) moves down or up with it, as when rows are inserted or removed in Excel;
//! ranges that end at its last row (the sums below it, a filter, a conditional format,
//! the range of an Excel table) follow its new length. Where something beside the table
//! goes on across its end (a second table in the same rows, with the totals of both in
//! one row), only the columns of the table move, unless that would tear a row below
//! apart. Everything else of the workbook
//! (formats, other cells, charts) stays as it is.
//!
//! [`verify`] gives the cells of a region whose value in a frame differs from the one in
//! the workbook.

use crate::frame::Area;
use crate::regions::{Region, RegionKind};
use crate::data::{Column, Data, Frame};
use std::collections::HashMap;
use std::sync::Arc;
use xl_cells::{Kind, Workbook};

/// The value of a cell to be.
#[derive(Debug, Clone, PartialEq)]
enum Put {
    Blank,
    Number(f64),
    Bool(bool),
    Text(String),
    /// A date serial and the number format to show it with.
    Moment(f64, &'static str),
}

/// The values of a column of a frame as cells.
fn puts(data: &Data, date1904: bool) -> Result<Vec<Put>, String> {
    let epoch = if date1904 { 24_107.0 } else { 25_569.0 };
    // The 1900 system counts a 29 February 1900: days before March 1900 are one less.
    let serial = move |days: f64| {
        let s = days + epoch;
        if !date1904 && s < 61.0 {
            s - 1.0
        } else {
            s
        }
    };
    Ok(match data {
        Data::Null(n) => vec![Put::Blank; *n],
        Data::Bool(v) => v.iter().map(|v| v.map_or(Put::Blank, Put::Bool)).collect(),
        Data::String(v) => v.iter().map(|v| v.as_ref().map_or(Put::Blank, |t| Put::Text(t.clone()))).collect(),
        Data::Date(v) => v.iter().map(|v| v.map_or(Put::Blank, |d| Put::Moment(serial(d as f64), "yyyy-mm-dd"))).collect(),
        Data::Datetime(v) => v.iter().map(|v| v.map_or(Put::Blank, |ms| Put::Moment(serial(ms.div_euclid(86_400_000) as f64) + ms.rem_euclid(86_400_000) as f64 / 86_400_000.0, "yyyy-mm-dd hh:mm:ss"))).collect(),
        Data::Time(v) => v.iter().map(|v| v.map_or(Put::Blank, |ns| Put::Moment(ns as f64 / 86_400e9, "hh:mm:ss"))).collect(),
        Data::Duration(v) => v.iter().map(|v| v.map_or(Put::Blank, |ms| Put::Moment(ms as f64 / 86_400_000.0, "[h]:mm:ss"))).collect(),
        Data::Int64(v) => v.iter().map(|v| v.map(|x| x as f64).filter(|x| x.is_finite()).map_or(Put::Blank, Put::Number)).collect(),
        Data::Float64(v) => v.iter().map(|v| v.filter(|x| x.is_finite()).map_or(Put::Blank, Put::Number)).collect(),
    })
}

struct Writer<'a> {
    wb: &'a mut Workbook,
    at: HashMap<(u32, u32, u32), usize>,
    /// The rows moved so far: sheet, columns (all if `None`), from which row, by how many.
    moves: Vec<(u32, Option<(u32, u32)>, u32, i64)>,
    /// Whole rows that came in and may be filled by a table beside: sheet, first, last.
    fresh: Vec<(u32, u32, u32)>,
}

fn index(wb: &Workbook) -> HashMap<(u32, u32, u32), usize> {
    let c = &wb.cells;
    (0..c.len()).map(|i| ((c.sheet[i], c.row[i], c.col[i]), i)).collect()
}

impl Writer<'_> {
    fn new(wb: &mut Workbook) -> Writer<'_> {
        let at = index(wb);
        Writer { wb, at, moves: Vec::new(), fresh: Vec::new() }
    }

    /// Move rows (see [`crate::rows::move_rows`]) and remember it for the regions to come.
    fn shift(&mut self, s: u32, cols: Option<(u32, u32)>, from: u32, by: i64) {
        crate::rows::move_rows(self.wb, s, cols, from, by);
        self.at = index(self.wb);
        self.moves.push((s, cols, from, by));
        self.fresh.retain(|f| f.0 != s);
        if cols.is_none() && by > 0 {
            self.fresh.push((s, from, from + by as u32 - 1));
        }
    }

    /// A region where it lies after the rows moved so far, from the move `since` on.
    fn placed(&self, region: &Region, since: usize) -> Region {
        let mut r = region.clone();
        let a = r.area;
        for (_, _, from, by) in self.moves[since..].iter().filter(|m| m.0 == a.sheet && m.1.is_none_or(|(c1, c2)| c1 <= a.first_col && a.last_col <= c2)) {
            let place = |row: &mut u32| {
                if *row >= *from {
                    *row = (*row as i64 + by).max(0) as u32;
                }
            };
            place(&mut r.area.first_row);
            place(&mut r.area.last_row);
            r.cells.iter_mut().for_each(|c| place(&mut c.row));
            r.columns.iter_mut().flat_map(|c| c.exceptions.iter_mut()).for_each(|e| place(&mut e.row));
        }
        r
    }

    /// The cell at a place, made if there is none (with the format of `like`).
    fn cell(&mut self, s: u32, r: u32, col: u32, like: Option<u32>) -> usize {
        if let Some(&i) = self.at.get(&(s, r, col)) {
            return i;
        }
        let style = like.and_then(|l| self.at.get(&(s, l, col))).map(|&i| self.wb.cells.style[i]).unwrap_or(0);
        self.wb.cells.push_raw(s, r, col, Kind::Blank, 0.0, None, None, style);
        let i = self.wb.cells.len() - 1;
        self.at.insert((s, r, col), i);
        i
    }

    fn put(&mut self, i: usize, value: &Put) {
        let style = match value {
            Put::Moment(_, format) => {
                let mut style = self.wb.styles.get(self.wb.cells.style[i] as usize).cloned().unwrap_or_default();
                if !style.is_date() {
                    style.num_format = format.to_string();
                    Some(self.wb.intern_style(&style))
                } else {
                    None
                }
            }
            _ => None,
        };
        let c = &mut self.wb.cells;
        let (kind, num, text) = match value {
            Put::Blank => (Kind::Blank, 0.0, None),
            Put::Number(n) | Put::Moment(n, _) => (Kind::Number, *n, None),
            Put::Bool(b) => (Kind::Bool, *b as u8 as f64, None),
            Put::Text(t) => (Kind::Text, 0.0, Some(Arc::<str>::from(t.as_str()))),
        };
        c.kind[i] = kind;
        c.num[i] = num;
        c.text[i] = text;
        c.formula[i] = None;
        c.array_ref[i] = None;
        c.rich[i] = None;
        c.iso[i] = xl_cells::IsoDate::No;
        c.float_text[i] = false;
        if let Some(style) = style {
            c.style[i] = style;
        }
    }

    fn put_formula(&mut self, i: usize, formula: String) {
        self.put(i, &Put::Blank);
        self.wb.cells.formula[i] = Some(formula);
    }
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", xl_formula::col_name(col + 1), row + 1)
}

/// What the region's frame holds now, by column: to leave alone what does not change.
fn current(wb: &Workbook, at: &HashMap<(u32, u32, u32), usize>, region: &Region, date1904: bool) -> HashMap<String, Vec<Put>> {
    let Ok(df) = crate::regions::region_frame_in(wb, at, region) else { return HashMap::new() };
    df.columns.iter().filter_map(|c| puts(&c.data, date1904).ok().map(|v| (c.name.clone(), v))).collect()
}

/// Put a frame into the cells of a region. Columns are matched by name; calculated
/// columns and cells keep their formulas (a column of the frame with their name is
/// not written). Returns the region as it lies afterwards.
pub fn write_region(wb: &mut Workbook, region: &Region, df: &Frame) -> Result<Region, String> {
    write_with(&mut Writer::new(wb), region, df)
}

/// [`write_region`] for several regions of one workbook, in one pass over its cells.
pub fn write_regions(wb: &mut Workbook, items: &[(Region, Frame)]) -> Result<Vec<Region>, String> {
    let mut w = Writer::new(wb);
    let mut out = Vec::new();
    for (region, df) in items {
        out.push((write_with(&mut w, region, df).map_err(|why| format!("{}: {why}", region.name))?, w.moves.len()));
    }
    // Where each lies in the end: what was written first may have moved with later rows.
    Ok(out.iter().map(|(region, since)| w.placed(region, *since)).collect())
}

/// Give cells of a workbook a value or a formula, as typed into them: a text that
/// begins with `=` is a formula, a number or boolean is that, a text that reads as a
/// date (`2026-01-31`) a date, any other text a text, and null empties the cell. `cells`
/// are (sheet, row, column, input), 0-based.
pub fn set_cells(wb: &mut Workbook, cells: &[(u32, u32, u32, serde_json::Value)]) {
    use serde_json::Value as J;
    let mut w = Writer::new(wb);
    for (s, row, col, input) in cells {
        let i = w.cell(*s, *row, *col, None);
        match input {
            J::String(t) if t.starts_with('=') && t.len() > 1 => w.put_formula(i, t[1..].to_string()),
            J::String(t) => match (t.trim().parse::<f64>(), crate::view::parse_date(t)) {
                (Ok(n), _) if !t.trim().is_empty() => w.put(i, &Put::Number(n)),
                (_, Some(days)) => {
                    let serial = puts(&Data::Date(vec![Some(days as i32)]), w.wb.date1904).ok().and_then(|mut v| v.pop());
                    w.put(i, &serial.unwrap_or(Put::Blank))
                }
                _ if t.is_empty() => w.put(i, &Put::Blank),
                _ => w.put(i, &Put::Text(t.clone())),
            },
            J::Number(n) => w.put(i, &n.as_f64().map_or(Put::Blank, Put::Number)),
            J::Bool(b) => w.put(i, &Put::Bool(*b)),
            _ => w.put(i, &Put::Blank),
        }
    }
}

fn write_with(w: &mut Writer, region: &Region, df: &Frame) -> Result<Region, String> {
    let region = &w.placed(region, 0);
    let s = region.area.sheet;
    let date1904 = w.wb.date1904;
    let column = |name: &str| df.column(name).map(|c| &c.data);
    if region.kind == RegionKind::Cells {
        if df.height != 1 {
            return Err(format!("{:?} is a group of single cells: its frame has one row, this one has {}", region.name, df.height));
        }
        let now = current(w.wb, &w.at, region, date1904);
        for cell in region.cells.iter().filter(|c| c.formula.is_none()) {
            if let Some(series) = column(&cell.name) {
                let value = puts(series, date1904)?.remove(0);
                if now.get(&cell.name).and_then(|v| v.first()) == Some(&value) {
                    continue;
                }
                let i = w.cell(s, cell.row, cell.col, None);
                w.put(i, &value);
            }
        }
        return Ok(region.clone());
    }
    let first = region.area.first_row + region.header_rows;
    let old_rows = region.area.last_row + 1 - first;
    let rows = df.height as u32;
    let inputs: Vec<&crate::regions::RegionColumn> = region.columns.iter().filter(|c| c.formula.is_none()).collect();
    let missing: Vec<&str> = inputs.iter().filter(|c| column(&c.name).is_none()).map(|c| c.name.as_str()).collect();
    if !missing.is_empty() && rows != old_rows {
        return Err(format!("the frame for {:?} has {rows} rows where the region has {old_rows}, and lacks the columns {missing:?}: with another number of rows every column that is not calculated is needed", region.name));
    }
    if region.kind == RegionKind::Block && rows != old_rows {
        return Err(format!("{:?} is a block of {old_rows} rows; the frame has {rows}", region.name));
    }
    if rows == 0 {
        return Err(format!("the frame for {:?} has no rows: a table keeps at least one", region.name));
    }
    // Every column with a row that holds a value: all of them, or exceptions of a calculated column.
    let holds_values = |c: &&crate::regions::RegionColumn| c.formula.is_none() || c.exceptions.iter().any(|e| e.formula.is_none());
    let values: Vec<(&crate::regions::RegionColumn, Vec<Put>)> = region.columns.iter().filter(holds_values).filter_map(|c| column(&c.name).map(|series| puts(series, date1904).map(|v| (c, v)))).collect::<Result<_, _>>()?;
    let now = current(w.wb, &w.at, region, date1904);
    let (last, cols) = (region.area.last_row, (region.area.first_col, region.area.last_col));
    let inside = |col: &u32| (cols.0..=cols.1).contains(col);
    // More rows need room: what lies below the table moves down. Not where rows came in
    // for a table beside this one and are still empty.
    if rows > old_rows {
        let more = rows - old_rows;
        let below = crate::rows::used(w.wb, s, last + 1..=u32::MAX);
        let free = !below.iter().any(|(r, c)| *r <= last + more && c.iter().any(inside));
        let came_in = w.fresh.iter().any(|f| f.0 == s && f.1 <= last + 1 && last + more <= f.2);
        if below.values().flatten().any(inside) && !(free && came_in) {
            let part = beside(w.wb, s, last, cols) && !tears(&below, cols);
            w.shift(s, part.then_some(cols), last + 1, more as i64);
        }
        crate::rows::end_ranges(w.wb, s, cols, first, last, first + rows - 1);
    }
    let like = region.area.last_row;
    for (c, values) in &values {
        let before = now.get(c.name.as_str());
        for (k, value) in values.iter().enumerate() {
            let row = first + k as u32;
            // A row that is calculated keeps its formula (new rows of a calculated column
            // are filled below).
            let calculated = if (k as u32) < old_rows { c.exception(row).map_or(c.formula.is_some(), |e| e.formula.is_some()) } else { c.formula.is_some() };
            // What the cell shows already stays as it is (a number in a column of mixed
            // kinds is text in the frame, and a number in the cell).
            if calculated || ((k as u32) < old_rows && before.and_then(|v| v.get(k)) == Some(value)) {
                continue;
            }
            // No value in a calculated column says nothing: the row keeps the value it has.
            if c.formula.is_some() && *value == Put::Blank {
                continue;
            }
            let i = w.cell(s, row, c.col, Some(like));
            w.put(i, value);
        }
    }
    for c in region.columns.iter() {
        let Some(formula) = &c.formula else { continue };
        let Ok(e) = xl_formula::parse(formula) else { continue };
        for k in old_rows..rows {
            let i = w.cell(s, first + k, c.col, Some(like));
            w.put_formula(i, e.shifted(k as i64, 0).to_string());
        }
    }
    // Fewer rows: what is left over is emptied, and what lies below the table moves up:
    // the whole rows where nothing else is in them, else the columns of the table, unless
    // that would tear rows below apart (then the rows stay, empty).
    if rows < old_rows {
        for r in first + rows..=last {
            for col in cols.0..=cols.1 {
                if let Some(&i) = w.at.get(&(s, r, col)) {
                    w.put(i, &Put::Blank);
                }
            }
        }
        crate::rows::end_ranges(w.wb, s, cols, first, last, first + rows - 1);
        let whole = crate::rows::used(w.wb, s, first + rows..=last).is_empty();
        let below = crate::rows::used(w.wb, s, last + 1..=u32::MAX);
        if below.values().flatten().any(inside) && (whole || !tears(&below, cols)) {
            w.shift(s, (!whole).then_some(cols), last + 1, rows as i64 - old_rows as i64);
        }
    }
    let mut after = region.clone();
    after.area.last_row = first + rows - 1;
    for c in after.columns.iter_mut() {
        c.exceptions.retain(|e| e.row <= after.area.last_row);
    }
    Ok(after)
}

/// Moving the columns `cols` alone would tear rows apart: one of the rows (by row, the
/// columns in use) holds something on both sides of an edge of these columns.
fn tears(rows: &HashMap<u32, Vec<u32>>, cols: (u32, u32)) -> bool {
    rows.values().any(|c| (cols.0 > 0 && c.contains(&(cols.0 - 1)) && c.contains(&cols.0)) || (c.contains(&cols.1) && c.contains(&(cols.1 + 1))))
}

/// Something beside the columns `cols` goes on across the row `last` of a sheet: a
/// column outside them holds something in that row and in the next.
fn beside(wb: &Workbook, s: u32, last: u32, cols: (u32, u32)) -> bool {
    let used = crate::rows::used(wb, s, last..=last + 1);
    let outside = |row: u32| used.get(&row).into_iter().flatten().filter(|c| !(cols.0..=cols.1).contains(c)).copied().collect::<Vec<u32>>();
    outside(last).iter().any(|c| outside(last + 1).contains(c))
}

fn cell_text(wb: &Workbook, i: Option<usize>) -> Option<String> {
    let c = &wb.cells;
    let i = i?;
    match c.kind[i] {
        Kind::Blank => None,
        Kind::Number => Some(format!("{}", c.num[i])),
        Kind::Bool => Some((if c.num[i] != 0.0 { "true" } else { "false" }).to_string()),
        Kind::Text | Kind::Error => c.text[i].as_deref().map(str::to_string),
    }
}

/// Two numbers are the same to the 12 digits a calculation can be trusted for.
fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1e-300) || (a - b).abs() < 1e-12
}

/// The cells of a region whose value in the frame differs from the one in the workbook:
/// a frame with the columns `region`, `cell`, `column`, `row` (of the frame, 0-based),
/// `expected` (the workbook's value) and `actual` (the frame's), both as text. Columns
/// are matched by name; a column the frame lacks is not compared. Empty when all agree.
pub fn verify(wb: &Workbook, region: &Region, df: &Frame) -> Result<Frame, String> {
    verify_in(wb, &crate::frame::cell_index(wb), region, df)
}

/// As [`verify`], with an index of the workbook's cells (for many regions).
pub fn verify_in(wb: &Workbook, at: &crate::frame::CellIndex, region: &Region, df: &Frame) -> Result<Frame, String> {
    let mut out: [Vec<String>; 5] = Default::default();
    let mut rows_out: Vec<u32> = Vec::new();
    let s = region.area.sheet;
    let first = region.area.first_row + region.header_rows;
    // (name, column, rows of the sheet)
    let targets: Vec<(String, u32, Vec<u32>)> = match region.kind {
        RegionKind::Cells => region.cells.iter().map(|c| (c.name.clone(), c.col, vec![c.row])).collect(),
        _ => region.columns.iter().map(|c| (c.name.clone(), c.col, (first..=region.area.last_row).collect())).collect(),
    };
    for (name, col, rows) in targets {
        let Some(series) = df.column(&name).map(|c| &c.data) else { continue };
        if series.len() != rows.len() {
            return Err(format!("the frame has {} rows where {:?} has {}", series.len(), region.name, rows.len()));
        }
        let got = puts(series, wb.date1904)?;
        for (k, (r, value)) in rows.iter().zip(&got).enumerate() {
            let i = at.get(&(s, *r, col)).copied();
            let c = &wb.cells;
            let same = match (i.map(|i| (c.kind[i], c.num[i], c.text[i].as_deref())), value) {
                (None | Some((Kind::Blank | Kind::Error, _, _)), Put::Blank) => true,
                (Some((Kind::Text, _, t)), Put::Blank) => t.is_none_or(str::is_empty),
                (Some((Kind::Number, n, _)), Put::Number(v) | Put::Moment(v, _)) => close(n, *v),
                (Some((Kind::Bool, n, _)), Put::Bool(b)) => (n != 0.0) == *b,
                (Some((Kind::Text, _, Some(t))), Put::Text(v)) => t == v,
                // A column of mixed kinds is text in a frame.
                (Some((Kind::Number | Kind::Bool, _, _)), Put::Text(v)) => crate::frame::frame_in(wb, at, Area { sheet: s, first_row: rows[0], first_col: col, last_row: *rows.last().unwrap(), last_col: col }, false)
                    .ok()
                    .and_then(|f| f.columns.first().and_then(|x| match &x.data { Data::String(t) => t.get(k).and_then(|t| t.as_deref()).map(|t| t == v), _ => None }))
                    .unwrap_or(false),
                _ => false,
            };
            if !same {
                let actual = match value {
                    Put::Blank => String::new(),
                    Put::Number(n) | Put::Moment(n, _) => format!("{n}"),
                    Put::Bool(b) => b.to_string(),
                    Put::Text(t) => t.clone(),
                };
                out[0].push(region.name.clone());
                out[1].push(format!("{}!{}", wb.sheets[s as usize].name, a1(*r, col)));
                out[2].push(name.clone());
                out[3].push(cell_text(wb, i).unwrap_or_default());
                out[4].push(actual);
                rows_out.push(k as u32);
            }
        }
    }
    let [region_names, cells, columns, expected, actual] = out;
    let text = |v: Vec<String>| Data::String(v.into_iter().map(Some).collect());
    Frame::of(vec![
        Column::new("region", text(region_names)),
        Column::new("cell", text(cells)),
        Column::new("column", text(columns)),
        Column::new("row", Data::Int64(rows_out.into_iter().map(|r| Some(r as i64)).collect())),
        Column::new("expected", text(expected)),
        Column::new("actual", text(actual)),
    ])
}
