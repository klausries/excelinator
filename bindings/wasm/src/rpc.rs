//! Messages across the border between the module and its host, both ways.
//!
//! A message is a JSON document and byte strings attached to it (a workbook file, a
//! frame as an Arrow IPC stream):
//!
//! ```text
//! u32 length of the JSON | JSON | u32 number of attachments | (u32 length | bytes)*
//! ```
//!
//! all numbers little-endian. In the JSON, `{"$b": i}` stands for attachment `i` as
//! bytes and `{"$f": i}` for attachment `i` as a frame.
//!
//! A call of the host: `{"fn": name, "args": [...]}`; the answer `{"ok": value}` or
//! `{"err": text}`.

use serde_json::{json, Value as J};
use xl_engine::data::Frame;

pub struct Msg {
    pub json: J,
    pub blobs: Vec<Vec<u8>>,
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let text = self.json.to_string();
        let mut out = Vec::with_capacity(text.len() + 8 + self.blobs.iter().map(|b| b.len() + 4).sum::<usize>());
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(text.as_bytes());
        out.extend((self.blobs.len() as u32).to_le_bytes());
        for b in &self.blobs {
            out.extend((b.len() as u32).to_le_bytes());
            out.extend(b);
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<Msg, String> {
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let part = b.get(at..at + n).ok_or("a message ended early")?;
            at += n;
            Ok(part)
        };
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let json = serde_json::from_slice(take(n)?).map_err(|e| format!("a message is not JSON: {e}"))?;
        let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let mut blobs = Vec::with_capacity(count);
        for _ in 0..count {
            let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
            blobs.push(take(n)?.to_vec());
        }
        Ok(Msg { json, blobs })
    }
}

/// The arguments of a call.
pub struct In {
    pub args: Vec<J>,
    pub blobs: Vec<Vec<u8>>,
}

impl In {
    pub fn arg(&self, i: usize) -> &J {
        self.args.get(i).unwrap_or(&J::Null)
    }

    pub fn str(&self, i: usize) -> Result<&str, String> {
        self.arg(i).as_str().ok_or_else(|| format!("argument {i} must be text"))
    }

    pub fn opt_str(&self, i: usize) -> Option<String> {
        self.arg(i).as_str().map(str::to_string)
    }

    pub fn u64(&self, i: usize) -> Result<u64, String> {
        self.arg(i).as_u64().ok_or_else(|| format!("argument {i} must be a whole number"))
    }

    pub fn u32(&self, i: usize) -> Result<u32, String> {
        Ok(self.u64(i)? as u32)
    }

    pub fn usize(&self, i: usize) -> Result<usize, String> {
        Ok(self.u64(i)? as usize)
    }

    pub fn f64(&self, i: usize) -> Result<f64, String> {
        self.arg(i).as_f64().ok_or_else(|| format!("argument {i} must be a number"))
    }

    /// A flag; missing or null is `default`.
    pub fn flag(&self, i: usize, default: bool) -> bool {
        self.arg(i).as_bool().unwrap_or(default)
    }

    pub fn bytes(&self, i: usize) -> Result<&[u8], String> {
        self.blob(self.arg(i), "$b").ok_or_else(|| format!("argument {i} must be bytes"))
    }

    pub fn frame(&self, i: usize) -> Result<Frame, String> {
        let b = self.blob(self.arg(i), "$f").ok_or_else(|| format!("argument {i} must be a dataframe"))?;
        xl_arrow::ipc::frame_from_ipc(b)
    }

    /// An attachment named in the JSON (`{"$b": i}`, `{"$f": i}`).
    pub fn blob(&self, v: &J, key: &str) -> Option<&[u8]> {
        v.get(key).and_then(J::as_u64).and_then(|i| self.blobs.get(i as usize)).map(Vec::as_slice)
    }
}

/// An answer being put together: its attachments.
#[derive(Default)]
pub struct Out {
    pub blobs: Vec<Vec<u8>>,
}

impl Out {
    pub fn bytes(&mut self, b: Vec<u8>) -> J {
        self.blobs.push(b);
        json!({ "$b": self.blobs.len() - 1 })
    }

    pub fn frame(&mut self, f: &Frame) -> Result<J, String> {
        self.blobs.push(xl_arrow::ipc::frame_to_ipc(f)?);
        Ok(json!({ "$f": self.blobs.len() - 1 }))
    }
}
