//! Compound File Binary (MS-CFB), the container of .xls files: sectors chained through
//! the file allocation table, a directory of storages and streams, and a mini stream for
//! small streams. Read only, and with every chain and size checked, so that a damaged or
//! hostile file gives an error and never a loop or an allocation beyond its own size.

use crate::{Error, Result};

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
/// Sector ids from here on are markers, not sectors.
const MAX_REGULAR: u32 = 0xFFFF_FFFA;
const NO_STREAM: u32 = 0xFFFF_FFFF;

/// One entry of the directory.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// 1 storage, 2 stream, 5 root.
    pub kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// An opened compound file.
pub struct Cfb<'a> {
    data: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini_stream: Vec<u8>,
    mini_cutoff: u64,
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| Error::Invalid("compound file: header cut off".into()))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| Error::Invalid("compound file: header cut off".into()))
}

/// Whether the bytes start like a compound file.
pub fn is_cfb(data: &[u8]) -> bool {
    data.len() >= 512 && data[..8] == SIGNATURE
}

impl<'a> Cfb<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self> {
        if !is_cfb(data) {
            return Err(Error::Invalid("not a compound file (no .xls)".into()));
        }
        let shift = u16_at(data, 0x1E)?;
        if shift != 9 && shift != 12 {
            return Err(Error::Invalid(format!("compound file: sector size 2^{shift}")));
        }
        let sector_size = 1usize << shift;
        let mini_shift = u16_at(data, 0x20)?;
        if mini_shift != 6 {
            return Err(Error::Invalid(format!("compound file: mini sector size 2^{mini_shift}")));
        }
        let fat_sectors = u32_at(data, 0x2C)? as usize;
        let first_dir = u32_at(data, 0x30)?;
        let mini_cutoff = u32_at(data, 0x38)? as u64;
        let first_mini_fat = u32_at(data, 0x3C)?;
        let first_difat = u32_at(data, 0x44)?;
        let difat_sectors = u32_at(data, 0x48)? as usize;
        let mut cfb = Cfb { data, sector_size, fat: Vec::new(), mini_fat: Vec::new(), mini_stream: Vec::new(), mini_cutoff, entries: Vec::new() };
        let total = data.len() / sector_size;
        if fat_sectors > total || difat_sectors > total {
            return Err(Error::Invalid("compound file: more allocation sectors than the file has".into()));
        }
        // The sectors of the allocation table: 109 in the header, the rest in a chain.
        let mut fat_ids: Vec<u32> = Vec::with_capacity(fat_sectors);
        for k in 0..109 {
            let id = u32_at(data, 0x4C + 4 * k)?;
            if id <= MAX_REGULAR {
                fat_ids.push(id);
            }
        }
        let mut next = first_difat;
        let per = sector_size / 4 - 1;
        for _ in 0..difat_sectors {
            if next > MAX_REGULAR {
                break;
            }
            let sector = cfb.sector(next)?;
            for k in 0..per {
                let id = u32_at(sector, 4 * k)?;
                if id <= MAX_REGULAR {
                    fat_ids.push(id);
                }
            }
            next = u32_at(sector, 4 * per)?;
        }
        fat_ids.truncate(fat_sectors);
        let mut fat = Vec::with_capacity(fat_ids.len() * sector_size / 4);
        for id in fat_ids {
            let sector = cfb.sector(id)?;
            for k in 0..sector_size / 4 {
                fat.push(u32_at(sector, 4 * k)?);
            }
        }
        cfb.fat = fat;
        // Directory.
        let dir = cfb.chain(first_dir, None)?;
        for chunk in dir.chunks_exact(128) {
            let len = (u16_at(chunk, 64)? as usize).min(64);
            let units: Vec<u16> = chunk[..len.saturating_sub(2)].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
            let size = if sector_size == 512 { u32_at(chunk, 120)? as u64 } else { u32_at(chunk, 120)? as u64 | (u32_at(chunk, 124)? as u64) << 32 };
            cfb.entries.push(Entry {
                name: String::from_utf16_lossy(&units),
                kind: chunk[66],
                left: u32_at(chunk, 68)?,
                right: u32_at(chunk, 72)?,
                child: u32_at(chunk, 76)?,
                start: u32_at(chunk, 116)?,
                size,
            });
        }
        let root = cfb.entries.first().cloned().ok_or_else(|| Error::Invalid("compound file: no root entry".into()))?;
        if root.kind != 5 {
            return Err(Error::Invalid("compound file: the first entry is not the root".into()));
        }
        if first_mini_fat <= MAX_REGULAR {
            let raw = cfb.chain(first_mini_fat, None)?;
            cfb.mini_fat = raw.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]])).collect();
        }
        if root.start <= MAX_REGULAR {
            cfb.mini_stream = cfb.chain(root.start, Some(root.size))?;
        }
        Ok(cfb)
    }

    fn sector(&self, id: u32) -> Result<&'a [u8]> {
        let at = (id as usize + 1).checked_mul(self.sector_size).ok_or_else(|| Error::Invalid("compound file: sector out of range".into()))?;
        // The last sector may be short in files cut at their logical end.
        let end = (at + self.sector_size).min(self.data.len());
        if at >= end {
            return Err(Error::Invalid(format!("compound file: sector {id} beyond the end")));
        }
        Ok(&self.data[at..end])
    }

    /// The bytes of a chain of sectors, at most `size` of them.
    fn chain(&self, start: u32, size: Option<u64>) -> Result<Vec<u8>> {
        let limit = size.map(|s| s as usize).unwrap_or(usize::MAX).min(self.data.len());
        let mut out = Vec::new();
        let mut id = start;
        let mut steps = 0usize;
        let max_steps = self.data.len() / self.sector_size + 1;
        while id <= MAX_REGULAR && out.len() < limit {
            steps += 1;
            if steps > max_steps {
                return Err(Error::Invalid("compound file: a sector chain loops".into()));
            }
            out.extend_from_slice(self.sector(id)?);
            id = *self.fat.get(id as usize).ok_or_else(|| Error::Invalid("compound file: chain leaves the allocation table".into()))?;
        }
        if id != END_OF_CHAIN && id != FREE && out.len() < limit {
            return Err(Error::Invalid("compound file: bad chain end".into()));
        }
        if size.is_some() {
            if out.len() < limit {
                return Err(Error::Invalid("compound file: stream shorter than its size".into()));
            }
            out.truncate(limit);
        }
        Ok(out)
    }

    fn mini_chain(&self, start: u32, size: u64) -> Result<Vec<u8>> {
        let limit = size as usize;
        let mut out = Vec::with_capacity(limit.min(self.mini_stream.len()));
        let mut id = start;
        let mut steps = 0usize;
        while id <= MAX_REGULAR && out.len() < limit {
            steps += 1;
            if steps > self.mini_fat.len() + 1 {
                return Err(Error::Invalid("compound file: a mini sector chain loops".into()));
            }
            let at = id as usize * 64;
            let piece = self.mini_stream.get(at..at + 64).ok_or_else(|| Error::Invalid("compound file: mini sector beyond the mini stream".into()))?;
            out.extend_from_slice(piece);
            id = *self.mini_fat.get(id as usize).ok_or_else(|| Error::Invalid("compound file: chain leaves the mini allocation table".into()))?;
        }
        if out.len() < limit {
            return Err(Error::Invalid("compound file: stream shorter than its size".into()));
        }
        out.truncate(limit);
        Ok(out)
    }

    /// The entries directly in the root storage.
    fn root_children(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut todo = vec![self.entries[0].child];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = todo.pop() {
            if id == NO_STREAM || id as usize >= self.entries.len() || !seen.insert(id) {
                continue;
            }
            let e = &self.entries[id as usize];
            out.push(id as usize);
            todo.push(e.left);
            todo.push(e.right);
        }
        out
    }

    /// A stream of the root storage by name (case ignored).
    pub fn stream(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(i) = self.root_children().into_iter().find(|&i| self.entries[i].kind == 2 && self.entries[i].name.eq_ignore_ascii_case(name)) else {
            return Ok(None);
        };
        let e = &self.entries[i];
        if e.size > self.data.len() as u64 * 2 + 4096 {
            return Err(Error::Invalid("compound file: stream larger than the file".into()));
        }
        if e.size < self.mini_cutoff {
            self.mini_chain(e.start, e.size).map(Some)
        } else {
            self.chain(e.start, Some(e.size)).map(Some)
        }
    }
}
