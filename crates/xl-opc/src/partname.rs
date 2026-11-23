//! Part name handling (ECMA-376 Part 2, part names and relationship targets).

/// Lookup key for a part name: no leading slash, forward slashes, ASCII lower case.
/// OPC part names are compared case-insensitively.
pub fn normalize_key(name: &str) -> String {
    name.trim_start_matches(['/', '\\']).replace('\\', "/").to_ascii_lowercase()
}

/// Name of the relationships part that belongs to `source_part`.
/// The package root ("" or "/") maps to `_rels/.rels`.
pub fn rels_part_for(source_part: &str) -> String {
    let s = source_part.trim_start_matches('/');
    match s.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &s[..i], &s[i + 1..]),
        None => format!("_rels/{s}.rels"),
    }
}

/// Inverse of [`rels_part_for`]: the source part of a relationships part,
/// "" for the package root, `None` if `name` is not a relationships part.
pub fn source_for_rels_part(name: &str) -> Option<String> {
    let s = name.trim_start_matches('/');
    let lower = s.to_ascii_lowercase();
    if !lower.ends_with(".rels") {
        return None;
    }
    let (dir, file) = match s.rfind('/') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => return None,
    };
    let parent = match dir.rfind('/') {
        Some(i) => {
            if !dir[i + 1..].eq_ignore_ascii_case("_rels") {
                return None;
            }
            &dir[..i]
        }
        None => {
            if !dir.eq_ignore_ascii_case("_rels") {
                return None;
            }
            ""
        }
    };
    let base = &file[..file.len() - ".rels".len()];
    Some(if parent.is_empty() { base.to_string() } else { format!("{parent}/{base}") })
}

/// Resolve an internal relationship target against its source part.
/// Returns the part name without a leading slash, or `None` if the target
/// leaves the package root. Query and fragment are dropped.
pub fn resolve_target(source_part: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next().unwrap_or("").replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    if !target.starts_with('/') {
        let s = source_part.trim_start_matches('/');
        if let Some(i) = s.rfind('/') {
            segments.extend(s[..i].split('/'));
        }
    }
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_part_names() {
        assert_eq!(rels_part_for(""), "_rels/.rels");
        assert_eq!(rels_part_for("/"), "_rels/.rels");
        assert_eq!(rels_part_for("/xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(rels_part_for("xl/worksheets/sheet1.xml"), "xl/worksheets/_rels/sheet1.xml.rels");
    }

    #[test]
    fn rels_part_roundtrip() {
        for s in ["", "xl/workbook.xml", "xl/worksheets/sheet1.xml", "docProps/app.xml"] {
            assert_eq!(source_for_rels_part(&rels_part_for(s)).as_deref(), Some(s));
        }
        assert_eq!(source_for_rels_part("xl/workbook.xml"), None);
        assert_eq!(source_for_rels_part("xl/foo.rels"), None);
    }

    #[test]
    fn target_resolution() {
        assert_eq!(resolve_target("", "xl/workbook.xml").as_deref(), Some("xl/workbook.xml"));
        assert_eq!(resolve_target("xl/workbook.xml", "worksheets/sheet1.xml").as_deref(), Some("xl/worksheets/sheet1.xml"));
        assert_eq!(resolve_target("xl/worksheets/sheet1.xml", "../drawings/drawing1.xml").as_deref(), Some("xl/drawings/drawing1.xml"));
        assert_eq!(resolve_target("xl/workbook.xml", "/xl/styles.xml").as_deref(), Some("xl/styles.xml"));
        assert_eq!(resolve_target("xl/workbook.xml", "sharedStrings.xml#frag").as_deref(), Some("xl/sharedStrings.xml"));
        assert_eq!(resolve_target("xl/workbook.xml", "../../evil.xml"), None);
    }

    #[test]
    fn keys() {
        assert_eq!(normalize_key("/XL/Workbook.xml"), "xl/workbook.xml");
        assert_eq!(normalize_key("xl\\workbook.xml"), "xl/workbook.xml");
    }
}
