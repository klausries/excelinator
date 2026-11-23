//! Sheets built alike, as one table.
//!
//! A workbook often holds the same table once per sheet: a sheet per month, per site,
//! per side of a reconciliation. Each of them is a region of its own (see
//! [`crate::regions`]); a *stack* is those regions seen as one table, with a column
//! [`SHEET`] in front that says which sheet a row is of.
//!
//! Tables form a stack when they lie in different sheets at the same place, with the
//! same header rows and the same column names, and their columns hold data of one kind.
//! Their logic need not be the same: every sheet keeps the formulas it has, as a row
//! that is not as its column keeps its own.
//!
//! The data of a stack are the data of its regions: [`stacked`] puts them one below the
//! other, [`unstacked`] takes a table apart again by its sheet column.

use std::collections::{HashMap, HashSet};

use xl_cells::Workbook;

use crate::data::{Column, Data, Frame};
use crate::regions::{Region, RegionKind};

/// The name of the column that tells the sheets of a stack apart.
pub const SHEET: &str = "sheet";

/// A table of one sheet in a stack.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub sheet: String,
    pub region: String,
}

/// Tables of several sheets that are built alike.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// Unique among the stacks and regions of the workbook.
    pub name: String,
    /// The columns of the tables (without [`SHEET`]).
    pub columns: Vec<String>,
    /// In the order of the sheets.
    pub members: Vec<Member>,
    /// The columns whose logic is not the same in every sheet: (column, the logic of
    /// each member in their order; `None` for plain data).
    pub differs: Vec<(String, Vec<Option<String>>)>,
}

/// Number and text kinds that go into one column.
fn kind_of(dtype: &str) -> &str {
    match dtype {
        "Int64" | "Float64" => "number",
        other => other,
    }
}

/// What the names of the members have in common at their start, without what numbers
/// them: the name of their stack, if it is a word.
fn common_name(names: &[&str]) -> Option<String> {
    let first: Vec<char> = names.first()?.chars().collect();
    let mut n = first.len();
    for other in &names[1..] {
        n = n.min(first.iter().zip(other.chars()).take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase())).count());
    }
    let prefix: String = first[..n].iter().collect();
    let prefix = prefix.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_' || c == '-' || c.is_whitespace());
    (prefix.chars().filter(|c| c.is_alphabetic()).count() >= 3).then(|| prefix.to_string())
}

/// The stacks of a workbook's regions.
pub fn stacks(wb: &Workbook, regions: &[Region]) -> Vec<Stack> {
    // The tables by what makes them alike.
    type Key = (Vec<String>, u32, u32, u32);
    let mut order: Vec<Key> = Vec::new();
    let mut groups: HashMap<Key, Vec<&Region>> = HashMap::new();
    for r in regions.iter().filter(|r| r.kind == RegionKind::Table && r.header_rows > 0 && r.columns.len() >= 2) {
        let names: Vec<String> = r.columns.iter().map(|c| c.name.to_lowercase()).collect();
        if names.iter().any(|n| n == SHEET) {
            continue;
        }
        let key = (names, r.area.first_row, r.area.first_col, r.header_rows);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(r);
    }
    let mut taken: HashSet<String> = regions.iter().map(|r| r.name.to_lowercase()).collect();
    let mut out = Vec::new();
    for key in order {
        let members = &groups[&key];
        let sheets: HashSet<u32> = members.iter().map(|r| r.area.sheet).collect();
        if members.len() < 2 || sheets.len() != members.len() {
            continue;
        }
        // One kind of data per column (a column without values goes with any).
        let alike = (0..key.0.len()).all(|k| {
            let kinds: HashSet<&str> = members.iter().map(|r| kind_of(&r.columns[k].dtype)).filter(|d| *d != "Null").collect();
            kinds.len() <= 1
        });
        if !alike {
            continue;
        }
        let names: Vec<&str> = members.iter().map(|r| r.name.as_str()).collect();
        let base = common_name(&names).unwrap_or_else(|| "stack".to_string());
        let mut name = base.clone();
        let mut n = 1;
        while !taken.insert(name.to_lowercase()) {
            n += 1;
            name = format!("{base}_{n}");
        }
        let differs = (0..key.0.len())
            .filter_map(|k| {
                let logic: Vec<Option<String>> = members.iter().map(|r| r.columns[k].logic.clone().or_else(|| r.columns[k].formula.clone())).collect();
                logic.iter().any(|l| *l != logic[0]).then(|| (members[0].columns[k].name.clone(), logic))
            })
            .collect();
        out.push(Stack {
            name,
            columns: members[0].columns.iter().map(|c| c.name.clone()).collect(),
            members: members.iter().map(|r| Member { sheet: wb.sheets[r.area.sheet as usize].name.clone(), region: r.name.clone() }).collect(),
            differs,
        });
    }
    out
}

/// The tables of a stack one below the other, the sheet of every row in front. `parts`
/// are the frames of the members, in their order. Columns are those of the stack; a
/// column a part lacks is empty for its rows.
pub fn stacked(stack: &Stack, parts: &[Frame]) -> Result<Frame, String> {
    if parts.len() != stack.members.len() {
        return Err(format!("the stack {:?} has {} tables, {} frames were given", stack.name, stack.members.len(), parts.len()));
    }
    let height: usize = parts.iter().map(|p| p.height).sum();
    let sheets: Vec<Option<String>> = stack.members.iter().zip(parts).flat_map(|(m, p)| std::iter::repeat_n(Some(m.sheet.clone()), p.height)).collect();
    let mut columns = vec![Column::new(SHEET, Data::String(sheets))];
    let find = |p: &Frame, name: &str| p.columns.iter().find(|c| c.name.eq_ignore_ascii_case(name)).map(|c| c.data.clone());
    for name in &stack.columns {
        let data: Vec<Data> = parts.iter().map(|p| find(p, name).unwrap_or(Data::Null(p.height))).collect();
        columns.push(Column::new(name.clone(), Data::concat(&data.iter().collect::<Vec<_>>())));
    }
    Frame::new(height, columns)
}

/// A table of a stack taken apart: the rows of each sheet as the frame of its region, in
/// the order they have. A sheet without rows in the table is left out (its table stays
/// as it is); a sheet the stack does not have is an error.
pub fn unstacked(stack: &Stack, df: &Frame) -> Result<Vec<(String, Frame)>, String> {
    let known: Vec<&str> = stack.members.iter().map(|m| m.sheet.as_str()).collect();
    let sheets = df.column(SHEET).ok_or_else(|| format!("the table for the stack {:?} needs the column {SHEET:?} that says which sheet a row is of; the sheets are {known:?}", stack.name))?;
    let of: Vec<Option<String>> = (0..df.height).map(|k| sheets.data.text(k)).collect();
    if let Some(k) = of.iter().position(|s| !s.as_deref().is_some_and(|s| known.iter().any(|m| m.eq_ignore_ascii_case(s)))) {
        return Err(match &of[k] {
            Some(s) => format!("row {} is of the sheet {s:?}, which the stack {:?} does not have; its sheets are {known:?} (a new sheet is made with add_sheet)", k + 1, stack.name),
            None => format!("row {} names no sheet; the sheets of the stack {:?} are {known:?}", k + 1, stack.name),
        });
    }
    let mut out = Vec::new();
    for m in &stack.members {
        let rows: Vec<usize> = (0..df.height).filter(|&k| of[k].as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&m.sheet))).collect();
        if rows.is_empty() {
            continue;
        }
        let columns: Vec<Column> = df.columns.iter().filter(|c| c.name != SHEET).map(|c| Column::new(c.name.clone(), c.data.take(&rows))).collect();
        out.push((m.region.clone(), Frame::new(rows.len(), columns)?));
    }
    Ok(out)
}
