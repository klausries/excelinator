//! The records of a BIFF8 `Workbook` stream (MS-XLS 2.4): the globals substream
//! (formats, fonts, cell formats, sheets, shared strings, names, references), then one
//! substream per sheet with its cells.

use std::collections::HashMap;
use std::sync::Arc;

use xl_cells::{Border, Column, DefinedName, Kind, Row, Sheet, Style, Visibility, Workbook};

use crate::cfb::{is_cfb, Cfb};
use crate::formula::{self, Ctx, SupBook};
use crate::{Error, Result};

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;

fn invalid(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

/// One record with the `Continue` records that follow it, as separate pieces: a string
/// that runs into a `Continue` starts there again with its own flags byte.
struct Record<'a> {
    kind: u16,
    pieces: Vec<&'a [u8]>,
}

impl<'a> Record<'a> {
    fn data(&self) -> std::borrow::Cow<'a, [u8]> {
        if self.pieces.len() == 1 {
            std::borrow::Cow::Borrowed(self.pieces[0])
        } else {
            std::borrow::Cow::Owned(self.pieces.concat())
        }
    }

    fn cursor(&self) -> Cursor<'_, 'a> {
        Cursor { pieces: &self.pieces, i: 0, at: 0 }
    }
}

struct Records<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Records<'a> {
    fn header(&self, at: usize) -> Option<(u16, usize)> {
        let h = self.data.get(at..at + 4)?;
        Some((u16::from_le_bytes([h[0], h[1]]), u16::from_le_bytes([h[2], h[3]]) as usize))
    }

    fn next(&mut self) -> Result<Option<Record<'a>>> {
        let Some((kind, len)) = self.header(self.at) else { return Ok(None) };
        let body = self.data.get(self.at + 4..self.at + 4 + len).ok_or_else(|| invalid("record cut off"))?;
        self.at += 4 + len;
        let mut pieces = vec![body];
        while let Some((CONTINUE, len)) = self.header(self.at) {
            let body = self.data.get(self.at + 4..self.at + 4 + len).ok_or_else(|| invalid("record cut off"))?;
            self.at += 4 + len;
            pieces.push(body);
        }
        Ok(Some(Record { kind, pieces }))
    }
}

/// Reads across the pieces of a record.
struct Cursor<'r, 'a> {
    pieces: &'r [&'a [u8]],
    i: usize,
    at: usize,
}

impl Cursor<'_, '_> {
    fn bytes(&mut self, mut n: usize, out: &mut Vec<u8>) -> Result<()> {
        while n > 0 {
            let piece = self.pieces.get(self.i).ok_or_else(|| invalid("record cut off"))?;
            let take = n.min(piece.len() - self.at);
            out.extend_from_slice(&piece[self.at..self.at + take]);
            self.at += take;
            n -= take;
            if self.at == piece.len() && n > 0 {
                self.i += 1;
                self.at = 0;
            }
        }
        Ok(())
    }

    fn skip(&mut self, mut n: usize) -> Result<()> {
        while n > 0 {
            let piece = self.pieces.get(self.i).ok_or_else(|| invalid("record cut off"))?;
            let take = n.min(piece.len() - self.at);
            self.at += take;
            n -= take;
            if self.at == piece.len() && n > 0 {
                self.i += 1;
                self.at = 0;
            }
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8> {
        let mut v = Vec::with_capacity(1);
        self.bytes(1, &mut v)?;
        Ok(v[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let mut v = Vec::with_capacity(2);
        self.bytes(2, &mut v)?;
        Ok(u16::from_le_bytes([v[0], v[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let mut v = Vec::with_capacity(4);
        self.bytes(4, &mut v)?;
        Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
    }

    /// `cch` characters after their flags byte; at the start of each further piece
    /// the characters continue after a flags byte of their own.
    fn chars(&mut self, cch: usize, mut high: bool) -> Result<String> {
        let mut units: Vec<u16> = Vec::with_capacity(cch.min(1 << 16));
        while units.len() < cch {
            let piece = self.pieces.get(self.i).ok_or_else(|| invalid("string cut off"))?;
            if self.at == piece.len() {
                self.i += 1;
                self.at = 0;
                high = self.u8()? & 0x01 != 0;
                continue;
            }
            let piece = &piece[self.at..];
            let width = if high { 2 } else { 1 };
            let n = (cch - units.len()).min(piece.len() / width);
            if n == 0 {
                return Err(invalid("string cut inside a character"));
            }
            if high {
                units.extend(piece[..2 * n].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])));
            } else {
                units.extend(piece[..n].iter().map(|&b| b as u16));
            }
            self.at += width * n;
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// `XLUnicodeString`: 16-bit length, flags, characters.
    fn string16(&mut self) -> Result<String> {
        let cch = self.u16()? as usize;
        let high = self.u8()? & 0x01 != 0;
        self.chars(cch, high)
    }

    /// `ShortXLUnicodeString`: 8-bit length, flags, characters.
    fn string8(&mut self) -> Result<String> {
        let cch = self.u8()? as usize;
        let high = self.u8()? & 0x01 != 0;
        self.chars(cch, high)
    }

    /// `XLUnicodeRichExtendedString` of the shared strings: runs and phonetic data are
    /// skipped.
    fn rich_string(&mut self) -> Result<String> {
        let cch = self.u16()? as usize;
        let flags = self.u8()?;
        let runs = if flags & 0x08 != 0 { self.u16()? as usize } else { 0 };
        let ext = if flags & 0x04 != 0 { self.u32()? as usize } else { 0 };
        let s = self.chars(cch, flags & 0x01 != 0)?;
        self.skip(4 * runs)?;
        self.skip(ext)?;
        Ok(s)
    }
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| invalid("record too short"))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| invalid("record too short"))
}

fn f64_at(b: &[u8], at: usize) -> Result<f64> {
    b.get(at..at + 8).map(|s| f64::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| invalid("record too short"))
}

/// The path of an external workbook (`VirtualPath`, MS-XLS 2.5.277): encoded with
/// control characters for drive, root, separator and parent directory.
fn virtual_path(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix('\u{1}') else {
        return raw.trim_start_matches('\u{2}').to_string();
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1}' => {
                match chars.next() {
                    Some('@') => out.push_str("\\\\"), // a UNC path
                    Some(drive) => {
                        out.push(drive);
                        out.push_str(":\\");
                    }
                    None => {}
                }
            }
            '\u{2}' | '\u{3}' => out.push('\\'),
            '\u{4}' => out.push_str("..\\"),
            '\u{5}' => {
                // A URL: its length, then the URL itself.
                let n = chars.next().map(|c| c as usize).unwrap_or(0);
                out.extend(chars.by_ref().take(n));
            }
            '\u{6}' | '\u{7}' | '\u{8}' => {}
            c => out.push(c),
        }
    }
    out
}

/// A number in the compressed form of `RkNumber`.
fn rk(v: u32) -> f64 {
    let n = if v & 0x02 != 0 { (v as i32 >> 2) as f64 } else { f64::from_bits(((v & 0xFFFF_FFFC) as u64) << 32) };
    if v & 0x01 != 0 {
        n / 100.0
    } else {
        n
    }
}

struct Font {
    height: u16,
    italic: bool,
    strike: bool,
    color: u16,
    bold: bool,
    underline: u8,
    name: String,
}

struct Xf {
    font: u16,
    format: u16,
    locked: bool,
    hidden: bool,
    h_align: u8,
    wrap: bool,
    v_align: u8,
    rotation: u8,
    indent: u8,
    /// Line style and colour: left, right, top, bottom.
    borders: [(u8, u16); 4],
    pattern: u8,
    fore: u16,
    back: u16,
}

const DEFAULT_PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080,
    0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC,
    0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
];

const BORDER_STYLES: [&str; 14] = [
    "", "thin", "medium", "dashed", "dotted", "thick", "double", "hair", "mediumDashed", "dashDot", "mediumDashDot", "dashDotDot", "mediumDashDotDot", "slantDashDot",
];

const PATTERNS: [&str; 19] = [
    "none", "solid", "mediumGray", "darkGray", "lightGray", "darkHorizontal", "darkVertical", "darkDown", "darkUp", "darkGrid", "darkTrellis",
    "lightHorizontal", "lightVertical", "lightDown", "lightUp", "lightGrid", "lightTrellis", "gray125", "gray0625",
];

/// Names of built-in defined names by their one-character code.
const BUILTIN_NAMES: [&str; 14] = [
    "Consolidate_Area", "Auto_Open", "Auto_Close", "Extract", "Database", "Criteria", "Print_Area", "Print_Titles", "Recorder", "Data_Form",
    "Auto_Activate", "Auto_Deactivate", "Sheet_Title", "_FilterDatabase",
];

struct Globals {
    date1904: bool,
    formats: HashMap<u16, String>,
    fonts: Vec<Font>,
    xfs: Vec<Xf>,
    palette: Vec<u32>,
    /// Stream position, visibility, type, name.
    sheets: Vec<(u32, u8, u8, String)>,
    sst: Vec<Arc<str>>,
    /// Flags, name as formulas spell it, sheet (1-based, 0 for the workbook), rgce, rgcb.
    names: Vec<(u16, String, u16, Vec<u8>, Vec<u8>)>,
    xti: Vec<(u16, i16, i16)>,
    supbooks: Vec<SupBook>,
    externals: Vec<xl_cells::External>,
    /// Definitions of names of other workbooks: workbook, name, rgce.
    extern_name_defs: Vec<(usize, usize, Vec<u8>)>,
    full_precision: bool,
}

/// What reading left out, by reason (formulas not read and why), for tests and reports.
#[derive(Debug, Default)]
pub struct Notes {
    pub skipped: HashMap<String, usize>,
}

impl Notes {
    fn note(&mut self, why: impl Into<String>) {
        *self.skipped.entry(why.into()).or_default() += 1;
    }
}

/// The calculation settings of an .xls file as an .xlsx `<calcPr>` (manual calculation,
/// iteration, precision as displayed), without reading the cells; `None` where all are
/// the defaults.
pub fn calc_settings(data: &[u8]) -> Result<Option<String>> {
    let cfb = Cfb::open(data)?;
    let stream = cfb.stream("Workbook")?.ok_or_else(|| invalid("no Workbook stream"))?;
    let mut records = Records { data: &stream, at: 0 };
    let mut calc = CalcSettings::default();
    let mut full_precision = true;
    let mut depth = 0usize;
    while let Some(r) = records.next()? {
        let d = r.data();
        match r.kind {
            BOF => depth += 1,
            EOF => {
                depth = depth.saturating_sub(1);
                // The first sheet after the globals has the settings.
                if depth == 0 && (calc.mode.is_some() || calc.iterate.is_some()) {
                    break;
                }
            }
            0x000E => full_precision = u16_at(&d, 0)? != 0,
            0x000D => calc.mode = Some(u16_at(&d, 0)? as i16),
            0x000C => calc.count = Some(u16_at(&d, 0)?),
            0x0011 => calc.iterate = Some(u16_at(&d, 0)? != 0),
            0x0010 => calc.delta = Some(f64_at(&d, 0)?),
            _ => {}
        }
    }
    Ok(calc.xml(full_precision))
}

/// Read an .xls file (BIFF8 in a compound file).
pub fn read_bytes(data: &[u8]) -> Result<Workbook> {
    read_with_notes(data).map(|(wb, _)| wb)
}

/// As [`read_bytes`], with what was left out.
pub fn read_with_notes(data: &[u8]) -> Result<(Workbook, Notes)> {
    if !is_cfb(data) {
        return Err(invalid("not a compound file"));
    }
    let cfb = Cfb::open(data)?;
    let stream = match cfb.stream("Workbook")? {
        Some(s) => s,
        None if cfb.stream("Book")?.is_some() => return Err(Error::Unsupported("Excel 5.0/95 workbook (BIFF5)".into())),
        None => return Err(invalid("no Workbook stream")),
    };
    let mut notes = Notes::default();
    let mut g = globals(&stream)?;
    let mut wb = Workbook { date1904: g.date1904, externals: std::mem::take(&mut g.externals), ..Workbook::default() };
    let sheet_names: Vec<String> = g.sheets.iter().map(|s| s.3.clone()).collect();
    let name_texts: Vec<String> = g.names.iter().map(|n| n.1.clone()).collect();
    let name_sheets: Vec<Option<usize>> = g.names.iter().map(|n| (n.2 as usize).checked_sub(1)).collect();
    let ctx = Ctx { sheets: &sheet_names, xti: &g.xti, supbooks: &g.supbooks, names: &name_texts, name_sheets: &name_sheets, sheet: None };
    // The definitions of names of other workbooks, as the formulas of this one spell them
    // (`[1]Sheet1!$A$1`); one that does not read is left empty (unknown).
    for (ext, k, rgce) in &g.extern_name_defs {
        if rgce.is_empty() {
            continue;
        }
        match formula::extern_definition(rgce, &wb.externals[*ext].sheets) {
            Ok(f) => wb.externals[*ext].names[*k].1 = f,
            Err(why) => notes.note(format!("name of another workbook: {why}")),
        }
    }
    let mut styles = StyleMap::new(&g);
    // The default format: the first cell format (`Normal`).
    wb.styles[0] = styles.style(&g, 0);
    let mut sheet_index: HashMap<usize, u32> = HashMap::new();
    let mut calc = CalcSettings::default();
    for (itab, (pos, state, dt, name)) in g.sheets.iter().enumerate() {
        if *dt != 0 {
            continue;
        }
        let at = *pos as usize;
        let Some(body) = stream.get(at..) else { return Err(invalid("sheet beyond the end of the stream")) };
        let mut records = Records { data: body, at: 0 };
        let Some(bof) = records.next()? else { continue };
        if bof.kind != BOF || u16_at(&bof.data(), 2)? != 0x0010 {
            continue; // a chart, macro or dialog sheet
        }
        let index = wb.sheets.len() as u32;
        let mut sheet = Sheet {
            name: name.clone(),
            visibility: match state & 0x03 {
                1 => Visibility::Hidden,
                2 => Visibility::VeryHidden,
                _ => Visibility::Visible,
            },
            origin: Some(index),
            ..Sheet::default()
        };
        let mut here = CalcSettings::default();
        read_sheet(&mut records, &g, &Ctx { sheet: Some(itab), ..ctx }, &mut styles, &mut wb, &mut sheet, index, &mut notes, &mut here)?;
        if calc.mode.is_none() && calc.iterate.is_none() {
            calc = here;
        }
        sheet_index.insert(itab, index);
        wb.sheets.push(sheet);
    }
    if let Some(xml) = calc.xml(g.full_precision) {
        wb.extra.push(("calcPr".into(), xml));
    }
    for (i, s) in styles.order.drain(..) {
        debug_assert_eq!(i as usize, wb.styles.len());
        wb.styles.push(s);
    }
    // Defined names.
    for (flags, name, itab, rgce, rgcb) in &g.names {
        if rgce.is_empty() {
            continue;
        }
        let local_sheet = match *itab {
            0 => None,
            i => match sheet_index.get(&(i as usize - 1)) {
                Some(&s) => Some(s),
                None => continue,
            },
        };
        match formula::decompile(rgce, rgcb, &Ctx { sheet: (*itab as usize).checked_sub(1), ..ctx }, None) {
            Ok(value) => wb.defined_names.push(DefinedName { hidden: flags & 0x01 != 0, name: name.clone(), local_sheet, value }),
            Err(why) => notes.note(format!("name: {why}")),
        }
    }
    Ok((wb, notes))
}

fn globals(stream: &[u8]) -> Result<Globals> {
    let mut records = Records { data: stream, at: 0 };
    let bof = records.next()?.ok_or_else(|| invalid("empty Workbook stream"))?;
    let d = bof.data();
    if bof.kind != BOF {
        return Err(Error::Unsupported("not a BIFF8 workbook (Excel 4.0 or older)".into()));
    }
    if u16_at(&d, 0)? != 0x0600 {
        return Err(Error::Unsupported(format!("BIFF version 0x{:04X} (only BIFF8, Excel 97-2003)", u16_at(&d, 0)?)));
    }
    let mut g = Globals {
        date1904: false,
        formats: HashMap::new(),
        fonts: Vec::new(),
        xfs: Vec::new(),
        palette: DEFAULT_PALETTE.to_vec(),
        sheets: Vec::new(),
        sst: Vec::new(),
        names: Vec::new(),
        xti: Vec::new(),
        supbooks: Vec::new(),
        externals: Vec::new(),
        extern_name_defs: Vec::new(),
        full_precision: true,
    };
    let mut xct_sheet: Option<u32> = None;
    while let Some(r) = records.next()? {
        let d = r.data();
        match r.kind {
            EOF => break,
            0x002F => return Err(Error::Unsupported("encrypted workbook".into())),
            0x0022 => g.date1904 = u16_at(&d, 0)? == 1,
            // Precision as displayed (0) instead of full precision (1).
            0x000E => g.full_precision = u16_at(&d, 0)? != 0,
            0x041E => {
                // Some writers state one character more than they write.
                let id = u16_at(&d, 0)?;
                let cch = u16_at(&d, 2)? as usize;
                let high = *d.get(4).ok_or_else(|| invalid("number format too short"))? & 0x01 != 0;
                let body = &d[5..];
                let code = if high {
                    let n = cch.min(body.len() / 2);
                    String::from_utf16_lossy(&body[..2 * n].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect::<Vec<_>>())
                } else {
                    body[..cch.min(body.len())].iter().map(|&b| b as char).collect()
                };
                g.formats.insert(id, code);
            }
            0x0031 => {
                let grbit = u16_at(&d, 2)?;
                let mut c = r.cursor();
                c.skip(14)?;
                g.fonts.push(Font {
                    height: u16_at(&d, 0)?,
                    italic: grbit & 0x02 != 0,
                    strike: grbit & 0x08 != 0,
                    color: u16_at(&d, 4)?,
                    bold: u16_at(&d, 6)? >= 700,
                    underline: *d.get(10).ok_or_else(|| invalid("font too short"))?,
                    name: c.string8()?,
                });
            }
            0x00E0 => {
                let flags = u16_at(&d, 4)?;
                let align = *d.get(6).ok_or_else(|| invalid("cell format too short"))?;
                let b1 = u32_at(&d, 10)?;
                let b2 = u32_at(&d, 14)?;
                let fill = u16_at(&d, 18)?;
                g.xfs.push(Xf {
                    font: u16_at(&d, 0)?,
                    format: u16_at(&d, 2)?,
                    locked: flags & 0x01 != 0,
                    hidden: flags & 0x02 != 0,
                    h_align: align & 0x07,
                    wrap: align & 0x08 != 0,
                    v_align: (align >> 4) & 0x07,
                    rotation: d[7],
                    indent: d[8] & 0x0F,
                    borders: [
                        ((b1 & 0x0F) as u8, ((b1 >> 16) & 0x7F) as u16),
                        (((b1 >> 4) & 0x0F) as u8, ((b1 >> 23) & 0x7F) as u16),
                        (((b1 >> 8) & 0x0F) as u8, (b2 & 0x7F) as u16),
                        (((b1 >> 12) & 0x0F) as u8, ((b2 >> 7) & 0x7F) as u16),
                    ],
                    pattern: ((b2 >> 26) & 0x3F) as u8,
                    fore: fill & 0x7F,
                    back: (fill >> 7) & 0x7F,
                });
            }
            0x0092 => {
                let n = u16_at(&d, 0)? as usize;
                for i in 0..n.min(56) {
                    let at = 2 + 4 * i;
                    let c = d.get(at..at + 3).ok_or_else(|| invalid("palette too short"))?;
                    g.palette[i] = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
                }
            }
            0x0085 => {
                let mut c = r.cursor();
                c.skip(6)?;
                g.sheets.push((u32_at(&d, 0)?, *d.get(4).unwrap_or(&0), *d.get(5).unwrap_or(&0), c.string8()?));
            }
            0x00FC => {
                let mut c = r.cursor();
                c.skip(4)?;
                let unique = c.u32()? as usize;
                // Each string takes at least three bytes.
                let total: usize = r.pieces.iter().map(|p| p.len()).sum();
                g.sst.reserve(unique.min(total / 3));
                for _ in 0..unique {
                    match c.rich_string() {
                        Ok(t) => g.sst.push(Arc::from(t)),
                        Err(_) => break, // fewer strings than announced
                    }
                }
            }
            0x0018 => {
                let flags = u16_at(&d, 0)?;
                let cch = *d.get(3).ok_or_else(|| invalid("name too short"))? as usize;
                let cce = u16_at(&d, 4)? as usize;
                let itab = u16_at(&d, 8)?;
                let mut c = r.cursor();
                c.skip(14)?;
                let high = c.u8()? & 0x01 != 0;
                let mut name = c.chars(cch, high)?;
                if flags & 0x20 != 0 {
                    let code = name.chars().next().map(|c| c as usize).unwrap_or(usize::MAX);
                    name = match BUILTIN_NAMES.get(code) {
                        Some(b) => format!("_xlnm.{b}"),
                        None => name,
                    };
                }
                let mut rgce = Vec::new();
                c.bytes(cce, &mut rgce).ok();
                let mut rgcb = Vec::new();
                let rest: usize = r.pieces.iter().map(|p| p.len()).sum::<usize>().saturating_sub(14 + 1 + if high { 2 * cch } else { cch } + cce);
                c.bytes(rest, &mut rgcb).ok();
                g.names.push((flags, name, itab, rgce, rgcb));
            }
            0x01AE => {
                let ctab = u16_at(&d, 0)? as usize;
                let cch = u16_at(&d, 2)?;
                g.supbooks.push(match cch {
                    0x0401 => SupBook::Internal,
                    0x3A01 => SupBook::AddIn(Vec::new()),
                    _ => {
                        let mut c = r.cursor();
                        c.skip(2)?;
                        let path = c.string16()?;
                        // A DDE or OLE link has no sheets and joins application and topic
                        // with 0x03; a workbook path is encoded (0x01, 0x02) or a plain name.
                        let dde = ctab == 0 && !path.starts_with(['\u{1}', '\u{2}']) && path.contains('\u{3}');
                        if !dde {
                            let mut sheets = Vec::with_capacity(ctab.min(256));
                            for _ in 0..ctab {
                                sheets.push(c.string16()?);
                            }
                            g.externals.push(xl_cells::External { target: Some(virtual_path(&path)), sheets: sheets.clone(), ..Default::default() });
                            SupBook::External { number: g.externals.len(), sheets, names: Vec::new() }
                        } else {
                            SupBook::Other(Vec::new())
                        }
                    }
                });
            }
            0x0023 => {
                let mut c = r.cursor();
                c.skip(6)?;
                let name = c.string8().unwrap_or_default();
                match g.supbooks.last_mut() {
                    Some(SupBook::AddIn(n)) | Some(SupBook::Other(n)) => n.push(name),
                    Some(SupBook::External { number, names, .. }) => {
                        names.push(name.clone());
                        // Its definition (`NameParsedFormula`) follows the name: cce, rgce.
                        let flags = u16_at(&d, 0).unwrap_or(0);
                        let mut rgce = Vec::new();
                        if flags & 0x7FE0 == 0 {
                            if let Ok(cce) = c.u16() {
                                let _ = c.bytes(cce as usize, &mut rgce);
                            }
                        }
                        let ext = *number - 1;
                        g.externals[ext].names.push((name, String::new()));
                        g.extern_name_defs.push((ext, g.externals[ext].names.len() - 1, rgce));
                    }
                    _ => {}
                }
            }
            // The cached cells of an external workbook: a sheet (XCT), then its rows (CRN).
            0x0059 => xct_sheet = Some(u16_at(&d, 2)? as u32),
            0x005A => {
                if let (Some(SupBook::External { number, .. }), Some(sheet)) = (g.supbooks.last(), xct_sheet) {
                    let ext = &mut g.externals[*number - 1];
                    let (last, first, row) = (*d.first().unwrap_or(&0) as u32, *d.get(1).unwrap_or(&0) as u32, u16_at(&d, 2)? as u32);
                    let mut c = r.cursor();
                    c.skip(4)?;
                    for col in first..=last.max(first) {
                        let v = match c.u8()? {
                            0x01 => {
                                let mut b = Vec::with_capacity(8);
                                c.bytes(8, &mut b)?;
                                xl_cells::Value::Number(f64::from_le_bytes(b.try_into().unwrap()))
                            }
                            0x02 => xl_cells::Value::Text(c.string16()?),
                            0x04 => {
                                let v = c.u8()?;
                                c.skip(7)?;
                                xl_cells::Value::Bool(v != 0)
                            }
                            0x10 => {
                                let e = c.u8()?;
                                c.skip(7)?;
                                xl_cells::Value::Error(formula::error_code(e).unwrap_or("#N/A").into())
                            }
                            _ => {
                                c.skip(8)?;
                                continue;
                            }
                        };
                        ext.cells.push((sheet, row, col, v));
                    }
                }
            }
            0x0017 => {
                let n = u16_at(&d, 0)? as usize;
                for i in 0..n {
                    let at = 2 + 6 * i;
                    g.xti.push((u16_at(&d, at)?, u16_at(&d, at + 2)? as i16, u16_at(&d, at + 4)? as i16));
                }
            }
            _ => {}
        }
    }
    Ok(g)
}

/// Cell formats (`XF` index) to styles of the workbook, made when first used.
struct StyleMap {
    by_xf: HashMap<u16, u32>,
    /// Styles in the order of their index, before they go into the workbook.
    order: Vec<(u32, Style)>,
    known: HashMap<String, u32>,
}

impl StyleMap {
    fn new(_g: &Globals) -> Self {
        StyleMap { by_xf: HashMap::new(), order: Vec::new(), known: HashMap::new() }
    }

    fn color(g: &Globals, icv: u16) -> Option<String> {
        let rgb = match icv {
            0..=7 => DEFAULT_PALETTE[icv as usize],
            8..=63 => g.palette[icv as usize - 8],
            _ => return None,
        };
        Some(format!("FF{rgb:06X}"))
    }

    fn style(&self, g: &Globals, xf: u16) -> Style {
        let mut s = Style::default();
        let Some(x) = g.xfs.get(xf as usize) else { return s };
        s.num_format = match g.formats.get(&x.format) {
            Some(f) => f.clone(),
            None => xl_cells::builtin_num_format(x.format as u32).unwrap_or("General").to_string(),
        };
        // Font 4 does not exist; indices from there on are one higher.
        let fi = if x.font >= 4 { x.font as usize - 1 } else { x.font as usize };
        if let Some(f) = g.fonts.get(fi) {
            s.bold = f.bold;
            s.italic = f.italic;
            s.strike = f.strike;
            s.underline = match f.underline {
                0x01 => Some("single".into()),
                0x02 => Some("double".into()),
                0x21 => Some("singleAccounting".into()),
                0x22 => Some("doubleAccounting".into()),
                _ => None,
            };
            s.font_name = Some(f.name.clone());
            s.font_size = Some(f.height as f64 / 20.0);
            s.font_color = Self::color(g, f.color);
        }
        match x.pattern {
            0 => {}
            1 => s.fill = Self::color(g, x.fore),
            p => {
                let name = PATTERNS.get(p as usize).copied().unwrap_or("solid");
                let c = |icv| Self::color(g, icv).map(|c| format!("<fgColor rgb=\"{c}\"/>")).unwrap_or_default();
                let fg = c(x.fore);
                let bg = Self::color(g, x.back).map(|c| format!("<bgColor rgb=\"{c}\"/>")).unwrap_or_default();
                s.fill_xml = Some(format!("<fill><patternFill patternType=\"{name}\">{fg}{bg}</patternFill></fill>"));
            }
        }
        for (k, &(line, icv)) in x.borders.iter().enumerate() {
            if line != 0 {
                s.borders[k] = Some(Border { style: BORDER_STYLES.get(line as usize).copied().unwrap_or("thin").into(), color: Self::color(g, icv) });
            }
        }
        s.h_align = ["", "left", "center", "right", "fill", "justify", "centerContinuous", "distributed"].get(x.h_align as usize).filter(|a| !a.is_empty()).map(|a| a.to_string());
        s.v_align = ["top", "center", "", "justify", "distributed"].get(x.v_align as usize).filter(|a| !a.is_empty()).map(|a| a.to_string());
        s.wrap = x.wrap;
        s.rotation = x.rotation as u32;
        s.indent = x.indent as u32;
        if !x.locked || x.hidden {
            s.protection_xml = Some(format!("<protection locked=\"{}\" hidden=\"{}\"/>", x.locked as u8, x.hidden as u8));
        }
        s
    }

    /// Index into the workbook's styles; index 0 is the default style.
    fn index(&mut self, g: &Globals, wb: &Workbook, xf: u16) -> u32 {
        if let Some(&i) = self.by_xf.get(&xf) {
            return i;
        }
        let s = self.style(g, xf);
        let i = if s == wb.styles[0] {
            0
        } else {
            let key = s.to_spec();
            match self.known.get(&key) {
                Some(&i) => i,
                None => {
                    let i = wb.styles.len() as u32 + self.order.len() as u32;
                    self.order.push((i, s));
                    self.known.insert(key, i);
                    i
                }
            }
        };
        self.by_xf.insert(xf, i);
        i
    }
}

struct Pending {
    cell: usize,
    row: u32,
    col: u32,
    rgce: Vec<u8>,
    rgcb: Vec<u8>,
}

/// The calculation settings of the workbook (`CalcMode`, `CalcCount`, `CalcIter`, `CalcDelta`).
#[derive(Default, Clone, Copy)]
struct CalcSettings {
    mode: Option<i16>,
    count: Option<u16>,
    iterate: Option<bool>,
    delta: Option<f64>,
}

impl CalcSettings {
    /// As the `<calcPr>` of an .xlsx, with only what differs from its defaults (automatic,
    /// no iteration, 100 iterations, 0.001); `None` if nothing does.
    fn xml(&self, full_precision: bool) -> Option<String> {
        let mut attrs = String::new();
        match self.mode {
            Some(0) => attrs.push_str(" calcMode=\"manual\""),
            Some(-1) => attrs.push_str(" calcMode=\"autoNoTable\""),
            _ => {}
        }
        if !full_precision {
            attrs.push_str(" fullPrecision=\"0\"");
        }
        if self.iterate == Some(true) {
            attrs.push_str(" iterate=\"1\"");
        }
        if let Some(c) = self.count.filter(|&c| c != 100) {
            attrs.push_str(&format!(" iterateCount=\"{c}\""));
        }
        if let Some(d) = self.delta.filter(|&d| d != 0.001 && d.is_finite()) {
            attrs.push_str(&format!(" iterateDelta=\"{d}\""));
        }
        (!attrs.is_empty()).then(|| format!("<calcPr{attrs}/>"))
    }
}

#[allow(clippy::too_many_arguments)]
fn read_sheet(
    records: &mut Records,
    g: &Globals,
    ctx: &Ctx,
    styles: &mut StyleMap,
    wb: &mut Workbook,
    sheet: &mut Sheet,
    index: u32,
    notes: &mut Notes,
    calc: &mut CalcSettings,
) -> Result<()> {
    let mut formulas: Vec<Pending> = Vec::new();
    // Shared and array formulas by their range: first row, last row, first column, last column.
    let mut shared: Vec<((u32, u32, u32, u32), Vec<u8>, Vec<u8>)> = Vec::new();
    let mut arrays: Vec<((u32, u32, u32, u32), Vec<u8>, Vec<u8>)> = Vec::new();
    let mut tables: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut string_for: Option<usize> = None;
    let mut frozen = false;
    let mut pane: Option<(u32, u32)> = None;
    let mut depth = 0usize;
    macro_rules! push {
        ($row:expr, $col:expr, $xf:expr, $kind:expr, $num:expr, $text:expr) => {{
            let style = styles.index(g, wb, $xf);
            wb.cells.push_raw(index, $row as u32, $col as u32, $kind, $num, $text, None, style);
        }};
    }
    while let Some(r) = records.next()? {
        let d = r.data();
        // Nested substreams (embedded charts) end with their own EOF.
        if r.kind == BOF {
            depth += 1;
            continue;
        }
        if r.kind == EOF {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        }
        if depth > 0 {
            continue;
        }
        // The cached string of a formula follows it, after its shared or array formula.
        if !matches!(r.kind, 0x0207 | 0x04BC | 0x0221 | 0x0236) {
            string_for = None;
        }
        match r.kind {
            0x0200 => {
                let (r0, r1) = (u32_at(&d, 0)?, u32_at(&d, 4)?);
                let (c0, c1) = (u16_at(&d, 8)? as u32, u16_at(&d, 10)? as u32);
                if r1 > r0 && c1 > c0 {
                    sheet.dimension = Some((r0, c0, r1 - 1, c1 - 1));
                }
            }
            // LabelSst
            0x00FD => {
                let i = u32_at(&d, 6)? as usize;
                let text = g.sst.get(i).cloned().ok_or_else(|| invalid("shared string index out of range"))?;
                push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Text, 0.0, Some(text));
            }
            // Label (rare in BIFF8)
            0x0204 => {
                let mut c = r.cursor();
                c.skip(6)?;
                let text: Arc<str> = Arc::from(c.string16()?);
                push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Text, 0.0, Some(text));
            }
            0x0203 => push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Number, f64_at(&d, 6)?, None),
            0x027E => push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Number, rk(u32_at(&d, 6)?), None),
            0x00BD => {
                let row = u16_at(&d, 0)?;
                let first = u16_at(&d, 2)? as usize;
                let n = (d.len().saturating_sub(6)) / 6;
                for k in 0..n {
                    let at = 4 + 6 * k;
                    push!(row, first + k, u16_at(&d, at)?, Kind::Number, rk(u32_at(&d, at + 2)?), None);
                }
            }
            0x0201 => push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Blank, 0.0, None),
            0x00BE => {
                let row = u16_at(&d, 0)?;
                let first = u16_at(&d, 2)? as usize;
                let n = (d.len().saturating_sub(6)) / 2;
                for k in 0..n {
                    push!(row, first + k, u16_at(&d, 4 + 2 * k)?, Kind::Blank, 0.0, None);
                }
            }
            0x0205 => {
                let v = *d.get(6).ok_or_else(|| invalid("cell too short"))?;
                if *d.get(7).unwrap_or(&0) != 0 {
                    let e = formula::error_code(v).unwrap_or("#N/A");
                    push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Error, 0.0, Some(Arc::from(e)));
                } else {
                    push!(u16_at(&d, 0)?, u16_at(&d, 2)?, u16_at(&d, 4)?, Kind::Bool, (v != 0) as u8 as f64, None);
                }
            }
            0x0006 => {
                let (row, col, xf) = (u16_at(&d, 0)? as u32, u16_at(&d, 2)? as u32, u16_at(&d, 4)?);
                let val = d.get(6..14).ok_or_else(|| invalid("formula too short"))?;
                let cce = u16_at(&d, 20)? as usize;
                let rgce = d.get(22..22 + cce).ok_or_else(|| invalid("formula cut off"))?.to_vec();
                let rgcb = d[22 + cce..].to_vec();
                let (kind, num, text) = if val[6] == 0xFF && val[7] == 0xFF {
                    match val[0] {
                        0 => (Kind::Text, 0.0, Some(Arc::from(""))),
                        1 => (Kind::Bool, (val[2] != 0) as u8 as f64, None),
                        2 => (Kind::Error, 0.0, Some(Arc::from(formula::error_code(val[2]).unwrap_or("#N/A")))),
                        3 => (Kind::Text, 0.0, Some(Arc::from(""))),
                        _ => (Kind::Blank, 0.0, None),
                    }
                } else {
                    (Kind::Number, f64::from_le_bytes(val.try_into().unwrap()), None)
                };
                push!(row, col, xf, kind, num, text);
                let cell = wb.cells.len() - 1;
                if val[6] == 0xFF && val[7] == 0xFF && val[0] == 0 {
                    string_for = Some(cell);
                }
                formulas.push(Pending { cell, row, col, rgce, rgcb });
            }
            0x0207 => {
                if let Some(cell) = string_for.take() {
                    let mut c = r.cursor();
                    wb.cells.text[cell] = Some(Arc::from(c.string16()?));
                }
            }
            0x04BC | 0x0221 => {
                let range = (u16_at(&d, 0)? as u32, u16_at(&d, 2)? as u32, *d.get(4).unwrap_or(&0) as u32, *d.get(5).unwrap_or(&0) as u32);
                let at = if r.kind == 0x04BC { 8 } else { 12 };
                let cce = u16_at(&d, at)? as usize;
                let rgce = d.get(at + 2..at + 2 + cce).ok_or_else(|| invalid("formula cut off"))?.to_vec();
                let rgcb = d[at + 2 + cce..].to_vec();
                if r.kind == 0x04BC {
                    shared.push((range, rgce, rgcb));
                } else {
                    arrays.push((range, rgce, rgcb));
                }
            }
            0x0236 => {
                tables.push((u16_at(&d, 0)? as u32, u16_at(&d, 2)? as u32, *d.get(4).unwrap_or(&0) as u32, *d.get(5).unwrap_or(&0) as u32));
            }
            0x0208 => {
                let row = u16_at(&d, 0)? as u32;
                let height = u16_at(&d, 6)? & 0x7FFF;
                let flags = u32_at(&d, 12)?;
                let mut info = Row {
                    row,
                    height: Some(height as f64 / 20.0),
                    custom_height: flags & 0x40 != 0,
                    hidden: flags & 0x20 != 0,
                    outline_level: (flags & 0x07) as u8,
                    collapsed: flags & 0x10 != 0,
                    custom_format: flags & 0x80 != 0,
                    thick_top: flags & (1 << 28) != 0,
                    thick_bot: flags & (1 << 29) != 0,
                    phonetic: flags & (1 << 30) != 0,
                    style: None,
                };
                if info.custom_format {
                    info.style = Some(styles.index(g, wb, ((flags >> 16) & 0x0FFF) as u16));
                }
                if info.custom_height || info.hidden || info.outline_level > 0 || info.collapsed || info.custom_format {
                    sheet.rows.push(info);
                }
            }
            0x007D => {
                let first = u16_at(&d, 0)? as u32;
                let last = (u16_at(&d, 2)? as u32).min(255);
                let width = u16_at(&d, 4)? as f64 / 256.0;
                let xf = u16_at(&d, 6)?;
                let flags = u16_at(&d, 8)?;
                if first > last {
                    continue;
                }
                let style = styles.index(g, wb, xf);
                sheet.columns.push(Column {
                    first,
                    last,
                    width: Some(width),
                    custom_width: flags & 0x02 != 0,
                    hidden: flags & 0x01 != 0,
                    best_fit: flags & 0x04 != 0,
                    outline_level: ((flags >> 8) & 0x07) as u8,
                    collapsed: flags & 0x1000 != 0,
                    style: (style != 0).then_some(style),
                    phonetic: flags & 0x08 != 0,
                });
            }
            0x00E5 => {
                let n = u16_at(&d, 0)? as usize;
                for k in 0..n {
                    let at = 2 + 8 * k;
                    let (r0, r1, c0, c1) = (u16_at(&d, at)? as u32, u16_at(&d, at + 2)? as u32, u16_at(&d, at + 4)? as u32, u16_at(&d, at + 6)? as u32);
                    if r0 <= r1 && c0 <= c1 && (r0, c0) != (r1, c1) {
                        sheet.merges.push((r0, c0, r1, c1));
                    }
                }
            }
            0x023E => frozen = u16_at(&d, 0)? & 0x0008 != 0,
            // The calculation settings (each sheet repeats those of the workbook).
            0x000D => calc.mode = Some(u16_at(&d, 0)? as i16),
            0x000C => calc.count = Some(u16_at(&d, 0)?),
            0x0011 => calc.iterate = Some(u16_at(&d, 0)? != 0),
            0x0010 => calc.delta = Some(f64_at(&d, 0)?),
            0x0041 => pane = Some((u16_at(&d, 2)? as u32, u16_at(&d, 0)? as u32)),
            _ => {}
        }
    }
    if frozen {
        if let Some((rows, cols)) = pane.filter(|&(r, c)| r > 0 || c > 0) {
            sheet.freeze = Some((rows, cols));
        }
    }
    sheet.rows.sort_by_key(|r| r.row);
    sheet.rows.dedup_by_key(|r| r.row);
    sheet.columns.sort_by_key(|c| c.first);
    let mut columns: Vec<Column> = Vec::with_capacity(sheet.columns.len());
    for c in sheet.columns.drain(..) {
        if columns.last().is_none_or(|p| p.last < c.first) {
            columns.push(c);
        }
    }
    sheet.columns = columns;
    // Formulas.
    let inside = |(r0, r1, c0, c1): (u32, u32, u32, u32), row: u32, col: u32| (r0..=r1).contains(&row) && (c0..=c1).contains(&col);
    for p in formulas {
        let text = if p.rgce.first() == Some(&0x01) && p.rgce.len() >= 5 {
            let (br, bc) = (u16_at(&p.rgce, 1)? as u32, u16_at(&p.rgce, 3)? as u32);
            if let Some((range, rgce, rgcb)) = arrays.iter().find(|a| (a.0 .0, a.0 .2) == (br, bc)) {
                if (p.row, p.col) != (br, bc) {
                    continue; // a cell of an array formula other than its first
                }
                let r = formula::decompile(rgce, rgcb, ctx, Some((br, bc)));
                if r.is_ok() {
                    let (r0, r1, c0, c1) = *range;
                    let a = format!("{}{}", formula::col_name(c0), r0 + 1);
                    let b = format!("{}{}", formula::col_name(c1), r1 + 1);
                    wb.cells.array_ref[p.cell] = Some(if a == b { a } else { format!("{a}:{b}") });
                }
                r
            } else if let Some((_, rgce, rgcb)) = shared.iter().find(|s| (s.0 .0, s.0 .2) == (br, bc)).or_else(|| shared.iter().find(|s| inside(s.0, p.row, p.col))) {
                formula::decompile(rgce, rgcb, ctx, Some((p.row, p.col)))
            } else if tables.iter().any(|t| inside(*t, p.row, p.col)) {
                Err("data table".into())
            } else {
                Err("shared formula not found".into())
            }
        } else {
            formula::decompile(&p.rgce, &p.rgcb, ctx, Some((p.row, p.col)))
        };
        match text {
            Ok(f) => wb.cells.formula[p.cell] = Some(f),
            Err(why) => notes.note(why),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rk_numbers() {
        assert_eq!(rk(0x0000_0002 | (5 << 2)), 5.0);
        assert_eq!(rk((-3i32 << 2) as u32 | 0x02), -3.0);
        assert_eq!(rk((1234 << 2) | 0x03), 12.34);
        assert_eq!(rk(0x3FF0_0000), 1.0);
    }
}
