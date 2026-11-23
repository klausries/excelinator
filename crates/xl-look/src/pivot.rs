//! Where the pivot tables of a workbook lie. Excel draws a pivot table's cells in its
//! pivot table style, which depends on the table's layout (row and column fields,
//! subtotals, grand totals): that is not worked out here, and the attributes it could set
//! are marked as not certain for cells of a pivot table.

use crate::table::parse_range;
use crate::tree;

/// A pivot table: its name, the range it fills and its style.
#[derive(Debug, Clone, PartialEq)]
pub struct PivotArea {
    pub name: String,
    /// First row, first column, last row, last column (0-based, inclusive).
    pub area: (u32, u32, u32, u32),
    pub style: Option<String>,
}

/// The pivot tables of each sheet of an xlsx package, in the order of the workbook's
/// sheets. Unreadable parts give no pivot tables.
pub fn pivot_areas(bytes: &[u8]) -> Vec<Vec<PivotArea>> {
    let Ok(pkg) = xl_opc::Package::open(bytes.to_vec()) else { return Vec::new() };
    let text = |name: &str| pkg.read(name).ok().and_then(|b| xl_opc::decode_xml(&b).ok());
    let Ok(root) = pkg.relationships("") else { return Vec::new() };
    let Some(book) = root.items.iter().find(|r| r.rel_type.ends_with("/officeDocument")).and_then(|r| xl_opc::resolve_target("", &r.target)) else { return Vec::new() };
    let Some(book_xml) = text(&book).and_then(|t| tree::parse(&t).ok()) else { return Vec::new() };
    let Ok(book_rels) = pkg.relationships(&book) else { return Vec::new() };
    let Some(sheets) = book_xml.find("sheets") else { return Vec::new() };
    let mut out = Vec::new();
    for s in sheets.all("sheet") {
        let mut list = Vec::new();
        let part = s.get("id").and_then(|id| book_rels.by_id(id)).and_then(|r| xl_opc::resolve_target(&book, &r.target));
        if let Some(part) = part {
            if let Ok(rels) = pkg.relationships(&part) {
                for r in rels.items.iter().filter(|r| r.rel_type.ends_with("/pivotTable")) {
                    let Some(p) = xl_opc::resolve_target(&part, &r.target) else { continue };
                    let Some(def) = text(&p).and_then(|t| tree::parse(&t).ok()) else { continue };
                    let Some(d) = def.child("pivotTableDefinition") else { continue };
                    let Some(loc) = d.child("location") else { continue };
                    let Some(mut area) = loc.get("ref").and_then(parse_range) else { continue };
                    // Page fields stand above the table, one row each, then an empty row.
                    let pages: u32 = loc.get("rowPageCount").and_then(|v| v.parse().ok()).unwrap_or(0);
                    if pages > 0 {
                        area.0 = area.0.saturating_sub(pages + 1);
                    }
                    list.push(PivotArea {
                        name: d.get("name").unwrap_or("").to_string(),
                        area,
                        style: d.child("pivotTableStyleInfo").and_then(|i| i.get("name")).map(str::to_string),
                    });
                }
            }
        }
        out.push(list);
    }
    out
}
