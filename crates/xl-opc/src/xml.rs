//! Small XML helpers shared by the content-types and relationships parsers.

use crate::error::{Error, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Decode a part to text. Handles UTF-8 (with or without BOM) and UTF-16 with BOM.
pub(crate) fn decode(bytes: &[u8]) -> Result<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|e| Error::Xml(e.to_string()));
    }
    let utf16 = |rest: &[u8], le: bool| -> Result<String> {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16(&units).map_err(|e| Error::Xml(e.to_string()))
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| Error::Xml(e.to_string()))
}

/// One element with its local name and attributes (local name, unescaped value).
pub(crate) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// All start and empty elements of a document in order. A DOCTYPE is rejected.
pub(crate) fn elements(bytes: &[u8]) -> Result<Vec<Element>> {
    let text = decode(bytes)?;
    let mut reader = Reader::from_str(&text);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(|e| Error::Xml(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().into_inner().to_string();
                let mut attrs = Vec::new();
                for a in e.attributes() {
                    let a = a.map_err(|e| Error::Xml(e.to_string()))?;
                    let key = a.key.local_name().into_inner().to_string();
                    let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| Error::Xml(e.to_string()))?.into_owned();
                    attrs.push((key, value));
                }
                out.push(Element { name, attrs });
            }
            Event::DocType(_) => return Err(Error::Xml("DOCTYPE is not allowed in a package part".into())),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Escape a string for use as an attribute value in double quotes.
pub(crate) fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}
