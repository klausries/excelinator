//! Formulas of BIFF8 (MS-XLS 2.2.2, 2.5.198): parsed expressions (`Rgce`, tokens in
//! reverse Polish order) turned back into formula text as .xlsx files spell it, without
//! the leading `=`.

use crate::ftab::FUNCTIONS;

/// A workbook a formula can refer to (`SupBook`).
#[derive(Debug, Clone)]
pub enum SupBook {
    /// This workbook.
    Internal,
    /// Add-in functions, by the names of their `ExternName` records.
    AddIn(Vec<String>),
    /// Another workbook: its number among the external workbooks (1-based, as formulas
    /// write it in `[1]Sheet1!A1`), its sheet names and its names.
    External { number: usize, sheets: Vec<String>, names: Vec<String> },
    /// A DDE or OLE link; formulas referring to it are not read.
    Other(Vec<String>),
}

/// What a formula needs from the workbook globals.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    /// Names of all sheets in the order of the file (`BoundSheet8`).
    pub sheets: &'a [String],
    /// `XTI` entries of `ExternSheet`: supporting workbook, first and last sheet.
    pub xti: &'a [(u16, i16, i16)],
    pub supbooks: &'a [SupBook],
    /// Defined names in the order of their `Lbl` records, with the sheet (index into
    /// `sheets`) a local name belongs to.
    pub names: &'a [String],
    pub name_sheets: &'a [Option<usize>],
    /// The sheet the formula is on (index into `sheets`): a name local to another sheet
    /// is written with that sheet (`Sheet2!Rate`).
    pub sheet: Option<usize>,
}

const ROWS: u32 = 65536;
const COLS: u32 = 256;

pub fn col_name(mut col: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// A sheet name as a formula writes it: quoted when it is not a plain word.
pub fn sheet_ref(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && !looks_like_cell(name);
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Whether a name could be read as a cell reference (`A1`, `XFD1048576`, `R1C1`, `R`, `C`).
fn looks_like_cell(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    let letters = u.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    let digits = &u[letters..];
    if (1..=3).contains(&letters) && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if u == "R" || u == "C" {
        return true;
    }
    // R1C1 forms such as R2C3, R, RC.
    let mut rest = u.as_str();
    if let Some(r) = rest.strip_prefix('R') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
        if let Some(c) = rest.strip_prefix('C') {
            return c.chars().all(|c| c.is_ascii_digit());
        }
        return rest.is_empty();
    }
    false
}

/// Reads one part of a reference: row and column with their relative flags.
#[derive(Clone, Copy)]
struct Loc {
    row: u32,
    col: u32,
    row_rel: bool,
    col_rel: bool,
}

impl Loc {
    fn new(rw: u16, col: u16) -> Loc {
        Loc { row: rw as u32, col: (col & 0x3FFF) as u32, row_rel: col & 0x8000 != 0, col_rel: col & 0x4000 != 0 }
    }

    /// For `PtgRefN` and `PtgAreaN`: relative parts are offsets from the base cell.
    fn relative_to(rw: u16, col: u16, base: (u32, u32)) -> Loc {
        let mut l = Loc::new(rw, col);
        if l.row_rel {
            l.row = (base.0 as i64 + rw as i16 as i64).rem_euclid(ROWS as i64) as u32;
        }
        if l.col_rel {
            l.col = (base.1 as i64 + (col & 0xFF) as u8 as i8 as i64).rem_euclid(COLS as i64) as u32;
        }
        l
    }

    fn text(&self) -> String {
        format!("{}{}{}{}", if self.col_rel { "" } else { "$" }, col_name(self.col), if self.row_rel { "" } else { "$" }, self.row + 1)
    }
}

fn area_text(a: Loc, b: Loc) -> String {
    // Whole rows and whole columns as Excel shows them (1:1, A:A; the whole sheet as
    // rows, 1:65536). In an .xlsx they keep meaning whole rows and columns.
    if a.col == 0 && b.col == COLS - 1 && a.col_rel == b.col_rel {
        let r = |l: Loc| format!("{}{}", if l.row_rel { "" } else { "$" }, l.row + 1);
        return format!("{}:{}", r(a), r(b));
    }
    if a.row == 0 && b.row == ROWS - 1 && a.row_rel == b.row_rel {
        let c = |l: Loc| format!("{}{}", if l.col_rel { "" } else { "$" }, col_name(l.col));
        return format!("{}:{}", c(a), c(b));
    }
    format!("{}:{}", a.text(), b.text())
}

fn error_text(code: u8) -> Option<&'static str> {
    Some(match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        0x2B => "#GETTING_DATA",
        _ => return None,
    })
}

pub fn error_code(code: u8) -> Option<&'static str> {
    error_text(code)
}

fn number_text(v: f64) -> String {
    let plain = format!("{v}");
    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else if v.is_finite() && plain.len() <= 20 {
        plain
    } else {
        // Shortest form that reads back to the same number, in Excel's exponent style.
        let s = format!("{v:?}");
        match s.split_once('e') {
            Some((m, e)) => {
                let e: i32 = e.parse().unwrap_or(0);
                format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
            }
            None => s,
        }
    }
}

struct Bytes<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or("formula cut off")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64(&mut self) -> Result<f64, String> {
        let s = self.take(8)?;
        Ok(f64::from_le_bytes(s.try_into().unwrap()))
    }
    /// A string with its length already read: flags byte, then 8- or 16-bit characters.
    fn chars(&mut self, cch: usize) -> Result<String, String> {
        let flags = self.u8()?;
        if flags & 0x01 != 0 {
            let s = self.take(cch * 2)?;
            Ok(String::from_utf16_lossy(&s.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect::<Vec<_>>()))
        } else {
            Ok(self.take(cch)?.iter().map(|&b| b as char).collect())
        }
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

impl Ctx<'_> {
    /// The sheet part of a 3D reference (`Sheet1!`, `'A b'!`, `S1:S3!`).
    fn sheet_prefix(&self, ixti: u16) -> Result<Option<String>, String> {
        let &(book, first, last) = self.xti.get(ixti as usize).ok_or("reference to an unknown sheet")?;
        let (sheets, book_prefix) = match self.supbooks.get(book as usize) {
            Some(SupBook::Internal) => (self.sheets, String::new()),
            Some(SupBook::External { number, sheets, .. }) => (sheets.as_slice(), format!("[{number}]")),
            _ => return Err("reference through a DDE or OLE link".into()),
        };
        if first < 0 || last < 0 {
            return Ok(None); // a deleted sheet: #REF!
        }
        let name = |i: i16| sheets.get(i as usize).cloned().ok_or_else(|| "reference to an unknown sheet".to_string());
        let (a, b) = (name(first)?, name(last)?);
        let plain = sheet_ref(&a) == a && sheet_ref(&b) == b;
        let joined = if first == last { format!("{book_prefix}{a}") } else { format!("{book_prefix}{a}:{b}") };
        // Quoted as a whole (book included) when a sheet name needs quoting.
        if plain {
            Ok(Some(format!("{joined}!")))
        } else {
            Ok(Some(format!("'{}'!", joined.replace('\'', "''"))))
        }
    }

    /// A defined name by its index; one local to another sheet with that sheet.
    fn name(&self, i: usize) -> Result<String, String> {
        let name = self.names.get(i).cloned().ok_or("unknown name")?;
        match self.name_sheets.get(i).copied().flatten() {
            Some(local) if Some(local) != self.sheet => {
                let sheet = self.sheets.get(local).ok_or("name of an unknown sheet")?;
                Ok(format!("{}!{name}", sheet_ref(sheet)))
            }
            _ => Ok(name),
        }
    }

    fn extern_name(&self, ixti: u16, iname: u16) -> Result<String, String> {
        let &(book, _, _) = self.xti.get(ixti as usize).ok_or("reference to an unknown sheet")?;
        match self.supbooks.get(book as usize) {
            Some(SupBook::AddIn(names)) => names.get((iname as usize).wrapping_sub(1)).cloned().ok_or_else(|| "unknown add-in function".into()),
            Some(SupBook::Internal) => self.name((iname as usize).wrapping_sub(1)),
            Some(SupBook::External { number, names, .. }) => {
                names.get((iname as usize).wrapping_sub(1)).map(|n| format!("[{number}]!{n}")).ok_or_else(|| "unknown name".into())
            }
            _ => Err("name through a DDE or OLE link".into()),
        }
    }
}

/// Formula text of a parsed expression. `rgcb` holds the extra data of array constants
/// and memory areas; `base` is the cell a shared formula is used in.
pub fn decompile(rgce: &[u8], rgcb: &[u8], ctx: &Ctx, base: Option<(u32, u32)>) -> Result<String, String> {
    let mut b = Bytes { b: rgce, at: 0 };
    let mut extra = Bytes { b: rgcb, at: 0 };
    let mut stack: Vec<String> = Vec::new();
    let pop = |stack: &mut Vec<String>| stack.pop().ok_or_else(|| "formula stack underflow".to_string());
    while b.at < rgce.len() {
        let ptg = b.u8()?;
        let base_ptg = if ptg >= 0x20 { (ptg & 0x1F) | 0x20 } else { ptg };
        match ptg {
            0x01 => return Err("shared or array formula reference".into()),
            0x02 => return Err("data table".into()),
            0x03..=0x11 => {
                let r = pop(&mut stack)?;
                let l = pop(&mut stack)?;
                let op = ["+", "-", "*", "/", "^", "&", "<", "<=", "=", ">=", ">", "<>", " ", ",", ":"][(ptg - 0x03) as usize];
                stack.push(format!("{l}{op}{r}"));
            }
            0x12 => {
                let x = pop(&mut stack)?;
                stack.push(format!("+{x}"));
            }
            0x13 => {
                let x = pop(&mut stack)?;
                stack.push(format!("-{x}"));
            }
            0x14 => {
                let x = pop(&mut stack)?;
                stack.push(format!("{x}%"));
            }
            0x15 => {
                let x = pop(&mut stack)?;
                stack.push(format!("({x})"));
            }
            0x16 => stack.push(String::new()),
            0x17 => {
                let cch = b.u8()? as usize;
                let s = b.chars(cch)?;
                stack.push(quote(&s));
            }
            0x18 => return Err("extended token".into()),
            0x19 => {
                let grbit = b.u8()?;
                let data = b.u16()?;
                if grbit & 0x04 != 0 {
                    // tAttrChoose: a jump table follows.
                    b.take(2 * (data as usize + 1))?;
                }
                if grbit & 0x10 != 0 {
                    // tAttrSum: SUM of the one argument on the stack.
                    let x = pop(&mut stack)?;
                    stack.push(format!("SUM({x})"));
                }
                // Volatile, if, goto, spaces: nothing to show.
            }
            0x1C => {
                let e = b.u8()?;
                stack.push(error_text(e).ok_or("unknown error code")?.to_string());
            }
            0x1D => {
                let v = b.u8()?;
                stack.push(if v != 0 { "TRUE" } else { "FALSE" }.to_string());
            }
            0x1E => {
                let v = b.u16()?;
                stack.push(v.to_string());
            }
            0x1F => {
                let v = b.f64()?;
                stack.push(number_text(v));
            }
            _ => match base_ptg {
                0x20 => {
                    b.take(7)?;
                    stack.push(array_constant(&mut extra)?);
                }
                0x21 => {
                    let iftab = b.u16()? as usize;
                    let &(name, argc) = FUNCTIONS.get(iftab).ok_or("unknown function")?;
                    if argc >= 128 {
                        return Err(format!("function {name} without its argument count"));
                    }
                    call(&mut stack, name, argc as usize)?;
                }
                0x22 => {
                    let argc = (b.u8()? & 0x7F) as usize;
                    let tab = b.u16()?;
                    if tab & 0x8000 != 0 {
                        return Err("command-equivalent function".into());
                    }
                    let iftab = (tab & 0x7FFF) as usize;
                    if iftab == 255 {
                        // A user-defined or add-in function: its name is the first argument.
                        if argc == 0 || stack.len() < argc {
                            return Err("formula stack underflow".into());
                        }
                        let args = stack.split_off(stack.len() - (argc - 1));
                        let name = pop(&mut stack)?;
                        stack.push(format!("{name}({})", args.join(",")));
                    } else {
                        let &(name, _) = FUNCTIONS.get(iftab).ok_or("unknown function")?;
                        call(&mut stack, name, argc)?;
                    }
                }
                0x23 => {
                    let i = (b.u32()? as usize).wrapping_sub(1);
                    stack.push(ctx.name(i)?);
                }
                0x24 => {
                    let (rw, col) = (b.u16()?, b.u16()?);
                    stack.push(Loc::new(rw, col).text());
                }
                0x25 => {
                    let (r1, r2, c1, c2) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
                    stack.push(area_text(Loc::new(r1, c1), Loc::new(r2, c2)));
                }
                0x26 => {
                    b.take(6)?;
                    let n = extra.u16()? as usize;
                    extra.take(8 * n)?;
                }
                0x27 | 0x28 => {
                    b.take(6)?;
                }
                0x29 | 0x2E | 0x2F => {
                    b.take(2)?;
                }
                0x2A => {
                    b.take(4)?;
                    stack.push("#REF!".into());
                }
                0x2B => {
                    b.take(8)?;
                    stack.push("#REF!".into());
                }
                0x2C => {
                    let (rw, col) = (b.u16()?, b.u16()?);
                    let base = base.ok_or("relative reference outside a shared formula")?;
                    stack.push(Loc::relative_to(rw, col, base).text());
                }
                0x2D => {
                    let (r1, r2, c1, c2) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
                    let base = base.ok_or("relative reference outside a shared formula")?;
                    stack.push(area_text(Loc::relative_to(r1, c1, base), Loc::relative_to(r2, c2, base)));
                }
                0x39 => {
                    let (ixti, iname) = (b.u16()?, b.u16()?);
                    b.take(2)?;
                    stack.push(ctx.extern_name(ixti, iname)?);
                }
                0x3A => {
                    let (ixti, rw, col) = (b.u16()?, b.u16()?, b.u16()?);
                    let sheet = ctx.sheet_prefix(ixti)?;
                    stack.push(match sheet {
                        Some(s) => format!("{s}{}", Loc::new(rw, col).text()),
                        None => "#REF!".into(),
                    });
                }
                0x3B => {
                    let ixti = b.u16()?;
                    let (r1, r2, c1, c2) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
                    let sheet = ctx.sheet_prefix(ixti)?;
                    stack.push(match sheet {
                        Some(s) => format!("{s}{}", area_text(Loc::new(r1, c1), Loc::new(r2, c2))),
                        None => "#REF!".into(),
                    });
                }
                0x3C => {
                    let ixti = b.u16()?;
                    b.take(4)?;
                    stack.push(match ctx.sheet_prefix(ixti)? {
                        Some(s) => format!("{s}#REF!"),
                        None => "#REF!".into(),
                    });
                }
                0x3D => {
                    let ixti = b.u16()?;
                    b.take(8)?;
                    stack.push(match ctx.sheet_prefix(ixti)? {
                        Some(s) => format!("{s}#REF!"),
                        None => "#REF!".into(),
                    });
                }
                _ => return Err(format!("unknown token 0x{ptg:02X}")),
            },
        }
    }
    if stack.len() != 1 {
        return Err(format!("formula leaves {} values", stack.len()));
    }
    Ok(stack.pop().unwrap())
}

fn call(stack: &mut Vec<String>, name: &str, argc: usize) -> Result<(), String> {
    if stack.len() < argc {
        return Err("formula stack underflow".into());
    }
    let args = stack.split_off(stack.len() - argc);
    stack.push(format!("{name}({})", args.join(",")));
    Ok(())
}

/// An array constant (`{1,2;3,4}`) from the extra data (`PtgExtraArray`).
fn array_constant(extra: &mut Bytes) -> Result<String, String> {
    let cols = extra.u8()? as usize + 1;
    let rows = extra.u16()? as usize + 1;
    let mut out = String::from("{");
    for r in 0..rows {
        if r > 0 {
            out.push(';');
        }
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            match extra.u8()? {
                0x00 => {
                    extra.take(8)?;
                }
                0x01 => out.push_str(&number_text(extra.f64()?)),
                0x02 => {
                    let cch = extra.u16()? as usize;
                    out.push_str(&quote(&extra.chars(cch)?));
                }
                0x04 => {
                    let v = extra.u8()?;
                    extra.take(7)?;
                    out.push_str(if v != 0 { "TRUE" } else { "FALSE" });
                }
                0x10 => {
                    let e = extra.u8()?;
                    extra.take(7)?;
                    out.push_str(error_text(e).ok_or("unknown error code")?);
                }
                t => return Err(format!("unknown array constant type {t}")),
            }
        }
    }
    out.push('}');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_of_names_of_other_workbooks() {
        let sheets = ["Curves".to_string(), "My Data".to_string()];
        // Area3d over the first sheet, rows 8 to 370, columns C to I (all fixed).
        let f = [0x3B, 0, 0, 0, 0, 7, 0, 0x71, 1, 2, 0, 8, 0];
        assert_eq!(extern_definition(&f, &sheets).unwrap(), "Curves!$C$8:$I$370");
        let f = [0x3A, 1, 0, 1, 0, 0, 0, 0, 0];
        assert_eq!(extern_definition(&f, &sheets).unwrap(), "'My Data'!$A$1");
        assert_eq!(extern_definition(&[0x1C, 0x17], &sheets).unwrap(), "#REF!");
    }

    #[test]
    fn names_and_references() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(255), "IV");
        assert_eq!(sheet_ref("Sheet1"), "Sheet1");
        assert_eq!(sheet_ref("My Sheet"), "'My Sheet'");
        assert_eq!(sheet_ref("A1"), "'A1'");
        assert_eq!(sheet_ref("R2C3"), "'R2C3'");
        assert_eq!(sheet_ref("2006"), "'2006'");
        assert_eq!(sheet_ref("O'Neil"), "'O''Neil'");
        assert_eq!(number_text(0.5), "0.5");
        assert_eq!(number_text(1e20), "1E+20");
        assert_eq!(number_text(1.5e-7), "0.00000015");
        assert_eq!(number_text(1.5e-27), "1.5E-27");
    }

    #[test]
    fn expressions() {
        let ctx = Ctx { sheets: &["Data".into(), "My Sheet".into()], xti: &[(0, 1, 1)], supbooks: &[SupBook::Internal], names: &["Rate".into()], name_sheets: &[None], sheet: Some(0) };
        // SUM(A1:B2)*2+'My Sheet'!$C$3-Rate
        let mut f = vec![0x25, 0, 0, 1, 0, 0, 0xC0, 1, 0xC0];
        f.extend([0x19, 0x10, 0, 0]);
        f.extend([0x1E, 2, 0, 0x05]);
        f.extend([0x3A, 0, 0, 2, 0, 2, 0]);
        f.push(0x03);
        f.extend([0x23, 1, 0, 0, 0]);
        f.push(0x04);
        assert_eq!(decompile(&f, &[], &ctx, None).unwrap(), "SUM(A1:B2)*2+'My Sheet'!$C$3-Rate");
        // IF(A1>0,"yes",{1,2;3,"x"}) with a constant array in the extra data.
        let mut f = vec![0x24, 0, 0, 0, 0xC0, 0x1E, 0, 0, 0x0D, 0x17, 3, 0, b'y', b'e', b's', 0x60, 0, 0, 0, 0, 0, 0, 0];
        f.extend([0x42, 3, 1, 0]);
        let mut extra = vec![1, 1, 0];
        for v in [1.0f64, 2.0, 3.0] {
            extra.push(1);
            extra.extend(v.to_le_bytes());
        }
        extra.extend([2, 1, 0, 0, b'x']);
        assert_eq!(decompile(&f, &extra, &ctx, None).unwrap(), "IF(A1>0,\"yes\",{1,2;3,\"x\"})");
        // Shared formula: A(-1 row) relative, whole column B.
        let f = vec![0x2C, 0xFF, 0xFF, 0x00, 0xC0, 0x2D, 0, 0, 0xFF, 0xFF, 0xFF, 0x40, 0xFF, 0x40, 0x03];
        assert_eq!(decompile(&f, &[], &ctx, Some((4, 2))).unwrap(), "C4+B:B");
        // A name local to another sheet carries that sheet.
        let local = Ctx { name_sheets: &[Some(1)], ..ctx };
        assert_eq!(decompile(&[0x23, 1, 0, 0, 0], &[], &local, None).unwrap(), "'My Sheet'!Rate");
        assert_eq!(decompile(&[0x23, 1, 0, 0, 0], &[], &Ctx { sheet: Some(1), ..local }, None).unwrap(), "Rate");
    }
}

/// The definition of a name of another workbook (`ExternName`): its references carry the
/// sheets of that workbook (first and last) instead of an `XTI`. Written as the external
/// workbook's own link part writes it (`Curves!$C$8:$I$8`). Only a single reference or
/// an error constant is read.
pub fn extern_definition(rgce: &[u8], sheets: &[String]) -> Result<String, String> {
    let mut b = Bytes { b: rgce, at: 0 };
    let ptg = b.u8()?;
    let sheet = |first: u16, last: u16| -> Result<String, String> {
        let name = |i: u16| sheets.get(i as usize).cloned().ok_or_else(|| "reference to an unknown sheet".to_string());
        let (a, z) = (name(first)?, name(last)?);
        Ok(if first == last { sheet_ref(&a) } else if sheet_ref(&a) == a && sheet_ref(&z) == z { format!("{a}:{z}") } else { format!("'{}'", format!("{a}:{z}").replace('\'', "''")) })
    };
    let out = match ptg {
        0x1C => error_text(b.u8()?).ok_or("unknown error code")?.to_string(),
        0x3A | 0x5A | 0x7A => {
            let (first, last, rw, col) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
            format!("{}!{}", sheet(first, last)?, Loc::new(rw, col).text())
        }
        0x3B | 0x5B | 0x7B => {
            let (first, last) = (b.u16()?, b.u16()?);
            let (r1, r2, c1, c2) = (b.u16()?, b.u16()?, b.u16()?, b.u16()?);
            format!("{}!{}", sheet(first, last)?, area_text(Loc::new(r1, c1), Loc::new(r2, c2)))
        }
        _ => return Err(format!("definition with token 0x{ptg:02X}")),
    };
    if b.at != rgce.len() {
        return Err("definition with more than one reference".into());
    }
    Ok(out)
}
