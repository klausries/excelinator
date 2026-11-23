//! [`Workbook`] → a new xlsx package.

use crate::model::*;
use crate::xml::{escape_attr, escape_text};
use crate::{Error, Result};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use xl_formula::col_name;
use xl_opc::{ContentTypeItem, ContentTypes, Package, Relationships, TargetMode};

const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT_PREFIX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";
pub(crate) const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// Longest text a cell can hold, in UTF-16 code units (Excel's limit).
pub const MAX_TEXT: usize = 32_767;

pub fn write_bytes(wb: &Workbook) -> Result<Vec<u8>> {
    Ok(write(wb)?.to_bytes()?)
}

/// Excel's rules for sheet names. The length limit counts UTF-16 code units.
fn check_sheet_name(name: &str) -> Result<()> {
    let bad = name.is_empty() || name.encode_utf16().count() > 31 || name.contains(['\\', '/', '?', '*', '[', ']', ':']) || name.starts_with('\'') || name.ends_with('\'');
    if bad {
        return Err(Error::Invalid(format!("sheet name {name:?} is not allowed")));
    }
    Ok(())
}

/// A number as xsd:double text that reads back to the same bits.
fn number(n: f64) -> String {
    let a = n.abs();
    if a != 0.0 && !(1e-5..1e16).contains(&a) {
        format!("{n:E}")
    } else {
        format!("{n}")
    }
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", col_name(col + 1), row + 1)
}

pub fn write(wb: &Workbook) -> Result<Package> {
    check_sheets(wb)?;
    for s in &wb.sheets {
        if s.kind != SheetKind::Worksheet {
            return Err(Error::Invalid(format!("sheet {:?}: only worksheets can be written to a new file", s.name)));
        }
    }
    let cells = &wb.cells;
    let order = cells.sorted_unique();
    check_cells(wb, &order)?;

    let mut pkg = Package::new();
    // Reserve the first entry: readers that sniff the file type expect it there.
    pkg.set(xl_opc::CONTENT_TYPES_PART, Vec::new());
    let mut ct = ContentTypes {
        items: vec![
            ContentTypeItem::Default { extension: "rels".into(), content_type: "application/vnd.openxmlformats-package.relationships+xml".into() },
            ContentTypeItem::Default { extension: "xml".into(), content_type: "application/xml".into() },
        ],
    };
    ct.set_override("/xl/workbook.xml", &format!("{CT_PREFIX}.sheet.main+xml"));
    ct.set_override("/xl/styles.xml", &format!("{CT_PREFIX}.styles+xml"));
    ct.set_override("/xl/sharedStrings.xml", &format!("{CT_PREFIX}.sharedStrings+xml"));

    let mut root = Relationships::default();
    root.add(&format!("{NS_REL}/officeDocument"), "xl/workbook.xml", TargetMode::Internal);
    pkg.set_relationships("", &root);

    // Shared strings and sheets.
    let mut dxfs: Vec<String> = Vec::new();
    let mut sst: Vec<(&str, bool)> = Vec::new();
    let mut sst_index: HashMap<(&str, bool), usize> = HashMap::new();
    let mut sst_refs = 0usize;

    let mut needs_calc = false;
    let mut table_id = 1u32;
    let keep_table_ids = table_ids_usable(wb);
    let mut table_names: Vec<String> = Vec::new();
    let mut pos = 0usize;
    let mut wb_rels = Relationships::default();
    let mut sheets_xml = String::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        let start = pos;
        while pos < order.len() && cells.sheet[order[pos]] as usize == si {
            pos += 1;
        }
        let mine = &order[start..pos];
        let body = sheet_body(wb, sheet, mine, Strings::Shared(&mut sst, &mut sst_index, &mut sst_refs), &mut needs_calc)?;
        let mut sheet_rels = Relationships::default();
        let link_ids = hyperlink_rels(sheet, &mut sheet_rels);
        let mut legacy = None;
        if !sheet.comments.is_empty() {
            let n = si + 1;
            pkg.set(&format!("xl/comments{n}.xml"), comments_xml(sheet).into_bytes());
            pkg.set(&format!("xl/drawings/vmlDrawing{n}.vml"), comments_vml(sheet)?.into_bytes());
            let (comments_type, vml_type) = comment_rel_types();
            sheet_rels.add(&comments_type, &format!("../comments{n}.xml"), TargetMode::Internal);
            legacy = Some(sheet_rels.add(&vml_type, &format!("../drawings/vmlDrawing{n}.vml"), TargetMode::Internal));
            ct.set_override(&format!("/xl/comments{n}.xml"), CT_COMMENTS);
            if !ct.items.iter().any(|i| matches!(i, ContentTypeItem::Default { extension, .. } if extension == "vml")) {
                ct.items.insert(0, ContentTypeItem::Default { extension: "vml".into(), content_type: CT_VML.into() });
            }
        }
        let drawing = if sheet.drawings.is_empty() { None } else { Some(add_drawing_parts(&mut pkg, "xl/", sheet, &mut sheet_rels, &mut ct)?) };
        let tables = add_table_parts(&mut pkg, "xl/", sheet, &mut sheet_rels, &mut ct, if keep_table_ids { None } else { Some(&mut table_id) }, &mut table_names)?;
        let x = sheet_xml(sheet, &body, &link_ids, &cond_formats_xml(sheet, &mut dxfs, 0), drawing.as_deref(), legacy.as_deref(), &tables);
        if !sheet_rels.items.is_empty() {
            pkg.set_relationships(&format!("xl/worksheets/sheet{}.xml", si + 1), &sheet_rels);
        }
        let part = format!("xl/worksheets/sheet{}.xml", si + 1);
        ct.set_override(&format!("/{part}"), &format!("{CT_PREFIX}.worksheet+xml"));
        let rid = wb_rels.add(&format!("{NS_REL}/worksheet"), &format!("worksheets/sheet{}.xml", si + 1), TargetMode::Internal);
        let state = match sheet.visibility {
            Visibility::Visible => "",
            Visibility::Hidden => " state=\"hidden\"",
            Visibility::VeryHidden => " state=\"veryHidden\"",
        };
        let _ = write!(sheets_xml, "<sheet name=\"{}\" sheetId=\"{}\"{state} r:id=\"{rid}\"/>", escape_attr(&sheet.name), si + 1);
        pkg.set(&part, x.into_bytes());
    }
    if wb.sheets.iter().all(|s| s.visibility != Visibility::Visible) {
        return Err(Error::Invalid("at least one sheet must be visible".into()));
    }

    let mut x = String::new();
    let _ = write!(x, "{HEADER}<sst xmlns=\"{NS_MAIN}\" count=\"{sst_refs}\" uniqueCount=\"{}\">", sst.len());
    for (s, rich) in &sst {
        if *rich {
            let _ = write!(x, "<si>{s}</si>");
            continue;
        }
        let space = if *s != s.trim() { " xml:space=\"preserve\"" } else { "" };
        let _ = write!(x, "<si><t{space}>{}</t></si>", escape_text(s));
    }
    x.push_str("</sst>");
    pkg.set("xl/sharedStrings.xml", x.into_bytes());

    pkg.set("xl/styles.xml", styles_xml(&wb.styles, &dxfs, &wb.named_styles, &StyleExtras::of(wb)).into_bytes());
    if let Some(theme) = &wb.theme {
        let body = match root_tag(theme) {
            Some((s, _)) => &theme[s..],
            None => return Err(Error::Invalid("theme: no root element".into())),
        };
        pkg.set("xl/theme/theme1.xml", format!("{HEADER}{body}").into_bytes());
        ct.set_override("/xl/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml");
        wb_rels.add(&format!("{NS_REL}/theme"), "theme/theme1.xml", TargetMode::Internal);
    }

    let mut parts = workbook_extra_xml(wb, needs_calc);
    parts.push(("sheets".into(), format!("<sheets>{sheets_xml}</sheets>")));
    if !wb.defined_names.is_empty() {
        parts.push(("definedNames".into(), format!("<definedNames>{}</definedNames>", wb.defined_names.iter().map(defined_name_xml).collect::<String>())));
    }
    let rank = |n: &str| WORKBOOK_ORDER.iter().position(|o| *o == n).unwrap_or(WORKBOOK_ORDER.len());
    parts.sort_by_key(|p| rank(&p.0));
    let mut x = String::new();
    let _ = write!(x, "{HEADER}<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">");
    for (_, p) in &parts {
        x.push_str(p);
    }
    x.push_str("</workbook>");
    pkg.set("xl/workbook.xml", x.into_bytes());

    let mut root = pkg.relationships("")?;
    for (kind, xml) in &wb.properties {
        let Some((_, part, rel_type, content_type)) = PROPERTY_PARTS.iter().find(|p| p.0 == kind) else {
            return Err(Error::Invalid(format!("unknown document property part {kind:?}")));
        };
        crate::xml::walk(xml, &mut |_| Ok(())).map_err(|e| Error::Invalid(format!("document properties {kind:?}: {e}")))?;
        let body = match root_tag(xml) {
            Some((s, _)) => &xml[s..],
            None => return Err(Error::Invalid(format!("document properties {kind:?}: no root element"))),
        };
        pkg.set(part, format!("{HEADER}{body}").into_bytes());
        ct.set_override(&format!("/{part}"), content_type);
        root.add(rel_type, part, TargetMode::Internal);
    }
    pkg.set_relationships("", &root);
    wb_rels.add(&format!("{NS_REL}/styles"), "styles.xml", TargetMode::Internal);
    wb_rels.add(&format!("{NS_REL}/sharedStrings"), "sharedStrings.xml", TargetMode::Internal);
    pkg.set_relationships("xl/workbook.xml", &wb_rels);
    pkg.set_content_types(&ct);
    Ok(pkg)
}

/// Sheet names must be valid and unique.
pub(crate) fn check_sheets(wb: &Workbook) -> Result<()> {
    if wb.sheets.is_empty() {
        return Err(Error::Invalid("a workbook needs at least one sheet".into()));
    }
    let mut seen: Vec<String> = Vec::new();
    for s in &wb.sheets {
        check_sheet_name(&s.name)?;
        let key = s.name.to_lowercase();
        if seen.contains(&key) {
            return Err(Error::Invalid(format!("duplicate sheet name {:?}", s.name)));
        }
        seen.push(key);
    }
    Ok(())
}

/// Limits every cell must keep: grid size, known sheet and style, text length.
pub(crate) fn check_cells(wb: &Workbook, order: &[usize]) -> Result<()> {
    let cells = &wb.cells;
    for &i in order {
        if cells.sheet[i] as usize >= wb.sheets.len() {
            return Err(Error::Invalid(format!("cell refers to sheet {} of {}", cells.sheet[i], wb.sheets.len())));
        }
        if cells.row[i] >= xl_formula::MAX_ROW || cells.col[i] >= xl_formula::MAX_COL {
            return Err(Error::Invalid(format!("cell ({}, {}) is outside the grid", cells.row[i], cells.col[i])));
        }
        if matches!(cells.kind[i], Kind::Text) && cells.text[i].as_deref().is_some_and(|t| t.len() > MAX_TEXT && t.encode_utf16().count() > MAX_TEXT) {
            return Err(Error::Invalid(format!("cell ({}, {}): text is longer than {MAX_TEXT} characters", cells.row[i], cells.col[i])));
        }
        if cells.style[i] as usize >= wb.styles.len() {
            return Err(Error::Invalid(format!("cell refers to style {} of {}", cells.style[i], wb.styles.len())));
        }
    }
    Ok(())
}

/// Children of `<workbook>` kept as XML fragments (`Workbook::extra`).
pub(crate) const WORKBOOK_EXTRA: &[&str] = &[
    "fileVersion", "fileSharing", "workbookPr", "workbookProtection", "bookViews", "calcPr", "oleSize", "customWorkbookViews", "smartTagPr", "smartTagTypes",
    "webPublishing", "fileRecoveryPr", "webPublishObjects",
];

/// Order of the children of `<workbook>` (ECMA-376 part 1, 18.2.27).
pub(crate) const WORKBOOK_ORDER: &[&str] = &[
    "fileVersion", "fileSharing", "workbookPr", "workbookProtection", "bookViews", "sheets", "functionGroups", "externalReferences", "definedNames", "calcPr", "oleSize",
    "customWorkbookViews", "pivotCaches", "smartTagPr", "smartTagTypes", "webPublishing", "fileRecoveryPr", "webPublishObjects", "extLst",
];

/// Document property parts: (kind, part, relationship type, content type).
pub(crate) const PROPERTY_PARTS: &[(&str, &str, &str, &str)] = &[
    ("core", "docProps/core.xml", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "application/vnd.openxmlformats-package.core-properties+xml"),
    ("app", "docProps/app.xml", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
    ("custom", "docProps/custom.xml", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties", "application/vnd.openxmlformats-officedocument.custom-properties+xml"),
];

/// Set (or add) an attribute on the root element of a fragment; `None` removes nothing
/// and adds nothing.
pub(crate) fn with_root_attr(xml: &str, name: &str, value: &str) -> String {
    let Some((s, e)) = root_tag(xml) else { return xml.to_string() };
    let tag = &xml[s..e];
    let new = crate::update::set_attr(tag, name, value).unwrap_or_else(|| {
        let close = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
        format!("{} {name}=\"{value}\"{}", tag[..close].trim_end(), &tag[close..])
    });
    format!("{}{new}{}", &xml[..s], &xml[e..])
}

/// The children of `<workbook>` other than the sheet list and the names, each as XML,
/// with what the model says about them applied.
pub(crate) fn workbook_extra_xml(wb: &Workbook, needs_calc: bool) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let extra = |n: &str| wb.extra.iter().find(|e| e.0 == n).map(|e| e.1.clone());
    for (name, xml) in &wb.extra {
        if WORKBOOK_EXTRA.contains(&name.as_str()) && !matches!(name.as_str(), "workbookPr" | "bookViews" | "calcPr") {
            out.push((name.clone(), xml.clone()));
        }
    }
    match extra("workbookPr") {
        Some(x) => {
            let given = root_attr(&x, "date1904").map(|v| v == "1" || v == "true");
            let x = if given.unwrap_or(false) == wb.date1904 { x } else { with_root_attr(&x, "date1904", if wb.date1904 { "1" } else { "0" }) };
            out.push(("workbookPr".into(), x));
        }
        None if wb.date1904 => out.push(("workbookPr".into(), "<workbookPr date1904=\"1\"/>".into())),
        None => {}
    }
    match extra("bookViews") {
        Some(x) => out.push(("bookViews".into(), clamp_tabs(&x, wb.sheets.len()))),
        None if needs_views(wb) => out.push(("bookViews".into(), "<bookViews><workbookView/></bookViews>".into())),
        None => {}
    }
    match extra("calcPr") {
        Some(x) if needs_calc => out.push(("calcPr".into(), with_root_attr(&x, "fullCalcOnLoad", "1"))),
        Some(x) => out.push(("calcPr".into(), x)),
        None if needs_calc => out.push(("calcPr".into(), "<calcPr fullCalcOnLoad=\"1\"/>".into())),
        None => {}
    }
    out
}

/// Workbook views whose active or first tab lies past the last sheet point to the first.
pub(crate) fn clamp_tabs(x: &str, sheets: usize) -> String {
    let tab = |a: &str| nested_attr(x, "workbookView", a).and_then(|v| v.parse::<usize>().ok());
    let mut out = x.to_string();
    for a in ["activeTab", "firstSheet"] {
        if let Some(t) = tab(a).filter(|t| *t >= sheets) {
            out = out.replacen(&format!("{a}=\"{t}\""), &format!("{a}=\"0\""), 1);
        }
    }
    out
}

/// The value of an attribute on the first element `name` inside a fragment.
pub(crate) fn nested_attr(xml: &str, name: &str, attr: &str) -> Option<String> {
    let start = xml.find(&format!("<{name}"))?;
    let end = start + xml[start..].find('>')? + 1;
    root_attr(&xml[start..end], attr)
}

fn needs_views(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|s| s.freeze.is_some())
}

/// Where string cells go.
pub(crate) enum Strings<'a, 'b> {
    /// Items (plain text, or a rich item's XML: `true`), their index, the reference count.
    Shared(&'b mut Vec<(&'a str, bool)>, &'b mut HashMap<(&'a str, bool), usize>, &'b mut usize),
    Inline,
}

pub(crate) struct SheetBody {
    pub dimension: String,
    pub cols: String,
    pub data: String,
    pub merges: String,
}

/// The comments part of a sheet.
pub(crate) fn comments_xml(sheet: &Sheet) -> String {
    let mut authors: Vec<&str> = Vec::new();
    let mut list = String::new();
    for c in &sheet.comments {
        let id = authors.iter().position(|a| *a == c.author).unwrap_or_else(|| {
            authors.push(&c.author);
            authors.len() - 1
        });
        let _ = write!(list, "<comment ref=\"{}\" authorId=\"{id}\"><text><t xml:space=\"preserve\">{}</t></text></comment>", escape_attr(&c.cell), escape_text(&c.text));
    }
    let authors: String = authors.iter().map(|a| format!("<author>{}</author>", escape_text(a))).collect();
    format!("{HEADER}<comments xmlns=\"{NS_MAIN}\"><authors>{authors}</authors><commentList>{list}</commentList></comments>")
}

/// The drawing part that makes a sheet's comments visible (one box per comment).
pub(crate) fn comments_vml(sheet: &Sheet) -> Result<String> {
    let mut x = String::from(concat!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">",
        "<o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout>",
        "<v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\">",
        "<v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>"
    ));
    for (i, c) in sheet.comments.iter().enumerate() {
        let Some((row, col)) = xl_formula::parse_cell_ref(&c.cell).map(|r| (r.row - 1, r.col - 1)) else {
            return Err(Error::Invalid(format!("sheet {:?}: comment on {:?} is not on a cell", sheet.name, c.cell)));
        };
        let _ = write!(
            x,
            concat!(
                "<v:shape id=\"_x0000_s{}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:59.25pt;margin-top:1.5pt;width:108pt;height:59.25pt;z-index:{};visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\">",
                "<v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/>",
                "<v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox>",
                "<x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column></x:ClientData></v:shape>"
            ),
            1025 + i,
            i + 1,
            row,
            col
        );
    }
    x.push_str("</xml>");
    Ok(x)
}

const NS_XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

fn image_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "svg" => "image/svg+xml",
        "pct" | "pict" => "image/x-pict",
        _ => return None,
    })
}

pub(crate) fn drawing_element_xml(id: &str) -> String {
    format!("<drawing xmlns:r=\"{NS_REL}\" r:id=\"{id}\"/>")
}

/// Create the drawing part of a sheet with its pictures and charts under unused part
/// names below `dir`. Returns the id of the relationship added to `sheet_rels`.
pub(crate) fn add_drawing_parts(pkg: &mut Package, dir: &str, sheet: &Sheet, sheet_rels: &mut Relationships, ct: &mut ContentTypes) -> Result<String> {
    let unused = |pkg: &Package, pattern: &dyn Fn(u32) -> String| (1..1_000_000).map(pattern).find(|p| !pkg.contains(p)).unwrap_or_default();
    let part = unused(pkg, &|n| format!("{dir}drawings/drawing{n}.xml"));
    let mut rels = Relationships::default();
    let mut x = format!("{HEADER}<xdr:wsDr xmlns:xdr=\"{NS_XDR}\" xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_REL}\" xmlns:c=\"{NS_C}\">");
    let mut media: Vec<(Arc<[u8]>, String)> = Vec::new();
    for (i, d) in sheet.drawings.iter().enumerate() {
        let point = |tag: &str, p: (u32, i64, u32, i64)| format!("<xdr:{tag}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{tag}>", p.0, p.1, p.2, p.3);
        let (open, close) = match (d.anchor.to, d.anchor.ext) {
            (Some(to), _) => (format!("<xdr:twoCellAnchor>{}{}", point("from", d.anchor.from), point("to", to)), "</xdr:twoCellAnchor>"),
            (None, Some((cx, cy))) => (format!("<xdr:oneCellAnchor>{}<xdr:ext cx=\"{cx}\" cy=\"{cy}\"/>", point("from", d.anchor.from)), "</xdr:oneCellAnchor>"),
            (None, None) => return Err(Error::Invalid(format!("sheet {:?}: a picture or chart needs a size or an end cell", sheet.name))),
        };
        x.push_str(&open);
        match (&d.image, &d.chart) {
            (Some((data, ext)), None) => {
                let Some(content_type) = image_type(ext) else { return Err(Error::Invalid(format!("sheet {:?}: picture type {ext:?} is not supported", sheet.name))) };
                // The same picture placed twice is stored once.
                let target = match media.iter().find(|m| m.0 == *data) {
                    Some(m) => m.1.clone(),
                    None => {
                        let p = unused(pkg, &|n| format!("{dir}media/image{n}.{ext}"));
                        pkg.set(&p, data.to_vec());
                        if !ct.items.iter().any(|it| matches!(it, ContentTypeItem::Default { extension, .. } if extension.eq_ignore_ascii_case(ext))) {
                            ct.items.insert(0, ContentTypeItem::Default { extension: ext.clone(), content_type: content_type.into() });
                        }
                        media.push((data.clone(), p.clone()));
                        p
                    }
                };
                let id = rels.add(&format!("{NS_REL}/image"), &format!("/{target}"), TargetMode::Internal);
                let _ = write!(x, "<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{n}\" name=\"Image {n}\"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>", n = i + 1);
            }
            (None, Some(chart)) => {
                let p = unused(pkg, &|n| format!("{dir}charts/chart{n}.xml"));
                pkg.set(&p, chart.as_bytes().to_vec());
                ct.set_override(&format!("/{p}"), "application/vnd.openxmlformats-officedocument.drawingml.chart+xml");
                let id = rels.add(&format!("{NS_REL}/chart"), &format!("/{p}"), TargetMode::Internal);
                let _ = write!(x, "<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{n}\" name=\"Chart {n}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"{NS_C}\"><c:chart r:id=\"{id}\"/></a:graphicData></a:graphic></xdr:graphicFrame>", n = i + 1);
            }
            _ => return Err(Error::Invalid(format!("sheet {:?}: a drawing is either a picture or a chart", sheet.name))),
        }
        x.push_str("<xdr:clientData/>");
        x.push_str(close);
    }
    x.push_str("</xdr:wsDr>");
    pkg.set(&part, x.into_bytes());
    pkg.set_relationships(&part, &rels);
    ct.set_override(&format!("/{part}"), "application/vnd.openxmlformats-officedocument.drawing+xml");
    Ok(sheet_rels.add(&format!("{NS_REL}/drawing"), &format!("/{part}"), TargetMode::Internal))
}

pub(crate) const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";

/// The start tag of an XML fragment's root element: (start, end) byte offsets.
fn root_tag(xml: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    loop {
        let start = from + xml[from..].find('<')?;
        if xml[start + 1..].starts_with(['?', '!']) {
            from = start + 1;
            continue;
        }
        let end = start + xml[start..].find('>')? + 1;
        return Some((start, end));
    }
}

/// The value of an attribute of a fragment's root element.
pub(crate) fn root_attr(xml: &str, name: &str) -> Option<String> {
    let (s, e) = root_tag(xml)?;
    let tag = &xml[s..e];
    ['"', '\''].iter().find_map(|q| {
        let key = format!(" {name}={q}");
        let from = tag.find(&key)? + key.len();
        let len = tag[from..].find(*q)?;
        Some(crate::xml::unescape_xstring(&tag[from..from + len].replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")))
    })
}

/// Whether the tables of a workbook keep their ids: all have one, and no two the same.
pub(crate) fn table_ids_usable(wb: &Workbook) -> bool {
    let mut seen = std::collections::HashSet::new();
    wb.sheets.iter().flat_map(|s| &s.tables).all(|t| root_attr(t, "id").and_then(|v| v.parse::<u32>().ok()).is_some_and(|id| id > 0 && seen.insert(id)))
}

/// Write the table parts of a sheet under `dir` (`xl/`). With `next_id`, ids are
/// numbered from it; without, each table keeps its own. Returns the `<tableParts>`
/// element, empty without tables.
pub(crate) fn add_table_parts(pkg: &mut Package, dir: &str, sheet: &Sheet, sheet_rels: &mut Relationships, ct: &mut ContentTypes, mut next_id: Option<&mut u32>, names: &mut Vec<String>) -> Result<String> {
    if sheet.tables.is_empty() {
        return Ok(String::new());
    }
    let mut x = format!("<tableParts xmlns:r=\"{NS_REL}\" count=\"{}\">", sheet.tables.len());
    for table in &sheet.tables {
        let bad = |what: &str| Error::Invalid(format!("sheet {:?}: table: {what}", sheet.name));
        let (s, e) = root_tag(table).ok_or_else(|| bad("no root element"))?;
        let tag = &table[s..e];
        let name_end = tag[1..].find([' ', '>', '/']).map(|i| i + 1).unwrap_or(tag.len());
        if tag[1..name_end].rsplit(':').next() != Some("table") {
            return Err(bad("the root element must be <table>"));
        }
        let name = root_attr(table, "displayName").ok_or_else(|| bad("no displayName"))?;
        if names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            return Err(bad(&format!("the name {name:?} is used twice")));
        }
        names.push(name);
        crate::xml::walk(table, &mut |_| Ok(())).map_err(|e| bad(&format!("not well-formed: {e}")))?;
        let tag = match next_id.as_deref_mut() {
            None => tag.to_string(),
            Some(next) => {
                let id = *next;
                *next += 1;
                match crate::update::set_attr(tag, "id", &id.to_string()) {
                    Some(t) => t,
                    None => format!("{} id=\"{id}\"{}", &tag[..tag.len() - 1].trim_end_matches('/'), if tag.ends_with("/>") { "/>" } else { ">" }),
                }
            }
        };
        let xml = format!("{HEADER}{tag}{}", &table[e..]);
        let part = (1..1_000_000).map(|n| format!("{dir}tables/table{n}.xml")).find(|p| !pkg.contains(p)).unwrap_or_default();
        pkg.set(&part, xml.into_bytes());
        ct.set_override(&format!("/{part}"), CT_TABLE);
        let rid = sheet_rels.add(&format!("{NS_REL}/table"), &format!("/{part}"), TargetMode::Internal);
        let _ = write!(x, "<tablePart r:id=\"{rid}\"/>");
    }
    x.push_str("</tableParts>");
    Ok(x)
}

pub(crate) const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
pub(crate) const CT_VML: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";

/// Number of a differential format in `dxfs`, adding it if new.
pub(crate) fn dxf_id(dxfs: &mut Vec<String>, base: usize, dxf: &str) -> usize {
    match dxfs.iter().position(|d| d == dxf) {
        Some(i) => base + i,
        None => {
            dxfs.push(dxf.to_string());
            base + dxfs.len() - 1
        }
    }
}

/// The `<conditionalFormatting>` elements of a sheet; new differential formats are
/// added to `dxfs`, which continues a list of `base` existing ones.
pub(crate) fn cond_formats_xml(sheet: &Sheet, dxfs: &mut Vec<String>, base: usize) -> String {
    let mut x = String::new();
    for c in &sheet.cond_formats {
        let _ = write!(x, "<conditionalFormatting sqref=\"{}\">", escape_attr(&c.sqref));
        for r in &c.rules {
            x.push_str("<cfRule");
            for (k, v) in &r.attrs {
                let _ = write!(x, " {k}=\"{}\"", escape_attr(v));
                // The schema wants the format right after the type.
                if k == "type" {
                    if let Some(d) = &r.dxf {
                        let _ = write!(x, " dxfId=\"{}\"", dxf_id(dxfs, base, d));
                    }
                }
            }
            let _ = write!(x, ">{}</cfRule>", r.inner);
        }
        x.push_str("</conditionalFormatting>");
    }
    x
}

pub(crate) fn dxfs_xml(dxfs: &[String]) -> String {
    dxfs.iter().map(|d| format!("<dxf>{d}</dxf>")).collect()
}

pub(crate) fn auto_filter_xml(sheet: &Sheet) -> String {
    sheet.auto_filter.as_ref().map(|r| format!("<autoFilter ref=\"{}\"/>", escape_attr(r))).unwrap_or_default()
}

pub(crate) fn validations_xml(sheet: &Sheet) -> String {
    if sheet.validations.is_empty() {
        return String::new();
    }
    let mut x = format!("<dataValidations count=\"{}\">", sheet.validations.len());
    for v in &sheet.validations {
        x.push_str("<dataValidation");
        for (k, val) in &v.attrs {
            let _ = write!(x, " {k}=\"{}\"", escape_attr(val));
        }
        x.push('>');
        if let Some(f) = &v.formula1 {
            let _ = write!(x, "<formula1>{}</formula1>", escape_text(f));
        }
        if let Some(f) = &v.formula2 {
            let _ = write!(x, "<formula2>{}</formula2>", escape_text(f));
        }
        x.push_str("</dataValidation>");
    }
    x.push_str("</dataValidations>");
    x
}

/// The `<hyperlinks>` element; `ids[i]` is the relationship id of link `i` if it has
/// an external target. The relationships namespace is declared on the element.
pub(crate) fn hyperlinks_xml(sheet: &Sheet, ids: &[Option<String>]) -> String {
    if sheet.hyperlinks.is_empty() {
        return String::new();
    }
    let mut x = format!("<hyperlinks xmlns:r=\"{NS_REL}\">");
    for (h, id) in sheet.hyperlinks.iter().zip(ids) {
        let _ = write!(x, "<hyperlink ref=\"{}\"", escape_attr(&h.cell));
        if let Some(id) = id {
            let _ = write!(x, " r:id=\"{id}\"");
        }
        for (k, v) in [("location", &h.location), ("tooltip", &h.tooltip), ("display", &h.display)] {
            if let Some(v) = v {
                let _ = write!(x, " {k}=\"{}\"", escape_attr(v));
            }
        }
        x.push_str("/>");
    }
    x.push_str("</hyperlinks>");
    x
}

/// Add relationships for the external targets of a sheet's hyperlinks.
pub(crate) fn hyperlink_rels(sheet: &Sheet, rels: &mut Relationships) -> Vec<Option<String>> {
    sheet.hyperlinks.iter().map(|h| h.target.as_ref().map(|t| rels.add(&format!("{NS_REL}/hyperlink"), t, TargetMode::External))).collect()
}

/// One `<c>` element.
pub(crate) fn cell_xml<'a>(cells: &'a Cells, i: usize, strings: &mut Strings<'a, '_>, needs_calc: &mut bool, date1904: bool, x: &mut String) {
    let _ = write!(x, "<c r=\"{}\"", a1(cells.row[i], cells.col[i]));
    let formula = cells.formula[i].as_deref().filter(|f| !f.is_empty());
    // An empty cell of format 0 says so: in a formatted row or column it has format 0,
    // not the row's or column's (a `<c>` without `s` would be read as nothing).
    if cells.style[i] != 0 || (cells.kind[i] == Kind::Blank && formula.is_none()) {
        let _ = write!(x, " s=\"{}\"", cells.style[i]);
    }
    let text = cells.text[i].as_deref().unwrap_or("");
    let rich = cells.rich[i].as_deref().filter(|_| formula.is_none() && cells.kind[i] == Kind::Text);
    let mut inline = false;
    let iso = if cells.kind[i] == Kind::Number && formula.is_none() { cells.iso[i].format(cells.num[i], date1904) } else { None };
    let (ty, value): (&str, Option<String>) = match cells.kind[i] {
        Kind::Blank => ("", None),
        Kind::Number if iso.is_some() => ("d", iso),
        Kind::Number if cells.float_text[i] && cells.num[i].fract() == 0.0 && cells.num[i].abs() < 1e16 => ("", Some(format!("{:.1}", cells.num[i]))),
        Kind::Number if cells.num[i].is_finite() => ("", Some(number(cells.num[i]))),
        Kind::Number => ("e", Some("#NUM!".into())),
        Kind::Bool => ("b", Some(if cells.num[i] != 0.0 { "1".into() } else { "0".into() })),
        Kind::Error => ("e", Some(escape_text(text))),
        Kind::Text if formula.is_some() => ("str", Some(escape_text(text))),
        Kind::Text => match strings {
            Strings::Shared(sst, index, refs) => {
                **refs += 1;
                let n = sst.len();
                let key = match rich {
                    Some(r) => (r, true),
                    None => (text, false),
                };
                let id = *index.entry(key).or_insert_with(|| {
                    sst.push(key);
                    n
                });
                ("s", Some(id.to_string()))
            }
            Strings::Inline => {
                inline = true;
                ("inlineStr", Some(escape_text(text)))
            }
        },
    };
    if !ty.is_empty() {
        let _ = write!(x, " t=\"{ty}\"");
    }
    if formula.is_none() && value.is_none() {
        x.push_str("/>");
        return;
    }
    x.push('>');
    if let Some(f) = formula {
        *needs_calc |= value.is_none();
        match &cells.array_ref[i] {
            Some(r) => {
                let _ = write!(x, "<f t=\"array\" ref=\"{}\">{}</f>", escape_attr(r), escape_text(f));
            }
            None => {
                let _ = write!(x, "<f>{}</f>", escape_text(f));
            }
        }
    }
    if let Some(v) = value {
        if let (true, Some(r)) = (inline, rich) {
            let _ = write!(x, "<is>{r}</is>");
        } else if inline {
            let space = if text != text.trim() { " xml:space=\"preserve\"" } else { "" };
            let _ = write!(x, "<is><t{space}>{v}</t></is>");
        } else {
            let _ = write!(x, "<v>{v}</v>");
        }
    }
    x.push_str("</c>");
}

pub(crate) fn legacy_drawing_xml(id: &str) -> String {
    format!("<legacyDrawing xmlns:r=\"{NS_REL}\" r:id=\"{id}\"/>")
}

/// Relationship types of the two parts behind a sheet's comments.
pub(crate) fn comment_rel_types() -> (String, String) {
    (format!("{NS_REL}/comments"), format!("{NS_REL}/vmlDrawing"))
}

pub(crate) fn defined_name_xml(n: &DefinedName) -> String {
    let local = n.local_sheet.map(|i| format!(" localSheetId=\"{i}\"")).unwrap_or_default();
    let hidden = if n.hidden { " hidden=\"1\"" } else { "" };
    format!("<definedName name=\"{}\"{local}{hidden}>{}</definedName>", escape_attr(&n.name), escape_text(&n.value))
}

/// A complete worksheet part from its body.
/// Order of the children of `<worksheet>` (ECMA-376 part 1, 18.3.1.99).
pub(crate) const SHEET_ORDER: &[&str] = &[
    "sheetPr", "dimension", "sheetViews", "sheetFormatPr", "cols", "sheetData", "sheetCalcPr", "sheetProtection", "protectedRanges", "scenarios", "autoFilter", "sortState",
    "dataConsolidate", "customSheetViews", "mergeCells", "phoneticPr", "conditionalFormatting", "dataValidations", "hyperlinks", "printOptions", "pageMargins", "pageSetup",
    "headerFooter", "rowBreaks", "colBreaks", "customProperties", "cellWatches", "ignoredErrors", "smartTags", "drawing", "legacyDrawing", "legacyDrawingHF", "drawingHF",
    "picture", "oleObjects", "controls", "webPublishItems", "tableParts", "extLst",
];

/// The children of `<worksheet>` kept and written as XML fragments (`Sheet::extra`).
pub(crate) const EXTRA_ELEMENTS: &[&str] = &[
    "sheetPr", "sheetViews", "sheetFormatPr", "sheetCalcPr", "sheetProtection", "protectedRanges", "autoFilter", "sortState", "printOptions", "pageMargins", "pageSetup",
    "headerFooter", "rowBreaks", "colBreaks", "ignoredErrors",
];

/// A fragment without its references to relationships (for a part that has none): what
/// writing a new file keeps of a sheet's printer settings and the like.
pub fn without_rel_ids(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find(" r:id=\"") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 7..];
        rest = after.find('"').map(|j| &after[j + 1..]).unwrap_or("");
    }
    out.push_str(rest);
    out.replace(&format!(" xmlns:r=\"{NS_REL}\""), "")
}

/// The sheet view for frozen panes (rows, columns), as this crate writes it.
pub(crate) fn freeze_views_xml(freeze: Option<(u32, u32)>) -> Option<String> {
    let (rows, cols) = freeze?;
    let pane = match (rows, cols) {
        (0, _) => "topRight",
        (_, 0) => "bottomLeft",
        _ => "bottomRight",
    };
    let mut x = String::from("<sheetViews><sheetView workbookViewId=\"0\"><pane");
    if cols > 0 {
        let _ = write!(x, " xSplit=\"{cols}\"");
    }
    if rows > 0 {
        let _ = write!(x, " ySplit=\"{rows}\"");
    }
    let _ = write!(x, " topLeftCell=\"{}\" activePane=\"{pane}\" state=\"frozen\"/></sheetView></sheetViews>", a1(rows, cols));
    Some(x)
}

pub(crate) fn sheet_xml(sheet: &Sheet, body: &SheetBody, link_ids: &[Option<String>], cond_formats: &str, drawing: Option<&str>, legacy_drawing: Option<&str>, table_parts: &str) -> String {
    let mut parts: Vec<(&str, String)> = Vec::new();
    let extra = |name: &str| sheet.extra.iter().find(|e| e.0 == name).map(|e| without_rel_ids(&e.1));
    parts.push(("dimension", body.dimension.clone()));
    match extra("sheetViews") {
        Some(x) => parts.push(("sheetViews", x)),
        None => {
            if let Some(x) = freeze_views_xml(sheet.freeze) {
                parts.push(("sheetViews", x));
            }
        }
    }
    parts.push(("cols", body.cols.clone()));
    parts.push(("sheetData", body.data.clone()));
    match extra("autoFilter") {
        Some(x) => parts.push(("autoFilter", x)),
        None => parts.push(("autoFilter", auto_filter_xml(sheet))),
    }
    parts.push(("mergeCells", body.merges.clone()));
    parts.push(("conditionalFormatting", cond_formats.to_string()));
    parts.push(("dataValidations", validations_xml(sheet)));
    parts.push(("hyperlinks", hyperlinks_xml(sheet, link_ids)));
    if let Some(id) = drawing {
        parts.push(("drawing", drawing_element_xml(id)));
    }
    if let Some(id) = legacy_drawing {
        parts.push(("legacyDrawing", legacy_drawing_xml(id)));
    }
    parts.push(("tableParts", table_parts.to_string()));
    for (name, xml) in &sheet.extra {
        if !matches!(name.as_str(), "sheetViews" | "autoFilter") && EXTRA_ELEMENTS.contains(&name.as_str()) {
            parts.push((name.as_str(), without_rel_ids(xml)));
        }
    }
    let rank = |n: &str| SHEET_ORDER.iter().position(|o| *o == n).unwrap_or(SHEET_ORDER.len());
    parts.sort_by_key(|p| rank(p.0));
    let mut x = String::with_capacity(256 + body.data.len());
    let _ = write!(x, "{HEADER}<worksheet xmlns=\"{NS_MAIN}\">");
    for (_, p) in parts {
        x.push_str(&p);
    }
    x.push_str("</worksheet>");
    x
}

/// `<cols>` of a sheet; empty without column attributes.
pub(crate) fn cols_xml(sheet: &Sheet, style_count: usize) -> Result<String> {
    if sheet.columns.is_empty() {
        return Ok(String::new());
    }
    let mut x = String::from("<cols>");
    let mut after = 0u32;
    for c in &sheet.columns {
        let bad = |what: &str| Error::Invalid(format!("sheet {:?}: column range {}..={}: {what}", sheet.name, c.first, c.last));
        if c.first > c.last || c.last >= xl_formula::MAX_COL || c.first < after {
            return Err(bad("out of order, overlapping or beyond the sheet"));
        }
        after = c.last + 1;
        let _ = write!(x, "<col min=\"{}\" max=\"{}\"", c.first + 1, c.last + 1);
        if let Some(w) = c.width {
            // Excel caps widths at 255 and heights at 409.5 when it opens a file; larger
            // values occur in files and are kept.
            if !w.is_finite() || w < 0.0 {
                return Err(bad("width not a non-negative number"));
            }
            let _ = write!(x, " width=\"{}\"", number(w));
        }
        if let Some(s) = c.style {
            if s as usize >= style_count {
                return Err(bad("unknown style"));
            }
            let _ = write!(x, " style=\"{s}\"");
        }
        if c.outline_level > 7 {
            return Err(bad("outline level beyond 7"));
        }
        for (on, name) in [(c.hidden, "hidden"), (c.best_fit, "bestFit"), (c.custom_width, "customWidth"), (c.phonetic, "phonetic")] {
            if on {
                let _ = write!(x, " {name}=\"1\"");
            }
        }
        if c.outline_level > 0 {
            let _ = write!(x, " outlineLevel=\"{}\"", c.outline_level);
        }
        if c.collapsed {
            x.push_str(" collapsed=\"1\"");
        }
        x.push_str("/>");
    }
    x.push_str("</cols>");
    Ok(x)
}

/// The start tag of a `<row>` with its attributes, without the closing `>`.
fn row_tag(a: &Row, style_count: usize) -> std::result::Result<String, String> {
    let mut x = format!("<row r=\"{}\"", a.row + 1);
    if let Some(s) = a.style {
        if s as usize >= style_count {
            return Err(format!("row {}: unknown style", a.row));
        }
        let _ = write!(x, " s=\"{s}\"");
    }
    if a.custom_format {
        x.push_str(" customFormat=\"1\"");
    }
    if let Some(h) = a.height {
        if !h.is_finite() || h < 0.0 {
            return Err(format!("row {}: height not a non-negative number", a.row));
        }
        let _ = write!(x, " ht=\"{}\"", number(h));
    }
    if a.hidden {
        x.push_str(" hidden=\"1\"");
    }
    if a.custom_height {
        x.push_str(" customHeight=\"1\"");
    }
    if a.outline_level > 7 {
        return Err(format!("row {}: outline level beyond 7", a.row));
    }
    if a.outline_level > 0 {
        let _ = write!(x, " outlineLevel=\"{}\"", a.outline_level);
    }
    for (on, name) in [(a.collapsed, "collapsed"), (a.thick_top, "thickTop"), (a.thick_bot, "thickBot"), (a.phonetic, "ph")] {
        if on {
            let _ = write!(x, " {name}=\"1\"");
        }
    }
    Ok(x)
}

/// The parts of a worksheet this crate models, as XML fragments.
pub(crate) fn sheet_body<'a>(wb: &'a Workbook, sheet: &Sheet, mine: &[usize], mut strings: Strings<'a, '_>, needs_calc: &mut bool) -> Result<SheetBody> {
    let cells = &wb.cells;
    let mut body = SheetBody { dimension: String::new(), cols: String::new(), data: String::with_capacity(64 + mine.len() * 32), merges: String::new() };
    if let (Some(&first), Some(&last)) = (mine.first(), mine.last()) {
        let (c1, c2) = mine.iter().fold((u32::MAX, 0), |(lo, hi), &i| (lo.min(cells.col[i]), hi.max(cells.col[i])));
        body.dimension = format!("<dimension ref=\"{}:{}\"/>", a1(cells.row[first], c1), a1(cells.row[last], c2));
    }
    body.cols = cols_xml(sheet, wb.styles.len().max(1))?;
    let style_count = wb.styles.len().max(1);
    let x = &mut body.data;
    x.push_str("<sheetData>");
    let mut attrs = sheet.rows.iter().peekable();
    let mut last_attr_row: Option<u32> = None;
    let mut open_row = |x: &mut String, r: u32| -> Result<()> {
        // Rows that only have attributes come first.
        while let Some(&a) = attrs.peek() {
            if a.row > r {
                break;
            }
            attrs.next();
            if a.row >= xl_formula::MAX_ROW || last_attr_row.is_some_and(|l| l >= a.row) {
                return Err(Error::Invalid(format!("sheet {:?}: rows out of order or beyond the sheet ({})", sheet.name, a.row)));
            }
            last_attr_row = Some(a.row);
            let tag = row_tag(a, style_count).map_err(|e| Error::Invalid(format!("sheet {:?}: {e}", sheet.name)))?;
            if a.row < r {
                let _ = write!(x, "{tag}/>");
            } else {
                let _ = write!(x, "{tag}>");
                return Ok(());
            }
        }
        if r != u32::MAX {
            let _ = write!(x, "<row r=\"{}\">", r + 1);
        }
        Ok(())
    };
    let mut row: Option<u32> = None;
    for &i in mine {
        if row != Some(cells.row[i]) {
            if row.is_some() {
                x.push_str("</row>");
            }
            row = Some(cells.row[i]);
            open_row(x, cells.row[i])?;
        }
        cell_xml(cells, i, &mut strings, needs_calc, wb.date1904, x);
    }
    if row.is_some() {
        x.push_str("</row>");
    }
    open_row(x, u32::MAX)?;
    x.push_str("</sheetData>");
    if !sheet.merges.is_empty() {
        let _ = write!(body.merges, "<mergeCells count=\"{}\">", sheet.merges.len());
        for (n, &(r1, c1, r2, c2)) in sheet.merges.iter().enumerate() {
            if r1 > r2 || c1 > c2 || r2 >= xl_formula::MAX_ROW || c2 >= xl_formula::MAX_COL {
                return Err(Error::Invalid(format!("sheet {:?}: bad merge ({r1}, {c1}, {r2}, {c2})", sheet.name)));
            }
            // Excel treats overlapping merged ranges as file corruption.
            if let Some(o) = sheet.merges[..n].iter().find(|o| r1 <= o.2 && o.0 <= r2 && c1 <= o.3 && o.1 <= c2) {
                return Err(Error::Invalid(format!("sheet {:?}: merged ranges ({r1}, {c1}, {r2}, {c2}) and {o:?} overlap", sheet.name)));
            }
            let _ = write!(body.merges, "<mergeCell ref=\"{}:{}\"/>", a1(r1, c1), a1(r2, c2));
        }
        body.merges.push_str("</mergeCells>");
    }
    Ok(body)
}

/// The components of a style sheet for a list of styles. `base` gives the number of
/// fonts, fills and borders that already exist (new ones are numbered after them)
/// and the known custom number formats.
pub(crate) struct StyleParts {
    pub num_formats: Vec<(u32, String)>,
    pub fonts: Vec<String>,
    pub fills: Vec<String>,
    pub borders: Vec<String>,
    pub xfs: Vec<String>,
}

fn color(c: &str) -> String {
    format!("<color rgb=\"{}\"/>", escape_attr(c))
}

pub(crate) fn font_xml(s: &Style) -> String {
    let mut x = String::from("<font>");
    if s.bold {
        x.push_str("<b/>");
    }
    if s.italic {
        x.push_str("<i/>");
    }
    if s.strike {
        x.push_str("<strike/>");
    }
    match s.underline.as_deref() {
        Some("single") => x.push_str("<u/>"),
        Some(u) => {
            let _ = write!(x, "<u val=\"{}\"/>", escape_attr(u));
        }
        None => {}
    }
    let _ = write!(x, "<sz val=\"{}\"/>", number(s.font_size.unwrap_or(11.0)));
    // No font given: Excel's own default (the theme's body font in the theme's text colour).
    let theme_font = s.font_name.is_none() && s.font_color.is_none();
    match &s.font_color {
        Some(c) => x.push_str(&color(c)),
        None if theme_font => x.push_str("<color theme=\"1\"/>"),
        None => {}
    }
    let _ = write!(x, "<name val=\"{}\"/>", escape_attr(s.font_name.as_deref().unwrap_or("Calibri")));
    if s.font_name.is_none() {
        x.push_str("<family val=\"2\"/>");
    }
    if theme_font {
        x.push_str("<scheme val=\"minor\"/>");
    }
    x.push_str("</font>");
    x
}

pub(crate) fn fill_xml(s: &Style) -> String {
    match &s.fill {
        Some(c) => format!("<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{}\"/><bgColor indexed=\"64\"/></patternFill></fill>", escape_attr(c)),
        None => "<fill><patternFill patternType=\"none\"/></fill>".into(),
    }
}

pub(crate) fn border_xml(s: &Style) -> String {
    let mut x = String::from("<border>");
    for (name, side) in ["left", "right", "top", "bottom"].iter().zip(&s.borders) {
        match side {
            Some(b) => {
                let _ = write!(x, "<{name} style=\"{}\">{}</{name}>", escape_attr(&b.style), b.color.as_deref().map(color).unwrap_or_default());
            }
            None => {
                let _ = write!(x, "<{name}/>");
            }
        }
    }
    x.push_str("<diagonal/></border>");
    x
}

/// The font, fill and border of a style as written (`<font>…</font>`, `<fill>…</fill>`,
/// `<border>…</border>`): its XML where it has some, else what its fields say.
pub fn part_xml(s: &Style) -> (String, String, String) {
    (
        s.font_xml.clone().unwrap_or_else(|| font_xml(s)),
        s.fill_xml.clone().unwrap_or_else(|| fill_xml(s)),
        s.border_xml.clone().unwrap_or_else(|| border_xml(s)),
    )
}

/// `<xf>` elements for `styles`; `named` gives the `cellStyleXfs` index of each named
/// style (`None`: the formats are those of `cellStyleXfs` themselves).
pub(crate) fn style_parts_named(styles: &[Style], mut fonts: Vec<String>, mut fills: Vec<String>, mut borders: Vec<String>, base: (usize, usize, usize), known_formats: &[(u32, String)], named: Option<&[(String, usize)]>) -> StyleParts {
    let mut num_formats: Vec<(u32, String)> = Vec::new();
    let mut next_format = known_formats.iter().map(|f| f.0 + 1).max().unwrap_or(164).max(164);
    let mut xfs = Vec::new();
    let intern = |list: &mut Vec<String>, xml: String, base: usize| -> usize {
        match list.iter().position(|f| *f == xml) {
            Some(i) => base + i,
            None => {
                list.push(xml);
                base + list.len() - 1
            }
        }
    };
    let default = Style::default();
    let (default_font, default_fill, default_border) = part_xml(&default);
    for s in styles {
        let (font_x, fill_x, border_x) = part_xml(s);
        let font = intern(&mut fonts, font_x.clone(), base.0);
        let fill = intern(&mut fills, fill_x.clone(), base.1);
        let border = intern(&mut borders, border_x.clone(), base.2);
        let fmt = match builtin_num_format_id(&s.num_format) {
            Some(id) => id,
            None => match known_formats.iter().chain(&num_formats).find(|f| f.1 == s.num_format) {
                Some(f) => f.0,
                None => {
                    num_formats.push((next_format, s.num_format.clone()));
                    next_format += 1;
                    next_format - 1
                }
            },
        };
        let mut x = format!("<xf numFmtId=\"{fmt}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\"");
        if let Some(named) = named {
            let id = s.named.as_ref().and_then(|n| named.iter().find(|m| m.0 == *n)).map(|m| m.1).unwrap_or(0);
            let _ = write!(x, " xfId=\"{id}\"");
        }
        if fmt != 0 {
            x.push_str(" applyNumberFormat=\"1\"");
        }
        if font_x != default_font {
            x.push_str(" applyFont=\"1\"");
        }
        if fill_x != default_fill {
            x.push_str(" applyFill=\"1\"");
        }
        if border_x != default_border {
            x.push_str(" applyBorder=\"1\"");
        }
        let aligned = s.alignment_xml.is_some() || s.h_align.is_some() || s.v_align.is_some() || s.wrap || s.rotation != 0 || s.indent != 0;
        if aligned {
            x.push_str(" applyAlignment=\"1\"");
        }
        if s.protection_xml.is_some() {
            x.push_str(" applyProtection=\"1\"");
        }
        for (k, v) in &s.xf_attrs {
            let _ = write!(x, " {k}=\"{}\"", escape_attr(v));
        }
        let mut inner = String::new();
        if let Some(a) = &s.alignment_xml {
            inner.push_str(a);
        } else if aligned {
            inner.push_str("<alignment");
            if let Some(h) = &s.h_align {
                let _ = write!(inner, " horizontal=\"{}\"", escape_attr(h));
            }
            if let Some(v) = &s.v_align {
                let _ = write!(inner, " vertical=\"{}\"", escape_attr(v));
            }
            if s.rotation != 0 {
                let _ = write!(inner, " textRotation=\"{}\"", s.rotation);
            }
            if s.wrap {
                inner.push_str(" wrapText=\"1\"");
            }
            if s.indent != 0 {
                let _ = write!(inner, " indent=\"{}\"", s.indent);
            }
            inner.push_str("/>");
        }
        if let Some(p) = &s.protection_xml {
            inner.push_str(p);
        }
        if inner.is_empty() {
            x.push_str("/>");
        } else {
            let _ = write!(x, ">{inner}</xf>");
        }
        xfs.push(x);
    }
    StyleParts { num_formats, fonts, fills, borders, xfs }
}

/// The named styles as written: `Normal` first (added when missing).
pub(crate) fn named_list(named: &[NamedStyle]) -> Vec<NamedStyle> {
    // The first is the base of all formats (Normal, whatever the file calls it).
    if named.is_empty() {
        return vec![NamedStyle { name: "Normal".into(), builtin_id: Some(0), ..NamedStyle::default() }];
    }
    named.to_vec()
}

/// `<cellStyle>` elements.
pub(crate) fn cell_styles_xml(list: &[NamedStyle], first_xf: usize) -> String {
    let mut x = String::new();
    for (i, n) in list.iter().enumerate() {
        let _ = write!(x, "<cellStyle name=\"{}\" xfId=\"{}\"", crate::xml::escape_attr_plain(&n.name), first_xf + i);
        if let Some(b) = n.builtin_id {
            let _ = write!(x, " builtinId=\"{b}\"");
        }
        if let Some(l) = n.i_level {
            let _ = write!(x, " iLevel=\"{l}\"");
        }
        if n.hidden {
            x.push_str(" hidden=\"1\"");
        }
        if n.custom_builtin {
            x.push_str(" customBuiltin=\"1\"");
        }
        x.push_str("/>");
    }
    x
}

pub(crate) fn num_formats_xml(formats: &[(u32, String)]) -> String {
    formats.iter().map(|(id, code)| format!("<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>", escape_attr(code))).collect()
}

/// What styles.xml holds besides the formats: table styles and the palette.
pub(crate) struct StyleExtras<'a> {
    pub palette: &'a [String],
    pub table_styles: &'a [TableStyle],
    pub default_table_style: Option<&'a str>,
    pub default_pivot_style: Option<&'a str>,
}

impl<'a> StyleExtras<'a> {
    pub fn of(wb: &'a Workbook) -> Self {
        StyleExtras { palette: &wb.palette, table_styles: &wb.table_styles, default_table_style: wb.default_table_style.as_deref(), default_pivot_style: wb.default_pivot_style.as_deref() }
    }

    fn is_empty(&self) -> bool {
        self.palette.is_empty() && self.table_styles.is_empty() && self.default_table_style.is_none() && self.default_pivot_style.is_none()
    }
}

fn styles_xml(styles: &[Style], dxfs: &[String], named: &[NamedStyle], extras: &StyleExtras) -> String {
    let default = Style::default();
    let gray = "<fill><patternFill patternType=\"gray125\"/></fill>".to_string();
    let only_default = [default.clone()];
    let styles = if styles.is_empty() { &only_default[..] } else { styles };
    let list = named_list(named);
    let names: Vec<(String, usize)> = list.iter().enumerate().map(|(i, n)| (n.name.clone(), i)).collect();
    let formats: Vec<Style> = list.iter().map(|n| n.style.clone()).collect();
    // The named styles' formats first, so that they share fonts, fills and borders.
    let (font0, fill0, border0) = part_xml(&default);
    let base = style_parts_named(&formats, vec![font0], vec![fill0, gray], vec![border0], (0, 0, 0), &[], None);
    let p = style_parts_named(styles, base.fonts, base.fills, base.borders, (0, 0, 0), &base.num_formats, Some(&names));
    let mut all_formats = base.num_formats.clone();
    all_formats.extend(p.num_formats.iter().cloned());
    let mut x = String::new();
    let _ = write!(x, "{HEADER}<styleSheet xmlns=\"{NS_MAIN}\">");
    if !all_formats.is_empty() {
        let _ = write!(x, "<numFmts count=\"{}\">{}</numFmts>", all_formats.len(), num_formats_xml(&all_formats));
    }
    let _ = write!(x, "<fonts count=\"{}\">{}</fonts>", p.fonts.len(), p.fonts.concat());
    let _ = write!(x, "<fills count=\"{}\">{}</fills>", p.fills.len(), p.fills.concat());
    let _ = write!(x, "<borders count=\"{}\">{}</borders>", p.borders.len(), p.borders.concat());
    let _ = write!(x, "<cellStyleXfs count=\"{}\">{}</cellStyleXfs>", base.xfs.len(), base.xfs.concat());
    let _ = write!(x, "<cellXfs count=\"{}\">{}</cellXfs>", p.xfs.len(), p.xfs.concat());
    let _ = write!(x, "<cellStyles count=\"{}\">{}</cellStyles>", list.len(), cell_styles_xml(&list, 0));
    // The table styles' formats follow those of the conditional formats.
    let mut all_dxfs = dxfs.to_vec();
    let mut styles_x = String::new();
    for t in extras.table_styles {
        let mut attrs = format!(" name=\"{}\"", escape_attr(&t.name));
        if !t.pivot {
            attrs.push_str(" pivot=\"0\"");
        }
        if !t.table {
            attrs.push_str(" table=\"0\"");
        }
        let _ = write!(styles_x, "<tableStyle{attrs} count=\"{}\">", t.elements.len());
        for (kind, size, dxf) in &t.elements {
            let size = size.map(|n| format!(" size=\"{n}\"")).unwrap_or_default();
            let _ = write!(styles_x, "<tableStyleElement type=\"{}\"{size} dxfId=\"{}\"/>", escape_attr(kind), all_dxfs.len());
            all_dxfs.push(dxf.clone());
        }
        styles_x.push_str("</tableStyle>");
    }
    if !all_dxfs.is_empty() {
        let _ = write!(x, "<dxfs count=\"{}\">{}</dxfs>", all_dxfs.len(), dxfs_xml(&all_dxfs));
    }
    if !extras.is_empty() {
        let mut attrs = String::new();
        if let Some(d) = extras.default_table_style {
            let _ = write!(attrs, " defaultTableStyle=\"{}\"", escape_attr(d));
        }
        if let Some(d) = extras.default_pivot_style {
            let _ = write!(attrs, " defaultPivotStyle=\"{}\"", escape_attr(d));
        }
        if !(extras.table_styles.is_empty() && attrs.is_empty()) {
            let _ = write!(x, "<tableStyles count=\"{}\"{attrs}>{styles_x}</tableStyles>", extras.table_styles.len());
        }
        if !extras.palette.is_empty() {
            let colors: String = extras.palette.iter().map(|c| format!("<rgbColor rgb=\"{}\"/>", escape_attr(c))).collect();
            let _ = write!(x, "<colors><indexedColors>{colors}</indexedColors></colors>");
        }
    }
    x.push_str("</styleSheet>");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_back_bit_for_bit() {
        for n in [0.0, -0.0, 1.0, 0.1, 1.0 / 3.0, 1e-7, 1.5e300, 5e-324, f64::MAX, f64::MIN_POSITIVE, 123456789012345680.0, -2.5e-5, 9007199254740993.0] {
            let back: f64 = number(n).parse().unwrap();
            assert_eq!(back.to_bits(), n.to_bits(), "{}", number(n));
        }
    }
}
