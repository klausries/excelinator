//! Table styles: the built-in ones of ECMA-376 and a file's own, and which of their
//! elements reach a cell of a table.

use crate::parts::{self, Partial, Raw};
use crate::tree::{self, Node};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One element of a table style: its type, stripe size and what it sets.
#[derive(Debug, Clone)]
pub struct Element {
    pub kind: String,
    pub size: u32,
    pub look: Partial,
}

static PRESETS: OnceLock<HashMap<String, Vec<Element>>> = OnceLock::new();

/// The built-in table and pivot styles by name (`TableStyleMedium9`, …).
pub fn presets() -> &'static HashMap<String, Vec<Element>> {
    PRESETS.get_or_init(|| {
        let xml = include_str!("../data/presetTableStyles.xml");
        let mut out = HashMap::new();
        let Ok(root) = tree::parse(xml) else { return out };
        let Some(all) = root.child("presetTableStyles") else { return out };
        for style in &all.children {
            let dxfs: Vec<&Node> = style.child("dxfs").map(|d| d.all("dxf").collect()).unwrap_or_default();
            let Some(ts) = style.find("tableStyle") else { continue };
            // The file numbers the formats of table styles from 1, those of pivot styles
            // from 0: an index equal to the count marks the former.
            let ids: Vec<usize> = ts.all("tableStyleElement").filter_map(|e| e.get("dxfId").and_then(|v| v.parse::<usize>().ok())).collect();
            let base = usize::from(ids.contains(&dxfs.len()) && !ids.contains(&0));
            let elements = ts
                .all("tableStyleElement")
                .map(|e| Element {
                    kind: e.get("type").unwrap_or("").to_string(),
                    size: e.get("size").and_then(|v| v.parse().ok()).unwrap_or(1),
                    look: e.get("dxfId").and_then(|v| v.parse::<usize>().ok()).and_then(|i| i.checked_sub(base)).and_then(|i| dxfs.get(i)).map(|d| dxf_look(d)).unwrap_or_default(),
                })
                .collect();
            out.insert(style.name.clone(), elements);
        }
        out
    })
}

fn dxf_look(n: &Node) -> Partial {
    let mut p = Partial::new();
    if let Some(f) = n.child("font") {
        parts::font(f, false, &mut p);
    }
    if let Some(f) = n.child("fill") {
        parts::fill(f, true, &mut p);
    }
    if let Some(b) = n.child("border") {
        parts::border(b, false, &mut p);
    }
    p
}

/// The elements of a style of the file (`Workbook::table_styles`).
pub fn own(s: &xl_cells::TableStyle) -> Vec<Element> {
    s.elements.iter().map(|(kind, size, xml)| Element { kind: kind.clone(), size: size.unwrap_or(1), look: parts::of_dxf(xml) }).collect()
}

/// A table of a sheet, as far as its look goes.
#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    /// First row, first column, last row, last column (0-based, inclusive).
    pub area: (u32, u32, u32, u32),
    pub header_rows: u32,
    pub total_rows: u32,
    pub style: Option<String>,
    pub first_column: bool,
    pub last_column: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
}

pub fn parse_range(r: &str) -> Option<(u32, u32, u32, u32)> {
    let r = r.trim().replace('$', "");
    let (a, b) = r.split_once(':').unwrap_or((&r, &r));
    let a = xl_formula::parse_cell_ref(a)?;
    let b = xl_formula::parse_cell_ref(b)?;
    Some((a.row.min(b.row) - 1, a.col.min(b.col) - 1, a.row.max(b.row) - 1, a.col.max(b.col) - 1))
}

impl Table {
    /// From the XML of a table part.
    pub fn of(xml: &str) -> Option<Table> {
        let root = tree::parse(xml).ok()?;
        let t = root.child("table")?;
        let area = parse_range(t.get("ref")?)?;
        let num = |k: &str, d: u32| t.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let info = t.child("tableStyleInfo");
        let flag = |k: &str| info.and_then(|i| i.bool_attr(k)).unwrap_or(false);
        Some(Table {
            name: t.get("displayName").or(t.get("name")).unwrap_or("").to_string(),
            area,
            header_rows: num("headerRowCount", 1),
            total_rows: num("totalsRowCount", 0),
            style: info.and_then(|i| i.get("name")).map(str::to_string),
            first_column: flag("showFirstColumn"),
            last_column: flag("showLastColumn"),
            row_stripes: flag("showRowStripes"),
            column_stripes: flag("showColumnStripes"),
        })
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        let (r0, c0, r1, c1) = self.area;
        (r0..=r1).contains(&row) && (c0..=c1).contains(&col)
    }

    /// The part of the table an element of this type covers and that holds the cell, if
    /// the element applies to the cell (ECMA-376 Part 1 §18.8.41; stripes run over the
    /// data rows and columns between header and total rows).
    pub fn region(&self, kind: &str, size_of: &dyn Fn(&str) -> u32, row: u32, col: u32) -> Option<(u32, u32, u32, u32)> {
        let (r0, c0, r1, c1) = self.area;
        let body0 = r0 + self.header_rows;
        let body1 = r1.checked_sub(self.total_rows)?;
        let stripe = |start: u32, at: u32, first: u32, second: u32, want_first: bool| -> Option<(u32, u32)> {
            if at < start {
                return None;
            }
            let period = first + second;
            let k = (at - start) / period;
            let p = (at - start) % period;
            let s = start + k * period;
            match (p < first, want_first) {
                (true, true) => Some((s, s + first - 1)),
                (false, false) => Some((s + first, s + period - 1)),
                _ => None,
            }
        };
        let r = match kind {
            "wholeTable" => Some(self.area),
            "headerRow" if self.header_rows > 0 => Some((r0, c0, r0 + self.header_rows - 1, c1)),
            "totalRow" if self.total_rows > 0 => Some((r1 + 1 - self.total_rows, c0, r1, c1)),
            "firstColumn" if self.first_column => Some((r0, c0, r1, c0)),
            "lastColumn" if self.last_column => Some((r0, c1, r1, c1)),
            "firstHeaderCell" if self.header_rows > 0 => Some((r0, c0, r0, c0)),
            "lastHeaderCell" if self.header_rows > 0 => Some((r0, c1, r0, c1)),
            "firstTotalCell" if self.total_rows > 0 => Some((r1, c0, r1, c0)),
            "lastTotalCell" if self.total_rows > 0 => Some((r1, c1, r1, c1)),
            "firstRowStripe" | "secondRowStripe" if self.row_stripes && (body0..=body1).contains(&row) => {
                let (a, b) = stripe(body0, row, size_of("firstRowStripe").max(1), size_of("secondRowStripe").max(1), kind == "firstRowStripe")?;
                Some((a, c0, b.min(body1), c1))
            }
            "firstColumnStripe" | "secondColumnStripe" if self.column_stripes && (body0..=body1).contains(&row) => {
                let (a, b) = stripe(c0, col, size_of("firstColumnStripe").max(1), size_of("secondColumnStripe").max(1), kind == "firstColumnStripe")?;
                Some((body0, a, body1, b.min(c1)))
            }
            _ => None,
        }?;
        ((r.0..=r.2).contains(&row) && (r.1..=r.3).contains(&col)).then_some(r)
    }
}

/// The order in which table style elements apply, later ones over earlier ones
/// (ECMA-376 Part 1 §18.8.41).
pub const ORDER: [&str; 13] = [
    "wholeTable",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstRowStripe",
    "secondRowStripe",
    "lastColumn",
    "firstColumn",
    "headerRow",
    "totalRow",
    "firstHeaderCell",
    "lastHeaderCell",
    "firstTotalCell",
    "lastTotalCell",
];

/// What an element sets for a cell of its region: font and fill as given, of the borders
/// the outer ones on the region's edges and the inner ones (`vertical`, `horizontal`)
/// between its cells.
pub fn element_for_cell(look: &Partial, region: (u32, u32, u32, u32), row: u32, col: u32) -> Partial {
    let mut p = Partial::new();
    for (k, v) in look {
        if !k.starts_with("border_") {
            p.insert(k, v.clone());
        }
    }
    let (r0, c0, r1, c1) = region;
    let side = |outer: usize, inner: usize, on_edge: bool| if on_edge { outer } else { inner };
    for (target, from) in [(0, side(0, 5, col == c0)), (1, side(1, 5, col == c1)), (2, side(2, 6, row == r0)), (3, side(3, 6, row == r1))] {
        let (sk, ck) = parts::SIDE_KEYS[from];
        if let Some(v) = look.get(sk) {
            let (tk, tck) = parts::SIDE_KEYS[target];
            p.insert(tk, v.clone());
            p.insert(tck, look.get(ck).cloned().unwrap_or(Raw::Nothing));
        }
    }
    p
}
