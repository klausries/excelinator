//! A workbook as a store: a directory with the data of its regions as Parquet files, a
//! model document that describes them and their logic, and the workbook itself as the
//! form the data returns to.
//!
//! ```text
//! store/
//!   model.json       the regions: tables, single cells, blocks; types; logic
//!   workbook.xlsx    the workbook (layout, formats, formulas)
//!   data/<region>.parquet
//!   notes.json       the notes of the Parquet files, once more
//! ```
//!
//! A table or block is a Parquet file with its columns. A group of single cells is one
//! with a row per cell: `name`, `cell`, `dtype`, `number`, `text` (see [`crate::long`]);
//! [`read_region`] gives it so, [`write_frame`] takes rows by name (or one row with a
//! column per cell), and [`read_wide`] gives it as one row with a column per cell.
//!
//! Every Parquet file carries a note in its file metadata (key [`NOTE_KEY`]): where the
//! data is from, and for each calculated column or cell the formula and, where the engine
//! has one, its translation as a plan over the data of the regions (see
//! [`crate::plans`]). The logic so travels with the data: [`recalculate`] works the
//! calculated columns and cells out again from the files alone, without the workbook.
//! A tool that writes a Parquet file anew usually drops the note; `notes.json` holds
//! them all once more for that case.
//!
//! ```text
//! {"version": 1, "region": "Orders", "kind": "table", "sheet": "Sales", "ref": "A2:D6",
//!  "source": "detected", "date1904": false,
//!  "columns": [
//!    {"name": "qty", "dtype": "Int64"},
//!    {"name": "amount", "dtype": "Float64", "formula": "B3*C3", "logic": "[@qty]*[@price]",
//!     "plan": 0, "inputs": [{"name": "i0", "column": "qty"}, {"name": "i2", "cell": ["Model", "Factor"]}, …],
//!     "first_row": 3, "rows": 4},
//!    {"name": "rank", "dtype": "Int64", "formula": "…", "why": "why there is no plan"}],
//!  "plans": [{"nodes": […], "steps": […], "outputs": […], "broadcast": true}]}
//! ```
//!
//! A column may have `"exceptions"`: the rows that are not as the column is, each with
//! its `row` (from 0 in the data) and, where it has a formula of its own, `formula`,
//! `logic`, `plan` and `inputs` as a single cell has them; without a formula the row
//! holds a plain value. A column of values with such rows has `exceptions` and no
//! `formula`. A calculated column that has no plan as a whole (`why`) but one for each
//! of its rows has them in `"each"`: `row`, `plan` and `inputs` for every row that is
//! not an exception; it is worked out row by row, top to bottom.
//!
//! The note of a group of single cells has `"form": "rows"`: its file has a row per cell,
//! and the `columns` of the note are the cells (`cell`: the address).
//!
//! `plan` is the number of one of `plans` (the same formula in many places is one plan);
//! `inputs` say where each input of the plan is in the data: a column of a region, a named
//! cell (by name in its own region, else `[region, name]`), a cell of a table (with
//! `"parse": true` a number in a column of mixed kinds, read from its text), the numbers
//! of a range, or a value that is not data; and `as` what it is taken as where that is not
//! a number.
//!
//! The Parquet files are the database: change them with whatever reads and writes
//! Parquet. [`run`] takes what is not calculated from them, puts it into the workbook,
//! calculates every formula with the engine and writes workbook, Parquet files and
//! model anew, so that all three agree again.
//!
//! The logic of a calculated column or cell can be changed in `model.json` (`logic`, in
//! the structured form: `[@qty]*[@price]`, `SUM(Orders[amount])`, names of single
//! cells); a column or cell that was not calculated becomes so by giving it a `logic`,
//! and stops being so by removing it. [`run`] carries such changes into the workbook.

use crate::regions::{regions, regions_like, unstructured, Known, Region, RegionKind};
use crate::data::{Column, Data, Frame};
pub use crate::view::{frame_from_rows, parse_clock, parse_date, parse_datetime, parse_duration, rows_json};
use crate::ir::Executor;
use serde_json::{json, Value as J};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use xl_cells::{Kind, Workbook};

/// What a store needs from outside the engine: something to run the formula
/// expressions, and Parquet files.
pub trait Backend {
    fn executor(&self) -> &dyn Executor;
    /// Write a frame; `note` goes into the metadata of the file under [`NOTE_KEY`].
    fn write_parquet(&self, path: &Path, frame: &Frame, note: Option<&str>) -> Result<(), String>;
    fn read_parquet(&self, path: &Path) -> Result<Frame, String>;
    /// The note of a file (its metadata under [`NOTE_KEY`]), if it has one.
    fn read_note(&self, path: &Path) -> Result<Option<String>, String>;
}

/// The key of a Parquet file's metadata that holds its note.
pub const NOTE_KEY: &str = "xlcells";

const MODEL: &str = "model.json";
const NOTES: &str = "notes.json";
const WORKBOOK: &str = "workbook.xlsx";

/// What [`run`] did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunReport {
    pub formula_cells: u64,
    pub computed: u64,
    /// Cells the engine could not calculate, by reason.
    pub not_computed: Vec<(String, u64)>,
    /// The regions and their rows.
    pub regions: Vec<(String, usize)>,
    /// Logic taken from the model into the workbook: (region, column or cell).
    pub logic_changed: Vec<(String, String)>,
    /// The cells not calculated (the first 50): address, formula, reason.
    pub uncomputed: Vec<(String, String, String)>,
    /// Regions the store had that the workbook does not have any more, and regions it
    /// has now: the changes made the sheet read differently (tables that touch, say).
    pub regions_gone: Vec<String>,
    pub regions_new: Vec<String>,
}

const PENDING: &str = ".pending";

/// Data or logic of the store was changed since its last run.
pub fn pending(dir: &Path) -> bool {
    dir.join(PENDING).exists()
}

fn mark_pending(dir: &Path) {
    if dir.join(MODEL).exists() {
        let _ = fs::write(dir.join(PENDING), "");
    }
}

/// The formula cells of a workbook that hold no value (never calculated, or an error).
pub fn without_value(wb: &Workbook) -> usize {
    (0..wb.cells.len()).filter(|&i| wb.cells.formula[i].as_deref().is_some_and(|f| !f.is_empty()) && matches!(wb.cells.kind[i], Kind::Blank | Kind::Error)).count()
}

fn file_name(region: &str) -> String {
    let safe: String = region.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    format!("data/{}.parquet", safe.trim())
}

fn io<T>(r: std::io::Result<T>, what: &str, path: &Path) -> Result<T, String> {
    r.map_err(|e| format!("{what} {}: {e}", path.display()))
}

fn write_parquet(b: &dyn Backend, path: &Path, df: &Frame, note: Option<&J>) -> Result<(), String> {
    // A column without any value has no type to store: it is kept as text.
    let mut df = df.clone();
    for c in df.columns.iter_mut() {
        if let Data::Null(n) = c.data {
            c.data = Data::String(vec![None; n]);
        }
    }
    let note = note.map(|n| n.to_string());
    b.write_parquet(path, &df, note.as_deref()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The notes of a workbook's regions (see the module), in the order of [`regions`].
pub fn notes(ex: &dyn Executor, wb: &Workbook) -> Vec<J> {
    notes_of(ex, wb, &regions(wb))
}

fn notes_of(ex: &dyn Executor, wb: &Workbook, found: &[Region]) -> Vec<J> {
    let context = crate::plans::Context::new(wb, found);
    let mut mapper = crate::plans::Mapper::new(wb, found, &context, Some(ex));
    mapper.mixed = true;
    found
        .iter()
        .enumerate()
        .map(|(k, r)| {
            // The plans of the region, each once: the same formula in many places is one plan.
            let mut plans: Vec<J> = Vec::new();
            let mut plan_at: HashMap<String, usize> = HashMap::new();
            let mut planned = |o: &mut J, mapped: Result<crate::plans::Mapped, String>| match mapped {
                Ok(m) => {
                    for (key, v) in mapper.json(k, &m).as_object().into_iter().flatten() {
                        if key == "plan" {
                            let n = *plan_at.entry(v.to_string()).or_insert_with(|| {
                                plans.push(v.clone());
                                plans.len() - 1
                            });
                            o["plan"] = json!(n);
                        } else {
                            o[key.as_str()] = v.clone();
                        }
                    }
                }
                Err(why) => o["why"] = json!(why),
            };
            let said = |o: &mut J, logic: &Option<String>, formula: &str| {
                o["formula"] = json!(formula);
                if let Some(l) = logic.as_ref().filter(|l| *l != formula) {
                    o["logic"] = json!(l);
                }
            };
            let first = r.data_rows().map_or(0, |d| d.0);
            let columns: Vec<J> = if r.kind == RegionKind::Cells {
                r.cells
                    .iter()
                    .map(|c| {
                        let mut x = json!({ "name": c.name, "dtype": c.dtype, "cell": format!("{}{}", xl_formula::col_name(c.col + 1), c.row + 1) });
                        if let Some(f) = &c.formula {
                            said(&mut x, &c.logic, f);
                            planned(&mut x, mapper.map(k, &c.name));
                        }
                        x
                    })
                    .collect()
            } else {
                r.columns
                    .iter()
                    .map(|c| {
                        let mut x = json!({ "name": c.name, "dtype": c.dtype });
                        // A column without a translation as a whole is worked out row by
                        // row, where rows of it have one.
                        let mut each = false;
                        if let Some(f) = &c.formula {
                            said(&mut x, &c.logic, f);
                            let whole = mapper.map(k, &c.name);
                            let rows: Vec<u32> = r.data_rows().map(|(a, z)| (a..=z).filter(|row| c.exception(*row).is_none()).collect()).unwrap_or_default();
                            // (Tried on its first, middle and last row.)
                            let tried = [rows.first(), rows.get(rows.len() / 2), rows.last()];
                            each = whole.is_err() && tried.iter().flatten().any(|row| mapper.map_row(k, &c.name, **row, true).is_ok());
                            planned(&mut x, whole);
                            if each {
                                let list: Vec<J> = rows
                                    .iter()
                                    .map(|row| {
                                        let mut o = json!({ "row": row - first });
                                        if let Some(d) = crate::regions::moment_dtype(wb, &context.at, r.area.sheet, *row, c.col).filter(|_| c.dtype == "String") {
                                            o["dtype"] = json!(d);
                                        }
                                        planned(&mut o, mapper.map_row(k, &c.name, *row, true));
                                        o
                                    })
                                    .collect();
                                x["each"] = J::Array(list);
                                x["rows"] = json!(r.data_rows().map_or(0, |(a, z)| z - a + 1));
                            }
                        }
                        if !c.exceptions.is_empty() {
                            let list: Vec<J> = c
                                .exceptions
                                .iter()
                                .map(|e| {
                                    let mut o = json!({ "row": e.row - first });
                                    if let Some(d) = &e.dtype {
                                        o["dtype"] = json!(d);
                                    }
                                    if let Some(f) = &e.formula {
                                        said(&mut o, &e.logic, f);
                                        planned(&mut o, mapper.map_row(k, &c.name, e.row, each));
                                    }
                                    o
                                })
                                .collect();
                            x["exceptions"] = J::Array(list);
                        }
                        x
                    })
                    .collect()
            };
            let mut note = json!({
                "version": 1,
                "region": r.name,
                "kind": r.kind.as_str(),
                "sheet": wb.sheets[r.area.sheet as usize].name,
                "ref": r.range(),
                "source": r.source,
                "date1904": wb.date1904,
                "columns": columns,
                "plans": plans,
            });
            if r.kind == RegionKind::Cells {
                // The data is a row per cell (see `crate::long`); `columns` are the cells.
                note["form"] = json!("rows");
            }
            note
        })
        .collect()
}

/// The notes kept in the store beside the files: file name (as in the model) → note.
fn kept_notes(dir: &Path) -> serde_json::Map<String, J> {
    fs::read_to_string(dir.join(NOTES)).ok().and_then(|t| serde_json::from_str::<J>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// The note of a Parquet file of a store: its own, else the one kept beside it.
fn note_of(b: &dyn Backend, dir: &Path, file: &str, kept: &serde_json::Map<String, J>) -> Option<J> {
    let own = b.read_note(&dir.join(file)).ok().flatten().and_then(|t| serde_json::from_str::<J>(&t).ok());
    own.or_else(|| kept.get(file).cloned())
}

fn read_parquet(b: &dyn Backend, path: &Path) -> Result<Frame, String> {
    if !path.exists() {
        return Err(format!("cannot read {}: no such file", path.display()));
    }
    b.read_parquet(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The model document of a workbook.
pub fn model(wb: &Workbook) -> J {
    model_of(wb, &regions(wb))
}

/// The regions a model names, as what is known of the workbook's regions: what
/// [`regions_like`] is told, so that a region of a store stays the one it is.
fn known(model: &J, wb: &Workbook) -> Vec<Known> {
    let one = |r: &J| Some(Known { name: r["name"].as_str()?.to_string(), kind: RegionKind::of(r["kind"].as_str()?)?, area: crate::regions::area_of(wb, r["sheet"].as_str()?, r["ref"].as_str()?)? });
    model["regions"].as_array().into_iter().flatten().filter_map(one).collect()
}

/// The regions of a store's workbook (see [`regions_like`]: they stay what they are).
pub fn regions_of(dir: &Path, wb: &Workbook) -> Vec<Region> {
    regions_like(wb, &read_model(dir).map(|m| known(&m, wb)).unwrap_or_default())
}

fn model_of(wb: &Workbook, found: &[Region]) -> J {
    let list: Vec<J> = found
        .iter()
        .map(|r| {
            let item = |name: &str, dtype: &str, logic: &Option<String>, formula: &Option<String>| {
                let mut o = json!({ "name": name, "dtype": dtype });
                if let Some(f) = formula {
                    o["logic"] = json!(logic.as_deref().unwrap_or(f));
                    o["formula"] = json!(f);
                }
                o
            };
            let mut o = json!({
                "name": r.name,
                "kind": r.kind.as_str(),
                "sheet": wb.sheets[r.area.sheet as usize].name,
                "ref": r.range(),
                "source": r.source,
                "data": file_name(&r.name),
            });
            if r.kind == RegionKind::Cells {
                o["cells"] = J::Array(
                    r.cells
                        .iter()
                        .map(|c| {
                            let mut x = item(&c.name, &c.dtype, &c.logic, &c.formula);
                            x["cell"] = json!(format!("{}{}", xl_formula::col_name(c.col + 1), c.row + 1));
                            x
                        })
                        .collect(),
                );
            } else {
                let first = r.data_rows().map_or(0, |d| d.0);
                o["columns"] = J::Array(
                    r.columns
                        .iter()
                        .map(|c| {
                            let mut x = item(&c.name, &c.dtype, &c.logic, &c.formula);
                            if !c.exceptions.is_empty() {
                                x["exceptions"] = c
                                    .exceptions
                                    .iter()
                                    .map(|e| {
                                        let mut o = json!({ "row": e.row - first, "cell": format!("{}{}", xl_formula::col_name(c.col + 1), e.row + 1) });
                                        if let Some(f) = &e.formula {
                                            o["logic"] = json!(e.logic.as_deref().unwrap_or(f));
                                            o["formula"] = json!(f);
                                        }
                                        o
                                    })
                                    .collect();
                            }
                            x
                        })
                        .collect(),
                );
            }
            o
        })
        .collect();
    let stacks: Vec<J> = crate::stack::stacks(wb, found).iter().map(stack_json).collect();
    json!({
        "version": 1,
        "workbook": WORKBOOK,
        "stacks": stacks,
        "about": "regions of the workbook; data/*.parquet hold their values. Change the Parquet files and the `logic` of calculated columns and cells, then run the store: the workbook, the Parquet files and this file are written anew. `formula` is the logic as the workbook has it (not read back). `exceptions` are the rows of a column (`row` from 0 in the data) that are not as the column is: a logic of their own, or a plain value in a calculated column; they stay as they are (not read back). `stacks` are tables of several sheets that are built alike (not read back): a stack is read and written by its name as one table with the column `sheet` in front.",
        "regions": list,
    })
}

fn stack_json(s: &crate::stack::Stack) -> J {
    json!({
        "name": s.name,
        "columns": s.columns,
        "members": s.members.iter().map(|m| json!({ "sheet": m.sheet, "region": m.region })).collect::<Vec<_>>(),
        "differs": s.differs.iter().map(|(column, logic)| json!({ "column": column, "logic": logic })).collect::<Vec<_>>(),
    })
}

/// The stacks of a store (see [`crate::stack`]), as its model has them.
pub fn stacks(dir: &Path) -> Result<Vec<crate::stack::Stack>, String> {
    let model = read_model(dir)?;
    let text = |v: &J| v.as_str().unwrap_or_default().to_string();
    Ok(model["stacks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| crate::stack::Stack {
            name: text(&s["name"]),
            columns: s["columns"].as_array().into_iter().flatten().map(text).collect(),
            members: s["members"].as_array().into_iter().flatten().map(|m| crate::stack::Member { sheet: text(&m["sheet"]), region: text(&m["region"]) }).collect(),
            differs: s["differs"].as_array().into_iter().flatten().map(|d| (text(&d["column"]), d["logic"].as_array().into_iter().flatten().map(|l| l.as_str().map(str::to_string)).collect())).collect(),
        })
        .collect())
}

fn stack_named(dir: &Path, name: &str) -> Option<crate::stack::Stack> {
    stacks(dir).ok()?.into_iter().find(|s| s.name == name)
}

fn save(b: &dyn Backend, dir: &Path, wb: &Workbook, bytes: &[u8], found: &[Region]) -> Result<Vec<(String, usize)>, String> {
    io(fs::create_dir_all(dir.join("data")), "cannot make", &dir.join("data"))?;
    let path = dir.join(WORKBOOK);
    io(fs::write(&path, bytes), "cannot write", &path)?;
    let mut rows = Vec::new();
    let at = crate::frame::cell_index(wb);
    let notes = notes_of(b.executor(), wb, found);
    let mut kept = serde_json::Map::new();
    for (r, note) in found.iter().zip(&notes) {
        let mut df = crate::regions::region_frame_in(wb, &at, r).map_err(|e| e.to_string())?;
        if r.kind == RegionKind::Cells {
            let cells: Vec<(String, String)> = r.cells.iter().map(|c| (c.name.clone(), format!("{}{}", xl_formula::col_name(c.col + 1), c.row + 1))).collect();
            df = crate::long::to_long(&df, &cells);
        }
        write_parquet(b, &dir.join(file_name(&r.name)), &df, Some(note))?;
        kept.insert(file_name(&r.name), note.clone());
        rows.push((r.name.clone(), df.height));
    }
    let path = dir.join(NOTES);
    io(fs::write(&path, J::Object(kept).to_string()), "cannot write", &path)?;
    let path = dir.join(MODEL);
    io(fs::write(&path, serde_json::to_string_pretty(&model_of(wb, found)).unwrap_or_default()), "cannot write", &path)?;
    Ok(rows)
}

/// Make a store in `dir` from a workbook file (xlsx or xls, as bytes). Returns the
/// regions and their rows.
pub fn create(b: &dyn Backend, dir: &Path, original: &[u8], wb: &Workbook, is_xlsx: bool) -> Result<Vec<(String, usize)>, String> {
    let bytes = if is_xlsx { original.to_vec() } else { xl_cells::write_bytes(wb).map_err(|e| e.to_string())? };
    save(b, dir, wb, &bytes, &regions(wb))
}

/// The paths of a store: (model, workbook, data directory).
pub fn paths(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    (dir.join(MODEL), dir.join(WORKBOOK), dir.join("data"))
}

pub fn read_model(dir: &Path) -> Result<J, String> {
    let path = dir.join(MODEL);
    let text = io(fs::read_to_string(&path), "cannot read", &path)?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))
}

/// The workbook of a store and the bytes of its file.
pub fn read_workbook(dir: &Path) -> Result<(Workbook, Vec<u8>), String> {
    let path = dir.join(WORKBOOK);
    let bytes = io(fs::read(&path), "cannot read", &path)?;
    let wb = xl_cells::read_bytes(bytes.clone()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((wb, bytes))
}

/// The frame of a region of a store, from its Parquet file.
pub fn read_region(b: &dyn Backend, dir: &Path, name: &str) -> Result<Frame, String> {
    if !dir.join(MODEL).exists() {
        return read_parquet(b, &file_of(b, dir, name)?.0);
    }
    let model = read_model(dir)?;
    let names: Vec<&str> = model["regions"].as_array().map(|a| a.iter().filter_map(|r| r["name"].as_str()).collect()).unwrap_or_default();
    if !names.contains(&name) {
        // A stack: its tables one below the other.
        if let Some(stack) = stack_named(dir, name) {
            let parts: Vec<Frame> = stack.members.iter().map(|m| read_parquet(b, &dir.join(file_name(&m.region)))).collect::<Result<_, _>>()?;
            return crate::stack::stacked(&stack, &parts);
        }
        let stacks: Vec<String> = model["stacks"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str().map(str::to_string)).collect();
        return Err(format!("no region named {name:?}; the regions are {names:?}{}", if stacks.is_empty() { String::new() } else { format!(", the stacks {stacks:?}") }));
    }
    read_parquet(b, &dir.join(file_name(name)))
}

/// The frame of a region known to be of the store, from its Parquet file (no look into
/// the model: for many regions).
pub fn read_frame(b: &dyn Backend, dir: &Path, name: &str) -> Result<Frame, String> {
    read_parquet(b, &dir.join(file_name(name)))
}

/// The frame of a region of a store with single cells as one row, a column per cell
/// (the form regions are written back and compared in).
pub fn read_wide(b: &dyn Backend, dir: &Path, name: &str) -> Result<Frame, String> {
    wide(read_frame(b, dir, name)?)
}

fn wide(df: Frame) -> Result<Frame, String> {
    if crate::long::is_long(&df) {
        crate::long::to_wide(&df)
    } else {
        Ok(df)
    }
}

/// Replace the frame of a region of a store (its Parquet file). [`run`] takes it into
/// the workbook. Of single cells, rows by name replace those rows (see [`crate::long`]);
/// one row with a column per cell is taken as well.
pub fn write_frame(b: &dyn Backend, dir: &Path, name: &str, df: &Frame) -> Result<(), String> {
    // A stack: every sheet's rows go to its table.
    if let Some(stack) = stack_named(dir, name) {
        for (region, part) in crate::stack::unstacked(&stack, df)? {
            write_frame(b, dir, &region, &part)?;
        }
        return Ok(());
    }
    let old = read_region(b, dir, name)?;
    let (path, note) = file_of(b, dir, name)?;
    if crate::long::is_long(&old) {
        let merged = crate::long::merged(&old, df, name)?;
        write_parquet(b, &path, &merged, note.as_ref())?;
        mark_pending(dir);
        return Ok(());
    }
    // Columns not given stay: with their values while the rows are the same, empty
    // (to be calculated, or to be filled) when the number of rows changes. A plain value
    // in a calculated column (an exception of the column) stays at its row either way,
    // as it stays in the workbook.
    let fixed = |column: &str| -> Vec<usize> {
        let of = note.as_ref().and_then(|n| n["columns"].as_array()).and_then(|a| a.iter().find(|c| c["name"] == column));
        of.and_then(|c| c["exceptions"].as_array()).map(|a| a.iter().filter(|e| e.get("formula").is_none()).filter_map(|e| e["row"].as_u64().map(|r| r as usize)).collect()).unwrap_or_default()
    };
    let mut columns: Vec<Column> = Vec::new();
    for c in &old.columns {
        columns.push(match df.column(&c.name) {
            Some(new) => new.clone(),
            None if old.height == df.height => c.clone(),
            None => Column::new(c.name.clone(), c.data.nulls_keeping(df.height, &fixed(&c.name))),
        });
    }
    for c in df.columns.iter().filter(|c| old.column(&c.name).is_none()) {
        columns.push(c.clone());
    }
    let merged = Frame::new(df.height, columns)?;
    write_parquet(b, &path, &merged, note.as_ref())?;
    mark_pending(dir);
    Ok(())
}

/// The file of a region and its note: in a store by the region's name; in a plain
/// directory of Parquet files the one whose note names the region.
fn file_of(b: &dyn Backend, dir: &Path, name: &str) -> Result<(PathBuf, Option<J>), String> {
    if dir.join(MODEL).exists() {
        let file = file_name(name);
        return Ok((dir.join(&file), note_of(b, dir, &file, &kept_notes(dir))));
    }
    let files = noted_files(b, dir)?;
    let have: Vec<&str> = files.iter().filter_map(|(_, n)| n["region"].as_str()).collect();
    files.iter().find(|(_, n)| n["region"] == name).map(|(p, n)| (p.clone(), Some(n.clone()))).ok_or_else(|| format!("no table named {name:?} among the Parquet files of {}; there are {have:?}", dir.display()))
}

/// Change rows of a region of a store, or of a plain directory of Parquet files with
/// notes (see [`crate::view::patched`]): values set, rows removed, rows added. Single
/// cells are set by name: `{"set": [{"name": "Tax rate", "value": 0.07}]}`. Returns the
/// rows the region has afterwards.
pub fn patch(b: &dyn Backend, dir: &Path, region: &str, patch: &J) -> Result<usize, String> {
    let old = read_region(b, dir, region)?;
    if crate::long::is_long(&old) {
        if !patch["delete"].is_null() || !patch["insert"].is_null() {
            return Err(format!("{region:?} is single cells: they are set by name, and none can be added or removed"));
        }
        let values: Vec<(String, J)> = patch["set"].as_array().into_iter().flatten().map(|s| s["name"].as_str().map(|n| (n.to_string(), s["value"].clone())).ok_or("a change of a single cell needs `name` and `value`")).collect::<Result<_, _>>()?;
        write_frame(b, dir, region, &crate::long::from_values(&values)?)?;
        return Ok(old.height);
    }
    let new = crate::view::patched(&old, patch)?;
    write_frame(b, dir, region, &new)?;
    Ok(new.height)
}

const UNDO: &str = ".undo";

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let (path, name) = (e.path(), e.file_name());
        if path.is_dir() && name != UNDO {
            copy_all(&path, &to.join(&name))?;
        } else if path.is_file() {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn undo_steps(dir: &Path) -> Vec<u64> {
    let mut steps: Vec<u64> = fs::read_dir(dir.join(UNDO)).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect();
    steps.sort_unstable();
    steps
}

/// Keep the state of a store or directory of Parquet files as it is, to come back to
/// with [`undo`]; `what` says what is about to change. The last 20 states are kept.
pub fn remember(dir: &Path, what: &str) -> Result<(), String> {
    let steps = undo_steps(dir);
    let to = dir.join(UNDO).join((steps.last().map_or(0, |n| n + 1)).to_string());
    io(copy_all(dir, &to), "cannot keep a copy in", &to)?;
    io(fs::write(to.join(".what"), what), "cannot write in", &to)?;
    for old in steps.iter().rev().skip(19) {
        let _ = fs::remove_dir_all(dir.join(UNDO).join(old.to_string()));
    }
    Ok(())
}

/// What can be undone, the last change first.
pub fn undoable(dir: &Path) -> Vec<String> {
    undo_steps(dir).iter().rev().map(|n| fs::read_to_string(dir.join(UNDO).join(n.to_string()).join(".what")).unwrap_or_default()).collect()
}

/// Bring back the state before the last change (see [`remember`]). Returns what was
/// undone, or `None` if there is nothing to undo.
pub fn undo(dir: &Path) -> Result<Option<String>, String> {
    let Some(last) = undo_steps(dir).pop() else { return Ok(None) };
    let from = dir.join(UNDO).join(last.to_string());
    let what = fs::read_to_string(from.join(".what")).unwrap_or_default();
    for e in io(fs::read_dir(dir), "cannot read", dir)?.flatten().filter(|e| e.file_name() != UNDO) {
        let path = e.path();
        io(if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) }, "cannot remove", &path)?;
    }
    io(copy_all(&from, dir), "cannot bring back", dir)?;
    let _ = fs::remove_file(dir.join(".what"));
    let _ = fs::remove_dir_all(&from);
    Ok(Some(what))
}

/// The tables of a store or of a plain directory of Parquet files, as their notes
/// describe them: for each its `name`, `kind`, `sheet` and `ref` (where it is from) and
/// its `columns` (of single cells: the cells), each with `name`, `dtype`, and if
/// calculated its `formula` and `logic`, what it `reads`, `state` (`calculated`: worked
/// out from the data alone; `kept`: only the workbook can work it out, `why` says why)
/// and the `exceptions` of its rows.
pub fn tables(b: &dyn Backend, dir: &Path) -> Result<Vec<J>, String> {
    let read = |input: &J| -> Option<String> {
        let name = |v: &J| v.as_str().map(str::to_string);
        if let Some(c) = name(&input["column"]) {
            return Some(c);
        }
        if let Some([region, cell]) = input["cell"].as_array().map(Vec::as_slice) {
            return Some(format!("{}.{}", name(region)?, name(cell)?));
        }
        if let Some([region, columns]) = input["range"]["columns"].as_array().map(Vec::as_slice) {
            return Some(format!("{}[{}]", name(region)?, columns.as_array()?.iter().filter_map(name).collect::<Vec<_>>().join(", ")));
        }
        name(&input["cell"])
    };
    let item = |c: &J| {
        let mut o = json!({ "name": c["name"], "dtype": c["dtype"] });
        for key in ["cell", "formula", "why"] {
            if !c[key].is_null() {
                o[key] = c[key].clone();
            }
        }
        if let Some(f) = c["formula"].as_str() {
            o["logic"] = json!(c["logic"].as_str().unwrap_or(f));
            o["state"] = json!(if c["plan"].is_null() && c["each"].is_null() { "kept" } else { "calculated" });
            let mut reads: Vec<String> = c["inputs"].as_array().into_iter().flatten().filter_map(read).collect();
            reads.dedup();
            o["reads"] = json!(reads);
        }
        if let Some(list) = c["exceptions"].as_array() {
            o["exceptions"] = list.iter().map(|e| json!({ "row": e["row"], "logic": e["logic"].as_str().or(e["formula"].as_str()) })).collect();
        }
        o
    };
    Ok(noted_files(b, dir)?
        .iter()
        .map(|(path, note)| {
            json!({
                "name": note["region"], "kind": note["kind"], "sheet": note["sheet"], "ref": note["ref"],
                "file": path.strip_prefix(dir).unwrap_or(path).display().to_string(),
                "columns": note["columns"].as_array().into_iter().flatten().map(item).collect::<Vec<_>>(),
            })
        })
        .collect())
}

/// Whether a logic (structured form) can be the formula of a column or cell of a
/// region of the store: `Err` says what is wrong with it.
pub fn check_logic(dir: &Path, region: &str, name: &str, logic: &str) -> Result<(), String> {
    if let Some(stack) = stack_named(dir, region) {
        return stack.members.iter().try_for_each(|m| check_logic(dir, &m.region, name, logic));
    }
    let (wb, _) = read_workbook(dir)?;
    let found = regions_of(dir, &wb);
    let r = found.iter().find(|r| r.name == region).ok_or_else(|| format!("no region named {region:?}; the regions are {:?}", found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()))?;
    let row = match r.kind {
        RegionKind::Cells => r.cells.iter().find(|c| c.name == name).map(|c| c.row),
        _ => r.columns.iter().find(|c| c.name == name).and_then(|_| r.data_rows().map(|x| x.0)),
    };
    let have: Vec<&str> = if r.kind == RegionKind::Cells { r.cells.iter().map(|c| c.name.as_str()).collect() } else { r.columns.iter().map(|c| c.name.as_str()).collect() };
    let row = row.ok_or_else(|| format!("{region:?} has no column or cell {name:?}; it has {have:?}"))?;
    unstructured(logic, &found, r, row, &wb).map(|_| ())
}

/// Give a column or cell the logic (structured form) as its formula, filled down a
/// table's column; `None` makes it a value again (the one it has).
fn set_logic(wb: &mut Workbook, all: &[Region], region: &Region, item: &str, logic: Option<&str>) -> Result<(), String> {
    let s = region.area.sheet;
    let targets: Vec<(u32, u32)> = match region.kind {
        RegionKind::Cells => {
            let cell = region.cells.iter().find(|c| c.name == item).ok_or_else(|| format!("{:?} has no cell {item:?}", region.name))?;
            vec![(cell.row, cell.col)]
        }
        _ => {
            let col = region.columns.iter().find(|c| c.name == item).ok_or_else(|| format!("{:?} has no column {item:?}", region.name))?;
            let (first, last) = region.data_rows().ok_or_else(|| format!("{:?} has no rows", region.name))?;
            // The rows that are exceptions of the column stay what they are.
            (first..=last).filter(|r| col.exception(*r).is_none()).map(|r| (r, col.col)).collect()
        }
    };
    if targets.is_empty() {
        return Ok(());
    }
    let first = match logic {
        Some(l) => Some(xl_formula::parse(&unstructured(l, all, region, targets[0].0, wb)?).map_err(|e| format!("{l:?}: {e:?}"))?),
        None => None,
    };
    let at: HashMap<(u32, u32, u32), usize> = (0..wb.cells.len()).map(|i| ((wb.cells.sheet[i], wb.cells.row[i], wb.cells.col[i]), i)).collect();
    for (r, col) in targets.iter() {
        let k = r - targets[0].0;
        let i = match at.get(&(s, *r, *col)) {
            Some(&i) => i,
            None => {
                wb.cells.push_raw(s, *r, *col, Kind::Blank, 0.0, None, None, 0);
                wb.cells.len() - 1
            }
        };
        wb.cells.formula[i] = first.as_ref().map(|e| e.shifted(k as i64, 0).to_string());
        wb.cells.array_ref[i] = None;
    }
    Ok(())
}

/// Bring a store up to date: logic changed in the model and the data of the Parquet
/// files go into the workbook, every formula is calculated, and workbook, Parquet
/// files and model are written anew.
pub fn run(b: &dyn Backend, dir: &Path) -> Result<RunReport, String> {
    run_with(b, dir, &[])
}

/// Set cells of a store's workbook as typed into them (see [`crate::apply::set_cells`]):
/// `cells` are (sheet, address, input). The store is run with them ([`run`]), so data,
/// model and workbook agree afterwards; what the cells mean for the regions is found anew.
pub fn set_cells(b: &dyn Backend, dir: &Path, cells: &[(String, String, J)]) -> Result<RunReport, String> {
    run_with(b, dir, cells)
}

fn run_with(b: &dyn Backend, dir: &Path, cells: &[(String, String, J)]) -> Result<RunReport, String> {
    let wanted = read_model(dir)?;
    let (mut wb, bytes) = read_workbook(dir)?;
    let mut report = RunReport::default();
    // 1. Logic the model states differently from the workbook.
    let tables = known(&wanted, &wb);
    let found = regions_like(&wb, &tables);
    let empty = Vec::new();
    for m in wanted["regions"].as_array().unwrap_or(&empty) {
        let Some(name) = m["name"].as_str() else { continue };
        let region = found.iter().find(|r| r.name == name).ok_or_else(|| format!("{MODEL} names a region {name:?} the workbook does not have; its regions are {:?} (regions cannot be added or renamed in the model)", found.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()))?;
        let items = m["columns"].as_array().or(m["cells"].as_array()).unwrap_or(&empty);
        for it in items {
            let Some(item) = it["name"].as_str() else { continue };
            let have: Option<(Option<&str>, Option<&str>)> = match region.kind {
                RegionKind::Cells => region.cells.iter().find(|c| c.name == item).map(|c| (c.logic.as_deref(), c.formula.as_deref())),
                _ => region.columns.iter().find(|c| c.name == item).map(|c| (c.logic.as_deref(), c.formula.as_deref())),
            };
            let Some((logic, formula)) = have else {
                return Err(format!("{MODEL}: {name:?} has no column or cell {item:?} in the workbook (columns and cells cannot be added or renamed in the model)"));
            };
            let want = it["logic"].as_str().map(|l| l.trim().trim_start_matches('=')).filter(|l| !l.is_empty());
            if want != logic.or(formula) {
                set_logic(&mut wb, &found, region, item, want).map_err(|why| format!("{MODEL}, {name}[{item}]: {why}"))?;
                report.logic_changed.push((name.to_string(), item.to_string()));
            }
        }
    }
    // 2. The data: what is not calculated, from the Parquet files.
    let mut items = Vec::new();
    for region in regions_like(&wb, &tables) {
        let path = dir.join(file_name(&region.name));
        if path.exists() {
            let df = read_parquet(b, &path)?;
            let df = if region.kind == RegionKind::Cells { wide(df).map_err(|why| format!("{}: {why}", region.name))? } else { df };
            items.push((region, df));
        }
    }
    // Where the regions lie afterwards: they stay what they are, by name and by kind.
    let tables: Vec<Known> = crate::apply::write_regions(&mut wb, &items)?.iter().map(Known::from).collect();
    let mut typed = Vec::new();
    for (sheet, at, input) in cells {
        let s = wb.sheets.iter().position(|x| x.name.eq_ignore_ascii_case(sheet)).ok_or_else(|| format!("no sheet named {sheet:?}; the sheets are {:?}", wb.sheets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()))?;
        let cell = xl_formula::parse_cell_ref(&at.replace('$', "")).ok_or_else(|| format!("{at:?} is not the address of a cell; write it like B7"))?;
        if let Some(f) = input.as_str().and_then(|t| t.strip_prefix('=')).filter(|f| !f.is_empty()) {
            xl_formula::parse(f).map_err(|e| format!("{sheet}!{at}: the formula ={f} cannot be read: {e:?}"))?;
        }
        typed.push((s as u32, cell.row - 1, cell.col - 1, input.clone()));
    }
    crate::apply::set_cells(&mut wb, &typed);
    // 3. Calculate, and write everything anew.
    let (done, calc) = crate::lift::calculate_with(b.executor(), &wb);
    report.formula_cells = calc.formula_cells;
    report.computed = calc.computed;
    report.not_computed = calc.not_computed.into_iter().collect();
    report.uncomputed = calc
        .uncomputed
        .iter()
        .take(50)
        .map(|(i, why)| {
            let c = &done.cells;
            let at = format!("{}!{}{}", done.sheets[c.sheet[*i] as usize].name, xl_formula::col_name(c.col[*i] + 1), c.row[*i] + 1);
            (at, c.formula[*i].clone().unwrap_or_default(), why.clone())
        })
        .collect();
    // What would be no region any more, and what would be one now: the sheet reads
    // differently (a table that touches what is beside it is read as one with it, rows
    // that come in tear apart what runs beside the table). With new data alone that is
    // not what was asked for, and the store stays as it is.
    let after = regions_like(&done, &tables);
    let before: Vec<&str> = wanted["regions"].as_array().into_iter().flatten().filter_map(|r| r["name"].as_str()).collect();
    report.regions_gone = before.iter().filter(|n| !after.iter().any(|r| r.name == **n)).map(|n| n.to_string()).collect();
    report.regions_new = after.iter().filter(|r| !before.contains(&r.name.as_str())).map(|r| r.name.clone()).collect();
    if cells.is_empty() && pending(dir) && !(report.regions_gone.is_empty() && report.regions_new.is_empty()) {
        return Err(format!("with this data the workbook would not be built as it was: its regions would be others (no more: {:?}; new: {:?}). A table that gets more or fewer rows moves what lies below it, and here that joins or tears apart what lies beside or below it. Nothing was changed; give the table another number of rows, or make room for it in the workbook", report.regions_gone, report.regions_new));
    }
    let out = xl_cells::update_bytes(bytes, &done).map_err(|e| e.to_string())?;
    report.regions = save(b, dir, &done, &out, &after)?;
    // What is no region any more (after cells were typed into) leaves no file behind.
    for gone in &report.regions_gone {
        let _ = fs::remove_file(dir.join(file_name(gone)));
    }
    let _ = fs::remove_file(dir.join(PENDING));
    Ok(report)
}

/// A copy of a sheet in the workbook `wb`, as its last sheet: cells with values, formulas
/// and formats, merged ranges, widths and heights, validations and conditional formats.
/// Excel tables, pictures and charts of the sheet are not copied. Returns its index.
pub fn copy_sheet(wb: &mut Workbook, from: u32, name: &str) -> u32 {
    let to = wb.sheets.len() as u32;
    let mut sheet = wb.sheets[from as usize].clone();
    sheet.name = name.to_string();
    sheet.origin = None;
    sheet.tables.clear();
    sheet.drawings.clear();
    // A code name is unique in a workbook.
    for (element, xml) in sheet.extra.iter_mut().filter(|(e, _)| e == "sheetPr") {
        let _ = element;
        if let Some(at) = xml.find(" codeName=\"") {
            if let Some(end) = xml[at + 11..].find('"') {
                xml.replace_range(at..at + 11 + end + 1, "");
            }
        }
    }
    wb.sheets.push(sheet);
    let c = &mut wb.cells;
    for i in (0..c.len()).filter(|&i| c.sheet[i] == from).collect::<Vec<_>>() {
        c.push_raw(to, c.row[i], c.col[i], c.kind[i], c.num[i], c.text[i].clone(), c.formula[i].clone(), c.style[i]);
        let n = c.len() - 1;
        c.float_text[n] = c.float_text[i];
        c.array_ref[n] = c.array_ref[i].clone();
        c.rich[n] = c.rich[i].clone();
        c.iso[n] = c.iso[i];
    }
    to
}

/// Add a sheet to a store's workbook as a copy of the sheet `like` (see [`copy_sheet`]):
/// one more sheet of a stack, with the layout and the formulas of the one copied and its
/// data, to be replaced. Changes not yet run stay as they are. Returns the regions of
/// the new sheet.
pub fn add_sheet(b: &dyn Backend, dir: &Path, like: &str, name: &str) -> Result<Vec<String>, String> {
    let (mut wb, bytes) = read_workbook(dir)?;
    let have: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    let from = have.iter().position(|s| s.eq_ignore_ascii_case(like)).ok_or_else(|| format!("no sheet named {like:?}; the sheets are {have:?}"))? as u32;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 31 || name.contains(['[', ']', ':', '*', '?', '/', '\\']) || name.starts_with('\'') || name.ends_with('\'') {
        return Err(format!("{name:?} cannot be the name of a sheet: 1 to 31 characters, none of [ ] : * ? / \\, no ' at either end"));
    }
    if have.iter().any(|s| s.eq_ignore_ascii_case(name)) {
        return Err(format!("there is a sheet named {name:?} already; the sheets are {have:?}"));
    }
    let tables = known(&read_model(dir)?, &wb);
    let before: HashSet<String> = regions_like(&wb, &tables).into_iter().map(|r| r.name).collect();
    let to = copy_sheet(&mut wb, from, name);
    let found = regions_like(&wb, &tables);
    if let Some(gone) = before.iter().find(|n| !found.iter().any(|r| r.name == **n)) {
        return Err(format!("with a sheet {name:?} the region {gone:?} would not be what it is"));
    }
    let out = xl_cells::update_bytes(bytes, &wb).map_err(|e| e.to_string())?;
    let path = dir.join(WORKBOOK);
    io(fs::write(&path, &out), "cannot write", &path)?;
    // The data and notes of the new sheet's regions; those of the others stay.
    let at = crate::frame::cell_index(&wb);
    let all = notes_of(b.executor(), &wb, &found);
    let mut kept = kept_notes(dir);
    let mut new = Vec::new();
    for (r, note) in found.iter().zip(&all).filter(|(r, _)| r.area.sheet == to) {
        let mut df = crate::regions::region_frame_in(&wb, &at, r).map_err(|e| e.to_string())?;
        if r.kind == RegionKind::Cells {
            let cells: Vec<(String, String)> = r.cells.iter().map(|c| (c.name.clone(), format!("{}{}", xl_formula::col_name(c.col + 1), c.row + 1))).collect();
            df = crate::long::to_long(&df, &cells);
        }
        write_parquet(b, &dir.join(file_name(&r.name)), &df, Some(note))?;
        kept.insert(file_name(&r.name), note.clone());
        new.push(r.name.clone());
    }
    let path = dir.join(NOTES);
    io(fs::write(&path, J::Object(kept).to_string()), "cannot write", &path)?;
    // The model: what it says of the regions it has stays (logic may be changed there).
    let mut wanted = read_model(dir)?;
    let fresh = model_of(&wb, &found);
    if let Some(list) = wanted["regions"].as_array_mut() {
        list.extend(fresh["regions"].as_array().into_iter().flatten().filter(|r| r["name"].as_str().is_some_and(|n| new.iter().any(|x| x == n))).cloned());
    }
    wanted["stacks"] = fresh["stacks"].clone();
    let path = dir.join(MODEL);
    io(fs::write(&path, serde_json::to_string_pretty(&wanted).unwrap_or_default()), "cannot write", &path)?;
    Ok(new)
}

/// Give a region of a store another name: its data, the notes and the logic that names it
/// follow (the store is run, before and after, so that nothing is left that says the old
/// name). The workbook is not changed by it: the name is the store's.
pub fn rename_region(b: &dyn Backend, dir: &Path, region: &str, name: &str) -> Result<RunReport, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(|c| c.is_control() || matches!(c, '[' | ']' | '!' | '\'' | '"')) {
        return Err(format!("{name:?} cannot be the name of a region: 1 to 60 characters, none of [ ] ! ' \""));
    }
    if pending(dir) {
        run(b, dir)?;
    }
    let mut model = read_model(dir)?;
    let taken = |list: &J| list.as_array().into_iter().flatten().any(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name) || file_name(n).eq_ignore_ascii_case(&file_name(name))));
    if taken(&model["regions"]) || taken(&model["stacks"]) {
        return Err(format!("there is a region or stack named {name:?} already"));
    }
    // The model anew, from the workbook read with the new name: the logic of other
    // regions that names this one says the new name too.
    let (wb, _) = read_workbook(dir)?;
    let mut hints = known(&model, &wb);
    let names: Vec<String> = hints.iter().map(|k| k.name.clone()).collect();
    hints.iter_mut().find(|k| k.name == region).ok_or_else(|| format!("no region named {region:?} (a stack is named after its tables); the regions are {names:?}"))?.name = name.to_string();
    model = model_of(&wb, &regions_like(&wb, &hints));
    let (from, to) = (dir.join(file_name(region)), dir.join(file_name(name)));
    io(fs::rename(&from, &to), "cannot rename", &from)?;
    let path = dir.join(MODEL);
    io(fs::write(&path, serde_json::to_string_pretty(&model).unwrap_or_default()), "cannot write", &path)?;
    run(b, dir)
}

/// Calculate the data of Parquet files again from their notes, without a workbook: every
/// calculated column and cell that has a plan, in the order of what reads what; the
/// files are written anew. `files` are (path, note): all regions the plans read must be
/// among them. What has no plan keeps its values and is named in the result.
pub fn recalculate_files(b: &dyn Backend, files: &[(PathBuf, J)]) -> Result<crate::plans::Recalculation, String> {
    let notes: Vec<J> = files.iter().map(|(_, n)| n.clone()).collect();
    // Single cells are rows in the files, and one row with a column per cell to the plans.
    let mut frames = Vec::new();
    let mut rows: Vec<bool> = Vec::new();
    for (path, note) in files {
        let df = read_parquet(b, path)?;
        rows.push(note["kind"] == "cells" && crate::long::is_long(&df));
        frames.push(if note["kind"] == "cells" { wide(df).map_err(|why| format!("{}: {why}", path.display()))? } else { df });
    }
    let report = crate::plans::recalculate(b.executor(), &notes, &mut frames)?;
    let changed: HashSet<&str> = report.done.iter().map(|(r, _)| r.as_str()).collect();
    for (((path, note), frame), as_rows) in files.iter().zip(&frames).zip(rows) {
        if note["region"].as_str().is_some_and(|r| changed.contains(r)) {
            if as_rows {
                let cells: Vec<(String, String)> = note["columns"].as_array().into_iter().flatten().filter_map(|c| Some((c["name"].as_str()?.to_string(), c["cell"].as_str().unwrap_or("").to_string()))).collect();
                write_parquet(b, path, &crate::long::to_long(frame, &cells), Some(note))?;
            } else {
                write_parquet(b, path, frame, Some(note))?;
            }
        }
    }
    Ok(report)
}

/// The Parquet files of a store, or of a plain directory, that have a note, with it
/// (see [`recalculate_files`]).
pub fn noted_files(b: &dyn Backend, dir: &Path) -> Result<Vec<(PathBuf, J)>, String> {
    let kept = kept_notes(dir);
    // A store has its files in `data`; a plain directory holds them itself.
    let sub = if dir.join("data").is_dir() { "data/" } else { "" };
    let data = dir.join(sub);
    let mut names: Vec<String> = kept.keys().cloned().collect();
    let mut others: Vec<String> = io(fs::read_dir(&data), "cannot read", &data)?.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".parquet")).map(|n| format!("{sub}{n}")).filter(|n| !kept.contains_key(n)).collect();
    others.sort();
    names.extend(others);
    Ok(names.into_iter().filter(|n| dir.join(n).exists()).filter_map(|n| note_of(b, dir, &n, &kept).map(|note| (dir.join(&n), note))).collect())
}

/// Calculate the data of a store again from the notes of its Parquet files (see
/// [`recalculate_files`]): the calculation of the workbook, on the data alone. The
/// workbook of the store is not touched; [`run`] brings it in line.
pub fn recalculate(b: &dyn Backend, dir: &Path) -> Result<crate::plans::Recalculation, String> {
    let report = recalculate_files(b, &noted_files(b, dir)?)?;
    mark_pending(dir);
    Ok(report)
}

/// Change the logic of a column or cell in the model document of a store (`None`:
/// plain data again). [`run`] carries it into the workbook.
pub fn set_model_logic(dir: &Path, region: &str, name: &str, logic: Option<&str>) -> Result<(), String> {
    // A stack: the column of every sheet's table.
    if let Some(stack) = stack_named(dir, region) {
        for m in &stack.members {
            set_model_logic(dir, &m.region, name, logic)?;
        }
        return Ok(());
    }
    let mut model = read_model(dir)?;
    let names: Vec<String> = model["regions"].as_array().map(|a| a.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    let regions = model["regions"].as_array_mut().ok_or("the model has no regions")?;
    let r = regions.iter_mut().find(|r| r["name"] == region).ok_or_else(|| format!("no region named {region:?}; the regions are {names:?}"))?;
    let key = if r["cells"].is_array() { "cells" } else { "columns" };
    let items = r[key].as_array_mut().ok_or_else(|| format!("{region:?} has no {key}"))?;
    let have: Vec<String> = items.iter().filter_map(|i| i["name"].as_str().map(str::to_string)).collect();
    let item = items.iter_mut().find(|i| i["name"] == name).ok_or_else(|| format!("{region:?} has no column or cell {name:?}; it has {have:?}"))?;
    match (logic, item.as_object_mut()) {
        (Some(l), Some(o)) => {
            o.insert("logic".into(), json!(l.trim().trim_start_matches('=')));
        }
        (None, Some(o)) => {
            o.remove("logic");
        }
        _ => {}
    }
    let path = dir.join(MODEL);
    io(fs::write(&path, serde_json::to_string_pretty(&model).unwrap_or_default()), "cannot write", &path)?;
    mark_pending(dir);
    Ok(())
}

