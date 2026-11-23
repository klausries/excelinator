//! Reading and writing workbooks: what `xlcells._xlcells` (bindings/python) gives, for
//! the module.

use crate::rpc::{In, Out};
use serde_json::{json, Value as J};
use xl_compat::{Font, Formula, Label, Value, Workbook, XFStyle};

/// A workbook as equally long columns, one entry per cell (as `_xlcells.read`).
pub fn read(data: &[u8], out: &mut Out) -> Result<J, String> {
    let t = xl_compat::read_table(data.to_vec()).map_err(|e| e.to_string())?;
    let n = t.len();
    let text: Vec<Option<&str>> = (0..n).map(|i| matches!(t.kind[i], 2 | 4).then(|| t.text(i))).collect();
    let formula: Vec<Option<&str>> = (0..n).map(|i| Some(t.formula(i)).filter(|f| !f.is_empty())).collect();
    let flags = |v: &[u8]| v.iter().map(|&b| b != 0).collect::<Vec<_>>();
    let drawings: Vec<J> = t
        .drawings
        .iter()
        .map(|(sheet, dr)| {
            let image = dr.image.as_ref().map(|(data, ext)| json!([ext, out.bytes(data.to_vec())]));
            json!([sheet, dr.anchor.from, dr.anchor.to, dr.anchor.ext, image, dr.chart.as_deref()])
        })
        .collect();
    Ok(json!({
        "sheet_names": t.sheet_names, "date1904": t.date1904, "sheet": t.sheet, "row": t.row, "col": t.col,
        "kind": t.kind, "num": t.num, "is_date": flags(&t.is_date), "float_text": flags(&t.float_text),
        "text": text, "formula": formula, "style": t.style, "num_format": t.num_format,
        "style_bold": flags(&t.bold), "style_italic": flags(&t.italic), "rich": t.rich, "iso": t.iso,
        "array_ref": t.array_ref.iter().map(|a| Some(a.as_str()).filter(|a| !a.is_empty())).collect::<Vec<_>>(),
        "defined_names": t.defined_names, "hyperlinks": t.hyperlinks, "auto_filter": t.auto_filter,
        "validations": t.validations, "cond_formats": t.cond_formats, "comments": t.comments, "tables": t.tables,
        "dimensions": t.dimensions, "workbook_extra": t.workbook_extra, "properties": t.properties,
        "drawings": drawings, "sheet_kind": t.sheet_kind, "style_spec": t.style_spec, "named_styles": t.named_styles,
        "rows": t.rows.iter().map(|(s, r)| json!([s, r.row, r.height, r.custom_height, r.hidden, r.outline_level, r.collapsed, r.style, r.custom_format, r.thick_top, r.thick_bot, r.phonetic])).collect::<Vec<_>>(),
        "freeze": t.freeze, "extra": t.extra, "merges": t.merges,
        "columns": t.columns.iter().map(|(s, c)| json!([s, c.first, c.last, c.width, c.custom_width, c.hidden, c.best_fit, c.outline_level, c.collapsed, c.style, c.phonetic])).collect::<Vec<_>>(),
    }))
}

pub fn date_serial(a: &In) -> Result<J, String> {
    let n = |i: usize| a.arg(i).as_u64().unwrap_or(0) as u32;
    Ok(json!(xl_compat::date_serial(a.arg(0).as_i64().ok_or("year must be a number")? as i32, n(1), n(2), n(3), n(4), n(5), n(6))))
}

pub fn translate_formula(a: &In) -> Result<J, String> {
    let shifted = xl_formula::parse(a.str(0)?).ok().map(|e| e.shifted(a.arg(1).as_i64().unwrap_or(0), a.arg(2).as_i64().unwrap_or(0)).to_string());
    Ok(json!(shifted))
}

fn strings(v: &J) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

fn pairs(v: &J) -> Vec<(String, String)> {
    v.as_array().into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect()
}

fn anchor(v: &J) -> Option<(u32, i64, u32, i64)> {
    Some((v.get(0)?.as_u64()? as u32, v.get(1)?.as_i64()?, v.get(2)?.as_u64()? as u32, v.get(3)?.as_i64()?))
}

/// One call of a writer's method, its arguments by position as `_xlcells.Writer` takes
/// them (missing or null ones as there); what it returns.
pub fn writer_call(wb: &mut Workbook, method: &str, a: &In) -> Result<J, String> {
    let opt_u32 = |i: usize| a.arg(i).as_u64().map(|v| v as u32);
    let opt_f64 = |i: usize| a.arg(i).as_f64();
    let sheet = || a.usize(0);
    Ok(match method {
        "add_sheet" => json!(wb.add_sheet(a.str(0)?)),
        "add_style" => json!(wb.add_style(&XFStyle { num_format_str: a.opt_str(0).unwrap_or_else(|| "General".into()), font: Font { bold: a.flag(1, false), italic: a.flag(2, false) } })),
        "write" => {
            let s = sheet()?;
            if s >= wb.sheet_count() {
                return Err(format!("no sheet {s}"));
            }
            let (num, text) = (opt_f64(4).unwrap_or(0.0), a.opt_str(5));
            let value = match a.u64(3)? {
                0 => Value::Blank,
                1 => Value::Number(num),
                2 => Value::Text(text.unwrap_or_default()),
                3 => Value::Bool(num != 0.0),
                4 => Value::Error(text.unwrap_or_default()),
                k => return Err(format!("unknown cell kind {k}")),
            };
            let label = match (a.opt_str(6), value) {
                (Some(f), v) => Label::FormulaWithValue(Formula::new(&f), v),
                (None, Value::Blank) => Label::Blank,
                (None, Value::Number(n)) => Label::Number(n),
                (None, Value::Text(s)) => Label::Text(s),
                (None, Value::Bool(b)) => Label::Bool(b),
                (None, Value::Error(e)) => Label::Error(e),
            };
            wb.write_styled(s, a.u32(1)?, a.u32(2)?, label, opt_u32(7).unwrap_or(0));
            let (array_ref, float_text) = (a.opt_str(8), a.flag(9, false));
            if array_ref.is_some() || float_text {
                wb.mark_last(array_ref.as_deref(), float_text);
            }
            if let Some(r) = a.opt_str(10) {
                wb.mark_last_rich(&r).map_err(|e| e.to_string())?;
            }
            let iso = a.arg(11).as_u64().unwrap_or(0) as u8;
            if iso != 0 {
                wb.mark_last_iso(iso).map_err(|e| e.to_string())?;
            }
            J::Null
        }
        "merge" => {
            wb.merge(sheet()?, a.u32(1)?, a.u32(2)?, a.u32(3)?, a.u32(4)?);
            J::Null
        }
        "set_col_width" => {
            wb.set_col_width(sheet()?, a.u32(1)?, a.u32(2)?);
            J::Null
        }
        "set_col_width_chars" => {
            wb.set_col_width_chars(sheet()?, a.u32(1)?, a.u32(2)?, a.f64(3)?);
            J::Null
        }
        "set_named_styles" => {
            wb.cells_mut().named_styles = a
                .arg(0)
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| xl_compat::cells::NamedStyle {
                    name: s[0].as_str().unwrap_or("").to_string(),
                    style: xl_compat::cells::Style::from_spec(s[1].as_str().unwrap_or("")),
                    builtin_id: s[2].as_i64().and_then(|b| u32::try_from(b).ok()),
                    hidden: s[3].as_bool().unwrap_or(false),
                    ..Default::default()
                })
                .collect();
            J::Null
        }
        "add_style_spec" => json!(wb.add_style_spec(a.str(0)?)),
        "set_styles" => {
            wb.set_styles(&strings(a.arg(0)));
            J::Null
        }
        "set_sheet_kind" => {
            wb.set_sheet_kind(sheet()?, a.u64(1)? as u8);
            J::Null
        }
        "add_defined_name" => {
            wb.cells_mut().defined_names.push(xl_compat::cells::DefinedName { hidden: a.flag(3, false), name: a.str(0)?.to_string(), local_sheet: opt_u32(2), value: a.str(1)?.to_string() });
            J::Null
        }
        "add_hyperlink" => {
            let (cell, target, location, tooltip, display) = (a.str(1)?.to_string(), a.opt_str(2), a.opt_str(3), a.opt_str(4), a.opt_str(5));
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.hyperlinks.push(xl_compat::cells::Hyperlink { cell, target, location, tooltip, display });
            }
            J::Null
        }
        "set_auto_filter" => {
            let range = a.opt_str(1);
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.auto_filter = range;
            }
            J::Null
        }
        "add_validation" => {
            let (attrs, formula1, formula2) = (pairs(a.arg(1)), a.opt_str(2), a.opt_str(3));
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.validations.push(xl_compat::cells::Validation { attrs, formula1, formula2 });
            }
            J::Null
        }
        "add_cond_format" => {
            let sqref = a.str(1)?.to_string();
            let rules = a.arg(2).as_array().into_iter().flatten().map(|r| xl_compat::cells::CondRule { attrs: pairs(&r[0]), inner: r[1].as_str().unwrap_or("").to_string(), dxf: r[2].as_str().map(str::to_string) }).collect();
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.cond_formats.push(xl_compat::cells::CondFormat { sqref, rules });
            }
            J::Null
        }
        "add_drawing" => {
            let from = anchor(a.arg(1)).ok_or("from must be (column, offset, row, offset)")?;
            let size = a.arg(3).as_array().and_then(|s| Some((s.first()?.as_i64()?, s.get(1)?.as_i64()?)));
            let image = match a.arg(4).as_array() {
                Some(i) => Some((std::sync::Arc::from(a.blob(&i[1], "$b").ok_or("an image is (extension, bytes)")?.to_vec()), i[0].as_str().unwrap_or("").to_string())),
                None => None,
            };
            let chart = a.opt_str(5).map(|c| std::sync::Arc::from(c.as_str()));
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.drawings.push(xl_compat::cells::Drawing { anchor: xl_compat::cells::Anchor { from, to: anchor(a.arg(2)), ext: size }, image, chart });
            }
            J::Null
        }
        "add_comment" => {
            let c = xl_compat::cells::Comment { cell: a.str(1)?.to_string(), author: a.str(2)?.to_string(), text: a.str(3)?.to_string() };
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.comments.push(c);
            }
            J::Null
        }
        "set_workbook_xml" | "set_properties_xml" => {
            let (name, xml) = (a.str(0)?.to_string(), a.str(1)?.to_string());
            let cells = wb.cells_mut();
            let list = if method == "set_workbook_xml" { &mut cells.extra } else { &mut cells.properties };
            list.retain(|e| e.0 != name);
            if !xml.is_empty() {
                list.push((name, xml));
            }
            J::Null
        }
        "add_table" => {
            let xml = a.str(1)?.to_string();
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.tables.push(xml);
            }
            J::Null
        }
        "set_sheet_origin" => {
            wb.set_sheet_origin(sheet()?, a.u32(1)?);
            J::Null
        }
        "set_row_height" => {
            wb.set_row_height(sheet()?, a.u32(1)?, a.f64(2)?);
            J::Null
        }
        "set_row" => {
            let row = xl_compat::cells::Row {
                row: a.u32(1)?,
                height: opt_f64(2),
                custom_height: a.flag(3, false),
                hidden: a.flag(4, false),
                outline_level: a.arg(5).as_u64().unwrap_or(0) as u8,
                collapsed: a.flag(6, false),
                style: opt_u32(7),
                custom_format: a.flag(8, false),
                thick_top: a.flag(9, false),
                thick_bot: a.flag(10, false),
                phonetic: a.flag(11, false),
            };
            wb.set_row(sheet()?, row);
            J::Null
        }
        "set_columns" => {
            let col = xl_compat::cells::Column {
                first: a.u32(1)?,
                last: a.u32(2)?,
                width: opt_f64(3),
                custom_width: a.flag(4, false),
                hidden: a.flag(5, false),
                best_fit: a.flag(6, false),
                outline_level: a.arg(7).as_u64().unwrap_or(0) as u8,
                collapsed: a.flag(8, false),
                style: opt_u32(9),
                phonetic: a.flag(10, false),
            };
            wb.set_columns(sheet()?, col);
            J::Null
        }
        "set_sheet_xml" => {
            let (name, xml) = (a.str(1)?.to_string(), a.str(2)?.to_string());
            if let Some(s) = wb.cells_mut().sheets.get_mut(sheet()?) {
                s.extra.retain(|e| e.0 != name);
                if !xml.is_empty() {
                    s.extra.push((name, xml));
                }
            }
            J::Null
        }
        "set_freeze" => {
            wb.set_freeze(sheet()?, a.u32(1)?, a.u32(2)?);
            J::Null
        }
        "set_date1904" => {
            wb.set_date1904(a.flag(0, false));
            J::Null
        }
        other => return Err(format!("a writer has no method {other:?}")),
    })
}
