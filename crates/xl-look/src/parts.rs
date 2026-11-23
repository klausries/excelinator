//! Formatting attributes one layer sets: from a cell format (where every attribute has a
//! value, the defaults included) or a differential format (only what it names).

use crate::color::Spec;
use crate::tree::{self, Node};
use std::collections::BTreeMap;

/// A value of a formatting attribute before colours are resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Raw {
    Text(String),
    Number(f64),
    Flag(bool),
    Color(Spec),
    /// Explicitly nothing (no fill colour, no border colour).
    Nothing,
}

/// The attributes a layer sets, by name.
pub type Partial = BTreeMap<&'static str, Raw>;

/// Border sides in the order of the keys below; `vertical` and `horizontal` are the inner
/// borders of a range (differential formats of table styles).
pub const SIDES: [&str; 7] = ["left", "right", "top", "bottom", "diagonal", "vertical", "horizontal"];
pub const SIDE_KEYS: [(&str, &str); 7] = [
    ("border_left", "border_left_color"),
    ("border_right", "border_right_color"),
    ("border_top", "border_top_color"),
    ("border_bottom", "border_bottom_color"),
    ("border_diagonal", "border_diagonal_color"),
    ("border_vertical", "border_vertical_color"),
    ("border_horizontal", "border_horizontal_color"),
];

/// All attributes of a cell's look, in a fixed order.
pub const KEYS: [&str; 32] = [
    "font_name",
    "font_size",
    "bold",
    "italic",
    "underline",
    "strike",
    "font_color",
    "font_script",
    "fill_pattern",
    "fill_color",
    "fill_background",
    "border_left",
    "border_left_color",
    "border_right",
    "border_right_color",
    "border_top",
    "border_top_color",
    "border_bottom",
    "border_bottom_color",
    "border_diagonal",
    "border_diagonal_color",
    "diagonal_up",
    "diagonal_down",
    "horizontal",
    "vertical",
    "wrap",
    "rotation",
    "indent",
    "shrink",
    "num_format",
    "locked",
    "hidden",
];

fn put(p: &mut Partial, k: &'static str, v: Raw) {
    p.insert(k, v);
}

/// Font attributes. `complete`: a cell format's font, where a missing element is the
/// default (not bold, no underline, …).
pub fn font(n: &Node, complete: bool, p: &mut Partial) {
    let flag = |name: &str| n.child(name).map(|c| c.flag());
    for (key, el) in [("bold", "b"), ("italic", "i"), ("strike", "strike")] {
        match flag(el) {
            Some(v) => put(p, key, Raw::Flag(v)),
            None if complete => put(p, key, Raw::Flag(false)),
            None => {}
        }
    }
    match n.child("u") {
        Some(u) => put(p, "underline", Raw::Text(u.get("val").unwrap_or("single").to_string())),
        None if complete => put(p, "underline", Raw::Text("none".into())),
        None => {}
    }
    match n.child("vertAlign").and_then(|v| v.get("val")) {
        Some(v) => put(p, "font_script", Raw::Text(v.to_string())),
        None if complete => put(p, "font_script", Raw::Text("baseline".into())),
        None => {}
    }
    if let Some(name) = n.child("name").or_else(|| n.child("rFont")).and_then(|c| c.get("val")) {
        put(p, "font_name", Raw::Text(xl_cells::unescape_xstring(name)));
    }
    if let Some(sz) = n.child("sz").and_then(|c| c.get("val")).and_then(|v| v.parse().ok()) {
        put(p, "font_size", Raw::Number(sz));
    }
    match n.child("color").and_then(Spec::of) {
        Some(c) => put(p, "font_color", Raw::Color(c)),
        None if complete => put(p, "font_color", Raw::Color(Spec::Auto)),
        None => {}
    }
}

/// Fill attributes. In a differential format a solid fill's colour is its `bgColor`
/// (Excel writes `fgColor` too, or only it, in table styles: then that one).
pub fn fill(n: &Node, differential: bool, p: &mut Partial) {
    if let Some(g) = n.child("gradientFill") {
        put(p, "fill_pattern", Raw::Text("gradient".into()));
        let stops: Vec<Spec> = g.all("stop").filter_map(|s| s.child("color").and_then(Spec::of)).collect();
        put(p, "fill_color", stops.first().cloned().map(Raw::Color).unwrap_or(Raw::Nothing));
        put(p, "fill_background", stops.last().cloned().map(Raw::Color).unwrap_or(Raw::Nothing));
        return;
    }
    let Some(pf) = n.child("patternFill") else {
        if !differential {
            put(p, "fill_pattern", Raw::Text("none".into()));
            put(p, "fill_color", Raw::Nothing);
            put(p, "fill_background", Raw::Nothing);
        }
        return;
    };
    let fg = pf.child("fgColor").and_then(Spec::of);
    let bg = pf.child("bgColor").and_then(Spec::of);
    let pattern = match pf.get("patternType") {
        Some(t) => t.to_string(),
        // A differential fill without a pattern is solid when it gives a colour.
        None if differential && (fg.is_some() || bg.is_some()) => "solid".into(),
        None if differential => return,
        None => "none".into(),
    };
    if pattern == "none" {
        put(p, "fill_pattern", Raw::Text(pattern));
        put(p, "fill_color", Raw::Nothing);
        put(p, "fill_background", Raw::Nothing);
        return;
    }
    let (color, background) = if differential && pattern == "solid" {
        (bg.clone().or(fg.clone()), None)
    } else if pattern == "solid" {
        (fg.clone().or(Some(Spec::Auto)), None)
    } else {
        // A pattern: its colour on the background.
        (fg.clone().or(if differential { None } else { Some(Spec::Auto) }), bg.clone().or(if differential { None } else { Some(Spec::Indexed(65, 0.0)) }))
    };
    put(p, "fill_pattern", Raw::Text(pattern.clone()));
    match color {
        Some(c) => put(p, "fill_color", Raw::Color(c)),
        None if !differential => put(p, "fill_color", Raw::Nothing),
        None => {}
    }
    match background {
        Some(c) => put(p, "fill_background", Raw::Color(c)),
        None if !differential || pattern == "solid" => put(p, "fill_background", Raw::Nothing),
        None => {}
    }
}

/// Border attributes. The sides a cell format leaves out have no border; `start` and
/// `end` are `left` and `right`.
pub fn border(n: &Node, complete: bool, p: &mut Partial) {
    for (i, side) in SIDES.iter().enumerate() {
        let el = n.child(side).or_else(|| match *side {
            "left" => n.child("start"),
            "right" => n.child("end"),
            _ => None,
        });
        let (sk, ck) = SIDE_KEYS[i];
        match el {
            Some(e) => {
                let style = e.get("style").unwrap_or("none").to_string();
                let none = style == "none";
                put(p, sk, Raw::Text(style));
                match e.child("color").and_then(Spec::of) {
                    Some(c) if !none => put(p, ck, Raw::Color(c)),
                    _ if none => put(p, ck, Raw::Nothing),
                    _ => put(p, ck, Raw::Color(Spec::Auto)),
                }
            }
            None if complete && i < 5 => {
                put(p, sk, Raw::Text("none".into()));
                put(p, ck, Raw::Nothing);
            }
            None => {}
        }
    }
    for (key, attr) in [("diagonal_up", "diagonalUp"), ("diagonal_down", "diagonalDown")] {
        match n.bool_attr(attr) {
            Some(v) => put(p, key, Raw::Flag(v)),
            None if complete => put(p, key, Raw::Flag(false)),
            None => {}
        }
    }
}

/// Alignment attributes.
pub fn alignment(n: Option<&Node>, complete: bool, p: &mut Partial) {
    let get = |k: &str| n.and_then(|n| n.get(k));
    let text = |p: &mut Partial, key: &'static str, attr: &str, default: &str| match get(attr) {
        Some(v) => put(p, key, Raw::Text(v.to_string())),
        None if complete => put(p, key, Raw::Text(default.to_string())),
        None => {}
    };
    text(p, "horizontal", "horizontal", "general");
    text(p, "vertical", "vertical", "bottom");
    let num = |p: &mut Partial, key: &'static str, attr: &str| match get(attr).and_then(|v| v.parse::<f64>().ok()) {
        Some(v) => put(p, key, Raw::Number(v)),
        None if complete => put(p, key, Raw::Number(0.0)),
        None => {}
    };
    num(p, "rotation", "textRotation");
    num(p, "indent", "indent");
    let flag = |p: &mut Partial, key: &'static str, attr: &str| match n.and_then(|n| n.bool_attr(attr)) {
        Some(v) => put(p, key, Raw::Flag(v)),
        None if complete => put(p, key, Raw::Flag(false)),
        None => {}
    };
    flag(p, "wrap", "wrapText");
    flag(p, "shrink", "shrinkToFit");
}

/// Protection attributes (a cell is locked and not hidden by default).
pub fn protection(n: Option<&Node>, complete: bool, p: &mut Partial) {
    for (key, attr, default) in [("locked", "locked", true), ("hidden", "hidden", false)] {
        match n.and_then(|n| n.bool_attr(attr)) {
            Some(v) => put(p, key, Raw::Flag(v)),
            None if complete => put(p, key, Raw::Flag(default)),
            None => {}
        }
    }
}

/// Every attribute of a cell format.
pub fn of_style(s: &xl_cells::Style) -> Partial {
    let mut p = Partial::new();
    let (font_xml, fill_xml, border_xml) = xl_cells::part_xml(s);
    if let Ok(t) = tree::parse(&font_xml) {
        if let Some(n) = t.child("font") {
            font(n, true, &mut p);
        }
    }
    if let Ok(t) = tree::parse(&fill_xml) {
        match t.child("fill") {
            Some(n) => fill(n, false, &mut p),
            None => fill(&Node::default(), false, &mut p),
        }
    }
    if let Ok(t) = tree::parse(&border_xml) {
        if let Some(n) = t.child("border") {
            border(n, true, &mut p);
        }
    }
    // The defaults of a font that names neither.
    p.entry("font_name").or_insert(Raw::Text("Calibri".into()));
    p.entry("font_size").or_insert(Raw::Number(11.0));
    match &s.alignment_xml {
        Some(a) => {
            let t = tree::parse(a).ok();
            alignment(t.as_ref().and_then(|t| t.child("alignment")), true, &mut p);
        }
        None => {
            let mut a = Node { name: "alignment".into(), ..Node::default() };
            if let Some(h) = &s.h_align {
                a.attrs.push(("horizontal".into(), h.clone()));
            }
            if let Some(v) = &s.v_align {
                a.attrs.push(("vertical".into(), v.clone()));
            }
            if s.wrap {
                a.attrs.push(("wrapText".into(), "1".into()));
            }
            a.attrs.push(("textRotation".into(), s.rotation.to_string()));
            a.attrs.push(("indent".into(), s.indent.to_string()));
            alignment(Some(&a), true, &mut p);
        }
    }
    let prot = s.protection_xml.as_deref().and_then(|x| tree::parse(x).ok());
    protection(prot.as_ref().and_then(|t| t.child("protection")), true, &mut p);
    p.insert("num_format", Raw::Text(s.num_format.clone()));
    p
}

/// The attributes a differential format (the content of a `<dxf>`) sets.
pub fn of_dxf(xml: &str) -> Partial {
    let mut p = Partial::new();
    let Ok(t) = tree::parse(xml) else { return p };
    let t = t.child("dxf").unwrap_or(&t);
    if let Some(n) = t.child("font") {
        font(n, false, &mut p);
    }
    if let Some(n) = t.child("fill") {
        fill(n, true, &mut p);
    }
    if let Some(n) = t.child("border") {
        border(n, false, &mut p);
    }
    if let Some(n) = t.child("alignment") {
        alignment(Some(n), false, &mut p);
    }
    if let Some(n) = t.child("protection") {
        protection(Some(n), false, &mut p);
    }
    if let Some(code) = t.child("numFmt").and_then(|n| n.get("formatCode")) {
        p.insert("num_format", Raw::Text(code.to_string()));
    }
    p
}
