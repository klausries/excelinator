//! xl-opc: lossless Open Packaging Conventions (OPC) container for OOXML files.
//!
//! Guarantees
//! - Every entry of the source archive is kept. Entries that are not touched are
//!   written back with their original compressed bytes (no recompression).
//! - Entry order is preserved; new parts are appended.
//! - `[Content_Types].xml` and `_rels/*.rels` are ordinary parts. Typed views are
//!   parsed on demand and only re-serialized when explicitly set.
//! - Input is bounded: entry count, part size, total size and compression ratio
//!   are limited (zip bombs). XML with a DOCTYPE is rejected (entity expansion).

mod content_types;
mod error;
mod partname;
mod rels;
mod xml;

pub use content_types::{ContentTypeItem, ContentTypes};
pub use error::{Error, Result};
pub use partname::{normalize_key, rels_part_for, resolve_target, source_for_rels_part};
pub use rels::{Relationship, Relationships, TargetMode};

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, Write};
use std::sync::Arc;

pub const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// Resource limits applied while opening a package.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    /// Maximum decompressed size of a single part in bytes.
    pub max_part_size: u64,
    /// Maximum decompressed size of all parts together in bytes.
    pub max_total_size: u64,
    /// Maximum ratio decompressed/compressed for parts larger than 1 MiB.
    pub max_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_entries: 100_000,
            max_part_size: 2 << 30,
            max_total_size: 8 << 30,
            max_ratio: 2_000,
        }
    }
}

#[derive(Debug, Clone)]
enum Body {
    /// Unmodified entry number `index` of the source archive.
    Source { index: usize, size: u64 },
    /// New or replaced content.
    Owned(Vec<u8>),
}

#[derive(Debug, Clone)]
struct Entry {
    /// Name exactly as stored in the zip directory.
    name: String,
    is_dir: bool,
    body: Body,
}

/// An OPC package held in memory.
#[derive(Debug, Clone)]
pub struct Package {
    source: Option<Arc<Vec<u8>>>,
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
    limits: Limits,
}

impl Package {
    /// A new, empty package.
    pub fn new() -> Self {
        Package { source: None, entries: Vec::new(), index: HashMap::new(), limits: Limits::default() }
    }

    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        Self::open_with(bytes, Limits::default())
    }

    pub fn open_with(bytes: Vec<u8>, limits: Limits) -> Result<Self> {
        let source = Arc::new(bytes);
        let mut archive = zip::ZipArchive::new(Cursor::new(&source[..])).map_err(Error::zip)?;
        if archive.len() > limits.max_entries {
            return Err(Error::Limit(format!("{} entries, limit {}", archive.len(), limits.max_entries)));
        }
        let mut entries = Vec::with_capacity(archive.len());
        let mut index = HashMap::with_capacity(archive.len());
        let mut total: u64 = 0;
        for i in 0..archive.len() {
            let file = archive.by_index_raw(i).map_err(Error::zip)?;
            let name = file.name().map_err(Error::zip)?.into_owned();
            let (size, csize) = (file.size(), file.compressed_size());
            if size > limits.max_part_size {
                return Err(Error::Limit(format!("part '{name}' declares {size} bytes")));
            }
            if size > (1 << 20) && csize > 0 && size / csize > limits.max_ratio {
                return Err(Error::Limit(format!("part '{name}' has compression ratio {}", size / csize)));
            }
            total = total.saturating_add(size);
            if total > limits.max_total_size {
                return Err(Error::Limit(format!("package declares more than {} bytes", limits.max_total_size)));
            }
            let key = normalize_key(&name);
            if index.insert(key, i).is_some() {
                return Err(Error::Invalid(format!("duplicate part name '{name}'")));
            }
            entries.push(Entry { name, is_dir: file.is_dir(), body: Body::Source { index: i, size } });
        }
        drop(archive);
        Ok(Package { source: Some(source), entries, index, limits })
    }

    /// Names of all parts in package order, exactly as stored (directories excluded).
    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(|e| !e.is_dir).map(|e| e.name.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_dir).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Part lookup is case-insensitive and ignores a leading slash (OPC part name rules).
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(&normalize_key(name))
    }

    /// True if the part still refers to the unmodified source entry.
    pub fn is_untouched(&self, name: &str) -> bool {
        self.index
            .get(&normalize_key(name))
            .map(|&i| matches!(self.entries[i].body, Body::Source { .. }))
            .unwrap_or(false)
    }

    /// Decompressed content of a part.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let i = *self.index.get(&normalize_key(name)).ok_or_else(|| Error::NotFound(name.to_string()))?;
        match &self.entries[i].body {
            Body::Owned(data) => Ok(data.clone()),
            Body::Source { index, size } => {
                let source = self.source.as_ref().expect("source entry without source");
                let mut archive = zip::ZipArchive::new(Cursor::new(&source[..])).map_err(Error::zip)?;
                let file = archive.by_index(*index).map_err(Error::zip)?;
                let cap = self.limits.max_part_size;
                let mut out = Vec::with_capacity((*size).min(64 << 20) as usize);
                file.take(cap + 1).read_to_end(&mut out).map_err(|e| Error::Zip(e.to_string()))?;
                if out.len() as u64 > cap {
                    return Err(Error::Limit(format!("part '{name}' exceeds {cap} bytes")));
                }
                Ok(out)
            }
        }
    }

    /// Replace a part or add a new one. A replaced part keeps its position and stored name.
    pub fn set(&mut self, name: &str, data: Vec<u8>) {
        let key = normalize_key(name);
        match self.index.get(&key) {
            Some(&i) => self.entries[i].body = Body::Owned(data),
            None => {
                let stored = name.trim_start_matches('/').to_string();
                self.index.insert(key, self.entries.len());
                self.entries.push(Entry { name: stored, is_dir: false, body: Body::Owned(data) });
            }
        }
    }

    /// Remove a part. Returns false if it did not exist.
    pub fn remove(&mut self, name: &str) -> bool {
        let Some(i) = self.index.remove(&normalize_key(name)) else { return false };
        self.entries.remove(i);
        for v in self.index.values_mut() {
            if *v > i {
                *v -= 1;
            }
        }
        true
    }

    /// Typed view of `[Content_Types].xml`.
    pub fn content_types(&self) -> Result<ContentTypes> {
        ContentTypes::parse(&self.read(CONTENT_TYPES_PART)?)
    }

    pub fn set_content_types(&mut self, ct: &ContentTypes) {
        self.set(CONTENT_TYPES_PART, ct.to_xml().into_bytes());
    }

    /// Relationships of a source part. Use "" or "/" for the package root.
    /// A missing relationships part yields an empty set.
    pub fn relationships(&self, source_part: &str) -> Result<Relationships> {
        let rels_part = rels_part_for(source_part);
        if !self.contains(&rels_part) {
            return Ok(Relationships::default());
        }
        Relationships::parse(&self.read(&rels_part)?)
    }

    pub fn set_relationships(&mut self, source_part: &str, rels: &Relationships) {
        self.set(&rels_part_for(source_part), rels.to_xml().into_bytes());
    }

    /// Write the package. Untouched entries are copied with their original compressed bytes.
    pub fn write_to<W: Write + Seek>(&self, w: W) -> Result<W> {
        let mut writer = zip::ZipWriter::new(w);
        let mut archive = match &self.source {
            Some(s) => Some(zip::ZipArchive::new(Cursor::new(&s[..])).map_err(Error::zip)?),
            None => None,
        };
        // Fixed timestamp so that output is deterministic.
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for e in &self.entries {
            match &e.body {
                Body::Source { index, .. } => {
                    let archive = archive.as_mut().expect("source entry without source");
                    let file = archive.by_index_raw(*index).map_err(Error::zip)?;
                    writer.raw_copy_file(file).map_err(Error::zip)?;
                }
                Body::Owned(data) => {
                    writer.start_file(e.name.as_str(), options).map_err(Error::zip)?;
                    writer.write_all(data).map_err(|e| Error::Zip(e.to_string()))?;
                }
            }
        }
        writer.finish().map_err(Error::zip)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.write_to(Cursor::new(Vec::new()))?.into_inner())
    }
}

impl Default for Package {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode an XML part to text: UTF-8 (with or without BOM) or UTF-16 with BOM.
pub fn decode_xml(bytes: &[u8]) -> Result<String> {
    xml::decode(bytes)
}
