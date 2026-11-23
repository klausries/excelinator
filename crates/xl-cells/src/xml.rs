//! A small push parser over quick-xml, and the text escapes SpreadsheetML adds on top of XML.

use crate::{Error, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

pub(crate) enum Ev<'a> {
    Start(&'a str, &'a Attrs),
    End(&'a str),
    Text(&'a str),
}

/// Attributes by local name; those of the `xml` namespace keep their prefix (`xml:space`).
/// `foreign` marks namespace declarations and attributes of other namespaces (`r:id`,
/// `x14ac:dyDescent`), which only mean something with their declarations.
#[derive(Default)]
pub(crate) struct Attrs(Vec<(String, String)>, Vec<bool>);

impl Attrs {
    /// The attributes that can be written again into a fragment of the main namespace.
    pub fn own(&self) -> impl Iterator<Item = &(String, String)> {
        self.0.iter().zip(&self.1).filter(|(_, f)| !**f).map(|(a, _)| a)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn all(&self) -> &[(String, String)] {
        &self.0
    }

    /// ST_Boolean with the element-present default of true (`<b/>`, `<b val="0"/>`).
    pub fn flag(&self) -> bool {
        !matches!(self.get("val"), Some("0") | Some("false"))
    }
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Xml(e.to_string())
}

fn attrs_of(e: &BytesStart) -> Result<Attrs> {
    let mut out = Vec::new();
    let mut foreign = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(err)?;
        let full = a.key.into_inner();
        let prefix = a.key.prefix().map(|p| p.into_inner());
        let key = match prefix {
            Some("xml") => full.to_string(),
            _ => a.key.local_name().into_inner().to_string(),
        };
        let value = a.normalized_value(XmlVersion::Implicit1_0).map_err(err)?.into_owned();
        foreign.push(full == "xmlns" || matches!(prefix, Some(p) if p != "xml"));
        out.push((key, value));
    }
    Ok(Attrs(out, foreign))
}

/// Feed all elements and character data of a document to `f`. Empty elements arrive as
/// a start followed by an end. Names are local names. A DOCTYPE is rejected.
pub(crate) fn walk(xml: &str, f: &mut dyn FnMut(Ev) -> Result<()>) -> Result<()> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(err)? {
            Event::Start(e) => {
                let attrs = attrs_of(&e)?;
                f(Ev::Start(e.local_name().into_inner(), &attrs))?;
            }
            Event::Empty(e) => {
                let attrs = attrs_of(&e)?;
                let name = e.local_name().into_inner();
                f(Ev::Start(name, &attrs))?;
                f(Ev::End(name))?;
            }
            Event::End(e) => f(Ev::End(e.local_name().into_inner()))?,
            Event::Text(t) => f(Ev::Text(&t.xml_content(XmlVersion::Implicit1_0)))?,
            Event::CData(t) => f(Ev::Text(&t.xml_content(XmlVersion::Implicit1_0)))?,
            Event::GeneralRef(r) => {
                if let Some(c) = r.resolve_char_ref().map_err(err)? {
                    f(Ev::Text(c.encode_utf8(&mut [0u8; 4])))?;
                } else {
                    let name: &str = &r;
                    let text = quick_xml::escape::resolve_predefined_entity(name).ok_or_else(|| err(format!("unknown entity &{name};")))?;
                    f(Ev::Text(text))?;
                }
            }
            Event::DocType(_) => return Err(err("DOCTYPE is not allowed in a package part")),
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// Rebuilds the XML below an element from parser events (local names, no prefixes),
/// leaving out extension lists, whose content needs namespaces of its own.
#[derive(Default)]
pub(crate) struct Capture {
    depth: usize,
    skip: usize,
    /// The last start tag still lacks its `>` (an empty element becomes `<x/>`).
    open: bool,
    pub xml: String,
}

impl Capture {
    pub fn active(&self) -> bool {
        self.depth > 0
    }

    /// Start capturing the content of the element whose start was just seen.
    pub fn begin(&mut self) {
        self.depth = 1;
        self.skip = 0;
        self.open = false;
        self.xml.clear();
    }

    /// Feed an event; returns true when the element has ended.
    pub fn event(&mut self, ev: &Ev) -> bool {
        match ev {
            Ev::Start(name, a) => {
                self.depth += 1;
                if self.skip > 0 || *name == "extLst" {
                    self.skip += 1;
                } else {
                    if self.open {
                        self.xml.push('>');
                    }
                    self.xml.push('<');
                    self.xml.push_str(name);
                    for (k, v) in a.own() {
                        self.xml.push_str(&format!(" {k}=\"{}\"", escape_attr_plain(v)));
                    }
                    self.open = true;
                }
            }
            Ev::End(name) => {
                self.depth -= 1;
                if self.depth == 0 {
                    return true;
                }
                if self.skip > 0 {
                    self.skip -= 1;
                } else if self.open {
                    self.xml.push_str("/>");
                    self.open = false;
                } else {
                    self.xml.push_str(&format!("</{name}>"));
                }
            }
            Ev::Text(t) => {
                if self.skip == 0 {
                    if self.open {
                        self.xml.push('>');
                        self.open = false;
                    }
                    self.xml.push_str(&t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
                }
            }
        }
        false
    }
}

/// Decode the `_xHHHH_` escapes of ST_Xstring (ECMA-376 part 1, 22.9.2.19).
pub fn unescape_xstring(s: &str) -> String {
    if !s.contains("_x") {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut units: Vec<u16> = Vec::new();
    let mut i = 0;
    let mut plain = 0;
    while i < b.len() {
        if b[i] == b'_' && i + 6 < b.len() && b[i + 1] == b'x' && b[i + 6] == b'_' && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit) {
            out.push_str(&s[plain..i]);
            units.clear();
            // Consecutive escapes may form a surrogate pair.
            while i + 6 < b.len() && b[i] == b'_' && b[i + 1] == b'x' && b[i + 6] == b'_' && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit) {
                units.push(u16::from_str_radix(&s[i + 2..i + 6], 16).unwrap());
                i += 7;
            }
            out.extend(char::decode_utf16(units.iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')));
            plain = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[plain..]);
    out
}

fn looks_like_escape(b: &[u8], i: usize) -> bool {
    i + 6 < b.len() && b[i] == b'_' && b[i + 1] == b'x' && b[i + 6] == b'_' && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit)
}

/// Escape text for element content, including the `_xHHHH_` form for characters XML
/// cannot carry and for carriage returns (which XML parsers would normalise away).
pub(crate) fn escape_text(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '_' if looks_like_escape(b, i) => out.push_str("_x005F_"),
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push_str(&format!("_x{:04X}_", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// An attribute value as it was read: XML escapes only (`_xHHHH_` stays as it is).
pub(crate) fn escape_attr_plain(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;").replace('\t', "&#9;").replace('\r', "&#13;")
}

pub(crate) fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;").replace('\n', "&#10;").replace('\t', "&#9;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xstring() {
        assert_eq!(unescape_xstring("a_x000D_b"), "a\rb");
        assert_eq!(unescape_xstring("_x005F_x000D_"), "_x000D_");
        assert_eq!(unescape_xstring("_xD83D__xDE00_"), "😀");
        assert_eq!(unescape_xstring("_x12"), "_x12");
        for s in ["a\rb", "_x000D_", "\u{1}x", "plain_text", "_x005F_", "a<b&c>\"d\""] {
            let mut back = String::new();
            walk(&format!("<t>{}</t>", escape_text(s)), &mut |ev| {
                if let Ev::Text(t) = ev {
                    back.push_str(t);
                }
                Ok(())
            })
            .unwrap();
            assert_eq!(unescape_xstring(&back), s);
        }
    }
}

/// The relationships namespace, transitional and strict.
const NS_RELS: [&str; 2] = ["http://schemas.openxmlformats.org/officeDocument/2006/relationships", "http://purl.oclc.org/ooxml/officeDocument/relationships"];

/// Children of the root element with the given local names, each as XML that stands on
/// its own: attributes in the relationships namespace are written `r:…` (with the
/// namespace declared on the fragment), other prefixed attributes and elements (markup
/// compatibility extras such as `x14ac:dyDescent`) are left out. Empty if the root has a
/// prefix.
pub(crate) fn children_raw(xml: &str, wanted: &[&str]) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut rel_prefixes: Vec<String> = Vec::new();
    // The prefix of the main namespace when the document uses one (`<x:worksheet>`): its
    // elements are written without it.
    let mut main: Option<String> = None;
    let mut depth = 0usize;
    // The fragment being built: its name, text, whether it uses r:, and how deep a skipped subtree is.
    let mut current: Option<(String, String, bool)> = None;
    let mut skip = 0usize;
    let tag = |e: &BytesStart, rel: &[String], main: &Option<String>, uses_r: &mut bool| -> Result<Option<String>> {
        if let Some(p) = e.name().prefix() {
            if main.as_deref() != Some(p.into_inner()) {
                return Ok(None);
            }
        }
        let mut s = format!("<{}", e.local_name().into_inner());
        for a in e.attributes() {
            let a = a.map_err(err)?;
            let key = a.key.into_inner();
            if key.starts_with("xmlns") {
                continue;
            }
            let value = a.normalized_value(XmlVersion::Implicit1_0).map_err(err)?;
            match key.split_once(':') {
                None => s.push_str(&format!(" {key}=\"{}\"", escape_attr(&value))),
                Some((p, local)) if rel.iter().any(|r| r == p) => {
                    *uses_r = true;
                    s.push_str(&format!(" r:{local}=\"{}\"", escape_attr(&value)));
                }
                Some(_) => {}
            }
        }
        Ok(Some(s))
    };
    loop {
        match reader.read_event().map_err(err)? {
            Event::Start(e) | Event::Empty(e) if depth == 0 => {
                let root_prefix = e.name().prefix().map(|p| p.into_inner().to_string());
                for a in e.attributes().flatten() {
                    let key = a.key.into_inner().to_string();
                    if let Some(p) = key.strip_prefix("xmlns:") {
                        if NS_RELS.contains(&a.value.as_ref()) {
                            rel_prefixes.push(p.to_string());
                        }
                        if Some(p) == root_prefix.as_deref() {
                            main = Some(p.to_string());
                        }
                    }
                }
                if root_prefix.is_some() && main.is_none() {
                    return Ok(Vec::new());
                }
                depth = 1;
            }
            ev @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(ev, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = ev else { unreachable!() };
                if !empty {
                    depth += 1;
                }
                if skip > 0 {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                let at_child = if empty { depth == 1 } else { depth == 2 };
                if at_child {
                    let name = e.local_name().into_inner().to_string();
                    let ours = e.name().prefix().is_none_or(|p| main.as_deref() == Some(p.into_inner()));
                    if ours && wanted.contains(&name.as_str()) {
                        let mut uses_r = false;
                        let mut s = tag(&e, &rel_prefixes, &main, &mut uses_r)?.unwrap_or_default();
                        if empty {
                            s.push_str("/>");
                            out.push((name, finish_fragment(s, uses_r)));
                        } else {
                            s.push('>');
                            current = Some((name, s, uses_r));
                        }
                    } else if !empty {
                        skip = 1;
                    }
                } else if let Some((_, s, uses_r)) = current.as_mut() {
                    match tag(&e, &rel_prefixes, &main, uses_r)? {
                        Some(t) => {
                            s.push_str(&t);
                            s.push_str(if empty { "/>" } else { ">" });
                        }
                        None if !empty => skip = 1,
                        None => {}
                    }
                }
            }
            Event::End(e) => {
                if skip > 0 {
                    skip -= 1;
                } else if depth == 2 {
                    if let Some((name, mut s, uses_r)) = current.take() {
                        s.push_str(&format!("</{name}>"));
                        out.push((name, finish_fragment(s, uses_r)));
                    }
                } else if let Some((_, s, _)) = current.as_mut() {
                    s.push_str(&format!("</{}>", e.local_name().into_inner()));
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(t) if skip == 0 => {
                if let Some((_, s, _)) = current.as_mut() {
                    // The text as it stands in the file: XML escapes only, `_xHHHH_` left alone.
                    s.push_str(&plain_escape(&t.xml_content(XmlVersion::Implicit1_0)));
                }
            }
            Event::GeneralRef(r) if skip == 0 => {
                if let Some((_, s, _)) = current.as_mut() {
                    let name: &str = &r;
                    s.push_str(&format!("&{name};"));
                }
            }
            Event::DocType(_) => return Err(err("DOCTYPE is not allowed in a package part")),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Declare the relationships namespace on a fragment that uses it.
fn finish_fragment(s: String, uses_r: bool) -> String {
    if !uses_r {
        return s;
    }
    let end = s.find(|c: char| c == ' ' || c == '>' || c == '/').unwrap_or(s.len());
    format!("{} xmlns:r=\"{}\"{}", &s[..end], NS_RELS[0], &s[end..])
}

/// `&`, `<` and `>` escaped; nothing else changed.
fn plain_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
