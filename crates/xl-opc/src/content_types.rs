//! `[Content_Types].xml`: media types by extension (Default) and by part name (Override).

use crate::error::{Error, Result};
use crate::partname::normalize_key;
use crate::xml::{elements, escape_attr};

const NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentTypeItem {
    Default { extension: String, content_type: String },
    Override { part_name: String, content_type: String },
}

/// Items in document order. Order is kept so that an unchanged model serializes stably.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentTypes {
    pub items: Vec<ContentTypeItem>,
}

impl ContentTypes {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let els = elements(bytes)?;
        if els.first().map(|e| e.name.as_str()) != Some("Types") {
            return Err(Error::Invalid("[Content_Types].xml: root element is not <Types>".into()));
        }
        let mut items = Vec::new();
        for e in &els[1..] {
            let ct = || {
                e.attr("ContentType")
                    .map(str::to_string)
                    .ok_or_else(|| Error::Invalid(format!("[Content_Types].xml: <{}> without ContentType", e.name)))
            };
            match e.name.as_str() {
                "Default" => {
                    let extension = e
                        .attr("Extension")
                        .ok_or_else(|| Error::Invalid("[Content_Types].xml: <Default> without Extension".into()))?
                        .to_string();
                    items.push(ContentTypeItem::Default { extension, content_type: ct()? });
                }
                "Override" => {
                    let part_name = e
                        .attr("PartName")
                        .ok_or_else(|| Error::Invalid("[Content_Types].xml: <Override> without PartName".into()))?
                        .to_string();
                    items.push(ContentTypeItem::Override { part_name, content_type: ct()? });
                }
                _ => {}
            }
        }
        Ok(ContentTypes { items })
    }

    /// Content type of a part: an Override wins over a Default for the extension.
    pub fn get(&self, part_name: &str) -> Option<&str> {
        let key = normalize_key(part_name);
        for item in &self.items {
            if let ContentTypeItem::Override { part_name, content_type } = item {
                if normalize_key(part_name) == key {
                    return Some(content_type);
                }
            }
        }
        let file = key.rsplit('/').next().unwrap_or("");
        let ext = file.rsplit_once('.').map(|(_, e)| e)?;
        for item in &self.items {
            if let ContentTypeItem::Default { extension, content_type } = item {
                if extension.eq_ignore_ascii_case(ext) {
                    return Some(content_type);
                }
            }
        }
        None
    }

    /// Set the content type of one part (adds or replaces an Override).
    pub fn set_override(&mut self, part_name: &str, content_type: &str) {
        let key = normalize_key(part_name);
        for item in &mut self.items {
            if let ContentTypeItem::Override { part_name: p, content_type: c } = item {
                if normalize_key(p) == key {
                    *c = content_type.to_string();
                    return;
                }
            }
        }
        self.items.push(ContentTypeItem::Override {
            part_name: format!("/{}", part_name.trim_start_matches('/')),
            content_type: content_type.to_string(),
        });
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        s.push_str(&format!("<Types xmlns=\"{NS}\">"));
        for item in &self.items {
            match item {
                ContentTypeItem::Default { extension, content_type } => s.push_str(&format!(
                    "<Default Extension=\"{}\" ContentType=\"{}\"/>",
                    escape_attr(extension),
                    escape_attr(content_type)
                )),
                ContentTypeItem::Override { part_name, content_type } => s.push_str(&format!(
                    "<Override PartName=\"{}\" ContentType=\"{}\"/>",
                    escape_attr(part_name),
                    escape_attr(content_type)
                )),
            }
        }
        s.push_str("</Types>");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;

    #[test]
    fn parse_and_lookup() {
        let ct = ContentTypes::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(ct.items.len(), 3);
        assert_eq!(ct.get("/xl/workbook.xml"), Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"));
        assert_eq!(ct.get("XL/Workbook.xml"), ct.get("/xl/workbook.xml"));
        assert_eq!(ct.get("xl/styles.xml"), Some("application/xml"));
        assert_eq!(ct.get("xl/media/image1.png"), None);
    }

    #[test]
    fn serialization_is_byte_identical_for_canonical_input() {
        let ct = ContentTypes::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(ct.to_xml(), SAMPLE);
    }

    #[test]
    fn set_override_replaces_or_adds() {
        let mut ct = ContentTypes::parse(SAMPLE.as_bytes()).unwrap();
        ct.set_override("xl/workbook.xml", "x/y");
        ct.set_override("xl/new.bin", "a/b");
        assert_eq!(ct.get("/xl/workbook.xml"), Some("x/y"));
        assert_eq!(ct.get("/xl/new.bin"), Some("a/b"));
        assert_eq!(ContentTypes::parse(ct.to_xml().as_bytes()).unwrap(), ct);
    }

    #[test]
    fn doctype_is_rejected() {
        let evil = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><Types/>"#;
        assert!(ContentTypes::parse(evil.as_bytes()).is_err());
    }
}
