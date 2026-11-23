//! A small element tree over quick-xml, for the fragments this crate reads (fonts, fills,
//! borders, differential formats, theme colour schemes, table parts).

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Local name (`color` for `a:color`).
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The first element of this name anywhere below, depth first.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(n) = c.find(name) {
                return Some(n);
            }
        }
        None
    }

    /// ST_Boolean with the element-present default of true (`<b/>`, `<b val="0"/>`).
    pub fn flag(&self) -> bool {
        !matches!(self.get("val"), Some("0") | Some("false"))
    }

    /// An optional ST_Boolean attribute.
    pub fn bool_attr(&self, k: &str) -> Option<bool> {
        self.get(k).map(|v| matches!(v, "1" | "true"))
    }
}

fn local(s: &str) -> String {
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

fn node_of(e: &BytesStart) -> Node {
    let mut n = Node { name: local(e.name().as_ref()), ..Node::default() };
    for a in e.attributes().flatten() {
        let key = local(a.key.as_ref());
        let value = a.normalized_value(XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
        n.attrs.push((key, value));
    }
    n
}

/// The elements of an XML document or fragment, under a root named `#root`.
pub fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Node { name: "#root".into(), ..Node::default() }];
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => stack.push(node_of(&e)),
            Event::Empty(e) => {
                let n = node_of(&e);
                stack.last_mut().unwrap().children.push(n);
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err("unbalanced XML".into());
                }
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Text(t) => stack.last_mut().unwrap().text.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Event::GeneralRef(r) => {
                if let Some(c) = r.resolve_char_ref().map_err(|e| e.to_string())? {
                    stack.last_mut().unwrap().text.push(c);
                } else {
                    let name: &str = &r;
                    let text = quick_xml::escape::resolve_predefined_entity(name).ok_or_else(|| format!("unknown entity &{name};"))?;
                    stack.last_mut().unwrap().text.push_str(text);
                }
            }
            Event::DocType(_) => return Err("DOCTYPE is not allowed".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Err("unclosed element".into());
    }
    Ok(stack.pop().unwrap())
}
