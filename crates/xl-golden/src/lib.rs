//! xl-golden: corpus manifest and golden checks.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub mod cells;
pub mod recalc;
pub mod shape;

/// One corpus source: directory below `corpus/`, licence, description.
pub struct Source {
    pub name: &'static str,
    pub dir: &'static str,
    pub license: &'static str,
}

pub const SOURCES: &[Source] = &[
    Source { name: "rust_xlsxwriter", dir: "src-rust_xlsxwriter/tests/input", license: "MIT OR Apache-2.0" },
    Source { name: "calamine", dir: "src-calamine/tests", license: "MIT" },
    Source { name: "poi", dir: "src-poi/test-data/spreadsheet", license: "Apache-2.0" },
    // Legacy .xls files of the sources above, recalculated and saved as xlsx by LibreOffice
    // (tools/lo_oracle.py). The cached values in these files are LibreOffice's.
    Source { name: "lo-oracle", dir: "lo-oracle", license: "derived: Apache-2.0 / MIT" },
    // Real-world workbooks (Enron corpus, as published by SheetJS), sampled and converted the same way.
    Source { name: "lo-enron", dir: "lo-enron", license: "derived: see src-enron/LICENSE" },
];

pub const OOXML_EXT: &[&str] = &["xlsx", "xlsm", "xltx", "xltm"];

pub struct CorpusFile {
    pub source: &'static str,
    pub license: &'static str,
    pub path: PathBuf,
    /// Path relative to the corpus root, with forward slashes.
    pub rel: String,
    pub ext: String,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// All spreadsheet files of the corpus, sorted, with source and licence.
pub fn corpus_files(root: &Path) -> Vec<CorpusFile> {
    let mut out = Vec::new();
    for s in SOURCES {
        let mut files = Vec::new();
        walk(&root.join(s.dir), &mut files);
        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "xlsx" | "xlsm" | "xltx" | "xltm" | "xls" | "xlsb" | "ods") {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(CorpusFile { source: s.name, license: s.license, path, rel, ext });
        }
    }
    out
}

/// For a file produced by `tools/lo_oracle.py`: the legacy original it was converted from.
/// Its cached values are the ones the original application (Excel) computed.
pub fn legacy_original(root: &Path, rel: &str) -> Option<PathBuf> {
    let (dir, rest) = rel.split_once('/')?;
    let rest = rest.strip_suffix(".xlsx")?;
    let p = match dir {
        "lo-oracle" => root.join(rest),
        "lo-enron" => root.join("src-enron").join(rest),
        _ => return None,
    };
    p.is_file().then_some(p)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Outcome of the lossless round-trip check for one file.
#[derive(Debug, Default)]
pub struct RoundTrip {
    /// "ok", "not-zip", "ole", "open-error", "mismatch", "panic"
    pub status: &'static str,
    pub detail: String,
    pub parts: usize,
    /// Parts without a content type.
    pub untyped_parts: usize,
    /// Internal relationship targets that do not exist in the package.
    pub dangling_rels: usize,
    pub rels_parts: usize,
    /// Relationship or content-type parts whose model does not survive re-serialization.
    pub model_mismatch: usize,
    /// Relationship or content-type parts our parser rejects.
    pub model_errors: usize,
}

fn raw_entries(bytes: &[u8]) -> Result<Vec<(String, u32, Vec<u8>)>, String> {
    use std::io::Read;
    let mut a = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..a.len() {
        let mut f = a.by_index_raw(i).map_err(|e| e.to_string())?;
        let name = f.name().map_err(|e| e.to_string())?.into_owned();
        let crc = f.crc32();
        let mut raw = Vec::new();
        f.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        out.push((name, crc, raw));
    }
    Ok(out)
}

/// Read, write unchanged, reopen and compare. Also checks the typed views.
pub fn roundtrip(bytes: &[u8]) -> RoundTrip {
    let mut r = RoundTrip::default();
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        r.status = "ole";
        r.detail = "OLE container (encrypted or legacy format)".into();
        return r;
    }
    let pkg = match xl_opc::Package::open(bytes.to_vec()) {
        Ok(p) => p,
        Err(e) => {
            r.status = if bytes.starts_with(b"PK") { "open-error" } else { "not-zip" };
            r.detail = e.to_string();
            return r;
        }
    };
    r.parts = pkg.len();
    let out = match pkg.to_bytes() {
        Ok(o) => o,
        Err(e) => {
            r.status = "mismatch";
            r.detail = format!("write failed: {e}");
            return r;
        }
    };
    // 1. Every entry: same name, same order, same compressed bytes, same CRC.
    match (raw_entries(bytes), raw_entries(&out)) {
        (Ok(a), Ok(b)) => {
            if a.len() != b.len() {
                r.status = "mismatch";
                r.detail = format!("entry count {} -> {}", a.len(), b.len());
                return r;
            }
            for (x, y) in a.iter().zip(&b) {
                if x != y {
                    r.status = "mismatch";
                    r.detail = format!("entry '{}' differs", x.0);
                    return r;
                }
            }
        }
        (Err(e), _) | (_, Err(e)) => {
            r.status = "mismatch";
            r.detail = format!("raw read failed: {e}");
            return r;
        }
    }
    // 2. Decompressed content identical after reopening.
    let again = match xl_opc::Package::open(out) {
        Ok(p) => p,
        Err(e) => {
            r.status = "mismatch";
            r.detail = format!("reopen failed: {e}");
            return r;
        }
    };
    let names: Vec<String> = pkg.part_names().map(str::to_string).collect();
    for n in &names {
        match (pkg.read(n), again.read(n)) {
            (Ok(a), Ok(b)) if a == b => {}
            (Err(a), Err(b)) if a == b => {} // unreadable in the source, equally unreadable after
            _ => {
                r.status = "mismatch";
                r.detail = format!("part '{n}' content differs");
                return r;
            }
        }
    }
    // 3. Typed views: parse, serialize, parse again; consistency counts.
    let ct = match pkg.content_types() {
        Ok(ct) => {
            match xl_opc::ContentTypes::parse(ct.to_xml().as_bytes()) {
                Ok(ct2) if ct2 == ct => {}
                _ => r.model_mismatch += 1,
            }
            Some(ct)
        }
        Err(_) => {
            r.model_errors += 1;
            None
        }
    };
    for n in &names {
        let is_ct = n.eq_ignore_ascii_case(xl_opc::CONTENT_TYPES_PART);
        if let Some(source) = xl_opc::source_for_rels_part(n) {
            r.rels_parts += 1;
            match pkg.read(n).and_then(|b| xl_opc::Relationships::parse(&b)) {
                Ok(rels) => {
                    match xl_opc::Relationships::parse(rels.to_xml().as_bytes()) {
                        Ok(r2) if r2 == rels => {}
                        _ => r.model_mismatch += 1,
                    }
                    for rel in &rels.items {
                        if rel.target_mode == xl_opc::TargetMode::Internal {
                            match xl_opc::resolve_target(&source, &rel.target) {
                                Some(t) if pkg.contains(&t) => {}
                                _ => r.dangling_rels += 1,
                            }
                        }
                    }
                }
                Err(_) => r.model_errors += 1,
            }
        }
        if !is_ct {
            if let Some(ct) = &ct {
                if ct.get(n).is_none() {
                    r.untyped_parts += 1;
                }
            }
        }
    }
    r.status = "ok";
    r
}

/// Run a check and turn a panic into a result.
pub fn guarded<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> Result<T, String> {
    std::panic::catch_unwind(f).map_err(|e| {
        e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
    })
}
