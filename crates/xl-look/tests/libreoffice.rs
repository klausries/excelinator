//! Golden test: the look of sampled cells of the corpus against LibreOffice.
//!
//! `tools/lo_look.py` stores, per workbook, the formatting LibreOffice gives each sampled
//! cell (its cell properties, without conditional formats) and the background and font
//! colours of its HTML export (with conditional formats). LibreOffice applies no table
//! styles, so tables are left out here (their styles are tested against ECMA-376 in
//! `look.rs`). What LibreOffice cannot show the same way (pattern fills it mixes into one
//! colour, alignments it has no equivalent for) is not compared and counted by name.

use serde_json::Value as J;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xl_look::{Look, Looker, Val};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "json") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Tally {
    compared: BTreeMap<&'static str, u64>,
    skipped: BTreeMap<String, u64>,
    wrong: BTreeMap<&'static str, u64>,
    examples: BTreeMap<&'static str, Vec<String>>,
    files: BTreeMap<&'static str, std::collections::BTreeSet<String>>,
    file: String,
}

impl Tally {
    fn check(&mut self, what: &'static str, ok: bool, example: impl FnOnce() -> String) {
        *self.compared.entry(what).or_default() += 1;
        if !ok {
            *self.wrong.entry(what).or_default() += 1;
            // A few examples of each kind, from different workbooks.
            let files = self.files.entry(what).or_default();
            let list = self.examples.entry(what).or_default();
            if list.len() < 8 && files.insert(self.file.clone()) {
                list.push(example());
            }
        }
    }

    fn skip(&mut self, why: &str) {
        *self.skipped.entry(why.to_string()).or_default() += 1;
    }
}

fn text<'a>(l: &'a Look, k: &str) -> &'a str {
    match l.value(k) {
        Some(Val::Text(t)) => t,
        _ => "",
    }
}

fn flag(l: &Look, k: &str) -> bool {
    matches!(l.value(k), Some(Val::Flag(true)))
}

fn num(l: &Look, k: &str) -> f64 {
    match l.value(k) {
        Some(Val::Number(n)) => *n,
        _ => f64::NAN,
    }
}

/// Colours as equal as LibreOffice can tell: a tinted colour is rounded as Excel does
/// (checked against Excel), LibreOffice's rounding can differ by two steps per channel.
fn same_color(ours: Option<&str>, theirs: Option<&str>, spec: Option<&str>) -> bool {
    match (ours, theirs) {
        (Some(a), Some(b)) if spec.is_some_and(|s| s.contains("tint")) && a.len() == 6 && b.len() == 6 => {
            (0..3).all(|i| (i64::from_str_radix(&a[2 * i..2 * i + 2], 16).unwrap_or(0) - i64::from_str_radix(&b[2 * i..2 * i + 2], 16).unwrap_or(0)).abs() <= 2)
        }
        _ => ours == theirs,
    }
}

fn rgb(l: &Look, k: &str) -> Option<String> {
    match l.value(k) {
        Some(Val::Rgb(c)) => Some(c.clone()),
        _ => None,
    }
}

/// What neighbours of a cell draw on its edges, and what kind of value it holds: the
/// context some of LibreOffice's own choices depend on.
struct Context {
    /// The left border of the cell to the right, the top border of the cell below.
    right_neighbour_left: bool,
    below_top: bool,
    value: xl_cells::Value,
    iso_date: bool,
    /// For the top left cell of a merged area: the looks of its right and bottom edge
    /// cells, whose borders LibreOffice gives the merged cell.
    merged_edges: Option<(Look, Look)>,
    /// The cell's font names no typeface.
    nameless_font: bool,
    /// Indexes of the file's own palette.
    own_palette: usize,
    /// A cell of format 0 in a column with a format.
    plain_in_formatted_column: bool,
    /// The cell's own format has no fill, the named style it is based on has one.
    no_fill_over_style_fill: bool,
}

fn compare_props(t: &mut Tally, at: &str, l: &Look, p: &J, cx: &Context) {
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    // LibreOffice's own default font (Liberation Sans 10) where the file's style sheet
    // gives none LibreOffice takes: no font, or a part it does not find.
    if cx.plain_in_formatted_column {
        t.skip("cell of format 0 in a formatted column (LibreOffice takes the column's format)");
        return;
    }
    if cx.nameless_font && s("font_name") != text(l, "font_name") {
        t.skip("font without a typeface (LibreOffice's own choice)");
        return;
    }
    // Text format in the file, General in LibreOffice: it dropped the cell's format (an
    // inline string that also holds a <v>).
    if text(l, "num_format") == "@" && s("num_format") == "General" {
        t.skip("cell whose format LibreOffice drops (inline string with a value)");
        return;
    }
    if s("font_name") == "Liberation Sans" && text(l, "font_name") != "Liberation Sans" {
        t.skip("LibreOffice's own default font (no font it takes from the file)");
        return;
    }
    t.check("font_name", text(l, "font_name") == s("font_name"), || format!("{at} font_name {} vs {}", text(l, "font_name"), s("font_name")));
    let size = p["font_size"].as_f64().unwrap_or(f64::NAN);
    t.check("font_size", (num(l, "font_size") - size).abs() < 0.01, || format!("{at} font_size {} vs {size}", num(l, "font_size")));
    t.check("bold", flag(l, "bold") == p["bold"].as_bool().unwrap_or(false), || format!("{at} bold {} vs {}", flag(l, "bold"), p["bold"]));
    t.check("italic", flag(l, "italic") == p["italic"].as_bool().unwrap_or(false), || format!("{at} italic"));
    let under = p["underline"].as_i64().unwrap_or(0) != 0;
    t.check("underline", (text(l, "underline") != "none") == under, || format!("{at} underline {} vs {}", text(l, "underline"), p["underline"]));
    let strike = p["strike"].as_i64().unwrap_or(0) != 0;
    t.check("strike", flag(l, "strike") == strike, || format!("{at} strike"));
    // Font colour: LibreOffice reports automatic as no colour.
    let fc = l.get("font_color").unwrap();
    let ours = rgb(l, "font_color");
    match p["font_color"].as_str() {
        None => t.check("font_color", fc.spec.as_deref() == Some("auto") || fc.spec.as_deref() == Some("indexed 64") || ours.as_deref() == Some("000000"), || format!("{at} font_color {ours:?} ({:?}) vs automatic", fc.spec)),
        Some(c) => t.check("font_color", same_color(ours.as_deref(), Some(c), fc.spec.as_deref()), || format!("{at} font_color {ours:?} ({:?}) vs {c}", fc.spec)),
    }
    // Fill: none and solid; patterns LibreOffice mixes into one colour.
    match text(l, "fill_pattern") {
        // Checked with Excel (ConditionalFormattingSamples.xlsx, Regional sales!B2): the
        // cell's own "no fill" holds; LibreOffice shows the named style's fill.
        "none" if cx.no_fill_over_style_fill && !p["back"].is_null() => t.skip("no fill of the cell's own format over a style with a fill (LibreOffice shows the style's)"),
        "none" => t.check("fill", p["back"].is_null(), || format!("{at} fill none vs {}", p["back"])),
        "solid" => {
            let ours = rgb(l, "fill_color");
            t.check("fill", same_color(ours.as_deref(), p["back"].as_str(), l.get("fill_color").unwrap().spec.as_deref()), || format!("{at} fill {ours:?} ({:?}) vs {}", l.get("fill_color").unwrap().spec, p["back"]))
        }
        other => t.skip(&format!("fill pattern {other} (LibreOffice mixes it into one colour)")),
    }
    for side in ["left", "right", "top", "bottom"] {
        let key = format!("border_{side}");
        // A merged cell has the borders of the merged area's edges.
        let edge = match (&cx.merged_edges, side) {
            (Some((right, _)), "right") => right,
            (Some((_, bottom)), "bottom") => bottom,
            _ => l,
        };
        let style = text(edge, &key).to_string();
        let lo = &p[key.as_str()];
        let what: &'static str = match side {
            "left" => "border_left",
            "right" => "border_right",
            "top" => "border_top",
            _ => "border_bottom",
        };
        // LibreOffice gives a cell the border its neighbour draws on the shared edge.
        if style == "none" && !lo.is_null() && ((side == "right" && cx.right_neighbour_left) || (side == "bottom" && cx.below_top)) {
            t.skip("border drawn by the neighbouring cell on the shared edge");
            continue;
        }
        t.check(what, (style == "none") == lo.is_null(), || format!("{at} {key} {style} vs {lo}"));
        if style != "none" && !lo.is_null() {
            let color_key = format!("{key}_color");
            let a = edge.get(&color_key).unwrap();
            let palette = a.spec.as_deref().and_then(|x| x.strip_prefix("indexed ")).and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n < cx.own_palette);
            if palette && lo[2].is_null() {
                t.skip("colour of the file's palette (LibreOffice reads its alpha FF as transparent)");
            } else if a.spec.as_deref() != Some("auto") {
                let ours = rgb(edge, &color_key);
                t.check("border_color", same_color(ours.as_deref(), lo[2].as_str(), a.spec.as_deref()), || format!("{at} {color_key} {ours:?} ({:?}) vs {}", a.spec, lo[2]));
            }
        }
    }
    let hori = match text(l, "horizontal") {
        "general" => Some("STANDARD"),
        "left" => Some("LEFT"),
        "center" | "centerContinuous" => Some("CENTER"),
        "right" => Some("RIGHT"),
        "justify" | "distributed" => Some("BLOCK"),
        "fill" => Some("REPEAT"),
        _ => None,
    };
    let rotation = num(l, "rotation");
    match hori {
        Some("STANDARD") if rotation != 0.0 => t.skip("rotated text in general alignment (LibreOffice fixes an alignment)"),
        Some(h) => t.check("horizontal", p["hori"].as_str() == Some(h), || format!("{at} horizontal {} vs {}", text(l, "horizontal"), p["hori"])),
        None => t.skip("horizontal alignment without an equivalent"),
    }
    let vert = match text(l, "vertical") {
        "top" => Some(1),
        "center" => Some(2),
        "bottom" => Some(3),
        "justify" | "distributed" => Some(4),
        _ => None,
    };
    match vert {
        Some(v) => t.check("vertical", p["vert"].as_i64() == Some(v), || format!("{at} vertical {} vs {}", text(l, "vertical"), p["vert"])),
        None => t.skip("vertical alignment without an equivalent"),
    }
    let justified = ["horizontal", "vertical"].iter().any(|k| matches!(text(l, k), "justify" | "distributed"));
    if justified && !flag(l, "wrap") && p["wrap"].as_bool() == Some(true) {
        t.skip("justified or distributed text (LibreOffice turns wrapping on)");
    } else {
        t.check("wrap", flag(l, "wrap") == p["wrap"].as_bool().unwrap_or(false), || format!("{at} wrap {} vs {}", flag(l, "wrap"), p["wrap"]));
    }
    t.check("shrink", flag(l, "shrink") == p["shrink"].as_bool().unwrap_or(false), || format!("{at} shrink"));
    let r = num(l, "rotation") as i64;
    let want = match r {
        0 => Some(0),
        1..=90 => Some(r * 100),
        91..=180 => Some((360 - (r - 90)) * 100),
        _ => None,
    };
    match want {
        Some(w) => t.check("rotation", p["rotate"].as_i64() == Some(w), || format!("{at} rotation {r} vs {}", p["rotate"])),
        None => t.skip("rotation 255 (stacked text)"),
    }
    t.check("indent", (num(l, "indent") > 0.0) == (p["indent"].as_i64().unwrap_or(0) > 0), || format!("{at} indent {} vs {}", num(l, "indent"), p["indent"]));
    t.check("locked", flag(l, "locked") == p["locked"].as_bool().unwrap_or(true), || format!("{at} locked"));
    t.check("hidden", flag(l, "hidden") == p["hidden"].as_bool().unwrap_or(false), || format!("{at} hidden"));
    if text(l, "num_format") == "General" && matches!(cx.value, xl_cells::Value::Bool(_)) && s("num_format") == "BOOLEAN" {
        t.skip("boolean cell (LibreOffice gives it its BOOLEAN format)");
    } else if text(l, "num_format") == "General" && cx.iso_date {
        t.skip("ISO date cell (LibreOffice gives it a date format)");
    } else if text(l, "num_format") == "General" && !s("num_format").is_empty() && !s("num_format").chars().any(|c| c.is_ascii_alphabetic()) {
        t.skip("General in the file's language (LibreOffice translates it)");
    } else if text(l, "num_format") == "General" {
        t.check("num_format General", p["num_format"].as_str() == Some("General"), || format!("{at} num_format General vs {}", p["num_format"]));
    } else {
        t.skip("number format other than General (LibreOffice rewrites codes)");
    }
}

#[test]
fn looks_match_libreoffice() {
    let dir = root().join("lo-look");
    let mut list = Vec::new();
    files(&dir, &mut list);
    if list.is_empty() {
        eprintln!("no LibreOffice looks (run tools/lo_look.py corpus corpus/lo-look 4)");
        return;
    }
    let mut t = Tally::default();
    let mut workbooks = 0;
    for json in &list {
        let golden: J = serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap();
        let rel = golden["file"].as_str().unwrap();
        let Ok(bytes) = std::fs::read(root().join(rel)) else {
            t.skip("workbook not readable");
            continue;
        };
        let pivots = xl_look::pivot::pivot_areas(&bytes);
        let Ok(wb) = xl_cells::read_bytes(bytes) else {
            t.skip("workbook not readable");
            continue;
        };
        workbooks += 1;
        t.file = rel.to_string();
        for gs in golden["sheets"].as_array().unwrap() {
            let name = gs["name"].as_str().unwrap();
            let Some(s) = wb.sheets.iter().position(|x| x.name == name) else {
                t.skip("sheet not found by name");
                continue;
            };
            let cells: Vec<(u32, u32, u32)> = gs["cells"].as_array().unwrap().iter().map(|c| (s as u32, c[0].as_u64().unwrap() as u32, c[1].as_u64().unwrap() as u32)).collect();
            let mut plain = Looker::new(&wb);
            plain.pivots = pivots.clone();
            plain.with_tables = false;
            plain.with_conditional = false;
            let base = plain.looks(&cells);
            let index: std::collections::HashMap<(u32, u32), usize> = (0..wb.cells.len()).filter(|&i| wb.cells.sheet[i] == s as u32).map(|i| ((wb.cells.row[i], wb.cells.col[i]), i)).collect();
            let near: Vec<(u32, u32, u32)> = cells.iter().flat_map(|&(s, r, c)| [(s, r, c + 1), (s, r + 1, c)]).collect();
            let near = plain.looks(&near);
            let sheet = &wb.sheets[s];
            for (k, c) in gs["cells"].as_array().unwrap().iter().enumerate() {
                let at = format!("{rel} {name}!{}{}", xl_formula::col_name(cells[k].2 + 1), cells[k].1 + 1);
                let i = index.get(&(cells[k].1, cells[k].2)).copied();
                let cx = Context {
                    right_neighbour_left: text(&near[2 * k], "border_left") != "none",
                    below_top: text(&near[2 * k + 1], "border_top") != "none",
                    value: i.map(|i| wb.cells.value(i)).unwrap_or(xl_cells::Value::Blank),
                    iso_date: i.is_some_and(|i| wb.cells.iso[i] != xl_cells::IsoDate::No),
                    merged_edges: sheet.merges.iter().find(|m| m.0 == cells[k].1 && m.1 == cells[k].2).map(|m| (plain.look(s as u32, m.0, m.3), plain.look(s as u32, m.2, m.1))),
                    nameless_font: i.and_then(|i| wb.styles.get(wb.cells.style[i] as usize)).and_then(|st| st.font_xml.as_deref()).is_some_and(|x| !x.contains("<name")),
                    own_palette: wb.palette.len(),
                    no_fill_over_style_fill: i.and_then(|i| wb.styles.get(wb.cells.style[i] as usize)).is_some_and(|st| {
                        st.fill.is_none() && st.named.as_deref().and_then(|n| wb.named_styles.iter().find(|x| x.name == n)).is_some_and(|ns| ns.style.fill.is_some())
                    }),
                    plain_in_formatted_column: i.is_some_and(|i| wb.cells.style[i] == 0) && sheet.columns.iter().any(|c| c.first <= cells[k].2 && cells[k].2 <= c.last && c.style.is_some_and(|x| x != 0)),
                };
                let (pr, pc) = (cells[k].1, cells[k].2);
                if pivots.get(s).is_some_and(|ps| ps.iter().any(|p| pr <= p.area.2 + 3 && pc <= p.area.3 + 2 && pr + 1 >= p.area.0 && pc + 1 >= p.area.1)) && base[k].undecided.iter().all(|u| !u.1.starts_with("pivot table")) {
                    t.skip("next to a pivot table (LibreOffice's own layout of it is larger)");
                    continue;
                }
                if base[k].undecided.iter().any(|u| u.1.starts_with("pivot table")) {
                    t.skip("cell of a pivot table (its style not applied; LibreOffice lays the table out itself)");
                    continue;
                }
                compare_props(&mut t, &at, &base[k], &c[2], &cx);
            }
            // Data of older versions of the oracle placed the HTML export from A1.
            if gs.get("html_origin").is_none() {
                t.skip("HTML export of an older oracle (not placed by the used area)");
                continue;
            }
            let Some(html) = gs["html"].as_array() else {
                t.skip("HTML export not cell by cell (hidden rows or columns, merges)");
                continue;
            };
            if wb.sheets[s].cond_formats.is_empty() {
                continue;
            }
            let mut full = Looker::new(&wb);
            full.pivots = pivots.clone();
            full.with_tables = false;
            let looks = full.looks(&cells);
            for (k, h) in html.iter().enumerate() {
                let h = &h[2];
                let at = format!("{rel} {name}!{}{} (conditional)", xl_formula::col_name(cells[k].2 + 1), cells[k].1 + 1);
                let l = &looks[k];
                if l.undecided.iter().any(|u| u.1.starts_with("pivot table")) {
                    t.skip("cell of a pivot table (its style not applied; LibreOffice lays the table out itself)");
                    continue;
                }
                if let Some((_, why)) = l.undecided.first() {
                    t.skip(&format!("conditional format not decided: {why}"));
                    continue;
                }
                if h.is_null() {
                    t.skip("cell not in the HTML table");
                    continue;
                }
                // Rules of several ranges over one cell: Excel orders them by priority
                // across the sheet, LibreOffice range by range.
                let blocks = wb.sheets[s].cond_formats.iter().filter(|cf| cf.sqref.split_whitespace().filter_map(xl_look::table::parse_range).any(|(r0, c0, r1, c1)| (r0..=r1).contains(&cells[k].1) && (c0..=c1).contains(&cells[k].2))).count();
                if blocks > 1 {
                    t.skip("cell under conditional formats of several ranges (LibreOffice orders them by range, not by priority)");
                    continue;
                }
                let cf_fill = matches!(l.get("fill_color").unwrap().source, xl_look::Source::Conditional { .. });
                let cf_font = matches!(l.get("font_color").unwrap().source, xl_look::Source::Conditional { .. });
                match text(l, "fill_pattern") {
                    "none" | "solid" => {
                        let ours = rgb(l, "fill_color");
                        let what = if cf_fill { "conditional fill" } else { "fill (HTML)" };
                        t.check(what, same_color(ours.as_deref(), h[0].as_str(), l.get("fill_color").unwrap().spec.as_deref()), || format!("{at} fill {ours:?} vs {}", h[0]));
                    }
                    _ => t.skip("fill pattern (HTML)"),
                }
                if let Some(lo) = h[1].as_str() {
                    let ours = rgb(l, "font_color");
                    if cf_font {
                        t.check("conditional font colour", same_color(ours.as_deref(), Some(lo), l.get("font_color").unwrap().spec.as_deref()), || format!("{at} font colour {ours:?} vs {lo}"));
                    }
                }
            }
        }
    }
    eprintln!("{workbooks} workbooks");
    eprintln!("compared: {:?}", t.compared);
    eprintln!("wrong: {:?}", t.wrong);
    eprintln!("not compared: {:#?}", t.skipped);
    for (what, list) in &t.examples {
        eprintln!("{what}: in {} workbooks", t.files[what].len());
        for e in list {
            eprintln!("  {e}");
        }
    }
    assert!(t.wrong.is_empty(), "looks differ from LibreOffice's");
}
