//! The engine: what `xlcells_engine._engine` (bindings/python-engine) gives, for the
//! module. Plans run and Parquet files are written by the host (see `host`).

use crate::host::{backend, inbound, HostExecutor};
use crate::rpc::{In, Out};
use serde_json::{json, Value as J};
use std::path::PathBuf;
use std::sync::OnceLock;
use xl_engine::data::Frame;
use xl_engine::regions::Region;

fn executor() -> Box<dyn xl_engine::ir::Executor> {
    Box::new(HostExecutor::default())
}

/// A path argument, as the host writes it.
fn dir(a: &In, i: usize) -> Result<PathBuf, String> {
    Ok(inbound(a.str(i)?))
}

/// A workbook file read once and held here.
pub struct Book {
    wb: xl_cells::Workbook,
    original: Vec<u8>,
    found: OnceLock<Vec<Region>>,
}

impl Book {
    pub fn new(data: &[u8]) -> Result<Book, String> {
        let wb = xl_compat::read_any(data.to_vec()).map_err(|e| e.to_string())?;
        Ok(Book { wb, original: data.to_vec(), found: OnceLock::new() })
    }

    fn all(&self) -> &[Region] {
        self.found.get_or_init(|| xl_engine::regions::regions(&self.wb))
    }

    fn region(&self, name: &str) -> Result<Region, String> {
        self.all().iter().find(|r| r.name == name).cloned().ok_or_else(|| format!("no region named {name:?}; the regions are {:?}", self.all().iter().map(|r| r.name.as_str()).collect::<Vec<_>>()))
    }

    /// A method of the book (as `_engine.Book` has them).
    pub fn call(&mut self, method: &str, a: &In, out: &mut Out) -> Result<J, String> {
        Ok(match method {
            "sheet_names" => json!(self.wb.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>()),
            "used_area" => json!(xl_engine::frame::used_area(&self.wb, a.u32(0)?).map(|x| (x.first_row, x.first_col, x.last_row, x.last_col))),
            "frame" => {
                let sheet = a.u32(0)?;
                if sheet as usize >= self.wb.sheets.len() {
                    return Err(format!("no sheet {sheet}; the workbook has {} sheets", self.wb.sheets.len()));
                }
                let area = a.arg(1).as_array().map(|v| (v[0].as_u64().unwrap_or(0) as u32, v[1].as_u64().unwrap_or(0) as u32, v[2].as_u64().unwrap_or(0) as u32, v[3].as_u64().unwrap_or(0) as u32));
                let has_header = a.flag(2, true);
                let df = match area {
                    Some((r0, c0, r1, c1)) if r0 <= r1 && c0 <= c1 => xl_engine::frame::frame(&self.wb, xl_engine::frame::Area { sheet, first_row: r0, first_col: c0, last_row: r1, last_col: c1 }, has_header)?,
                    Some(_) => Frame::empty(),
                    None => xl_engine::frame::sheet_frame(&self.wb, sheet, has_header)?,
                };
                out.frame(&df)?
            }
            "regions" => J::Array(
                self.all()
                    .iter()
                    .map(|r| {
                        json!({
                            "name": r.name, "kind": r.kind.as_str(), "sheet": r.area.sheet, "ref": r.range(), "has_header": r.has_header,
                            "source": r.source, "first_row": r.data_rows().map_or(0, |x| x.0),
                            "columns": r.columns.iter().map(|c| json!([c.name, c.col, c.dtype, c.formula, c.logic, c.exceptions.iter().map(|e| json!([e.row, e.formula, e.logic])).collect::<Vec<_>>()])).collect::<Vec<_>>(),
                            "cells": r.cells.iter().map(|c| json!([c.name, c.row, c.col, c.dtype, c.formula, c.logic])).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            ),
            "region_frame" => {
                let region = self.region(a.str(0)?)?;
                out.frame(&xl_engine::regions::region_frame(&self.wb, &region)?)?
            }
            "write" => {
                let region = self.region(a.str(0)?)?;
                let after = xl_engine::apply::write_region(&mut self.wb, &region, &a.frame(1)?)?;
                self.found = OnceLock::new();
                json!(after.range())
            }
            "verify" => {
                let region = self.region(a.str(0)?)?;
                out.frame(&xl_engine::apply::verify(&self.wb, &region, &a.frame(1)?)?)?
            }
            "calculate" => {
                let (done, calc) = xl_engine::lift::calculate_with(&*executor(), &self.wb);
                self.wb = done;
                self.found = OnceLock::new();
                json!([calc.formula_cells, calc.computed, calc.not_computed.into_iter().collect::<Vec<_>>()])
            }
            "polars_program" => {
                let accuracy = if a.flag(1, false) { xl_engine::codegen::Accuracy::Exact } else { xl_engine::codegen::Accuracy::Plain };
                let engine: Vec<(String, String)> = a.arg(2).as_array().into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect();
                let ex = executor();
                let p = xl_engine::codegen::polars_program(&self.wb, a.str(0)?, accuracy, &engine, Some(&*ex));
                json!([p.source, p.items.into_iter().map(|i| json!([i.region, i.name, i.formula, i.how, i.reason, i.line, i.fallback, i.check])).collect::<Vec<_>>(), p.check_line, p.exact_statement])
            }
            "to_bytes" => {
                let data = if xl_compat::is_xls(&self.original) { xl_cells::write_bytes(&self.wb) } else { xl_cells::update_bytes(self.original.clone(), &self.wb) };
                out.bytes(data.map_err(|e| e.to_string())?)
            }
            other => return Err(format!("a book has no method {other:?}")),
        })
    }
}

/// The looks of the cells of one workbook file, read once.
pub struct Looker(xl_look::Looker<'static>);

impl Looker {
    pub fn new(data: &[u8]) -> Result<Looker, String> {
        let wb = xl_cells::read_bytes(data.to_vec()).map_err(|e| e.to_string())?;
        let mut looker = xl_look::Looker::owning_with_engine(wb, executor);
        looker.pivots = xl_look::pivot::pivot_areas(data);
        Ok(Looker(looker))
    }

    pub fn looks(&mut self, a: &In) -> Result<J, String> {
        let cells: Vec<(u32, u32, u32)> = a.arg(0).as_array().into_iter().flatten().filter_map(|c| Some((c.get(0)?.as_u64()? as u32, c.get(1)?.as_u64()? as u32, c.get(2)?.as_u64()? as u32))).collect();
        Ok(J::Array(self.0.looks(&cells).into_iter().map(look_json).collect()))
    }
}

fn source_parts(s: &xl_look::Source) -> (&'static str, Vec<String>) {
    use xl_look::Source;
    let parts = match s {
        Source::Default | Source::Cell => Vec::new(),
        Source::Style(n) => vec![n.clone()],
        Source::Row(r) => vec![(r + 1).to_string()],
        Source::Column(c) => vec![xl_formula::col_name(c + 1)],
        Source::Table { table, style, element } => vec![table.clone(), style.clone(), element.clone()],
        Source::Conditional { range, priority, kind } => vec![range.clone(), priority.to_string(), kind.clone()],
    };
    (s.kind(), parts)
}

fn source_range(s: &xl_look::Source) -> Option<String> {
    match s {
        xl_look::Source::Conditional { range, .. } => Some(range.clone()),
        _ => None,
    }
}

/// A look as `_engine.Looker.looks` gives it: every attribute as (value, source kind,
/// source parts, certain, colour as stated), and `data_bar`, `icon`, `undecided`.
fn look_json(l: xl_look::Look) -> J {
    let mut d = serde_json::Map::new();
    for (k, a) in &l.attrs {
        let value = match &a.value {
            xl_look::Val::Text(t) => json!(t),
            xl_look::Val::Number(n) => json!(n),
            xl_look::Val::Flag(b) => json!(b),
            xl_look::Val::Rgb(c) => json!(c),
            xl_look::Val::None => J::Null,
        };
        let (kind, parts) = source_parts(&a.source);
        d.insert(k.to_string(), json!([value, kind, parts, a.certain, a.spec]));
    }
    d.insert("data_bar".into(), l.data_bar.as_ref().map_or(J::Null, |b| json!([b.color, b.fraction, source_range(&b.source)])));
    d.insert("icon".into(), l.icon.as_ref().map_or(J::Null, |i| json!([i.set, i.index, source_range(&i.source)])));
    d.insert("undecided".into(), json!(l.undecided));
    J::Object(d)
}

fn report(r: xl_engine::store::RunReport) -> J {
    json!([r.formula_cells, r.computed, r.not_computed, r.regions, r.logic_changed])
}

fn recalculated(r: xl_engine::plans::Recalculation) -> J {
    json!([r.done, r.kept, r.from_kept])
}

/// The functions of the engine module that hold no object.
pub fn call(name: &str, a: &In, out: &mut Out) -> Result<J, String> {
    let b = backend;
    Ok(match name {
        "calculate" => {
            let wb = xl_cells::read_bytes(a.bytes(0)?.to_vec()).map_err(|e| e.to_string())?;
            let (done, calc) = xl_engine::lift::calculate_with(&*executor(), &wb);
            let (before, c) = (&wb.cells, &done.cells);
            let values: Vec<J> = (0..c.len())
                .filter(|&i| c.formula[i].is_some() && !(before.kind[i] == c.kind[i] && before.num[i].to_bits() == c.num[i].to_bits() && before.text[i] == c.text[i]))
                .map(|i| json!([c.sheet[i], c.row[i], c.col[i], c.kind[i] as u8, c.num[i], c.text[i].as_deref()]))
                .collect();
            json!({ "values": values, "formula_cells": calc.formula_cells, "computed": calc.computed, "changed": calc.changed, "settled": calc.settled, "rounds": calc.rounds, "not_computed": calc.not_computed.into_iter().collect::<Vec<_>>() })
        }
        "recalculation" => {
            let wb = xl_compat::read_any(a.bytes(0)?.to_vec()).map_err(|e| e.to_string())?;
            json!(xl_engine::lift::recalculation(&*executor(), &wb))
        }
        "store_create" => {
            let data = a.bytes(1)?;
            let wb = xl_compat::read_any(data.to_vec()).map_err(|e| e.to_string())?;
            json!(xl_engine::store::create(&*b(), &dir(a, 0)?, data, &wb, !xl_compat::is_xls(data))?)
        }
        "store_run" => report(xl_engine::store::run(&*b(), &dir(a, 0)?)?),
        "store_read" => out.frame(&xl_engine::store::read_region(&*b(), &dir(a, 0)?, a.str(1)?)?)?,
        "store_read_wide" => out.frame(&xl_engine::store::read_wide(&*b(), &dir(a, 0)?, a.str(1)?)?)?,
        "store_write" => {
            xl_engine::store::write_frame(&*b(), &dir(a, 0)?, a.str(1)?, &a.frame(2)?)?;
            J::Null
        }
        "store_add_sheet" => json!(xl_engine::store::add_sheet(&*b(), &dir(a, 0)?, a.str(1)?, a.str(2)?)?),
        "store_patch" => {
            let patch: J = serde_json::from_str(a.str(2)?).map_err(|e| e.to_string())?;
            json!(xl_engine::store::patch(&*b(), &dir(a, 0)?, a.str(1)?, &patch)?)
        }
        "store_tables" => json!(J::Array(xl_engine::store::tables(&*b(), &dir(a, 0)?)?).to_string()),
        "store_rename_region" => report(xl_engine::store::rename_region(&*b(), &dir(a, 0)?, a.str(1)?, a.str(2)?)?),
        "store_remember" => {
            xl_engine::store::remember(&dir(a, 0)?, a.str(1)?)?;
            J::Null
        }
        "store_undo" => json!(xl_engine::store::undo(&dir(a, 0)?)?),
        "store_set_cells" => {
            let cells: Vec<(String, String, J)> = serde_json::from_str(a.str(1)?).map_err(|e| e.to_string())?;
            report(xl_engine::store::set_cells(&*b(), &dir(a, 0)?, &cells)?)
        }
        "store_recalculate" => recalculated(xl_engine::store::recalculate(&*b(), &dir(a, 0)?)?),
        "recalculate_files" => {
            let backend = b();
            let mut files = Vec::new();
            for p in a.arg(0).as_array().into_iter().flatten().filter_map(J::as_str) {
                let path = inbound(p);
                let note = backend.read_note(&path)?.ok_or_else(|| format!("{p} carries no note (file metadata {:?}): it was not written by a store, or written anew by a tool that dropped it", xl_engine::store::NOTE_KEY))?;
                files.push((path, serde_json::from_str(&note).map_err(|e| format!("the note of {p} is not JSON: {e}"))?));
            }
            recalculated(xl_engine::store::recalculate_files(&*backend, &files)?)
        }
        "read_note" => json!(b().read_note(&dir(a, 0)?)?),
        "store_port" => {
            let accuracy = if a.flag(2, false) { xl_engine::codegen::Accuracy::Exact } else { xl_engine::codegen::Accuracy::Plain };
            let check: Option<xl_engine::port::Check> = if a.flag(4, false) { Some(&crate::host::check_port) } else { None };
            let p = xl_engine::port::write(&*b(), &dir(a, 0)?, &dir(a, 1)?, accuracy, a.flag(3, false), check)?;
            json!([p.items.iter().map(|i| json!([i.region, i.name, i.formula, i.logic, i.how, i.reason, i.line])).collect::<Vec<_>>(), p.summary()])
        }
        other => return Err(format!("the engine has no function {other:?}")),
    })
}
