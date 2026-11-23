//! Parts of a sheet as typed frames.
//!
//! A frame has one data type per column, chosen from what the cells hold: whole numbers
//! `Int64` (unless the file spells one as `1.0`), other numbers `Float64`, booleans `Boolean`, date serials `Date`, `Datetime`,
//! `Time` or `Duration` (by value and number format), text `String`. A column of mixed
//! kinds becomes `String`; empty cells and error values are null; a column without any
//! value has the type `Null`.
//! The types are those of Polars; bindings turn the frames into Polars or Arrow frames.

use crate::data::{Column, Data, Frame};
use xl_cells::{Kind, Workbook};

/// A rectangle of a sheet; rows and columns 0-based, the last ones included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub sheet: u32,
    pub first_row: u32,
    pub first_col: u32,
    pub last_row: u32,
    pub last_col: u32,
}

/// A cell holds something a table can show: not blank, and not the empty text.
fn holds(wb: &Workbook, i: usize) -> bool {
    let c = &wb.cells;
    match c.kind[i] {
        Kind::Blank => false,
        Kind::Text => c.text[i].as_deref().is_some_and(|t| !t.is_empty()),
        _ => true,
    }
}

/// The block of cells of a sheet that hold something; `None` for a sheet without any.
pub fn used_area(wb: &Workbook, sheet: u32) -> Option<Area> {
    let c = &wb.cells;
    let mut area: Option<Area> = None;
    for i in (0..c.len()).filter(|&i| c.sheet[i] == sheet && holds(wb, i)) {
        let (r, col) = (c.row[i], c.col[i]);
        area = Some(match area {
            None => Area { sheet, first_row: r, first_col: col, last_row: r, last_col: col },
            Some(a) => Area { sheet, first_row: a.first_row.min(r), first_col: a.first_col.min(col), last_row: a.last_row.max(r), last_col: a.last_col.max(col) },
        });
    }
    area
}

const MS_PER_DAY: f64 = 86_400_000.0;
/// The largest serial Excel shows as a date (9999-12-31).
const LAST_SERIAL: f64 = 2_958_465.999_999;

/// What a date serial stands for, by its value and number format (as openpyxl reads it).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Moment {
    /// Milliseconds since 1970-01-01.
    Datetime(i64),
    /// Milliseconds since midnight.
    Time(i64),
    /// Milliseconds.
    Duration(i64),
}

/// The number format counts elapsed time (`[h]:mm:ss`, `[mm]:ss`, `[s]`).
fn elapsed_format(format: &str) -> bool {
    let first = format.split(';').next().unwrap_or("").to_ascii_lowercase();
    ["[h]", "[hh]", "[m]", "[mm]", "[s]", "[ss]"].iter().any(|t| first.contains(t))
}

fn moment(value: f64, date1904: bool, format: &str) -> Option<Moment> {
    if !value.is_finite() {
        return None;
    }
    if elapsed_format(format) {
        // Whole microseconds, of those the part below a second rounded to milliseconds.
        let us = (value * 86_400e6).round_ties_even();
        let secs = (us / 1e6).floor();
        let ms = ((us - secs * 1e6) / 1000.0).round_ties_even();
        return Some(Moment::Duration((secs * 1000.0 + ms) as i64));
    }
    if !(0.0..=LAST_SERIAL).contains(&value) {
        return None;
    }
    let mut day = value.floor();
    let ms = ((value - day) * MS_PER_DAY).round_ties_even();
    if value < 1.0 && ms < MS_PER_DAY {
        return Some(Moment::Time(ms as i64));
    }
    // The 1900 system counts a 29 February 1900 that never was.
    if !date1904 && value < 60.0 {
        day += 1.0;
    }
    // 1899-12-30 and 1904-01-01 in days since 1970-01-01.
    let epoch = if date1904 { -24_107.0 } else { -25_569.0 };
    Some(Moment::Datetime(((epoch + day) * MS_PER_DAY + ms) as i64))
}

use crate::data::civil;

fn clock(ms: i64) -> String {
    let (s, frac) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let base = format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60);
    if frac == 0 {
        base
    } else {
        format!("{base}.{:06}", frac * 1000)
    }
}

/// A moment as text, the way Python prints the value (`2020-01-31 00:00:00`, `12:30:00`,
/// `1 day, 2:00:00`); `date_only` prints a date without its time.
fn moment_text(m: Moment, date_only: bool) -> String {
    match m {
        Moment::Datetime(ms) => {
            let (y, mo, d) = civil(ms.div_euclid(86_400_000));
            let date = format!("{y:04}-{mo:02}-{d:02}");
            if date_only {
                date
            } else {
                format!("{date} {}", clock(ms.rem_euclid(86_400_000)))
            }
        }
        Moment::Time(ms) => clock(ms),
        Moment::Duration(ms) => {
            let days = ms.div_euclid(86_400_000);
            let rest = ms.rem_euclid(86_400_000);
            let (s, frac) = (rest / 1000, rest % 1000);
            let mut out = match days {
                0 => String::new(),
                1 | -1 => format!("{days} day, "),
                _ => format!("{days} days, "),
            };
            out.push_str(&format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60));
            if frac != 0 {
                out.push_str(&format!(".{:06}", frac * 1000));
            }
            out
        }
    }
}

/// A value of a column as it is in a column of mixed kinds, which is text: numbers and
/// moments as [`frame`] writes them there.
pub fn mixed_text(data: &Data, k: usize) -> Option<String> {
    match data {
        Data::Int64(v) => v.get(k).copied().flatten().map(|x| x.to_string()),
        Data::Float64(v) => v.get(k).copied().flatten().map(number_text),
        Data::Date(v) => v.get(k).copied().flatten().map(|d| moment_text(Moment::Datetime(d as i64 * 86_400_000), false)),
        Data::Datetime(v) => v.get(k).copied().flatten().map(|ms| moment_text(Moment::Datetime(ms), false)),
        Data::Time(v) => v.get(k).copied().flatten().map(|ns| moment_text(Moment::Time(ns / 1_000_000), false)),
        Data::Duration(v) => v.get(k).copied().flatten().map(|ms| moment_text(Moment::Duration(ms), false)),
        other => other.text(k),
    }
}

fn whole(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0
}

/// A number as text: whole numbers without a fraction, others with the fewest digits
/// that give the number back (exponents written as Python writes them).
fn number_text(n: f64) -> String {
    if whole(n) {
        return format!("{}", n as i64);
    }
    let a = n.abs();
    if a.is_finite() && (a >= 1e16 || a < 1e-4) {
        let e = format!("{n:e}");
        let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!("{mantissa}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    match n {
        n if n.is_nan() => "nan".into(),
        n if n.is_infinite() => (if n > 0.0 { "inf" } else { "-inf" }).into(),
        n => format!("{n}"),
    }
}

/// The cells of one column in the rows asked for, typed.
struct ColumnCells<'a> {
    wb: &'a Workbook,
    /// For each row the cell's index, if the cell holds something (errors included).
    cells: Vec<Option<usize>>,
}

impl ColumnCells<'_> {
    fn moment_of(&self, i: usize) -> Option<Moment> {
        let c = &self.wb.cells;
        let style = self.wb.styles.get(c.style[i] as usize)?;
        if c.kind[i] != Kind::Number || !style.is_date() {
            return None;
        }
        moment(c.num[i], self.wb.date1904, &style.num_format)
    }

    /// The cells that count for the type: errors do not (they are null in any type).
    fn present(&self) -> impl Iterator<Item = usize> + '_ {
        self.cells.iter().flatten().copied().filter(|&i| self.wb.cells.kind[i] != Kind::Error)
    }

    fn data(&self) -> Data {
        let c = &self.wb.cells;
        let n = self.cells.len();
        if self.present().next().is_none() {
            return Data::Null(n);
        }
        let number = |i: usize| (c.kind[i] == Kind::Number).then_some(c.num[i]);
        let per_row = |f: &dyn Fn(usize) -> Option<i64>| -> Vec<Option<i64>> { self.cells.iter().map(|x| x.and_then(f)).collect() };
        if self.present().all(|i| c.kind[i] == Kind::Number) {
            let moments: Vec<Option<Moment>> = self.present().map(|i| self.moment_of(i)).collect();
            if moments.iter().all(Option::is_some) {
                let ms = |want: fn(Moment) -> Option<i64>| per_row(&|i| self.moment_of(i).and_then(want));
                if moments.iter().all(|m| matches!(m, Some(Moment::Datetime(ms)) if ms.rem_euclid(86_400_000) == 0)) {
                    let days: Vec<Option<i32>> = ms(|m| if let Moment::Datetime(v) = m { Some(v) } else { None }).into_iter().map(|v| v.map(|v| v.div_euclid(86_400_000) as i32)).collect();
                    return Data::Date(days);
                }
                if moments.iter().all(|m| matches!(m, Some(Moment::Datetime(_)))) {
                    return Data::Datetime(ms(|m| if let Moment::Datetime(v) = m { Some(v) } else { None }));
                }
                if moments.iter().all(|m| matches!(m, Some(Moment::Time(_)))) {
                    return Data::Time(ms(|m| if let Moment::Time(v) = m { Some(v * 1_000_000) } else { None }));
                }
                if moments.iter().all(|m| matches!(m, Some(Moment::Duration(_)))) {
                    return Data::Duration(ms(|m| if let Moment::Duration(v) = m { Some(v) } else { None }));
                }
            }
            // Whole numbers are integers, unless the file spells one as a float (`1.0`).
            if self.present().all(|i| whole(c.num[i]) && !c.float_text[i]) {
                return Data::Int64(per_row(&|i| number(i).map(|v| v as i64)));
            }
            return Data::Float64(self.cells.iter().map(|x| x.and_then(number)).collect());
        }
        if self.present().all(|i| c.kind[i] == Kind::Bool) {
            return Data::Bool(self.cells.iter().map(|x| x.and_then(|i| (c.kind[i] == Kind::Bool).then_some(c.num[i] != 0.0))).collect());
        }
        Data::String(self.cells.iter().map(|x| x.and_then(|i| self.text(i, false))).collect())
    }

    /// A cell as text, for a column of mixed kinds (`header`: as a column's name, where
    /// a date without a time is printed as a date and booleans as `True` and `False`).
    fn text(&self, i: usize, header: bool) -> Option<String> {
        let c = &self.wb.cells;
        match c.kind[i] {
            Kind::Blank | Kind::Error => None,
            Kind::Text => c.text[i].as_deref().map(str::to_string),
            Kind::Bool => Some(match (c.num[i] != 0.0, header) {
                (true, true) => "True".into(),
                (false, true) => "False".into(),
                (true, false) => "true".into(),
                (false, false) => "false".into(),
            }),
            Kind::Number => Some(match self.moment_of(i) {
                Some(m) => moment_text(m, header && matches!(m, Moment::Datetime(ms) if ms.rem_euclid(86_400_000) == 0)),
                None => number_text(c.num[i]),
            }),
        }
    }
}

/// An area of a sheet as a frame. With `has_header` the first row names the columns
/// (a column without a name is `column_<n>`, a name used before gets `_<n>` appended).
pub fn frame(wb: &Workbook, area: Area, has_header: bool) -> Result<Frame, String> {
    frame_at(wb, None, area, has_header as usize)
}

/// The cells of a workbook by place (sheet, row, column): for many frames of one
/// workbook, so that each need not look through all cells.
pub type CellIndex = std::collections::HashMap<(u32, u32, u32), usize>;

pub fn cell_index(wb: &Workbook) -> CellIndex {
    let c = &wb.cells;
    (0..c.len()).map(|i| ((c.sheet[i], c.row[i], c.col[i]), i)).collect()
}

/// As [`frame`], with the cells looked up in an index of the workbook.
pub fn frame_in(wb: &Workbook, at: &CellIndex, area: Area, has_header: bool) -> Result<Frame, String> {
    frame_at(wb, Some(at), area, has_header as usize)
}

/// As [`frame_in`], with `header_rows` rows of names above the data. A column is named
/// by its labels from top to bottom, joined with a space (`MW volume` over `Per model`
/// is `MW volume Per model`); a label that is merged over several columns names each.
pub fn frame_headed(wb: &Workbook, at: &CellIndex, area: Area, header_rows: u32) -> Result<Frame, String> {
    frame_at(wb, Some(at), area, header_rows as usize)
}

fn frame_at(wb: &Workbook, at: Option<&CellIndex>, area: Area, header: usize) -> Result<Frame, String> {
    let c = &wb.cells;
    let (rows, cols) = ((area.last_row + 1 - area.first_row) as usize, (area.last_col + 1 - area.first_col) as usize);
    let mut grid: Vec<Option<usize>> = vec![None; rows * cols];
    match at {
        // An area smaller than the workbook: its places looked up one by one.
        Some(at) if rows * cols <= c.len() => {
            for r in 0..rows {
                for k in 0..cols {
                    grid[r * cols + k] = at.get(&(area.sheet, area.first_row + r as u32, area.first_col + k as u32)).copied().filter(|&i| holds(wb, i));
                }
            }
        }
        _ => {
            for i in 0..c.len() {
                if c.sheet[i] == area.sheet && (area.first_row..=area.last_row).contains(&c.row[i]) && (area.first_col..=area.last_col).contains(&c.col[i]) && holds(wb, i) {
                    grid[(c.row[i] - area.first_row) as usize * cols + (c.col[i] - area.first_col) as usize] = Some(i);
                }
            }
        }
    }
    let first = header.min(rows);
    let merges = &wb.sheets[area.sheet as usize].merges;
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut columns: Vec<Column> = Vec::with_capacity(cols);
    for k in 0..cols {
        let cells = ColumnCells { wb, cells: (first..rows).map(|r| grid[r * cols + k]).collect() };
        // The labels of the header rows, top to bottom; a merged label counts for every
        // column it spans.
        let mut parts: Vec<String> = Vec::new();
        for h in 0..first {
            let (row, col) = (area.first_row + h as u32, area.first_col + k as u32);
            let at_merge = || {
                let m = merges.iter().find(|m| (m.0..=m.2).contains(&row) && (m.1..=m.3).contains(&col) && m.1 < m.3)?;
                (m.0 >= area.first_row && m.1 >= area.first_col && ((m.0 - area.first_row) as usize) < first).then(|| grid[(m.0 - area.first_row) as usize * cols + (m.1 - area.first_col) as usize])?
            };
            let label = grid[h * cols + k].or_else(at_merge).and_then(|i| cells.text(i, true)).map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
            if let Some(label) = label.filter(|l| parts.last() != Some(l)) {
                parts.push(label);
            }
        }
        let mut name = match first {
            // One header row: the label as it is written.
            1 => grid[k].and_then(|i| cells.text(i, true)).filter(|n| !n.is_empty()),
            _ => (!parts.is_empty()).then(|| parts.join(" ")),
        }
        .unwrap_or_else(|| format!("column_{}", k + 1));
        let base = name.clone();
        let mut n = 1;
        while !used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        columns.push(Column::new(name, cells.data()));
    }
    Frame::new(rows.saturating_sub(first), columns)
}

/// The used block of a sheet as a frame; an empty frame for a sheet without cells.
pub fn sheet_frame(wb: &Workbook, sheet: u32, has_header: bool) -> Result<Frame, String> {
    match used_area(wb, sheet) {
        Some(area) => frame(wb, area, has_header),
        None => Ok(Frame::empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_are_read_as_openpyxl_reads_them() {
        // 2026-11-15 12:00 is serial 46341.5.
        assert_eq!(moment_text(moment(46341.5, false, "yyyy-mm-dd").unwrap(), false), "2026-11-15 12:00:00");
        assert_eq!(moment_text(moment(46341.0, false, "yyyy-mm-dd").unwrap(), true), "2026-11-15");
        // Before March 1900 the serial is one ahead; 1904 system.
        assert_eq!(moment_text(moment(59.0, false, "d").unwrap(), true), "1900-02-28");
        assert_eq!(moment_text(moment(61.0, false, "d").unwrap(), true), "1900-03-01");
        assert_eq!(moment_text(moment(1.0, true, "d").unwrap(), true), "1904-01-02");
        assert_eq!(moment(0.5, false, "h:mm"), Some(Moment::Time(43_200_000)));
        assert_eq!(moment_text(moment(1.5, false, "[h]:mm:ss").unwrap(), false), "1 day, 12:00:00");
        assert_eq!(moment(-1.0, false, "d"), None);
    }

    #[test]
    fn numbers_as_text() {
        assert_eq!(number_text(5.0), "5");
        assert_eq!(number_text(0.1), "0.1");
        assert_eq!(number_text(1.5e22), "1.5e+22");
        assert_eq!(number_text(2.5e-7), "2.5e-07");
    }
}
