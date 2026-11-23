//! xlsx → [`Workbook`].

use crate::model::*;
use crate::xml::{unescape_xstring, walk, Attrs, Capture, Ev};
use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use xl_formula::Expr;
use xl_opc::{resolve_target, Package};

pub fn read_bytes(bytes: Vec<u8>) -> Result<Workbook> {
    read(&Package::open(bytes)?)
}

fn part_text(pkg: &Package, name: &str) -> Result<String> {
    Ok(xl_opc::decode_xml(&pkg.read(name)?)?)
}

fn target(source: &str, t: &str) -> Result<String> {
    resolve_target(source, t).ok_or_else(|| Error::Invalid(format!("bad relationship target {t:?}")))
}

pub fn read(pkg: &Package) -> Result<Workbook> {
    let root = pkg.relationships("")?;
    let wb_rel = root.items.iter().find(|r| r.rel_type.ends_with("/officeDocument")).ok_or_else(|| Error::Invalid("no workbook part".into()))?;
    let wb_part = resolve_target("", &wb_rel.target).ok_or_else(|| Error::Invalid("bad workbook target".into()))?;
    let rels = pkg.relationships(&wb_part)?;

    let mut wb = Workbook::default();
    let mut sheet_rids: Vec<String> = Vec::new();
    let mut external_rids: Vec<String> = Vec::new();
    let mut name: Option<(String, Option<u32>, String, bool)> = None;
    let mut depth = 0usize;
    let wb_xml = part_text(pkg, &wb_part)?;
    walk(&wb_xml, &mut |ev| {
        match &ev {
            Ev::Start(..) => depth += 1,
            Ev::End(_) => depth -= 1,
            Ev::Text(_) => {}
        }
        match ev {
            Ev::Start("externalReference", a) => external_rids.push(a.get("id").unwrap_or("").to_string()),
            Ev::Start("sheet", a) => {
                let visibility = match a.get("state") {
                    Some("hidden") => Visibility::Hidden,
                    Some("veryHidden") => Visibility::VeryHidden,
                    _ => Visibility::Visible,
                };
                wb.sheets.push(Sheet { name: unescape_xstring(a.get("name").unwrap_or("")), visibility, origin: Some(wb.sheets.len() as u32), ..Sheet::default() });
                sheet_rids.push(a.get("id").unwrap_or("").to_string());
            }
            // Only the workbook's own element counts; extension lists may hold others of that name.
            Ev::Start("workbookPr", a) if depth == 2 => wb.date1904 = matches!(a.get("date1904"), Some("1") | Some("true")),
            Ev::Start("definedName", a) => name = Some((unescape_xstring(a.get("name").unwrap_or("")), a.get("localSheetId").and_then(|v| v.parse().ok()), String::new(), matches!(a.get("hidden"), Some("1") | Some("true")))),
            Ev::Text(t) => {
                if let Some(n) = name.as_mut() {
                    n.2.push_str(t)
                }
            }
            Ev::End("definedName") => {
                if let Some((name, local_sheet, value, hidden)) = name.take() {
                    wb.defined_names.push(DefinedName { hidden, name, local_sheet, value: unescape_xstring(&value) });
                }
            }
            _ => {}
        }
        Ok(())
    })?;

    for rid in &external_rids {
        if let Some(r) = rels.by_id(rid) {
            let part = target(&wb_part, &r.target)?;
            if pkg.contains(&part) {
                // A broken part costs the cached values, not the workbook.
                let mut ext = part_text(pkg, &part).and_then(|t| external(&t)).unwrap_or_default();
                ext.target = pkg.relationships(&part)?.items.iter().find(|x| x.rel_type.ends_with("/externalLinkPath")).map(|x| x.target.clone());
                wb.externals.push(ext);
                continue;
            }
        }
        wb.externals.push(External::default());
    }

    // Children of <workbook> kept as XML, unless they say no more than the model.
    wb.extra = crate::xml::children_raw(&wb_xml, crate::write::WORKBOOK_EXTRA)?;
    let plain_pr = if wb.date1904 { "<workbookPr date1904=\"1\"/>" } else { "<workbookPr/>" };
    wb.extra.retain(|(n, x)| !(n == "workbookPr" && x == plain_pr));
    // Document properties.
    for (kind, part_name, rel_type, _) in crate::write::PROPERTY_PARTS {
        // Some writers leave out the relationship; the part is then found at its usual place.
        let by_rel = root.items.iter().find(|r| r.rel_type == *rel_type || r.rel_type.ends_with(rel_type.rsplit('/').next().unwrap_or(""))).and_then(|r| resolve_target("", &r.target));
        if let Some(part) = by_rel.or_else(|| Some(part_name.to_string())).filter(|p| pkg.contains(p)) {
            let text = part_text(pkg, &part)?;
            let body = match text.trim_start().strip_prefix("<?xml") {
                Some(rest) => rest.split_once("?>").map(|r| r.1).unwrap_or("").trim_start().to_string(),
                None => text.trim_start().to_string(),
            };
            wb.properties.push((kind.to_string(), body));
        }
    }

    // The theme: what theme colours and fonts stand for.
    if let Some(r) = rels.items.iter().find(|r| r.rel_type.ends_with("/theme")) {
        let part = target(&wb_part, &r.target)?;
        if pkg.contains(&part) {
            // Without its XML declaration, which the writer adds again.
            wb.theme = part_text(pkg, &part).ok().map(|t| {
                let body = match t.trim_start().strip_prefix("<?xml") {
                    Some(rest) => rest.split_once("?>").map(|r| r.1).unwrap_or("").trim_start(),
                    None => t.trim_start(),
                };
                Arc::from(body)
            });
        }
    }
    let strings = match rels.items.iter().find(|r| r.rel_type.ends_with("/sharedStrings")) {
        Some(r) => shared_strings(&part_text(pkg, &target(&wb_part, &r.target)?)?)?,
        None => Vec::new(),
    };
    let mut dxfs: Vec<String> = Vec::new();
    if let Some(r) = rels.items.iter().find(|r| r.rel_type.ends_with("/styles")) {
        let part = target(&wb_part, &r.target)?;
        if pkg.contains(&part) {
            let sheet_styles = styles(&part_text(pkg, &part)?)?;
            wb.styles = sheet_styles.styles;
            wb.named_styles = sheet_styles.named_styles;
            wb.palette = sheet_styles.palette;
            wb.table_styles = sheet_styles.table_styles;
            wb.default_table_style = sheet_styles.default_table_style;
            wb.default_pivot_style = sheet_styles.default_pivot_style;
            dxfs = sheet_styles.dxfs;
        }
    }

    for (i, rid) in sheet_rids.iter().enumerate() {
        let Some(rel) = rels.by_id(rid) else {
            wb.sheets[i].kind = SheetKind::Other;
            continue;
        };
        if !rel.rel_type.ends_with("/worksheet") {
            wb.sheets[i].kind = if rel.rel_type.ends_with("/chartsheet") { SheetKind::Chartsheet } else { SheetKind::Other };
            continue;
        }
        let part = target(&wb_part, &rel.target)?;
        let xml = part_text(pkg, &part)?;
        let style_count = wb.styles.len() as u32;
        let date1904 = wb.date1904;
        sheet(&xml, i as u32, &strings, style_count, &dxfs, date1904, &mut wb.sheets[i], &mut wb.cells)?;
        let mut extra = crate::xml::children_raw(&xml, crate::write::EXTRA_ELEMENTS)?;
        // A sheet view or filter that says no more than the frozen panes or the filter
        // range is not kept twice.
        let s = &wb.sheets[i];
        let (views, filter) = (crate::write::freeze_views_xml(s.freeze), crate::write::auto_filter_xml(s));
        extra.retain(|(n, x)| !(n == "sheetViews" && Some(x) == views.as_ref() || n == "autoFilter" && *x == filter));
        wb.sheets[i].extra = extra;
        // External hyperlink addresses and the comments live behind the sheet's relationships.
        let sheet_rels = pkg.relationships(&part)?;
        for h in &mut wb.sheets[i].hyperlinks {
            h.target = h.target.take().and_then(|id| sheet_rels.by_id(&id).map(|r| r.target.clone()));
        }
        if let Some(drawing_part) = sheet_rels.items.iter().find(|r| r.rel_type.ends_with("/drawing")).and_then(|r| resolve_target(&part, &r.target)).filter(|p| pkg.contains(p)) {
            wb.sheets[i].drawings = drawings(pkg, &drawing_part).unwrap_or_default();
        }
        if let Some(r) = sheet_rels.items.iter().find(|r| r.rel_type.ends_with("/comments")) {
            if let Some(comments_part) = resolve_target(&part, &r.target).filter(|p| pkg.contains(p)) {
                wb.sheets[i].comments = comments(&part_text(pkg, &comments_part)?)?;
            }
        }
        // Tables in the order of the sheet's <tableParts>.
        for id in table_part_ids(&xml)? {
            if let Some(table_part) = sheet_rels.by_id(&id).filter(|r| r.rel_type.ends_with("/table")).and_then(|r| resolve_target(&part, &r.target)).filter(|p| pkg.contains(p)) {
                let text = part_text(pkg, &table_part)?;
                let body = match text.trim_start().strip_prefix("<?xml") {
                    Some(rest) => rest.split_once("?>").map(|r| r.1).unwrap_or("").trim_start().to_string(),
                    None => text.trim_start().to_string(),
                };
                wb.sheets[i].tables.push(body);
            }
        }
    }
    // The request for recalculation the writer adds anyway (formulas without values) is
    // not kept in calcPr.
    let c = &wb.cells;
    if (0..c.len()).any(|i| c.formula[i].as_deref().is_some_and(|f| !f.is_empty()) && c.kind[i] == Kind::Blank) {
        for e in wb.extra.iter_mut().filter(|e| e.0 == "calcPr") {
            e.1 = e.1.replacen(" fullCalcOnLoad=\"1\"", "", 1).replacen(" fullCalcOnLoad=\"true\"", "", 1);
        }
        wb.extra.retain(|(n, x)| !(n == "calcPr" && x == "<calcPr/>"));
    }
    // Tabs past the last sheet (some writers leave them after removing sheets) point to the first.
    let n = wb.sheets.len();
    for e in wb.extra.iter_mut().filter(|e| e.0 == "bookViews") {
        e.1 = crate::write::clamp_tabs(&e.1, n);
    }
    // Likewise the plain window the writer adds for frozen panes.
    if wb.sheets.iter().any(|s| s.freeze.is_some()) {
        wb.extra.retain(|(n, x)| !(n == "bookViews" && x == "<bookViews><workbookView/></bookViews>"));
    }
    Ok(wb)
}

/// The relationship ids of a worksheet's `<tablePart>` elements.
fn table_part_ids(xml: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut inside = false;
    walk(xml, &mut |ev| {
        match ev {
            Ev::Start("tableParts", _) => inside = true,
            Ev::End("tableParts") => inside = false,
            Ev::Start("tablePart", a) if inside => {
                if let Some(id) = a.get("id") {
                    out.push(id.to_string());
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(out)
}

/// Collects the text of `<t>` elements of a string item, skipping phonetic runs.
#[derive(Default)]
struct StringItem {
    text: String,
    in_t: bool,
    in_phonetic: bool,
}

impl StringItem {
    fn event(&mut self, ev: &Ev) {
        match ev {
            Ev::Start("rPh", _) => self.in_phonetic = true,
            Ev::End("rPh") => self.in_phonetic = false,
            Ev::Start("t", _) => self.in_t = !self.in_phonetic,
            Ev::End("t") => self.in_t = false,
            Ev::Text(t) if self.in_t => self.text.push_str(t),
            _ => {}
        }
    }

    fn take(&mut self) -> String {
        self.in_t = false;
        self.in_phonetic = false;
        unescape_xstring(&std::mem::take(&mut self.text))
    }
}

/// Remove an element (with any prefix) from XML text.
fn remove_element(xml: &str, local: &str) -> String {
    let mut out = xml.to_string();
    while let Some(start) = out.match_indices('<').map(|m| m.0).find(|&i| {
        let rest = &out[i + 1..];
        let name_end = rest.find([' ', '>', '/']).unwrap_or(rest.len());
        let name = &rest[..name_end];
        name == local || name.ends_with(&format!(":{local}"))
    }) {
        let tag_end = out[start..].find('>').map(|i| start + i + 1).unwrap_or(out.len());
        let end = if out[start..tag_end].ends_with("/>") {
            tag_end
        } else {
            let name = out[start + 1..].split([' ', '>']).next().unwrap_or("").to_string();
            let close = format!("</{name}>");
            out[tag_end..].find(&close).map(|i| tag_end + i + close.len()).unwrap_or(tag_end)
        };
        out.replace_range(start..end, "");
    }
    out
}

/// The pictures and charts of a drawing part. Empty if it holds anything else.
fn drawings(pkg: &Package, part: &str) -> Result<Vec<Drawing>> {
    let rels = pkg.relationships(part)?;
    let xml = part_text(pkg, part)?;
    // (anchor, relationship id of a picture, relationship id of a chart)
    let mut found: Vec<(Anchor, Option<String>, Option<String>)> = Vec::new();
    let mut supported = true;
    let mut path: Vec<String> = Vec::new();
    let mut point = [String::new(), String::new(), String::new(), String::new()];
    let mut slot: Option<usize> = None;
    walk(&xml, &mut |ev| {
        match ev {
            Ev::Start(name, a) => {
                let parent = path.last().map(String::as_str).unwrap_or("");
                match (path.len(), name) {
                    (1, "oneCellAnchor") | (1, "twoCellAnchor") => found.push((Anchor::default(), None, None)),
                    (1, _) => supported = false,
                    (2, "ext") => {
                        let n = |k: &str| a.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                        if let Some(f) = found.last_mut() {
                            f.0.ext = Some((n("cx"), n("cy")));
                        }
                    }
                    (2, "from") | (2, "to") => point.iter_mut().for_each(String::clear),
                    (2, "pic") | (2, "graphicFrame") | (2, "clientData") => {}
                    (2, _) => supported = false,
                    (3, _) if parent == "from" || parent == "to" => slot = ["col", "colOff", "row", "rowOff"].iter().position(|k| *k == name),
                    (_, "blip") => {
                        if let Some(f) = found.last_mut() {
                            f.1 = a.get("embed").map(str::to_string);
                            supported &= a.get("link").is_none() && f.1.is_some();
                        }
                    }
                    (_, "chart") if path.iter().any(|p| p == "graphicFrame") => {
                        if let Some(f) = found.last_mut() {
                            f.2 = a.get("id").map(str::to_string);
                        }
                    }
                    _ => {}
                }
                path.push(name.to_string());
            }
            Ev::Text(t) => {
                if let Some(i) = slot {
                    point[i].push_str(t);
                }
            }
            Ev::End(name) => {
                path.pop();
                slot = None;
                if path.len() == 2 && (name == "from" || name == "to") {
                    let n = |i: usize| point[i].trim().parse::<i64>().unwrap_or(0);
                    let p = (n(0).max(0) as u32, n(1), n(2).max(0) as u32, n(3));
                    if let Some(f) = found.last_mut() {
                        if name == "from" {
                            f.0.from = p;
                        } else {
                            f.0.to = Some(p);
                        }
                    }
                }
            }
        }
        Ok(())
    })?;
    let mut out = Vec::new();
    for (anchor, picture, chart) in found {
        let target = |id: &String| rels.by_id(id).and_then(|r| resolve_target(part, &r.target)).filter(|p| pkg.contains(p));
        match (picture.as_ref().and_then(target), chart.as_ref().and_then(target)) {
            (Some(p), None) => {
                let ext = p.rsplit_once('.').map(|e| e.1.to_lowercase()).unwrap_or_default();
                out.push(Drawing { anchor, image: Some((Arc::from(pkg.read(&p)?), ext)), chart: None });
            }
            (None, Some(p)) => {
                // References to parts behind the chart (embedded workbook, shapes) cannot be carried along.
                let text = remove_element(&remove_element(&part_text(pkg, &p)?, "externalData"), "userShapes");
                out.push(Drawing { anchor, image: None, chart: Some(Arc::from(text)) });
            }
            _ => supported = false,
        }
    }
    Ok(if supported { out } else { Vec::new() })
}

/// The cells an external workbook is remembered by.
fn external(xml: &str) -> Result<External> {
    let mut out = External::default();
    let mut sheet = 0u32;
    let mut cell: Option<(u32, u32, String, String)> = None;
    let mut in_value = false;
    walk(xml, &mut |ev| {
        match ev {
            Ev::Start("sheetName", a) => out.sheets.push(unescape_xstring(a.get("val").unwrap_or(""))),
            Ev::Start("definedName", a) => {
                if let (Some(n), Some(r)) = (a.get("name"), a.get("refersTo")) {
                    out.names.push((unescape_xstring(n), unescape_xstring(r)));
                }
            }
            Ev::Start("sheetData", a) => sheet = a.get("sheetId").and_then(|v| v.parse().ok()).unwrap_or(0),
            Ev::Start("cell", a) => {
                if let Some((row, col)) = a.get("r").and_then(parse_a1) {
                    cell = Some((row, col, a.get("t").unwrap_or("n").to_string(), String::new()));
                }
            }
            Ev::Start("v", _) => in_value = true,
            Ev::Text(t) if in_value => {
                if let Some(c) = cell.as_mut() {
                    c.3.push_str(t)
                }
            }
            Ev::End("v") => in_value = false,
            Ev::End("cell") => {
                if let Some((row, col, ty, v)) = cell.take() {
                    let value = match ty.as_str() {
                        "str" | "s" | "inlineStr" => Value::Text(unescape_xstring(&v)),
                        "b" => Value::Bool(v.trim() == "1" || v.trim().eq_ignore_ascii_case("true")),
                        "e" => Value::Error(v),
                        _ => match v.trim().parse::<f64>() {
                            Ok(n) if n.is_finite() => Value::Number(n),
                            _ => Value::Blank,
                        },
                    };
                    if !matches!(value, Value::Blank) {
                        out.cells.push((sheet, row, col, value));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(out)
}

fn comments(xml: &str) -> Result<Vec<Comment>> {
    let mut authors: Vec<String> = Vec::new();
    let mut out: Vec<Comment> = Vec::new();
    let mut item = StringItem::default();
    let mut in_author = false;
    let mut author_id = 0usize;
    walk(xml, &mut |ev| {
        match ev {
            Ev::Start("author", _) => {
                in_author = true;
                authors.push(String::new());
            }
            Ev::End("author") => in_author = false,
            Ev::Text(t) if in_author => authors.last_mut().unwrap().push_str(t),
            Ev::Start("comment", a) => {
                author_id = a.get("authorId").and_then(|v| v.parse().ok()).unwrap_or(0);
                out.push(Comment { cell: a.get("ref").unwrap_or("").to_string(), author: String::new(), text: String::new() });
            }
            Ev::End("comment") => {
                if let Some(c) = out.last_mut() {
                    c.text = item.take();
                    c.author = authors.get(author_id).map(|a| unescape_xstring(a)).unwrap_or_default();
                }
            }
            ev => item.event(&ev),
        }
        Ok(())
    })?;
    Ok(out)
}

/// A shared or inline string: its plain text, and its XML where it says more (runs).
pub(crate) type StringEntry = (Arc<str>, Option<Arc<str>>);

/// The plain text of a string item's content given as XML (the runs' `<t>`, without
/// phonetic guides), as a reader sees it.
pub fn rich_text(xml: &str) -> Result<String> {
    let mut item = StringItem::default();
    walk(&format!("<si>{xml}</si>"), &mut |ev| {
        item.event(&ev);
        Ok(())
    })?;
    Ok(item.take())
}

/// The XML of a string item's content, if it is more than one plain `<t>`.
fn rich_xml(xml: &str) -> Option<Arc<str>> {
    let starts = xml.match_indices('<').filter(|(i, _)| !xml[i + 1..].starts_with('/')).count();
    let plain = starts == 1 && xml.starts_with("<t") && xml[2..].starts_with(['>', ' ', '/']);
    (!plain && !xml.is_empty()).then(|| Arc::from(xml))
}

fn shared_strings(xml: &str) -> Result<Vec<StringEntry>> {
    let mut out = Vec::new();
    let mut item = StringItem::default();
    let mut raw = Capture::default();
    walk(xml, &mut |ev| {
        if raw.active() {
            if raw.event(&ev) {
                out.push((Arc::from(item.take()), rich_xml(&raw.xml)));
            } else {
                item.event(&ev);
            }
            return Ok(());
        }
        if let Ev::Start("si", _) = ev {
            raw.begin();
        }
        Ok(())
    })?;
    Ok(out)
}

fn argb(a: &Attrs) -> Option<String> {
    a.get("rgb").filter(|c| c.len() == 8).map(|c| c.to_uppercase())
}

#[derive(Default, Clone)]
struct FontDef {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: Option<String>,
    name: Option<String>,
    size: Option<f64>,
    color: Option<String>,
}

pub(crate) struct StyleSheet {
    pub styles: Vec<Style>,
    /// Custom number formats (id, code) as declared.
    pub num_formats: Vec<(u32, String)>,
    /// Differential formats (used by conditional formatting), each as XML.
    pub dxfs: Vec<String>,
    /// The named cell styles, empty when the file has `Normal` alone.
    pub named_styles: Vec<NamedStyle>,
    /// (name, index in `cellStyleXfs`) of the named styles as the file lists them.
    pub style_xf_ids: Vec<(String, usize)>,
    /// Number of `cellStyleXfs` entries.
    pub style_xf_count: usize,
    pub palette: Vec<String>,
    pub table_styles: Vec<TableStyle>,
    pub default_table_style: Option<String>,
    pub default_pivot_style: Option<String>,
}

/// XML in the form `Capture` writes it (start and end tags, attributes in double
/// quotes), for comparing what was read with what the writer would write.
fn normalized(xml: &str) -> Result<String> {
    let mut cap = Capture::default();
    let mut out = String::new();
    let mut started = false;
    walk(xml, &mut |ev| {
        if !started {
            if let Ev::Start(name, a) = &ev {
                out.push_str(&open_tag(name, a));
                cap.begin();
                started = true;
            }
            return Ok(());
        }
        if cap.active() && cap.event(&ev) {
            out.push_str(&cap.xml);
        }
        Ok(())
    })?;
    let name = out.trim_start_matches('<').split(|c: char| c == ' ' || c == '>').next().unwrap_or("").to_string();
    out.push_str(&format!("</{name}>"));
    Ok(out)
}

fn open_tag(name: &str, a: &crate::xml::Attrs) -> String {
    let mut x = format!("<{name}");
    for (k, v) in a.own() {
        x.push_str(&format!(" {k}=\"{}\"", crate::xml::escape_attr_plain(v)));
    }
    x.push('>');
    x
}

/// One `<xf>` of `cellXfs` or `cellStyleXfs`, as far as it has been read.
#[derive(Default, Clone)]
struct XfDef {
    num_format: String,
    font: Option<usize>,
    fill: Option<usize>,
    border: Option<usize>,
    xf_id: Option<usize>,
    alignment: Option<(String, Vec<(String, String)>)>,
    protection: Option<String>,
    attrs: Vec<(String, String)>,
}

pub(crate) fn styles(xml: &str) -> Result<StyleSheet> {
    let mut formats: HashMap<u32, String> = HashMap::new();
    let mut declared: Vec<(u32, String)> = Vec::new();
    let mut fonts: Vec<FontDef> = Vec::new();
    let mut fills: Vec<Option<String>> = Vec::new();
    let mut solid = false;
    let mut borders: Vec<[Option<Border>; 4]> = Vec::new();
    let mut side: Option<usize> = None;
    let mut raw_fonts: Vec<String> = Vec::new();
    let mut raw_fills: Vec<String> = Vec::new();
    let mut raw_borders: Vec<String> = Vec::new();
    let mut part = Capture::default();
    let mut part_open = String::new();
    let mut cell_xfs: Vec<XfDef> = Vec::new();
    let mut style_xfs: Vec<XfDef> = Vec::new();
    let mut cell_styles: Vec<(String, usize, Option<u32>, bool, bool, Option<u32>)> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut dxfs: Vec<String> = Vec::new();
    let mut dxf = Capture::default();
    let mut palette: Vec<String> = Vec::new();
    let mut table_styles: Vec<(String, bool, bool, Vec<(String, Option<u32>, Option<usize>)>)> = Vec::new();
    let (mut default_table_style, mut default_pivot_style) = (None, None);
    let mut choice = 0usize;
    walk(xml, &mut |ev| {
        // Markup compatibility: the fallback counts, the choice (an extension) does not, and
        // the wrapper is no element of its own.
        match &ev {
            Ev::Start("Choice", _) => {
                choice += 1;
                return Ok(());
            }
            Ev::End("Choice") => {
                choice -= 1;
                return Ok(());
            }
            _ if choice > 0 => {
                if let Ev::Start(..) = ev {
                    choice += 1;
                } else if let Ev::End(_) = ev {
                    choice -= 1;
                }
                return Ok(());
            }
            Ev::Start("AlternateContent" | "Fallback", _) | Ev::End("AlternateContent" | "Fallback") => return Ok(()),
            _ => {}
        }
        if dxf.active() {
            if dxf.event(&ev) {
                dxfs.push(std::mem::take(&mut dxf.xml));
            }
            return Ok(());
        }
        // A font, fill or border is also kept as XML.
        if part.active() && part.event(&ev) {
            let name = path.last().cloned().unwrap_or_default();
            // Markup-compatibility branches lose their namespace in the capture: such a
            // part is kept by its fields only.
            let x = if part.xml.contains("<AlternateContent") { String::new() } else { format!("{part_open}{}</{name}>", part.xml) };
            match name.as_str() {
                "font" => raw_fonts.push(x),
                "fill" => raw_fills.push(x),
                _ => raw_borders.push(x),
            }
        }
        match ev {
            Ev::Start("dxf", _) if path.last().map(String::as_str) == Some("dxfs") && path.len() == 2 => dxf.begin(),
            Ev::Start(name, a) => {
                let parent = path.last().map(String::as_str).unwrap_or("");
                let grand = path.len().checked_sub(2).map(|i| path[i].as_str()).unwrap_or("");
                if path.len() == 2 && matches!((parent, name), ("fonts", "font") | ("fills", "fill") | ("borders", "border")) {
                    part.begin();
                    part_open = open_tag(name, a);
                }
                match (grand, parent, name) {
                    ("colors", "indexedColors", "rgbColor") => palette.push(a.get("rgb").unwrap_or("FF000000").to_ascii_uppercase()),
                    (_, "styleSheet", "tableStyles") => {
                        default_table_style = a.get("defaultTableStyle").map(str::to_string);
                        default_pivot_style = a.get("defaultPivotStyle").map(str::to_string);
                    }
                    (_, "tableStyles", "tableStyle") => {
                        let flag = |k: &str| a.get(k).map(|v| !matches!(v, "0" | "false"));
                        table_styles.push((
                            a.get("name").unwrap_or("").to_string(),
                            flag("table").unwrap_or(true),
                            flag("pivot").unwrap_or(true),
                            Vec::new(),
                        ));
                    }
                    ("tableStyles", "tableStyle", "tableStyleElement") => {
                        if let Some(t) = table_styles.last_mut() {
                            let num = |k: &str| a.get(k).and_then(|v| v.parse::<usize>().ok());
                            t.3.push((a.get("type").unwrap_or("").to_string(), num("size").map(|v| v as u32), num("dxfId")));
                        }
                    }
                    (_, "numFmts", "numFmt") => {
                        if let Some(id) = a.get("numFmtId").and_then(|v| v.parse().ok()) {
                            let code = unescape_xstring(a.get("formatCode").unwrap_or(""));
                            declared.push((id, code.clone()));
                            formats.insert(id, code);
                        }
                    }
                    (_, "fonts", "font") => fonts.push(FontDef::default()),
                    ("fonts", "font", _) => {
                        let f = fonts.last_mut().unwrap();
                        match name {
                            "b" => f.bold = a.flag(),
                            "i" => f.italic = a.flag(),
                            "strike" => f.strike = a.flag(),
                            "u" => f.underline = Some(a.get("val").unwrap_or("single").to_string()).filter(|v| v != "none"),
                            "name" => f.name = a.get("val").map(unescape_xstring),
                            "sz" => f.size = a.get("val").and_then(|v| v.parse().ok()),
                            "color" => f.color = argb(a),
                            _ => {}
                        }
                    }
                    (_, "fills", "fill") => fills.push(None),
                    ("fills", "fill", "patternFill") => solid = a.get("patternType") == Some("solid"),
                    ("fill", "patternFill", "fgColor") if solid => {
                        if let Some(f) = fills.last_mut() {
                            *f = argb(a);
                        }
                    }
                    (_, "borders", "border") => borders.push([None, None, None, None]),
                    ("borders", "border", _) => {
                        side = ["left", "right", "top", "bottom"].iter().position(|s| *s == name).or(match name {
                            "start" => Some(0),
                            "end" => Some(1),
                            _ => None,
                        });
                        if let (Some(i), Some(style)) = (side, a.get("style").filter(|s| *s != "none")) {
                            borders.last_mut().unwrap()[i] = Some(Border { style: style.to_string(), color: None });
                        }
                    }
                    ("border", _, "color") => {
                        if let Some(b) = side.and_then(|i| borders.last_mut().and_then(|b| b[i].as_mut())) {
                            b.color = argb(a);
                        }
                    }
                    (_, "cellXfs" | "cellStyleXfs", "xf") => {
                        let id: u32 = a.get("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
                        let index = |k: &str| a.get(k).and_then(|v| v.parse::<usize>().ok());
                        let def = XfDef {
                            num_format: formats.get(&id).cloned().or_else(|| builtin_num_format(id).map(str::to_string)).unwrap_or_else(|| "General".into()),
                            font: index("fontId"),
                            fill: index("fillId"),
                            border: index("borderId"),
                            xf_id: index("xfId"),
                            attrs: a.all().iter().filter(|(k, _)| matches!(k.as_str(), "quotePrefix" | "pivotButton")).cloned().collect(),
                            ..XfDef::default()
                        };
                        if parent == "cellXfs" { cell_xfs.push(def) } else { style_xfs.push(def) }
                    }
                    ("cellXfs" | "cellStyleXfs", "xf", "alignment" | "protection") => {
                        let list = if grand == "cellXfs" { &mut cell_xfs } else { &mut style_xfs };
                        if let Some(x) = list.last_mut() {
                            let mut tag = format!("<{name}");
                            for (k, v) in a.own() {
                                tag.push_str(&format!(" {k}=\"{}\"", crate::xml::escape_attr_plain(v)));
                            }
                            tag.push_str("/>");
                            if name == "alignment" {
                                x.alignment = Some((tag, a.all().to_vec()));
                            } else {
                                x.protection = Some(tag);
                            }
                        }
                    }
                    (_, "cellStyles", "cellStyle") => {
                        let num = |k: &str| a.get(k).and_then(|v| v.parse::<u32>().ok());
                        let yes = |k: &str| matches!(a.get(k), Some("1") | Some("true"));
                        cell_styles.push((a.get("name").unwrap_or("").to_string(), num("xfId").unwrap_or(0) as usize, num("builtinId"), yes("hidden"), yes("customBuiltin"), num("iLevel")));
                    }
                    _ => {}
                }
                path.push(name.to_string());
            }
            Ev::End(_) => {
                path.pop();
            }
            Ev::Text(_) => {}
        }
        Ok(())
    })?;
    // A style from the parts of an xf: the fields, and the XML of what the fields cannot say.
    let build = |x: &XfDef| -> Result<Style> {
        let font = x.font.and_then(|i| fonts.get(i).cloned()).unwrap_or_default();
        // The writer's default font is Calibri 11; that is what "no font given" means.
        // A font given only by name or only by size gets the other default.
        let mut font = font;
        if font.name.is_some() || font.size.is_some() {
            font.name.get_or_insert_with(|| "Calibri".to_string());
            font.size.get_or_insert(11.0);
        }
        let default_font = font.name.as_deref() == Some("Calibri") && font.size == Some(11.0);
        let mut style = Style {
            num_format: x.num_format.clone(),
            bold: font.bold,
            italic: font.italic,
            underline: font.underline,
            strike: font.strike,
            font_name: if default_font { None } else { font.name },
            font_size: if default_font { None } else { font.size },
            font_color: font.color,
            fill: x.fill.and_then(|i| fills.get(i).cloned()).flatten(),
            borders: x.border.and_then(|i| borders.get(i).cloned()).unwrap_or_default(),
            xf_attrs: x.attrs.clone(),
            protection_xml: x.protection.clone(),
            ..Style::default()
        };
        if let Some((tag, a)) = &x.alignment {
            let get = |k: &str| a.iter().find(|p| p.0 == k).map(|p| p.1.as_str());
            style.h_align = get("horizontal").map(str::to_string);
            style.v_align = get("vertical").map(str::to_string);
            style.wrap = matches!(get("wrapText"), Some("1") | Some("true"));
            style.rotation = get("textRotation").and_then(|v| v.parse().ok()).unwrap_or(0);
            style.indent = get("indent").and_then(|v| v.parse().ok()).unwrap_or(0);
            let known = ["horizontal", "vertical", "wrapText", "textRotation", "indent"];
            let plain = a.iter().all(|(k, v)| known.contains(&k.as_str()) && !(k == "wrapText" && !matches!(v.as_str(), "1" | "true")));
            if !plain {
                style.alignment_xml = Some(tag.clone());
            }
        }
        // The XML of a part, where the fields do not say all of it.
        let fields = style.clone();
        if let Some(raw) = x.font.and_then(|i| raw_fonts.get(i)).filter(|r| !r.is_empty()) {
            if normalized(&crate::write::font_xml(&fields))? != *raw {
                style.font_xml = Some(raw.clone());
            }
        }
        if let Some(raw) = x.fill.and_then(|i| raw_fills.get(i)).filter(|r| !r.is_empty()) {
            if normalized(&crate::write::fill_xml(&fields))? != *raw {
                style.fill_xml = Some(raw.clone());
            }
        }
        if let Some(raw) = x.border.and_then(|i| raw_borders.get(i)).filter(|r| !r.is_empty()) {
            if normalized(&crate::write::border_xml(&fields))? != *raw {
                style.border_xml = Some(raw.clone());
            }
        }
        Ok(style)
    };
    // Named styles, unless the file has the plain Normal alone.
    let plain_normal = cell_styles.len() <= 1
        && cell_styles.first().is_none_or(|c| c.0 == "Normal" && c.1 == 0 && c.2 == Some(0) && !c.3)
        && style_xfs.len() <= 1
        && style_xfs.first().map(&build).transpose()?.is_none_or(|s| s == Style::default());
    let mut named_styles = Vec::new();
    if !plain_normal {
        // In the order of their formats (Normal, at 0, first), as the writer lays them out.
        let mut ordered: Vec<_> = cell_styles.iter().collect();
        ordered.sort_by_key(|c| c.1);
        for (name, xf_id, builtin_id, hidden, custom_builtin, i_level) in ordered {
            let style = match style_xfs.get(*xf_id) {
                Some(x) => build(x)?,
                None => Style::default(),
            };
            named_styles.push(NamedStyle { name: name.clone(), style, builtin_id: *builtin_id, hidden: *hidden, custom_builtin: *custom_builtin, i_level: *i_level });
        }
    }
    // The base of all formats is the named style with the lowest xfId (normally 0, Normal).
    let base_xf = cell_styles.iter().map(|c| c.1).min().unwrap_or(0);
    let mut out = Vec::with_capacity(cell_xfs.len());
    for x in &cell_xfs {
        let mut style = build(x)?;
        let xf_id = x.xf_id.unwrap_or(0);
        // The base is Normal, whatever the file calls it.
        style.named = cell_styles.iter().find(|c| c.1 == xf_id && xf_id != base_xf && c.2 != Some(0)).map(|c| c.0.clone());
        if plain_normal {
            style.named = None;
        }
        out.push(style);
    }
    if out.is_empty() {
        out.push(Style::default());
    }
    let style_xf_ids = cell_styles.iter().map(|c| (c.0.clone(), c.1)).collect();
    let table_styles = table_styles
        .into_iter()
        .map(|(name, table, pivot, elements)| TableStyle {
            name,
            table,
            pivot,
            elements: elements.into_iter().map(|(t, size, dxf)| (t, size, dxf.and_then(|i| dxfs.get(i).cloned()).unwrap_or_default())).collect(),
        })
        .collect();
    Ok(StyleSheet { styles: out, num_formats: declared, dxfs, named_styles, style_xf_ids, style_xf_count: style_xfs.len(), palette, table_styles, default_table_style, default_pivot_style })
}

/// `AB12` → (row, col), 0-based. Also `$A$1`.
pub(crate) fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let mut col: u32 = 0;
    let mut row: u32 = 0;
    let mut seen_digit = false;
    for b in s.bytes() {
        match b {
            b'$' => {}
            b'A'..=b'Z' | b'a'..=b'z' if !seen_digit => col = col.checked_mul(26)?.checked_add((b.to_ascii_uppercase() - b'A' + 1) as u32)?,
            b'0'..=b'9' => {
                seen_digit = true;
                row = row.checked_mul(10)?.checked_add((b - b'0') as u32)?;
            }
            _ => return None,
        }
    }
    (col > 0 && row > 0).then(|| (row - 1, col - 1))
}

fn parse_range(s: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    let ((r1, c1), (r2, c2)) = (parse_a1(a)?, parse_a1(b)?);
    Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

struct Master {
    expr: Option<Expr>,
    text: String,
    row: u32,
    col: u32,
}

#[derive(Default)]
struct CellState {
    row: u32,
    col: u32,
    ty: String,
    style: u32,
    value: String,
    has_value: bool,
    formula: String,
    has_formula: bool,
    shared: Option<u32>,
    array_ref: Option<String>,
    has_inline: bool,
    inline: StringItem,
    inline_xml: Capture,
    /// The `<c>` has an `s` of its own. Empty and in a formatted row or column it is
    /// kept, since its format (even 0) is not the row's or column's.
    explicit_style: bool,
}

#[allow(clippy::too_many_arguments)]
fn sheet(xml: &str, index: u32, strings: &[StringEntry], style_count: u32, dxfs: &[String], date1904: bool, info: &mut Sheet, cells: &mut Cells) -> Result<()> {
    #[derive(PartialEq)]
    enum In {
        None,
        V,
        F,
        Is,
    }
    let mut masters: HashMap<u32, Master> = HashMap::new();
    let mut next_row: u32 = 0;
    let mut next_col: u32 = 0;
    let mut cell: Option<CellState> = None;
    let mut inside = In::None;
    let mut in_data = false;
    let mut in_validation = false;
    let mut in_ext = false;
    let mut formula_slot = 0u8;
    let mut depth_auto = 0usize;
    let mut row_formatted = false;

    // `formatted`: the cell's row or column has a format (asked only for empty cells).
    let mut finish = |c: CellState, masters: &mut HashMap<u32, Master>, formatted: &dyn Fn(u32) -> bool| -> Result<()> {
        let mut formula = if c.has_formula && !c.formula.is_empty() { Some(unescape_xstring(&c.formula)) } else { None };
        if let Some(si) = c.shared {
            match formula.as_ref() {
                Some(text) => {
                    masters.insert(si, Master { expr: xl_formula::parse(text).ok(), text: text.clone(), row: c.row, col: c.col });
                }
                None => {
                    if let Some(m) = masters.get(&si) {
                        formula = Some(match &m.expr {
                            Some(e) => e.shifted(c.row as i64 - m.row as i64, c.col as i64 - m.col as i64).to_string(),
                            None => m.text.clone(),
                        });
                    }
                }
            }
        }
        let text = |s: String| Some(Arc::<str>::from(s));
        let mut rich: Option<Arc<str>> = None;
        let mut iso = IsoDate::No;
        let (kind, num, text): (Kind, f64, Option<Arc<str>>) = match c.ty.as_str() {
            "s" => match c.value.trim().parse::<usize>().ok().and_then(|i| strings.get(i)) {
                Some(s) => {
                    rich = s.1.clone();
                    (Kind::Text, 0.0, Some(s.0.clone()))
                }
                None if c.has_value && !c.value.trim().is_empty() => return Err(Error::Invalid(format!("shared string index {:?} out of range", c.value))),
                None => (Kind::Blank, 0.0, None),
            },
            "str" if c.has_value => (Kind::Text, 0.0, text(unescape_xstring(&c.value))),
            "str" => (Kind::Blank, 0.0, None),
            // `<c t="inlineStr"/>` without an `<is>` element holds nothing.
            "inlineStr" if !c.has_inline => (Kind::Blank, 0.0, None),
            "inlineStr" => {
                let mut inline = c.inline;
                rich = rich_xml(&c.inline_xml.xml);
                (Kind::Text, 0.0, text(inline.take()))
            }
            "b" => match c.value.trim() {
                "1" | "true" => (Kind::Bool, 1.0, None),
                "0" | "false" => (Kind::Bool, 0.0, None),
                _ => (Kind::Blank, 0.0, None),
            },
            "e" => (Kind::Error, 0.0, text(c.value.clone())),
            // ISO 8601 date cells become serial numbers; text that is no such date stays text.
            "d" => match IsoDate::parse(&c.value, date1904) {
                Some((n, form)) => {
                    iso = form;
                    (Kind::Number, n, None)
                }
                None if c.value.trim().is_empty() => (Kind::Blank, 0.0, None),
                None => (Kind::Text, 0.0, text(c.value.clone())),
            },
            _ => match c.value.trim() {
                "" => (Kind::Blank, 0.0, None),
                v => match v.parse::<f64>().map_err(|_| Error::Invalid(format!("not a number: {v:?}")))? {
                    n if n.is_finite() => (Kind::Number, n, None),
                    // Not a value Excel can hold; some writers emit NaN or INF.
                    _ => (Kind::Error, 0.0, text("#NUM!".to_string())),
                },
            },
        };
        let style = if c.style < style_count { c.style } else { 0 };
        if kind == Kind::Blank && formula.is_none() && style == 0 && !(c.explicit_style && formatted(c.col)) {
            return Ok(());
        }
        let array_ref = c.array_ref.filter(|_| formula.is_some());
        cells.push_raw(index, c.row, c.col, kind, num, text, formula, style);
        *cells.array_ref.last_mut().unwrap() = array_ref;
        *cells.rich.last_mut().unwrap() = rich.filter(|_| cells.formula.last().is_some_and(Option::is_none));
        *cells.iso.last_mut().unwrap() = iso;
        if kind == Kind::Number && num.fract() == 0.0 && num.abs() < 1e16 && c.value.contains(['.', 'e', 'E']) {
            *cells.float_text.last_mut().unwrap() = true;
        }
        Ok(())
    };

    let mut rule = Capture::default();
    let mut in_cf = false;
    walk(xml, &mut |ev| {
        if rule.active() {
            if rule.event(&ev) {
                if let Some(r) = info.cond_formats.last_mut().and_then(|c| c.rules.last_mut()) {
                    r.inner = std::mem::take(&mut rule.xml);
                }
            }
            return Ok(());
        }
        match ev {
            Ev::Start("conditionalFormatting", a) if !in_data && !in_ext => {
                info.cond_formats.push(CondFormat { sqref: a.get("sqref").unwrap_or("").to_string(), rules: Vec::new() });
                in_cf = true;
            }
            Ev::End("conditionalFormatting") => in_cf = false,
            Ev::Start("cfRule", a) if in_cf => {
                let dxf = a.get("dxfId").and_then(|v| v.parse::<usize>().ok()).and_then(|i| dxfs.get(i).cloned());
                let attrs = a.all().iter().filter(|(k, _)| k != "dxfId").cloned().collect();
                if let Some(c) = info.cond_formats.last_mut() {
                    c.rules.push(CondRule { attrs, inner: String::new(), dxf });
                }
                rule.begin();
            }
            Ev::Start("sheetData", _) => in_data = true,
            Ev::End("sheetData") => in_data = false,
            Ev::Start("row", a) if in_data => {
                if let Some(r) = a.get("r").and_then(|v| v.parse::<u32>().ok()) {
                    next_row = r.saturating_sub(1);
                }
                next_col = 0;
                let flag = |k: &str| matches!(a.get(k), Some("1") | Some("true"));
                let row = Row {
                    row: next_row,
                    height: a.get("ht").and_then(|v| v.parse::<f64>().ok()).filter(|h| h.is_finite() && *h >= 0.0),
                    custom_height: flag("customHeight"),
                    hidden: flag("hidden"),
                    outline_level: a.get("outlineLevel").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0).min(7),
                    collapsed: flag("collapsed"),
                    style: a.get("s").and_then(|v| v.parse::<u32>().ok()).filter(|s| *s < style_count),
                    custom_format: flag("customFormat"),
                    thick_top: flag("thickTop"),
                    thick_bot: flag("thickBot"),
                    phonetic: flag("ph"),
                };
                row_formatted = row.custom_format && row.style.unwrap_or(0) != 0;
                if row != (Row { row: next_row, ..Row::default() }) && info.rows.last().is_none_or(|l| l.row < next_row) {
                    info.rows.push(row);
                }
            }
            Ev::Start("pane", a) if !in_data && matches!(a.get("state"), Some("frozen") | Some("frozenSplit")) && info.freeze.is_none() => {
                let n = |k: &str| a.get(k).and_then(|v| v.parse::<f64>().ok()).map(|v| v as u32).unwrap_or(0);
                info.freeze = Some((n("ySplit"), n("xSplit"))).filter(|f| *f != (0, 0));
            }
            Ev::End("row") if in_data => next_row += 1,
            Ev::Start("c", a) if in_data => {
                let (row, col) = a.get("r").and_then(parse_a1).unwrap_or((next_row, next_col));
                next_col = col + 1;
                cell = Some(CellState {
                    row,
                    col,
                    ty: a.get("t").unwrap_or("n").to_string(),
                    style: a.get("s").and_then(|v| v.parse().ok()).unwrap_or(0),
                    explicit_style: a.get("s").is_some(),
                    ..CellState::default()
                });
            }
            Ev::End("c") if in_data => {
                inside = In::None;
                if let Some(c) = cell.take() {
                    finish(c, &mut masters, &|col| row_formatted || info.columns.iter().any(|k| k.first <= col && col <= k.last && k.style.unwrap_or(0) != 0))?;
                }
            }
            Ev::Start("v", _) if cell.is_some() && inside == In::None => {
                inside = In::V;
                cell.as_mut().unwrap().has_value = true;
            }
            Ev::Start("f", a) if cell.is_some() && inside == In::None => {
                inside = In::F;
                let c = cell.as_mut().unwrap();
                c.has_formula = true;
                if a.get("t") == Some("shared") {
                    c.shared = a.get("si").and_then(|v| v.parse().ok());
                }
                if a.get("t") == Some("array") {
                    c.array_ref = a.get("ref").map(str::to_string);
                }
            }
            Ev::Start("is", _) if cell.is_some() && inside == In::None => {
                inside = In::Is;
                let c = cell.as_mut().unwrap();
                c.has_inline = true;
                c.inline_xml.begin();
            }
            Ev::End("v") | Ev::End("f") | Ev::End("is") if cell.is_some() => inside = In::None,
            Ev::Text(t) => {
                if formula_slot != 0 {
                    if let Some(v) = info.validations.last_mut() {
                        let slot = if formula_slot == 1 { &mut v.formula1 } else { &mut v.formula2 };
                        slot.get_or_insert_with(String::new).push_str(t);
                    }
                }
                if let Some(c) = cell.as_mut() {
                    match inside {
                        In::V => c.value.push_str(t),
                        In::F => c.formula.push_str(t),
                        In::Is => {
                            c.inline.event(&Ev::Text(t));
                            c.inline_xml.event(&Ev::Text(t));
                        }
                        In::None => {}
                    }
                }
            }
            Ev::Start("autoFilter", a) if !in_data && depth_auto == 0 => info.auto_filter = a.get("ref").map(str::to_string),
            Ev::Start("dataValidation", a) if !in_data && !in_ext => {
                info.validations.push(Validation { attrs: a.all().iter().map(|(k, v)| (k.clone(), unescape_xstring(v))).collect(), formula1: None, formula2: None });
                in_validation = true;
            }
            Ev::End("dataValidation") => in_validation = false,
            Ev::Start("formula1", _) if in_validation => formula_slot = 1,
            Ev::Start("formula2", _) if in_validation => formula_slot = 2,
            Ev::End("formula1") | Ev::End("formula2") => formula_slot = 0,
            Ev::Start("extLst", _) => in_ext = true,
            Ev::End("extLst") => in_ext = false,
            Ev::Start("customSheetView", _) | Ev::Start("tablePart", _) => depth_auto += 1,
            Ev::End("customSheetView") => depth_auto = depth_auto.saturating_sub(1),
            Ev::Start("hyperlink", a) if !in_data => {
                if let Some(cell) = a.get("ref") {
                    // `target` holds the relationship id until the caller resolves it.
                    info.hyperlinks.push(Hyperlink { cell: cell.to_string(), target: a.get("id").map(str::to_string), location: a.get("location").map(unescape_xstring), tooltip: a.get("tooltip").map(unescape_xstring), display: a.get("display").map(unescape_xstring) });
                }
            }
            Ev::Start("dimension", a) if !in_data && info.dimension.is_none() => {
                info.dimension = a.get("ref").and_then(|r| match r.split_once(':') {
                    Some(_) => parse_range(r),
                    None => parse_a1(r).map(|(row, col)| (row, col, row, col)),
                });
            }
            Ev::Start("mergeCell", a) => {
                if let Some(r) = a.get("ref").and_then(parse_range) {
                    info.merges.push(r);
                }
            }
            Ev::Start("col", a) if !in_data => {
                let get = |k: &str| a.get(k).and_then(|v| v.parse::<u32>().ok());
                let flag = |k: &str| matches!(a.get(k), Some("1") | Some("true"));
                if let (Some(min), Some(max)) = (get("min"), get("max")) {
                    let max = max.min(xl_formula::MAX_COL);
                    if min >= 1 && max >= min && info.columns.last().is_none_or(|c| c.last < min - 1) {
                        let col = Column {
                            first: min - 1,
                            last: max - 1,
                            width: a.get("width").and_then(|v| v.parse::<f64>().ok()).filter(|w| w.is_finite() && *w >= 0.0),
                            custom_width: flag("customWidth"),
                            hidden: flag("hidden"),
                            best_fit: flag("bestFit"),
                            outline_level: a.get("outlineLevel").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0).min(7),
                            collapsed: flag("collapsed"),
                            style: get("style").filter(|s| *s < style_count),
                            phonetic: flag("phonetic"),
                        };
                        if col != (Column { first: min - 1, last: max - 1, ..Column::default() }) {
                            info.columns.push(col);
                        }
                    }
                }
            }
            ev => {
                if inside == In::Is {
                    if let Some(c) = cell.as_mut() {
                        c.inline.event(&ev);
                        c.inline_xml.event(&ev);
                    }
                }
            }
        }
        Ok(())
    })
}

#[allow(dead_code)]
fn _attrs_used(_: &Attrs) {}
