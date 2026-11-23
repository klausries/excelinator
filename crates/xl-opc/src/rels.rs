//! Relationship parts (`_rels/*.rels`).

use crate::error::{Error, Result};
use crate::xml::{elements, escape_attr};

const NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMode {
    #[default]
    Internal,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub target_mode: TargetMode,
}

/// Relationships in document order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Relationships {
    pub items: Vec<Relationship>,
}

impl Relationships {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let els = elements(bytes)?;
        if els.first().map(|e| e.name.as_str()) != Some("Relationships") {
            return Err(Error::Invalid("relationships part: root element is not <Relationships>".into()));
        }
        let mut items = Vec::new();
        for e in els[1..].iter().filter(|e| e.name == "Relationship") {
            let need = |n: &str| {
                e.attr(n).map(str::to_string).ok_or_else(|| Error::Invalid(format!("<Relationship> without {n}")))
            };
            let target_mode = match e.attr("TargetMode") {
                Some(m) if m.eq_ignore_ascii_case("External") => TargetMode::External,
                _ => TargetMode::Internal,
            };
            items.push(Relationship { id: need("Id")?, rel_type: need("Type")?, target: need("Target")?, target_mode });
        }
        Ok(Relationships { items })
    }

    pub fn by_id(&self, id: &str) -> Option<&Relationship> {
        self.items.iter().find(|r| r.id == id)
    }

    pub fn by_type<'a>(&'a self, rel_type: &'a str) -> impl Iterator<Item = &'a Relationship> {
        self.items.iter().filter(move |r| r.rel_type == rel_type)
    }

    /// Add a relationship with the next free `rIdN` and return its id.
    pub fn add(&mut self, rel_type: &str, target: &str, target_mode: TargetMode) -> String {
        let mut n = self.items.len() + 1;
        while self.by_id(&format!("rId{n}")).is_some() {
            n += 1;
        }
        let id = format!("rId{n}");
        self.items.push(Relationship { id: id.clone(), rel_type: rel_type.into(), target: target.into(), target_mode });
        id
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        s.push_str(&format!("<Relationships xmlns=\"{NS}\">"));
        for r in &self.items {
            s.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"",
                escape_attr(&r.id),
                escape_attr(&r.rel_type),
                escape_attr(&r.target)
            ));
            if r.target_mode == TargetMode::External {
                s.push_str(" TargetMode=\"External\"");
            }
            s.push_str("/>");
        }
        s.push_str("</Relationships>");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/?a=1&amp;b=2" TargetMode="External"/></Relationships>"#;

    #[test]
    fn parse() {
        let r = Relationships::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.by_id("rId1").unwrap().target, "worksheets/sheet1.xml");
        assert_eq!(r.by_id("rId2").unwrap().target, "https://example.org/?a=1&b=2");
        assert_eq!(r.by_id("rId2").unwrap().target_mode, TargetMode::External);
    }

    #[test]
    fn serialization_is_byte_identical_for_canonical_input() {
        let r = Relationships::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r.to_xml(), SAMPLE);
    }

    #[test]
    fn add_picks_free_id() {
        let mut r = Relationships::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(r.add("t", "x.xml", TargetMode::Internal), "rId3");
        assert_eq!(Relationships::parse(r.to_xml().as_bytes()).unwrap(), r);
    }
}
