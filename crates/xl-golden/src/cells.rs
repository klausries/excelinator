//! Differential check of the cell reader against calamine, and the semantic
//! round trip read → write → read.

use calamine::{Data, Reader, Xlsx};
use std::collections::BTreeMap;
use std::io::Cursor;
use xl_cells::{Kind, SheetKind, Value, Workbook};

#[derive(Debug, Default, Clone)]
pub struct CellStats {
    /// ok | rejected-both | rejected-ours | rejected-calamine
    pub status: String,
    pub cells: u64,
    pub value_diff: u64,
    pub only_ours: u64,
    pub only_calamine: u64,
    pub formulas: u64,
    pub formula_diff: u64,
    pub formula_only_ours: u64,
    pub formula_only_calamine: u64,
    /// read → write → read gives the same table
    pub roundtrip: Option<bool>,
    /// sheets too sparse for calamine's dense ranges
    pub skipped_sparse: u64,
    /// strings that differ from calamine only by its whitespace trimming
    pub whitespace: u64,
    pub examples: Vec<String>,
}

fn same(ours: &Value, theirs: &Data) -> bool {
    match (ours, theirs) {
        (Value::Number(a), Data::Float(b)) => a.to_bits() == b.to_bits() || a == b,
        (Value::Number(a), Data::Int(b)) => *a == *b as f64,
        (Value::Number(a), Data::DateTime(d)) => *a == d.as_f64(),
        (Value::Text(a), Data::String(b)) => a == b,
        (Value::Text(a), Data::DateTimeIso(b)) | (Value::Text(a), Data::DurationIso(b)) => a == b,
        // An ISO date cell (`t="d"`) is a serial number for us, its text for calamine.
        (Value::Number(a), Data::DateTimeIso(b)) => xl_cells::IsoDate::parse(b, false).is_some_and(|(n, _)| (n - a).abs() < 1e-9),
        (Value::Bool(a), Data::Bool(b)) => a == b,
        (Value::Error(a), Data::Error(e)) => *a == e.to_string(),
        // NaN or INF in a file is no number Excel can hold; we read it as #NUM!.
        (Value::Error(a), Data::Float(b)) => a == "#NUM!" && !b.is_finite(),
        _ => false,
    }
}

pub fn compare(bytes: &[u8]) -> CellStats {
    let mut st = CellStats::default();
    let ours = xl_cells::read_bytes(bytes.to_vec());
    let theirs: Result<Xlsx<_>, _> = Xlsx::new(Cursor::new(bytes.to_vec()));
    let (wb, mut cal) = match (ours, theirs) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(_), Err(_)) => {
            st.status = "rejected-both".into();
            return st;
        }
        (Err(e), Ok(_)) => {
            st.status = "rejected-ours".into();
            st.examples.push(e.to_string());
            return st;
        }
        (Ok(_), Err(e)) => {
            st.status = "rejected-calamine".into();
            st.examples.push(e.to_string());
            return st;
        }
    };
    // A shared string referenced by many cells: calamine copies it per cell.
    let text_bytes: u64 = wb.cells.text.iter().flatten().map(|t| t.len() as u64).sum();
    if text_bytes > 500_000_000 {
        st.status = "skipped-amplification".into();
        return st;
    }
    st.status = "ok".into();
    let note = |st: &mut CellStats, s: String| {
        if st.examples.len() < 5 {
            st.examples.push(s);
        }
    };
    for (si, sheet) in wb.sheets.iter().enumerate() {
        if sheet.kind != SheetKind::Worksheet {
            continue;
        }
        let mut values: BTreeMap<(u32, u32), Value> = BTreeMap::new();
        let mut formulas: BTreeMap<(u32, u32), &str> = BTreeMap::new();
        for i in 0..wb.cells.len() {
            if wb.cells.sheet[i] as usize != si {
                continue;
            }
            let at = (wb.cells.row[i], wb.cells.col[i]);
            if wb.cells.kind[i] != Kind::Blank {
                values.insert(at, wb.cells.value(i));
            }
            if let Some(f) = wb.cells.formula[i].as_deref() {
                formulas.insert(at, f);
            }
        }
        // calamine materialises the bounding box; skip sheets where that would not fit.
        let rows = values.keys().chain(formulas.keys()).map(|k| k.0).max().unwrap_or(0) as u64 + 1;
        let cols = values.keys().chain(formulas.keys()).map(|k| k.1).max().unwrap_or(0) as u64 + 1;
        if rows * cols > 30_000_000 {
            st.skipped_sparse += 1;
            continue;
        }
        let Ok(range) = cal.worksheet_range(&sheet.name) else {
            note(&mut st, format!("{}: calamine cannot read the sheet", sheet.name));
            continue;
        };
        let (r0, c0) = range.start().unwrap_or((0, 0));
        for (r, c, d) in range.used_cells() {
            let at = (r as u32 + r0, c as u32 + c0);
            if matches!(d, Data::Empty) {
                continue;
            }
            // calamine reports an empty cached string as a cell; we treat `<v></v>` the same way.
            match values.remove(&at) {
                Some(v) => {
                    st.cells += 1;
                    if let (Value::Text(a), Data::String(b)) = (&v, d) {
                        if a != b && a.trim() == b.as_str() {
                            // calamine trims strings without xml:space="preserve"; LibreOffice and openpyxl do not.
                            st.whitespace += 1;
                            continue;
                        }
                    }
                    if !same(&v, d) {
                        st.value_diff += 1;
                        note(&mut st, format!("{}!{:?}: ours {:?}, calamine {:?}", sheet.name, at, v, d));
                    }
                }
                None => {
                    st.only_calamine += 1;
                    note(&mut st, format!("{}!{:?}: only calamine {:?}", sheet.name, at, d));
                }
            }
        }
        for (at, v) in values {
            if matches!(&v, Value::Text(t) if t.trim().is_empty()) {
                // A string that is empty (after calamine's trimming) is no cell for calamine.
                st.whitespace += 1;
                continue;
            }
            st.only_ours += 1;
            note(&mut st, format!("{}!{:?}: only ours {:?}", sheet.name, at, v));
        }
        if let Ok(frange) = cal.worksheet_formula(&sheet.name) {
            let (r0, c0) = frange.start().unwrap_or((0, 0));
            for (r, c, f) in frange.used_cells() {
                if f.is_empty() {
                    continue;
                }
                let at = (r as u32 + r0, c as u32 + c0);
                match formulas.remove(&at) {
                    Some(o) => {
                        st.formulas += 1;
                        // calamine leaves the _xHHHH_ escapes of formulas undecoded.
                        if o != f && o != xl_cells::unescape_xstring(f) {
                            st.formula_diff += 1;
                            note(&mut st, format!("{}!{:?}: formula ours {:?}, calamine {:?}", sheet.name, at, o, f));
                        }
                    }
                    None => {
                        st.formula_only_calamine += 1;
                        note(&mut st, format!("{}!{:?}: formula only calamine {:?}", sheet.name, at, f));
                    }
                }
            }
            for (at, f) in formulas {
                st.formula_only_ours += 1;
                note(&mut st, format!("{}!{:?}: formula only ours {:?}", sheet.name, at, f));
            }
        }
    }
    // Semantic round trip over the worksheets.
    let mut plain = wb.clone();
    // Writing a new file does not carry the cached cells of other workbooks, nor the
    // parts (printer settings) that sheet elements refer to.
    plain.externals.clear();
    // The used range is computed anew when writing.
    let dims: Vec<_> = plain.sheets.iter().map(|s| s.dimension).collect();
    for s in plain.sheets.iter_mut() {
        for e in s.extra.iter_mut() {
            e.1 = xl_cells::without_rel_ids(&e.1);
        }
    }
    if plain.sheets.iter().all(|s| s.kind == SheetKind::Worksheet) {
        // Hidden-only workbooks cannot be written; make the comparison independent of that.
        if plain.sheets.iter().all(|s| s.visibility != xl_cells::Visibility::Visible) {
            if let Some(s) = plain.sheets.first_mut() {
                s.visibility = xl_cells::Visibility::Visible;
            }
        }
        st.roundtrip = Some(match xl_cells::write_bytes(&plain).and_then(xl_cells::read_bytes) {
            Ok(mut back) => {
                for (s, d) in back.sheets.iter_mut().zip(&dims) {
                    s.dimension = *d;
                }
                let ok = back == plain;
                if !ok {
                    let what = if back.sheets != plain.sheets {
                        let (a, b) = plain.sheets.iter().zip(&back.sheets).find(|(a, b)| a != b).map(|(a, b)| (a.clone(), b.clone())).unwrap_or_default();
                        let field = if a.extra != b.extra {
                            let mut x: Vec<_> = a.extra.iter().filter(|e| !b.extra.contains(e)).map(|e| e.1.chars().take(120).collect::<String>()).collect();
                            x.extend(b.extra.iter().filter(|e| !a.extra.contains(e)).map(|e| format!("-> {}", e.1.chars().take(120).collect::<String>())));
                            format!("extra {x:?}")
                        } else if a.freeze != b.freeze {
                            format!("freeze {:?} -> {:?}", a.freeze, b.freeze)
                        } else {
                            "other".into()
                        };
                        format!("sheets: {field}")
                    } else if back.styles != plain.styles {
                        match plain.styles.iter().zip(&back.styles).find(|(a, b)| a != b) {
                            Some((a, b)) => format!("styles: {} -> {}", a.to_spec().chars().take(160).collect::<String>(), b.to_spec().chars().take(160).collect::<String>()),
                            None => format!("styles: {} -> {}", plain.styles.len(), back.styles.len()),
                        }
                    } else if back.defined_names != plain.defined_names {
                        "defined names".to_string()
                    } else if back.cells.len() != plain.cells.len() {
                        format!("cell count {} -> {}", plain.cells.len(), back.cells.len())
                    } else {
                        let c = (&plain.cells, &back.cells);
                        (0..c.0.len())
                            .find(|&i| c.0.value(i) != c.1.value(i) || c.0.formula[i] != c.1.formula[i] || c.0.style[i] != c.1.style[i] || (c.0.row[i], c.0.col[i]) != (c.1.row[i], c.1.col[i]))
                            .map(|i| format!("cell ({}, {}): {:?} {:?} -> {:?} {:?}", c.0.row[i], c.0.col[i], c.0.value(i), c.0.formula[i], c.1.value(i), c.1.formula[i]))
                            .unwrap_or_else(|| "order".into())
                    };
                    note(&mut st, format!("round trip differs: {what}"));
                }
                ok
            }
            Err(e) => {
                note(&mut st, format!("round trip: {e}"));
                false
            }
        });
    }
    st
}

/// Check saving onto the original file. Unchanged: the file comes back byte for byte.
/// One cell changed and one style added: the change is there, every other cell is as
/// before, and all parts except the touched sheet, the style sheet, the calculation
/// chain and the package bookkeeping are byte-identical.
pub fn update_check(bytes: &[u8]) -> Result<&'static str, String> {
    let Ok(wb) = xl_cells::read_bytes(bytes.to_vec()) else { return Ok("unreadable") };
    let same = xl_cells::update_bytes(bytes.to_vec(), &wb).map_err(|e| format!("unchanged: {e}"))?;
    let Ok(original) = xl_opc::Package::open(bytes.to_vec()) else { return Ok("unreadable") };
    let only_worksheets = wb.sheets.iter().all(|s| s.kind == SheetKind::Worksheet);
    if same != original.to_bytes().map_err(|e| e.to_string())? {
        // Only allowed when the layout is not patchable and a fresh file was written.
        if !only_worksheets {
            return Err("unchanged workbook with chart sheets was not kept".into());
        }
        let back = xl_cells::read_bytes(same).map_err(|e| format!("fresh fallback unreadable: {e}"))?;
        if back.cells != wb.cells {
            return Err("fresh fallback changed cells".into());
        }
        return Ok("fallback");
    }
    let Some(target) = wb.sheets.iter().position(|s| s.kind == SheetKind::Worksheet) else { return Ok("no worksheet") };
    let mut changed = wb.clone();
    let style = changed.intern_style(&xl_cells::Style { bold: true, fill: Some("FFFFFF00".into()), num_format: "0.000\" changed\"".into(), ..Default::default() });
    changed.cells.push(target as u32, 0, 0, Value::Text(" changed <&> ".into()), None, style);
    let (out, kept) = match xl_cells::update_bytes_report(bytes.to_vec(), &changed) {
        Ok(r) => r,
        // The sheet already held a text beyond Excel's limit; writing it is refused.
        Err(e) if e.to_string().contains("longer than") => return Ok("refused"),
        Err(e) => return Err(format!("changed: {e}")),
    };
    if !kept {
        return Ok("fallback after change");
    }
    let back = xl_cells::read_bytes(out.clone()).map_err(|e| format!("result unreadable: {e}"))?;
    if back.styles.get(style as usize) != changed.styles.get(style as usize) || back.styles[..wb.styles.len()] != wb.styles[..] {
        return Err("styles differ after update".into());
    }
    let mut expect = Workbook { cells: Default::default(), ..changed.clone() };
    for i in changed.cells.sorted_unique() {
        let c = &changed.cells;
        expect.cells.push_raw(c.sheet[i], c.row[i], c.col[i], c.kind[i], c.num[i], c.text[i].clone(), c.formula[i].clone(), c.style[i]);
        *expect.cells.float_text.last_mut().unwrap() = c.float_text[i];
        *expect.cells.array_ref.last_mut().unwrap() = c.array_ref[i].clone();
        *expect.cells.rich.last_mut().unwrap() = c.rich[i].clone();
        *expect.cells.iso.last_mut().unwrap() = c.iso[i];
    }
    let mut got = Workbook { cells: Default::default(), ..back.clone() };
    for i in back.cells.sorted_unique() {
        let c = &back.cells;
        got.cells.push_raw(c.sheet[i], c.row[i], c.col[i], c.kind[i], c.num[i], c.text[i].clone(), c.formula[i].clone(), c.style[i]);
        *got.cells.float_text.last_mut().unwrap() = c.float_text[i];
        *got.cells.array_ref.last_mut().unwrap() = c.array_ref[i].clone();
        *got.cells.rich.last_mut().unwrap() = c.rich[i].clone();
        *got.cells.iso.last_mut().unwrap() = c.iso[i];
    }
    if got.cells != expect.cells {
        let n = (0..got.cells.len().min(expect.cells.len())).find(|&i| got.cells.value(i) != expect.cells.value(i) || got.cells.row[i] != expect.cells.row[i] || got.cells.formula[i] != expect.cells.formula[i] || got.cells.style[i] != expect.cells.style[i]);
        return Err(format!("cells differ after update ({} vs {}), first at {:?}", got.cells.len(), expect.cells.len(), n.map(|i| (expect.cells.sheet[i], expect.cells.row[i], expect.cells.col[i], expect.cells.value(i), got.cells.value(i)))));
    }
    // The used range (`dimension`) follows the changed cells.
    let no_dim = |s: &[xl_cells::Sheet]| s.iter().map(|s| xl_cells::Sheet { dimension: None, ..s.clone() }).collect::<Vec<_>>();
    if no_dim(&back.sheets) != no_dim(&changed.sheets) {
        return Err("sheet layout differs after update".into());
    }
    let result = xl_opc::Package::open(out).map_err(|e| e.to_string())?;
    let mut touched = 0;
    for name in original.part_names() {
        if !result.contains(name) {
            if !name.to_lowercase().contains("calcchain") {
                return Err(format!("part {name} was lost"));
            }
            continue;
        }
        if original.read(name).ok() != result.read(name).ok() {
            touched += 1;
        }
    }
    // The sheet, the style sheet, content types and workbook relationships at most.
    if touched > 4 {
        return Err(format!("{touched} parts changed"));
    }
    Ok("patched")
}

/// Check structural changes when saving onto the original: the first sheet is
/// renamed, a sheet is added, and (if there are several) the last sheet is removed.
/// The remaining sheets must keep their cells and their parts byte for byte.
pub fn structure_check(bytes: &[u8]) -> Result<&'static str, String> {
    let Ok(wb) = xl_cells::read_bytes(bytes.to_vec()) else { return Ok("unreadable") };
    let Ok(original) = xl_opc::Package::open(bytes.to_vec()) else { return Ok("unreadable") };
    if wb.sheets.iter().any(|s| s.name == "Umbenannt Ä" || s.name == "Neu <&>") {
        return Ok("name clash");
    }
    let mut changed = wb.clone();
    let removed = if changed.sheets.len() > 1 { Some(changed.sheets.len() as u32 - 1) } else { None };
    if let Some(r) = removed {
        changed.sheets.pop();
        let keep: Vec<usize> = (0..changed.cells.len()).filter(|&i| changed.cells.sheet[i] != r).collect();
        let c = &wb.cells;
        let mut cells = xl_cells::Cells::default();
        for i in keep {
            cells.push_raw(c.sheet[i], c.row[i], c.col[i], c.kind[i], c.num[i], c.text[i].clone(), c.formula[i].clone(), c.style[i]);
            *cells.float_text.last_mut().unwrap() = c.float_text[i];
            *cells.array_ref.last_mut().unwrap() = c.array_ref[i].clone();
            *cells.rich.last_mut().unwrap() = c.rich[i].clone();
            *cells.iso.last_mut().unwrap() = c.iso[i];
        }
        changed.cells = cells;
    }
    changed.sheets[0].name = "Umbenannt Ä".into();
    let added = changed.sheets.len() as u32;
    changed.sheets.push(xl_cells::Sheet { name: "Neu <&>".into(), ..Default::default() });
    changed.cells.push(added, 2, 1, Value::Text("neu".into()), Some("1+1".into()), 0);
    let (out, kept) = match xl_cells::update_bytes_report(bytes.to_vec(), &changed) {
        Ok(r) => r,
        Err(e) if e.to_string().contains("longer than") || e.to_string().contains("visible") || e.to_string().contains("not allowed") || e.to_string().contains("only worksheets") => return Ok("refused"),
        Err(e) => return Err(format!("changed: {e}")),
    };
    if !kept {
        return Ok("fallback");
    }
    let back = xl_cells::read_bytes(out.clone()).map_err(|e| format!("result unreadable: {e}"))?;
    let names: Vec<&str> = back.sheets.iter().map(|s| s.name.as_str()).collect();
    let want: Vec<&str> = changed.sheets.iter().map(|s| s.name.as_str()).collect();
    if names != want {
        return Err(format!("sheets {names:?} instead of {want:?}"));
    }
    if back.defined_names.iter().any(|n| n.local_sheet.is_some_and(|l| l as usize >= back.sheets.len())) {
        return Err("a defined name points past the sheet list".into());
    }
    // Texts are shared allocations; cloning the handle does not copy them.
    let cells = |w: &Workbook| -> Vec<(u32, u32, u32, Kind, u64, Option<std::sync::Arc<str>>, Option<String>, u32)> {
        w.cells.sorted_unique().into_iter().map(|i| (w.cells.sheet[i], w.cells.row[i], w.cells.col[i], w.cells.kind[i], w.cells.num[i].to_bits(), w.cells.text[i].clone(), w.cells.formula[i].clone(), w.cells.style[i])).collect()
    };
    if cells(&back) != cells(&changed) {
        return Err("cells differ after structural change".into());
    }
    let result = xl_opc::Package::open(out).map_err(|e| e.to_string())?;
    let mut lost = 0;
    let mut touched = 0;
    for name in original.part_names() {
        if !result.contains(name) {
            lost += 1;
        } else if original.read(name).ok() != result.read(name).ok() {
            touched += 1;
        }
    }
    // Gone: at most the removed sheet, its relationships and the calculation chain.
    // Touched: workbook, its relationships, content types.
    if lost > 3 || touched > 3 {
        return Err(format!("{lost} parts lost, {touched} changed"));
    }
    Ok("patched")
}
