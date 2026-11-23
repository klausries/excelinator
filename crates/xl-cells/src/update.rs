//! Save a changed workbook onto its original file, keeping everything this crate
//! does not model: charts, images, comments, defined names, conditional formats,
//! data validation, pivot tables, macros, themes, document properties.
//!
//! Sheets whose cells did not change stay byte for byte as they were. In a changed
//! sheet the cell data (and, if changed, column widths and merged ranges) is replaced
//! inside the original XML; new styles are appended to the original style sheet.

use crate::model::*;
use crate::write::{cell_xml, check_cells, num_formats_xml, sheet_body, style_parts_named, write_bytes, Strings};
use crate::xml::{walk, Ev};
use crate::{Error, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use xl_opc::{resolve_target, ContentTypeItem, Package};

/// Write `wb` using `original` (the file it was read from) as the base. Falls back to
/// a fresh file when the sheets were added, removed, renamed or reordered, or when the
/// original uses a layout this function does not patch.
pub fn update_bytes(original: Vec<u8>, wb: &Workbook) -> Result<Vec<u8>> {
    Ok(update_bytes_report(original, wb)?.0)
}

/// Like [`update_bytes`]; the flag tells whether the original was kept as the base
/// (`true`) or a fresh file had to be written (`false`).
pub fn update_bytes_report(original: Vec<u8>, wb: &Workbook) -> Result<(Vec<u8>, bool)> {
    // A workbook that says nothing of its theme, palette and table styles (one built
    // without them) keeps the original's.
    let pkg = Package::open(original)?;
    let old = crate::read(&pkg)?;
    let old_look = (old.theme.clone(), old.palette.clone(), old.table_styles.clone(), old.default_table_style.clone(), old.default_pivot_style.clone());
    match patch(pkg, old, wb)? {
        Some(bytes) => Ok((bytes, true)),
        None if look_unset(wb) => {
            let mut w = wb.clone();
            (w.theme, w.palette, w.table_styles, w.default_table_style, w.default_pivot_style) = old_look;
            Ok((write_bytes(&w)?, false))
        }
        None => Ok((write_bytes(wb)?, false)),
    }
}

fn look_unset(wb: &Workbook) -> bool {
    wb.theme.is_none() && wb.palette.is_empty() && wb.table_styles.is_empty() && wb.default_table_style.is_none() && wb.default_pivot_style.is_none()
}

struct Spans {
    root_open_end: usize,
    /// (local name, start, end) of the root's child elements.
    children: Vec<(String, usize, usize)>,
}

/// Byte ranges of the root's children. `None` if the root element has a namespace
/// prefix (then our unprefixed fragments would land in the wrong namespace).
fn spans(xml: &str) -> Result<Option<Spans>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Spans { root_open_end: 0, children: Vec::new() };
    let mut depth = 0usize;
    let mut current: Option<(String, usize)> = None;
    let err = |e: quick_xml::Error| Error::Xml(e.to_string());
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event().map_err(err)? {
            Event::Start(e) => {
                depth += 1;
                if depth == 1 {
                    if e.name().into_inner().contains(':') {
                        return Ok(None);
                    }
                    out.root_open_end = reader.buffer_position() as usize;
                } else if depth == 2 {
                    current = Some((e.local_name().into_inner().to_string(), before));
                }
            }
            Event::Empty(e) => {
                if depth == 0 {
                    return Ok(None);
                }
                if depth == 1 {
                    out.children.push((e.local_name().into_inner().to_string(), before, reader.buffer_position() as usize));
                }
            }
            Event::End(_) => {
                if depth == 2 {
                    if let Some((name, start)) = current.take() {
                        out.children.push((name, start, reader.buffer_position() as usize));
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::DocType(_) => return Err(Error::Xml("DOCTYPE is not allowed in a package part".into())),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(Some(out))
}

use crate::write::SHEET_ORDER;

fn drop_override(ct: &mut xl_opc::ContentTypes, part: &str) {
    let key = xl_opc::normalize_key(part);
    ct.items.retain(|item| !matches!(item, ContentTypeItem::Override { part_name, .. } if xl_opc::normalize_key(part_name) == key));
}

/// Create the comments part of a sheet and, if `with_boxes`, the drawing part that
/// shows them. Returns the relationship id for the sheet's `<legacyDrawing>`.
fn add_comment_parts(pkg: &mut Package, dir: &str, sheet: &Sheet, rels: &mut xl_opc::Relationships, ct: &mut xl_opc::ContentTypes, with_boxes: bool) -> Result<Option<String>> {
    let (comments_type, vml_type) = crate::write::comment_rel_types();
    let n = (1..100_000).find(|n| !pkg.contains(&format!("{dir}comments{n}.xml")) && !pkg.contains(&format!("{dir}drawings/vmlDrawing{n}.vml"))).unwrap_or(99_999);
    let part = format!("{dir}comments{n}.xml");
    pkg.set(&part, crate::write::comments_xml(sheet).into_bytes());
    ct.set_override(&format!("/{part}"), crate::write::CT_COMMENTS);
    rels.add(&comments_type, &format!("/{part}"), xl_opc::TargetMode::Internal);
    if !with_boxes {
        return Ok(None);
    }
    let vml = format!("{dir}drawings/vmlDrawing{n}.vml");
    pkg.set(&vml, crate::write::comments_vml(sheet)?.into_bytes());
    if !ct.items.iter().any(|i| matches!(i, ContentTypeItem::Default { extension, .. } if extension.eq_ignore_ascii_case("vml"))) {
        ct.items.insert(0, ContentTypeItem::Default { extension: "vml".into(), content_type: crate::write::CT_VML.into() });
    }
    Ok(Some(rels.add(&vml_type, &format!("/{vml}"), xl_opc::TargetMode::Internal)))
}

/// Order of the children of `<styleSheet>`.
const STYLE_ORDER: &[&str] = &["numFmts", "fonts", "fills", "borders", "cellStyleXfs", "cellXfs", "cellStyles", "dxfs", "tableStyles", "colors", "extLst"];

/// Number of differential formats in the original style sheet (looked up once).
fn dxf_base(pkg: &Package, styles_part: &Option<String>, cache: &mut Option<usize>) -> Result<usize> {
    if let Some(n) = cache {
        return Ok(*n);
    }
    let n = match styles_part {
        Some(part) => crate::read::styles(&xl_opc::decode_xml(&pkg.read(part)?)?)?.dxfs.len(),
        None => 0,
    };
    *cache = Some(n);
    Ok(n)
}

/// Order of the children of `<workbook>` up to the defined names.
use crate::write::WORKBOOK_ORDER;

/// Replace children of the root element by name, or insert them at the place the
/// schema gives them. An empty replacement removes the element.
/// The elements kept as XML are the same, whatever their order in the list.
fn same_extra(a: &[(String, String)], b: &[(String, String)]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort();
    b.sort();
    a == b
}

fn replace_children(xml: &str, sp: &Spans, order: &[&str], replace: &[(&str, &str)], drop: &[&str]) -> String {
    let rank = |name: &str| order.iter().position(|o| *o == name);
    let mut pending: Vec<(usize, &str, &str)> = replace.iter().filter_map(|(n, x)| rank(n).map(|r| (r, *n, *x))).collect();
    pending.sort_by_key(|p| p.0);
    let mut out = String::with_capacity(xml.len() + replace.iter().map(|r| r.1.len()).sum::<usize>());
    let mut pos = 0usize;
    let mut last_rank = 0usize;
    for (name, start, end) in &sp.children {
        out.push_str(&xml[pos..*start]);
        pos = *end;
        // Elements the schema does not list (extensions) stay where they are.
        let r = rank(name).unwrap_or(last_rank);
        last_rank = r;
        while pending.first().is_some_and(|p| p.0 < r) {
            out.push_str(pending.remove(0).2);
        }
        if let Some(i) = pending.iter().position(|p| p.1 == name) {
            out.push_str(pending.remove(i).2);
        } else if replace.iter().any(|p| p.0 == name) || drop.contains(&name.as_str()) {
            // A second element of a replaced name, or one to drop.
        } else {
            out.push_str(&xml[*start..*end]);
        }
    }
    for p in pending {
        out.push_str(p.2);
    }
    out.push_str(&xml[pos..]);
    out
}

fn splice_sheet(xml: &str, replace: &[(&str, &str)]) -> Result<Option<String>> {
    let Some(sp) = spans(xml)? else { return Ok(None) };
    if !sp.children.iter().any(|c| c.0 == "sheetData") {
        return Ok(None);
    }
    // The dimension is optional, and stale once cells change.
    Ok(Some(replace_children(xml, &sp, SHEET_ORDER, replace, &["dimension"])))
}

/// Number of child elements of a container such as `<fonts>…</fonts>`.
fn count_items(segment: &str) -> Result<usize> {
    let mut depth = 0usize;
    let mut n = 0usize;
    walk(segment, &mut |ev| {
        match ev {
            Ev::Start(..) => {
                depth += 1;
                n += (depth == 2) as usize;
            }
            Ev::End(_) => depth -= 1,
            Ev::Text(_) => {}
        }
        Ok(())
    })?;
    Ok(n)
}

/// The container with `items` appended and its count set.
fn extend(segment: &str, name: &str, existing: usize, items: &[String]) -> String {
    let inner = match (segment.find('>'), segment.rfind("</")) {
        (Some(a), Some(b)) if b > a && !segment.ends_with("/>") => &segment[a + 1..b],
        _ => "",
    };
    format!("<{name} count=\"{}\">{inner}{}</{name}>", existing + items.len(), items.concat())
}

fn splice_styles(xml: &str, new: &[Style], named: &[NamedStyle]) -> Result<Option<String>> {
    let Some(sp) = spans(xml)? else { return Ok(None) };
    let find = |name: &str| sp.children.iter().find(|c| c.0 == name);
    let (Some(fonts), Some(fills), Some(borders), Some(_)) = (find("fonts"), find("fills"), find("borders"), find("cellXfs")) else { return Ok(None) };
    let seg = |c: &(String, usize, usize)| &xml[c.1..c.2];
    let base = (count_items(seg(fonts))?, count_items(seg(fills))?, count_items(seg(borders))?);
    let sheet = crate::read::styles(xml)?;
    let known = sheet.num_formats;
    // Named styles the new formats use that the file does not have yet.
    let mut ids = sheet.style_xf_ids.clone();
    let mut added: Vec<NamedStyle> = Vec::new();
    for s in new {
        if let Some(n) = &s.named {
            if !ids.iter().any(|i| i.0 == *n) {
                let style = named.iter().find(|m| m.name == *n).cloned().unwrap_or(NamedStyle { name: n.clone(), ..NamedStyle::default() });
                ids.push((n.clone(), sheet.style_xf_count + added.len()));
                added.push(style);
            }
        }
    }
    if !added.is_empty() && (find("cellStyleXfs").is_none() || find("cellStyles").is_none()) {
        return Ok(None);
    }
    let formats: Vec<Style> = added.iter().map(|n| n.style.clone()).collect();
    let extra = style_parts_named(&formats, Vec::new(), Vec::new(), Vec::new(), base, &known, None);
    let mut known_all = known.clone();
    known_all.extend(extra.num_formats.iter().cloned());
    let parts = style_parts_named(new, extra.fonts.clone(), extra.fills.clone(), extra.borders.clone(), base, &known_all, Some(&ids));
    let mut new_formats = extra.num_formats.clone();
    new_formats.extend(parts.num_formats.iter().cloned());
    let mut out = String::with_capacity(xml.len() + 512);
    out.push_str(&xml[..sp.root_open_end]);
    let formats_el = find("numFmts");
    if formats_el.is_none() && !new_formats.is_empty() {
        out.push_str(&format!("<numFmts count=\"{}\">{}</numFmts>", new_formats.len(), num_formats_xml(&new_formats)));
    }
    let mut pos = sp.root_open_end;
    for c in &sp.children {
        out.push_str(&xml[pos..c.1]);
        pos = c.2;
        match c.0.as_str() {
            "numFmts" => out.push_str(&extend(seg(c), "numFmts", count_items(seg(c))?, &[num_formats_xml(&new_formats)])),
            "fonts" => out.push_str(&extend(seg(c), "fonts", base.0, &parts.fonts)),
            "fills" => out.push_str(&extend(seg(c), "fills", base.1, &parts.fills)),
            "borders" => out.push_str(&extend(seg(c), "borders", base.2, &parts.borders)),
            "cellStyleXfs" if !added.is_empty() => out.push_str(&extend(seg(c), "cellStyleXfs", count_items(seg(c))?, &extra.xfs)),
            "cellXfs" => out.push_str(&extend(seg(c), "cellXfs", count_items(seg(c))?, &parts.xfs)),
            "cellStyles" if !added.is_empty() => {
                let first = sheet.style_xf_count;
                let items: Vec<String> = added.iter().enumerate().map(|(i, n)| crate::write::cell_styles_xml(std::slice::from_ref(n), first + i)).collect();
                out.push_str(&extend(seg(c), "cellStyles", count_items(seg(c))?, &items));
            }
            _ => out.push_str(seg(c)),
        }
    }
    out.push_str(&xml[pos..]);
    // `extend` counted the number formats as one item; set the real count.
    if let Some(c) = formats_el {
        let total = count_items(seg(c))? + new_formats.len();
        let wrong = format!("<numFmts count=\"{}\">", count_items(seg(c))? + 1);
        out = out.replacen(&wrong, &format!("<numFmts count=\"{total}\">"), 1);
    }
    Ok(Some(out))
}

/// Value of an attribute in a start tag.
fn get_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    ['"', '\''].iter().find_map(|q| {
        let key = format!(" {name}={q}");
        let from = tag.find(&key)? + key.len();
        Some(&tag[from..from + tag[from..].find(*q)?])
    })
}

/// `<sheetData>` with only the changed cells replaced: every other cell and every
/// row keeps its original XML (shared strings with their formatting, row attributes,
/// cells this crate reads as empty). `None` when the original is not laid out with
/// explicit, ascending references, or when a shared formula would be cut apart; the
/// caller then writes the cell data anew.
fn patch_cells(segment: &str, old: &Cells, old_idx: &[usize], wb: &Workbook, new_idx: &[usize], needs_calc: &mut bool) -> Result<Option<String>> {
    let Some(sp) = spans(segment)? else { return Ok(None) };
    let new = &wb.cells;
    let old_at: std::collections::HashMap<(u32, u32), usize> = old_idx.iter().map(|&i| ((old.row[i], old.col[i]), i)).collect();
    let mut out = String::with_capacity(segment.len() + 256);
    out.push_str(&segment[..sp.root_open_end]);
    let mut n = 0usize; // position in new_idx
    let mut last_row: Option<u32> = None;
    let emit_new_rows = |out: &mut String, n: &mut usize, before: u32, needs_calc: &mut bool| {
        while *n < new_idx.len() && new.row[new_idx[*n]] < before {
            let r = new.row[new_idx[*n]];
            out.push_str(&format!("<row r=\"{}\">", r + 1));
            while *n < new_idx.len() && new.row[new_idx[*n]] == r {
                cell_xml(new, new_idx[*n], &mut Strings::Inline, needs_calc, wb.date1904, out);
                *n += 1;
            }
            out.push_str("</row>");
        }
    };
    for (name, start, end) in &sp.children {
        let row_xml = &segment[*start..*end];
        if name != "row" {
            return Ok(None);
        }
        let open_end = row_xml.find('>').map(|i| i + 1).unwrap_or(row_xml.len());
        let empty = row_xml[..open_end].ends_with("/>");
        let Some(r) = get_attr(&row_xml[..open_end], "r").and_then(|v| v.parse::<u32>().ok()).and_then(|v| v.checked_sub(1)) else { return Ok(None) };
        if last_row.is_some_and(|l| l >= r) {
            return Ok(None);
        }
        last_row = Some(r);
        emit_new_rows(&mut out, &mut n, r, needs_calc);
        let cells: Vec<(String, usize, usize)> = if empty {
            Vec::new()
        } else {
            match spans(row_xml)? {
                Some(c) => c.children,
                None => return Ok(None),
            }
        };
        let mut body = String::new();
        let mut modified = false;
        let mut last_col: Option<u32> = None;
        let add_new = |body: &mut String, n: &mut usize, before: u32, modified: &mut bool, needs_calc: &mut bool| {
            while *n < new_idx.len() && new.row[new_idx[*n]] == r && new.col[new_idx[*n]] < before {
                cell_xml(new, new_idx[*n], &mut Strings::Inline, needs_calc, wb.date1904, body);
                *n += 1;
                *modified = true;
            }
        };
        for (cname, cs, ce) in &cells {
            let element = &row_xml[*cs..*ce];
            if cname != "c" {
                return Ok(None);
            }
            let tag_end = element.find('>').unwrap_or(element.len());
            let Some((cr, col)) = get_attr(&element[..tag_end], "r").and_then(crate::read::parse_a1) else { return Ok(None) };
            if cr != r || last_col.is_some_and(|l| l >= col) {
                return Ok(None);
            }
            last_col = Some(col);
            add_new(&mut body, &mut n, col, &mut modified, needs_calc);
            let here = n < new_idx.len() && new.row[new_idx[n]] == r && new.col[new_idx[n]] == col;
            let was = old_at.get(&(r, col)).copied();
            let shared = element.contains("t=\"shared\"") || element.contains("t='shared'");
            match (was, here) {
                (Some(o), true) if same_cells(old, &[o], new, &[new_idx[n]]) => {
                    body.push_str(element);
                    n += 1;
                }
                (_, true) => {
                    if shared {
                        return Ok(None);
                    }
                    cell_xml(new, new_idx[n], &mut Strings::Inline, needs_calc, wb.date1904, &mut body);
                    n += 1;
                    modified = true;
                }
                (Some(_), false) => {
                    if shared {
                        return Ok(None);
                    }
                    modified = true;
                }
                (None, false) => body.push_str(element),
            }
        }
        add_new(&mut body, &mut n, u32::MAX, &mut modified, needs_calc);
        let mut open = if empty { format!("{}>", &row_xml[..open_end - 2]) } else { row_xml[..open_end].to_string() };
        if modified {
            // The span hint no longer has to be right.
            if let Some(v) = get_attr(&open, "spans").map(str::to_string) {
                open = open.replacen(&format!(" spans=\"{v}\""), "", 1).replacen(&format!(" spans='{v}'"), "", 1);
            }
        }
        out.push_str(&open);
        out.push_str(&body);
        out.push_str("</row>");
    }
    emit_new_rows(&mut out, &mut n, u32::MAX, needs_calc);
    out.push_str("</sheetData>");
    Ok(Some(out))
}

fn by_sheet(wb: &Workbook) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); wb.sheets.len()];
    for i in wb.cells.sorted_unique() {
        if let Some(list) = out.get_mut(wb.cells.sheet[i] as usize) {
            list.push(i);
        }
    }
    out
}

fn same_cells(a: &Cells, ia: &[usize], b: &Cells, ib: &[usize]) -> bool {
    ia.len() == ib.len()
        && ia.iter().zip(ib).all(|(&x, &y)| {
            a.row[x] == b.row[y]
                && a.col[x] == b.col[y]
                && a.kind[x] == b.kind[y]
                && a.num[x].to_bits() == b.num[y].to_bits()
                && a.text[x].as_deref() == b.text[y].as_deref()
                && a.formula[x] == b.formula[y]
                && a.style[x] == b.style[y]
                && a.float_text[x] == b.float_text[y]
                && a.array_ref[x] == b.array_ref[y]
                && a.rich[x].as_deref() == b.rich[y].as_deref()
                && a.iso[x] == b.iso[y]
        })
}

/// Replace the value of an attribute inside one start tag. `None` if it is not there.
pub(crate) fn set_attr(tag: &str, name: &str, value: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let key = format!(" {name}={quote}");
        if let Some(start) = tag.find(&key) {
            let from = start + key.len();
            let len = tag[from..].find(quote)?;
            return Some(format!("{}{}{}", &tag[..from], value, &tag[from + len..]));
        }
    }
    None
}

/// The workbook part with its sheet list replaced, names local to sheets renumbered,
/// and the active tab reset if sheets went away. `order[j]` is the original position
/// of the sheet now at `j` (`None` for a new sheet, then `added[j]` is its element).
fn splice_workbook(xml: &str, wb: &Workbook, order: &[Option<usize>], added: &[Option<String>], old_count: usize, names: Option<&[DefinedName]>) -> Result<Option<String>> {
    let Some(sp) = spans(xml)? else { return Ok(None) };
    let Some(sheets) = sp.children.iter().find(|c| c.0 == "sheets") else { return Ok(None) };
    let segment = &xml[sheets.1..sheets.2];
    let Some(inner) = spans(segment)? else { return Ok(None) };
    if inner.children.len() != old_count {
        return Ok(None);
    }
    let mut list = String::from("<sheets>");
    for (j, sheet) in wb.sheets.iter().enumerate() {
        match order[j] {
            Some(o) => {
                let c = &inner.children[o];
                let Some(tag) = set_attr(&segment[c.1..c.2], "name", &crate::xml::escape_attr(&sheet.name)) else { return Ok(None) };
                list.push_str(&tag);
            }
            None => list.push_str(added[j].as_deref().unwrap_or("")),
        }
    }
    list.push_str("</sheets>");
    let new_index = |old: usize| order.iter().position(|o| *o == Some(old));
    let removed = (0..old_count).any(|o| new_index(o).is_none());
    // First pass: the sheet list, tab numbers, and names local to sheets.
    let mut replace: Vec<(&str, String)> = vec![("sheets", list)];
    for c in &sp.children {
        let seg = &xml[c.1..c.2];
        match c.0.as_str() {
            "bookViews" if removed => {
                // Tab numbers may now point past the end.
                let mut v = seg.to_string();
                for attr in ["activeTab", "firstSheet"] {
                    if let Some(n) = set_attr(&v, attr, "0") {
                        v = n;
                    }
                }
                replace.push(("bookViews", v));
            }
            "definedNames" if names.is_none() => {
                let Some(inner) = spans(seg)? else { return Ok(None) };
                let mut out = String::from(&seg[..inner.root_open_end]);
                let mut last = inner.root_open_end;
                for n in &inner.children {
                    out.push_str(&seg[last..n.1]);
                    last = n.2;
                    let element = &seg[n.1..n.2];
                    let tag_end = element.find('>').unwrap_or(element.len());
                    match get_attr(&element[..tag_end], "localSheetId").and_then(|v| v.parse::<usize>().ok()) {
                        None => out.push_str(element),
                        Some(old) => {
                            if let Some(new) = new_index(old) {
                                let Some(tag) = set_attr(&element[..tag_end], "localSheetId", &new.to_string()) else { return Ok(None) };
                                out.push_str(&tag);
                                out.push_str(&element[tag_end..]);
                            }
                        }
                    }
                }
                out.push_str(&seg[last..]);
                replace.push(("definedNames", out));
            }
            _ => {}
        }
    }
    if let Some(names) = names {
        let xml = if names.is_empty() { String::new() } else { format!("<definedNames>{}</definedNames>", names.iter().map(crate::write::defined_name_xml).collect::<String>()) };
        replace.push(("definedNames", xml));
    }
    let replace: Vec<(&str, &str)> = replace.iter().map(|(n, x)| (*n, x.as_str())).collect();
    Ok(Some(replace_children(xml, &sp, WORKBOOK_ORDER, &replace, &[])))
}

fn patch(mut pkg: Package, old: Workbook, wb: &Workbook) -> Result<Option<Vec<u8>>> {
    if wb.sheets.is_empty() || wb.styles.len() < old.styles.len() || wb.styles[..old.styles.len()] != old.styles[..] {
        return Ok(None);
    }
    // A changed theme, palette or table styles: the file is written anew. A workbook
    // that says nothing of them (one built without them) keeps the original's.
    if !look_unset(wb) && (&wb.theme, &wb.palette, &wb.table_styles, &wb.default_table_style, &wb.default_pivot_style) != (&old.theme, &old.palette, &old.table_styles, &old.default_table_style, &old.default_pivot_style) {
        return Ok(None);
    }
    // Which original sheet is each sheet? Each original at most once, kinds unchanged;
    // a new sheet must be a worksheet.
    let mut order: Vec<Option<usize>> = Vec::with_capacity(wb.sheets.len());
    for sheet in &wb.sheets {
        let o = sheet.origin.map(|o| o as usize);
        match o {
            Some(o) if o >= old.sheets.len() || order.contains(&Some(o)) || old.sheets[o].kind != sheet.kind => return Ok(None),
            None if sheet.kind != SheetKind::Worksheet => return Ok(None),
            _ => order.push(o),
        }
    }
    let structure_same = order.len() == old.sheets.len() && order.iter().enumerate().all(|(j, o)| *o == Some(j)) && old.sheets.iter().zip(&wb.sheets).all(|(a, b)| a.name == b.name);
    if !structure_same {
        crate::write::check_sheets(wb)?;
    }
    let new_order = by_sheet(wb);

    // Locate the parts.
    let root = pkg.relationships("")?;
    let Some(wb_part) = root.items.iter().find(|r| r.rel_type.ends_with("/officeDocument")).and_then(|r| resolve_target("", &r.target)) else { return Ok(None) };
    let mut rels = pkg.relationships(&wb_part)?;
    let rel_count = rels.items.len();
    let wb_xml = xl_opc::decode_xml(&pkg.read(&wb_part)?)?;
    let mut rids: Vec<String> = Vec::new();
    walk(&wb_xml, &mut |ev| {
        if let Ev::Start("sheet", a) = ev {
            rids.push(a.get("id").unwrap_or("").to_string());
        }
        Ok(())
    })?;
    if rids.len() != old.sheets.len() {
        return Ok(None);
    }
    let part_of = |rels: &xl_opc::Relationships, o: usize| rels.by_id(&rids[o]).and_then(|r| resolve_target(&wb_part, &r.target));

    // New styles go behind the original ones.
    if wb.styles.len() > old.styles.len() {
        let Some(part) = rels.items.iter().find(|r| r.rel_type.ends_with("/styles")).and_then(|r| resolve_target(&wb_part, &r.target)) else { return Ok(None) };
        if !pkg.contains(&part) {
            return Ok(None);
        }
        let Some(xml) = splice_styles(&xl_opc::decode_xml(&pkg.read(&part)?)?, &wb.styles[old.styles.len()..], &wb.named_styles)? else { return Ok(None) };
        pkg.set(&part, xml.into_bytes());
    }

    let styles_part = rels.items.iter().find(|r| r.rel_type.ends_with("/styles")).and_then(|r| resolve_target(&wb_part, &r.target)).filter(|p| pkg.contains(p));
    let mut dxf_count: Option<usize> = None;
    let mut new_dxfs: Vec<String> = Vec::new();
    let old_order = by_sheet(&old);
    let mut changed = !structure_same;
    let mut content_types = None;
    let mut added: Vec<Option<String>> = vec![None; wb.sheets.len()];
    for (j, sheet) in wb.sheets.iter().enumerate() {
        if sheet.kind != SheetKind::Worksheet {
            continue;
        }
        let Some(o) = order[j] else {
            // A new sheet: its own part, relationship, content type and list entry.
            check_cells(wb, &new_order[j])?;
            let mut needs_calc = false;
            let body = sheet_body(wb, sheet, &new_order[j], Strings::Inline, &mut needs_calc)?;
            let dir = wb_part.rsplit_once('/').map(|d| format!("{}/", d.0)).unwrap_or_default();
            let Some(n) = (1..100_000).find(|n| !pkg.contains(&format!("{dir}worksheets/sheet{n}.xml"))) else { return Ok(None) };
            let part = format!("{dir}worksheets/sheet{n}.xml");
            let mut sheet_rels = xl_opc::Relationships::default();
            let link_ids = crate::write::hyperlink_rels(sheet, &mut sheet_rels);
            let base = dxf_base(&pkg, &styles_part, &mut dxf_count)?;
            let cond = crate::write::cond_formats_xml(sheet, &mut new_dxfs, base);
            let mut legacy = None;
            if !sheet.comments.is_empty() {
                let ct = content_types.get_or_insert(pkg.content_types()?);
                legacy = add_comment_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct, true)?;
            }
            let drawing = if sheet.drawings.is_empty() {
                None
            } else {
                let ct = content_types.get_or_insert(pkg.content_types()?);
                Some(crate::write::add_drawing_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct)?)
            };
            if !sheet.tables.is_empty() {
                return Ok(None); // table ids and names are checked against the whole workbook when writing anew
            }
            pkg.set(&part, crate::write::sheet_xml(sheet, &body, &link_ids, &cond, drawing.as_deref(), legacy.as_deref(), "").into_bytes());
            if !sheet_rels.items.is_empty() {
                pkg.set_relationships(&part, &sheet_rels);
            }
            // Same relationship vocabulary (transitional or strict) as the existing sheets.
            let Some(kind) = rels.items.iter().find(|r| r.rel_type.ends_with("/worksheet")).map(|r| r.rel_type.clone()) else { return Ok(None) };
            let id = rels.add(&kind, &format!("worksheets/sheet{n}.xml"), xl_opc::TargetMode::Internal);
            let ct = content_types.get_or_insert(pkg.content_types()?);
            ct.set_override(&format!("/{part}"), "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml");
            // The relationships namespace is declared on the element itself, so it does
            // not matter which prefix the rest of the document uses for it.
            let namespace = kind.strip_suffix("/worksheet").unwrap_or("http://schemas.openxmlformats.org/officeDocument/2006/relationships").to_string();
            let sheet_id = 1 + old.sheets.len() + j;
            let taken: Vec<u32> = wb_xml.split("sheetId=\"").skip(1).filter_map(|t| t.split('"').next()?.parse().ok()).collect();
            let sheet_id = (sheet_id as u32..).find(|i| !taken.contains(i)).unwrap_or(sheet_id as u32);
            added[j] = Some(format!("<sheet xmlns:r=\"{namespace}\" name=\"{}\" sheetId=\"{sheet_id}\" r:id=\"{id}\"/>", crate::xml::escape_attr(&sheet.name)));
            continue;
        };
        let before = &old.sheets[o];
        let cells_same = before.rows == sheet.rows && same_cells(&old.cells, &old_order[o], &wb.cells, &new_order[j]);
        let rest_same = before.merges == sheet.merges && before.columns == sheet.columns && before.auto_filter == sheet.auto_filter && before.validations == sheet.validations && before.hyperlinks == sheet.hyperlinks && before.cond_formats == sheet.cond_formats && before.comments == sheet.comments && before.drawings == sheet.drawings && same_extra(&before.extra, &sheet.extra);
        if cells_same && rest_same && before.tables == sheet.tables {
            continue;
        }
        if before.tables != sheet.tables {
            return Ok(None); // written anew, with the table parts of all sheets
        }
        let Some(part) = part_of(&rels, o) else { return Ok(None) };
        check_cells(wb, &new_order[j])?;
        let mut needs_calc = false;
        let body = sheet_body(wb, sheet, &new_order[j], Strings::Inline, &mut needs_calc)?;
        let sheet_xml = xl_opc::decode_xml(&pkg.read(&part)?)?;
        let mut replace: Vec<(&str, String)> = Vec::new();
        if !cells_same {
            // Prefer replacing single cells; write all cell data anew only if that is not possible.
            let mut data = None;
            if before.rows == sheet.rows {
                if let Some(c) = spans(&sheet_xml)?.and_then(|sp| sp.children.into_iter().find(|c| c.0 == "sheetData")) {
                    data = patch_cells(&sheet_xml[c.1..c.2], &old.cells, &old_order[o], wb, &new_order[j], &mut needs_calc)?;
                }
            }
            replace.push(("sheetData", data.unwrap_or(body.data)));
        }
        if before.columns != sheet.columns {
            replace.push(("cols", body.cols));
        }
        if before.merges != sheet.merges {
            replace.push(("mergeCells", body.merges));
        }
        let has_extra = |s: &Sheet, n: &str| s.extra.iter().any(|e| e.0 == n);
        if before.auto_filter != sheet.auto_filter && !has_extra(sheet, "autoFilter") {
            replace.push(("autoFilter", crate::write::auto_filter_xml(sheet)));
        }
        // Elements kept as XML: the ones that changed, came or went.
        let mut names: Vec<&str> = before.extra.iter().chain(sheet.extra.iter()).map(|e| e.0.as_str()).collect();
        names.sort();
        names.dedup();
        for name in names {
            let old_x: Vec<&String> = before.extra.iter().filter(|e| e.0 == name).map(|e| &e.1).collect();
            let new_x: Vec<&String> = sheet.extra.iter().filter(|e| e.0 == name).map(|e| &e.1).collect();
            if old_x != new_x {
                replace.push((name, new_x.into_iter().cloned().collect::<String>()));
            }
        }
        if before.cond_formats != sheet.cond_formats {
            let base = dxf_base(&pkg, &styles_part, &mut dxf_count)?;
            replace.push(("conditionalFormatting", crate::write::cond_formats_xml(sheet, &mut new_dxfs, base)));
        }
        if before.validations != sheet.validations {
            replace.push(("dataValidations", crate::write::validations_xml(sheet)));
        }
        if before.hyperlinks != sheet.hyperlinks {
            // New relationships for the addresses; the old ones stay behind unused.
            let mut sheet_rels = pkg.relationships(&part)?;
            let ids = crate::write::hyperlink_rels(sheet, &mut sheet_rels);
            if ids.iter().any(Option::is_some) {
                pkg.set_relationships(&part, &sheet_rels);
            }
            replace.push(("hyperlinks", crate::write::hyperlinks_xml(sheet, &ids)));
        }
        if before.drawings != sheet.drawings {
            let mut sheet_rels = pkg.relationships(&part)?;
            let ct = content_types.get_or_insert(pkg.content_types()?);
            if let Some(r) = sheet_rels.items.iter().find(|r| r.rel_type.ends_with("/drawing")).cloned() {
                if before.drawings.is_empty() {
                    return Err(Error::Invalid(format!("sheet {:?} has a drawing with shapes; pictures and charts cannot be added to it", sheet.name)));
                }
                // The old drawing goes, together with the chart parts only it used.
                if let Some(p) = resolve_target(&part, &r.target) {
                    for c in pkg.relationships(&p)?.items.iter().filter(|c| c.rel_type.ends_with("/chart")) {
                        if let Some(chart) = resolve_target(&p, &c.target) {
                            pkg.remove(&chart);
                            pkg.remove(&xl_opc::rels_part_for(&chart));
                            drop_override(ct, &chart);
                        }
                    }
                    pkg.remove(&p);
                    pkg.remove(&xl_opc::rels_part_for(&p));
                    drop_override(ct, &p);
                }
                sheet_rels.items.retain(|x| x.id != r.id);
            }
            let dir = wb_part.rsplit_once('/').map(|d| format!("{}/", d.0)).unwrap_or_default();
            let element = if sheet.drawings.is_empty() { String::new() } else { crate::write::drawing_element_xml(&crate::write::add_drawing_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct)?) };
            replace.push(("drawing", element));
            pkg.set_relationships(&part, &sheet_rels);
        }
        if before.comments != sheet.comments {
            let mut sheet_rels = pkg.relationships(&part)?;
            let ct = content_types.get_or_insert(pkg.content_types()?);
            // The old comments part goes; a new one is written.
            if let Some(r) = sheet_rels.items.iter().find(|r| r.rel_type.ends_with("/comments")).cloned() {
                if let Some(p) = resolve_target(&part, &r.target) {
                    pkg.remove(&p);
                    drop_override(ct, &p);
                }
                sheet_rels.items.retain(|x| x.id != r.id);
            }
            // The drawing part may also hold form controls; it is only rewritten or
            // removed when it holds nothing but comment boxes.
            let old_vml = sheet_rels.items.iter().find(|r| r.rel_type.ends_with("/vmlDrawing")).cloned();
            let old_vml_part = old_vml.as_ref().and_then(|r| resolve_target(&part, &r.target)).filter(|p| pkg.contains(p));
            let only_notes = match &old_vml_part {
                Some(p) => {
                    let text = String::from_utf8_lossy(&pkg.read(p)?).into_owned();
                    text.matches("ObjectType=").count() == text.matches("ObjectType=\"Note\"").count()
                }
                None => true,
            };
            let dir = wb_part.rsplit_once('/').map(|d| format!("{}/", d.0)).unwrap_or_default();
            if !sheet.comments.is_empty() {
                match (&old_vml_part, only_notes) {
                    (Some(p), true) => {
                        add_comment_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct, false)?;
                        pkg.set(p, crate::write::comments_vml(sheet)?.into_bytes());
                    }
                    (Some(_), false) => {
                        add_comment_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct, false)?;
                    }
                    (None, _) => {
                        if let Some(id) = add_comment_parts(&mut pkg, &dir, sheet, &mut sheet_rels, ct, true)? {
                            replace.push(("legacyDrawing", crate::write::legacy_drawing_xml(&id)));
                        }
                    }
                }
            } else if let (Some(r), Some(p), true) = (&old_vml, &old_vml_part, only_notes) {
                pkg.remove(p);
                sheet_rels.items.retain(|x| x.id != r.id);
                replace.push(("legacyDrawing", String::new()));
            }
            pkg.set_relationships(&part, &sheet_rels);
        }
        let replace: Vec<(&str, &str)> = replace.iter().map(|(n, x)| (*n, x.as_str())).collect();
        let Some(xml) = splice_sheet(&sheet_xml, &replace)? else { return Ok(None) };
        pkg.set(&part, xml.into_bytes());
        changed = true;
    }

    // Differential formats of new conditional formatting go behind the existing ones.
    if !new_dxfs.is_empty() {
        let Some(part) = &styles_part else { return Ok(None) };
        let xml = xl_opc::decode_xml(&pkg.read(part)?)?;
        let Some(sp) = spans(&xml)? else { return Ok(None) };
        let items = [crate::write::dxfs_xml(&new_dxfs)];
        let element = match sp.children.iter().find(|c| c.0 == "dxfs") {
            Some(c) => extend(&xml[c.1..c.2], "dxfs", dxf_count.unwrap_or(0), &items).replacen(&format!("count=\"{}\"", dxf_count.unwrap_or(0) + 1), &format!("count=\"{}\"", dxf_count.unwrap_or(0) + new_dxfs.len()), 1),
            None => format!("<dxfs count=\"{}\">{}</dxfs>", new_dxfs.len(), items[0]),
        };
        pkg.set(part, replace_children(&xml, &sp, STYLE_ORDER, &[("dxfs", element.as_str())], &[]).into_bytes());
    }
    let names_changed = old.defined_names != wb.defined_names;
    if !structure_same || names_changed {
        let names = names_changed.then_some(wb.defined_names.as_slice());
        let Some(xml) = splice_workbook(&wb_xml, wb, &order, &added, old.sheets.len(), names)? else { return Ok(None) };
        pkg.set(&wb_part, xml.into_bytes());
    }
    if old.date1904 != wb.date1904 {
        return Ok(None); // every date cell would change its meaning
    }
    // Children of <workbook> kept as XML: the ones that changed, came or went.
    if !same_extra(&old.extra, &wb.extra) {
        let xml = xl_opc::decode_xml(&pkg.read(&wb_part)?)?;
        let Some(sp) = spans(&xml)? else { return Ok(None) };
        let mut names: Vec<&str> = old.extra.iter().chain(wb.extra.iter()).map(|e| e.0.as_str()).collect();
        names.sort();
        names.dedup();
        let now = crate::write::workbook_extra_xml(wb, false);
        let mut replace: Vec<(&str, String)> = Vec::new();
        for name in names {
            let old_x: Vec<&String> = old.extra.iter().filter(|e| e.0 == name).map(|e| &e.1).collect();
            let new_x: Vec<&String> = wb.extra.iter().filter(|e| e.0 == name).map(|e| &e.1).collect();
            if old_x != new_x {
                replace.push((name, now.iter().filter(|e| e.0 == name).map(|e| e.1.as_str()).collect::<String>()));
            }
        }
        let replace: Vec<(&str, &str)> = replace.iter().map(|(n, x)| (*n, x.as_str())).collect();
        pkg.set(&wb_part, replace_children(&xml, &sp, WORKBOOK_ORDER, &replace, &[]).into_bytes());
    }
    // Document properties that changed, came or went.
    if old.properties != wb.properties {
        let mut root = pkg.relationships("")?;
        let root_count = root.items.len();
        for (kind, part_name, rel_type, content_type) in crate::write::PROPERTY_PARTS {
            let before = old.properties.iter().find(|p| p.0 == *kind).map(|p| &p.1);
            let now = wb.properties.iter().find(|p| p.0 == *kind).map(|p| &p.1);
            if before == now {
                continue;
            }
            let existing = root.items.iter().find(|r| r.rel_type == *rel_type || r.rel_type.ends_with(rel_type.rsplit('/').next().unwrap_or(""))).cloned();
            match (existing, now) {
                (Some(r), Some(xml)) => {
                    let Some(part) = resolve_target("", &r.target) else { return Ok(None) };
                    // The original's declaration (and anything before its root) stays.
                    let original = pkg.read(&part).ok().and_then(|b| xl_opc::decode_xml(&b).ok()).unwrap_or_default();
                    let prefix = original.find(|c: char| c == '<').map(|i| {
                        let mut at = i;
                        while original[at..].starts_with("<?") || original[at..].starts_with("<!") {
                            match original[at..].find('>').and_then(|e| original[at + e..].find('<').map(|n| at + e + n)) {
                                Some(next) => at = next,
                                None => break,
                            }
                        }
                        original[..at].to_string()
                    });
                    let prefix = prefix.unwrap_or_else(|| crate::write::HEADER.to_string());
                    pkg.set(&part, format!("{prefix}{xml}").into_bytes());
                }
                (Some(r), None) => {
                    let ct = content_types.get_or_insert(pkg.content_types()?);
                    if let Some(part) = resolve_target("", &r.target) {
                        pkg.remove(&part);
                        drop_override(ct, &part);
                    }
                    root.items.retain(|x| x.id != r.id);
                }
                (None, Some(xml)) => {
                    let ct = content_types.get_or_insert(pkg.content_types()?);
                    // A part at the usual place without a relationship is taken over.
                    pkg.set(part_name, format!("{}{xml}", crate::write::HEADER).into_bytes());
                    ct.set_override(&format!("/{part_name}"), content_type);
                    root.add(rel_type, part_name, xl_opc::TargetMode::Internal);
                }
                (None, None) => {}
            }
        }
        if root.items.len() != root_count {
            pkg.set_relationships("", &root);
        }
    }
    if !structure_same {
        // Sheets that are gone: their part, its relationships, the entry pointing to it.
        for o in (0..old.sheets.len()).filter(|o| !order.contains(&Some(*o))) {
            if let Some(part) = part_of(&rels, o) {
                pkg.remove(&part);
                pkg.remove(&xl_opc::rels_part_for(&part));
                let ct = content_types.get_or_insert(pkg.content_types()?);
                let key = xl_opc::normalize_key(&part);
                ct.items.retain(|item| !matches!(item, ContentTypeItem::Override { part_name, .. } if xl_opc::normalize_key(part_name) == key));
            }
            rels.items.retain(|r| r.id != rids[o]);
        }
    }

    // The calculation chain lists formula cells; it is optional and now stale.
    if changed {
        if let Some(rel) = rels.items.iter().find(|r| r.rel_type.ends_with("/calcChain")).cloned() {
            if let Some(part) = resolve_target(&wb_part, &rel.target) {
                pkg.remove(&part);
                let ct = content_types.get_or_insert(pkg.content_types()?);
                let key = xl_opc::normalize_key(&part);
                ct.items.retain(|item| !matches!(item, ContentTypeItem::Override { part_name, .. } if xl_opc::normalize_key(part_name) == key));
            }
            rels.items.retain(|r| r.id != rel.id);
        }
    }
    // Written only if a relationship was added or removed, so that the part
    // otherwise stays byte for byte.
    if rels.items.len() != rel_count || !structure_same {
        pkg.set_relationships(&wb_part, &rels);
    }
    if let Some(ct) = content_types {
        pkg.set_content_types(&ct);
    }
    Ok(Some(pkg.to_bytes()?))
}
