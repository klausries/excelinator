//! A store as a Python project on Polars: a *port* of the workbook.
//!
//! ```text
//! data/<region>.parquet      the inputs (what is not calculated)
//! expected/<region>.parquet  what the workbook calculates: the measure of the port
//! workbook.xlsx              the workbook (the way back; xlcells' engine for the rest)
//! cells.json                 the cells of the sheets, for formulas that read cells outside regions
//! logic.json                 the regions, their inputs, every calculated column and cell
//! port/calc.py               the program: calculate(folder) gives every region as a frame
//! port/xl.py                 what it needs besides Polars (the files above)
//! run.py, check.py, tests/   run it, check it against expected/, its tests
//! README.md, pyproject.toml
//! ```
//!
//! The program is that of [`codegen::polars_program`](crate::codegen::polars_program),
//! one translator for both: it was written against xlcells' interface, and `port/xl.py`
//! serves those names from the project's files with Polars alone. Every calculated
//! column and cell is ported as `polars` (readable), `plan` (the engine's translation
//! written out) or `rest` (not yet: xlcells' engine where it is installed, else the
//! values of `expected/`). The project runs with Polars alone from the start; what is
//! left is named, and `check.py` measures it.

use crate::codegen::{polars_program_with, Accuracy};
use crate::data::Frame;
use crate::regions::{region_frame, regions, RegionKind};
use crate::store::{self, Backend};
use serde_json::{json, Value as J};
use std::fs;
use std::path::Path;
use xl_cells::Kind;

const XL: &str = include_str!("../py/xl.py");
const CHECK: &str = include_str!("../py/check.py");
const RUN: &str = include_str!("../py/run.py");
const TEST: &str = include_str!("../py/test_port.py");
const README: &str = include_str!("../py/README.md");
const PYPROJECT: &str = include_str!("../py/pyproject.toml");
const GITIGNORE: &str = include_str!("../py/gitignore");

/// How a calculated column or cell is ported.
#[derive(Debug, Clone, PartialEq)]
pub struct PortItem {
    pub region: String,
    pub name: String,
    pub formula: String,
    pub logic: Option<String>,
    /// `polars`, `plan` or `rest`.
    pub how: &'static str,
    /// Why not `polars`.
    pub reason: Option<String>,
    /// Its line in `port/calc.py`.
    pub line: usize,
    /// Its value depends on the moment or on chance (TODAY, NOW, RAND …): no value to
    /// compare it with.
    pub volatile: bool,
}

/// Whether a formula calls a function whose value depends on the moment or on chance.
fn volatile(formula: &str) -> bool {
    let upper = formula.to_ascii_uppercase();
    ["TODAY(", "NOW(", "RAND(", "RANDBETWEEN(", "RANDARRAY("].iter().any(|f| upper.match_indices(f).any(|(i, _)| i == 0 || !upper.as_bytes()[i - 1].is_ascii_alphanumeric() && upper.as_bytes()[i - 1] != b'.'))
}

/// What [`write`] made.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub items: Vec<PortItem>,
    pub regions: usize,
    /// Columns and cells whose first form gave other values than the workbook (found
    /// by running the project's check) and were written another way.
    pub replaced: usize,
    /// The files written, relative to the project's folder.
    pub files: Vec<String>,
}

impl Port {
    /// How far the port is, in a sentence.
    pub fn summary(&self) -> String {
        let n = |how: &str| self.items.iter().filter(|i| i.how == how).count();
        let replaced = if self.replaced > 0 { format!(" The project's check ran: {} gave other values than the workbook at first and are written another way.", self.replaced) } else { String::new() };
        format!(
            "{} calculated columns and cells: {} worked out by Polars alone ({} readable, {} the engine's translation written out), {} not ported yet (rest).{replaced}",
            self.items.len(),
            n("polars") + n("plan"),
            n("polars"),
            n("plan"),
            n("rest")
        )
    }
}

/// The file of a region in data/ and expected/ (a name every system takes).
fn file_name(region: &str) -> String {
    let safe: String = region.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    format!("{}.parquet", safe.trim())
}

fn project_name(title: &str) -> String {
    let mut s: String = title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("port-{s}")
    } else {
        s
    }
}

/// The cells of the sheets as `calc.py` reads cells outside regions: (row, column,
/// value), 0-based, a formula as `=...` (as openpyxl gives it).
fn cells_json(wb: &xl_cells::Workbook) -> J {
    let mut sheets = serde_json::Map::new();
    for s in &wb.sheets {
        sheets.insert(s.name.clone(), J::Array(Vec::new()));
    }
    let c = &wb.cells;
    for i in 0..c.len() {
        let value = match (&c.formula[i], c.kind[i]) {
            (Some(f), _) if !f.is_empty() => json!(format!("={f}")),
            (_, Kind::Blank) => continue,
            (_, Kind::Number) if c.num[i].fract() == 0.0 && c.num[i].abs() < 1e15 && !c.float_text[i] => json!(c.num[i] as i64),
            (_, Kind::Number) => json!(c.num[i]),
            (_, Kind::Bool) => json!(c.num[i] != 0.0),
            (_, Kind::Text | Kind::Error) => json!(c.text[i].as_deref().unwrap_or("")),
        };
        if let Some(J::Array(list)) = wb.sheets.get(c.sheet[i] as usize).and_then(|s| sheets.get_mut(&s.name)) {
            list.push(json!([c.row[i], c.col[i], value]));
        }
    }
    J::Object(sheets)
}

/// The program of `codegen` as `port/calc.py`: its helpers as they are, its body in a
/// function `calculate(folder)` that gives the regions as frames. The lines of the
/// items are those of the new file.
fn calc_py(source: &str, items: &mut [PortItem], title: &str) -> Result<String, String> {
    let start = source.find("\nwb = xo.load_workbook(").ok_or("the program has no line that loads the workbook")? + 1;
    let check = source.find("\n# --- check: every value against the workbook ---\n").ok_or("the program has no check")?;
    let results_at = source[check..].find("results = {").ok_or("the program gives no results")? + check;
    let results_end = source[results_at..].find("\n}\n").ok_or("the program's results do not end")? + results_at + 3;
    let old_head_lines = source[..start].lines().count();
    // The helpers, without the program's own docstring; xlcells' names served by port/xl.py.
    let mut head = source[..start].to_string();
    if head.starts_with("\"\"\"") {
        if let Some(end) = head[3..].find("\"\"\"\n") {
            head = head[end + 7..].trim_start_matches('\n').to_string();
        }
    }
    let head = head.replace("import xlcells.openpyxl as xo", "from . import xl as xo");
    let mut out = format!(
        "\"\"\"{title} as a Polars program (written by xlcells; from here on yours to change).\n\n`calculate(folder)` reads the inputs of a folder (as data/) and gives every region of the\nworkbook as a frame, with its calculated columns and cells worked out in the order of\ntheir dependence. Each is marked with its logic in the workbook; check.py tells whether\nits values are right. A `rest` (see README.md) is not ported yet: replace its line\nwith Polars code of your own.\n\"\"\"\n"
    );
    out.push_str(&head);
    if !out.ends_with("\n\n\n") {
        out.push_str(if out.ends_with("\n\n") { "\n" } else { "\n\n" });
    }
    out.push_str("def calculate(folder=xo.HOME / \"data\"):\n    \"\"\"Every region as a frame, from the inputs in `folder`.\"\"\"\n");
    let prefix_lines = out.lines().count();
    let body = source[start..check].replacen("wb = xo.load_workbook(\"workbook.xlsx\")", "wb = xo.load_workbook(folder)", 1);
    let indent = |text: &str, out: &mut String| {
        for line in text.lines() {
            if !line.is_empty() {
                out.push_str("    ");
                out.push_str(line);
            }
            out.push('\n');
        }
    };
    indent(&body, &mut out);
    indent(&source[results_at..results_end], &mut out);
    out.push_str("    return results\n");
    for item in items.iter_mut() {
        item.line = item.line.saturating_sub(old_head_lines) + prefix_lines;
    }
    Ok(out)
}

/// A file of the project, its folder made where it is not there.
fn put(dest: &Path, files: &mut Vec<String>, name: &str, content: &[u8]) -> Result<(), String> {
    let path = dest.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    fs::write(&path, content).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    files.push(name.to_string());
    Ok(())
}

/// Runs the check of a project (its check.py, in Python) and gives the columns and
/// cells whose values differ from the workbook's: (region, name, how they were ported).
pub type Check<'a> = &'a dyn Fn(&Path) -> Result<Vec<(String, String, String)>, String>;

/// Write the port of a store into `dest` (made if it is not there). The store is run
/// first. With `check` (where Python can run the project) the project is checked as it
/// is written: a readable form whose values are not the workbook's is written as the
/// engine's translation, a translation that is not right either is left as `rest`. `dest` must be empty unless
/// `replace`: a port may hold work of its own.
pub fn write(b: &dyn Backend, dir: &Path, dest: &Path, accuracy: Accuracy, replace: bool, check: Option<Check>) -> Result<Port, String> {
    if dest.exists() && fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(false) {
        if !replace {
            return Err(format!("{} is not empty: a port may hold work of its own; name another folder, or replace it", dest.display()));
        }
        fs::remove_dir_all(dest).map_err(|e| format!("cannot replace {}: {e}", dest.display()))?;
    }
    // Run first: the measure is what the workbook calculates, as Excel does, from the
    // data of now (a file saved without calculated values has none of its own).
    store::run(b, dir)?;
    let (mut plan_only, mut engine) = (Vec::new(), Vec::new());
    let mut port = write_once(b, dir, dest, accuracy, &engine, &plan_only)?;
    let Some(check) = check else { return Ok(port) };
    for _ in 0..3 {
        let wrong = check(dest)?;
        if wrong.is_empty() {
            break;
        }
        for (region, name, how) in wrong {
            let key = (region, name);
            if how == "polars" && !plan_only.contains(&key) {
                plan_only.push(key);
            } else if !engine.contains(&key) {
                engine.push(key);
            }
        }
        fs::remove_dir_all(dest).map_err(|e| format!("cannot write {} anew: {e}", dest.display()))?;
        port = write_once(b, dir, dest, accuracy, &engine, &plan_only)?;
    }
    port.replaced = plan_only.len() + engine.iter().filter(|k| !plan_only.contains(k)).count();
    Ok(port)
}

/// The project, written once (into a folder without it).
fn write_once(b: &dyn Backend, dir: &Path, dest: &Path, accuracy: Accuracy, engine: &[(String, String)], plan_only: &[(String, String)]) -> Result<Port, String> {
    let (wb, bytes) = store::read_workbook(dir)?;
    let found = regions(&wb);
    let program = polars_program_with(&wb, "workbook.xlsx", accuracy, engine, plan_only, Some(b.executor()));
    let workbook_name = dir.file_name().map(|n| n.to_string_lossy().trim_end_matches(".store").to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "workbook.xlsx".into());
    let title = workbook_name.rsplit_once('.').map_or(workbook_name.as_str(), |x| x.0).to_string();
    let mut files = Vec::new();
    put(dest, &mut files, "workbook.xlsx", &bytes)?;
    // The regions: all of each as expected, its inputs as data.
    let mut regions_json = Vec::new();
    for r in &found {
        let full = region_frame(&wb, r)?;
        let calculated: Vec<&str> = match r.kind {
            RegionKind::Cells => r.cells.iter().filter(|c| c.formula.is_some()).map(|c| c.name.as_str()).collect(),
            _ => r.columns.iter().filter(|c| c.formula.is_some()).map(|c| c.name.as_str()).collect(),
        };
        let inputs = Frame::new(full.height, full.columns.iter().filter(|c| !calculated.contains(&c.name.as_str())).cloned().collect())?;
        let file = file_name(&r.name);
        for (folder, frame) in [("expected", &full), ("data", &inputs)] {
            let path = dest.join(folder).join(&file);
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("cannot make {}: {e}", dest.join(folder).display()))?;
            b.write_parquet(&path, frame, None)?;
            files.push(format!("{folder}/{file}"));
        }
        regions_json.push(json!({
            "name": r.name, "kind": r.kind.as_str(), "sheet": wb.sheets[r.area.sheet as usize].name, "ref": r.range(), "file": file,
            "inputs": inputs.columns.iter().map(|c| json!({ "name": c.name, "dtype": c.data.type_name() })).collect::<Vec<_>>(),
            "calculated": calculated,
        }));
    }
    let logic_of = |region: &str, name: &str| -> Option<String> {
        let r = found.iter().find(|r| r.name == region)?;
        r.columns.iter().find(|c| c.name == name).and_then(|c| c.logic.clone()).or_else(|| r.cells.iter().find(|c| c.name == name).and_then(|c| c.logic.clone()))
    };
    let mut items: Vec<PortItem> = program
        .items
        .iter()
        .map(|i| PortItem { region: i.region.clone(), name: i.name.clone(), formula: i.formula.clone(), logic: logic_of(&i.region, &i.name), how: if i.how == "engine" { "rest" } else { i.how }, reason: i.reason.clone(), line: i.line, volatile: volatile(&i.formula) })
        .collect();
    let calc = calc_py(&program.source, &mut items, &title)?;
    put(dest, &mut files, "port/__init__.py", b"\"\"\"The port: calc.py, the program; xl.py, what it needs besides Polars.\"\"\"\n")?;
    put(dest, &mut files, "port/calc.py", calc.as_bytes())?;
    put(dest, &mut files, "port/xl.py", XL.as_bytes())?;
    put(dest, &mut files, "check.py", CHECK.as_bytes())?;
    put(dest, &mut files, "run.py", RUN.as_bytes())?;
    put(dest, &mut files, "tests/test_port.py", TEST.as_bytes())?;
    put(dest, &mut files, ".gitignore", GITIGNORE.as_bytes())?;
    let item_json: Vec<J> = items.iter().map(|i| json!({ "region": i.region, "name": i.name, "formula": i.formula, "logic": i.logic, "how": i.how, "reason": i.reason, "line": i.line, "volatile": i.volatile })).collect();
    let logic = json!({ "workbook": "workbook.xlsx", "source": workbook_name, "date1904": wb.date1904, "regions": regions_json, "items": item_json });
    put(dest, &mut files, "logic.json", serde_json::to_string_pretty(&logic).map_err(|e| e.to_string())?.as_bytes())?;
    put(dest, &mut files, "cells.json", cells_json(&wb).to_string().as_bytes())?;
    let port = Port { items, regions: found.len(), replaced: 0, files: Vec::new() };
    let table: String = port
        .items
        .iter()
        .map(|i| format!("| {} | {} | `{}` | {} | line {} |\n", i.region, i.name, i.logic.as_deref().unwrap_or(&i.formula).replace('|', "\\|"), i.how, i.line))
        .collect();
    let readme = README.replace("@TITLE@", &title).replace("@WORKBOOK@", &workbook_name).replace("@SUMMARY@", &port.summary()).replace("@ITEMS@\n", &table);
    put(dest, &mut files, "README.md", readme.as_bytes())?;
    put(dest, &mut files, "pyproject.toml", PYPROJECT.replace("@NAME@", &project_name(&title)).replace("@TITLE@", &title).replace("@WORKBOOK@", &workbook_name).as_bytes())?;
    Ok(Port { files, ..port })
}

#[cfg(test)]
mod tests {
    #[test]
    fn functions_of_the_moment_are_known_by_name() {
        assert!(super::volatile("TODAY()"));
        assert!(super::volatile("A1+now()"));
        assert!(super::volatile("ROUND(RAND()*6,0)"));
        assert!(!super::volatile("MYTODAY()"));
        assert!(!super::volatile("SUM(Sales[amount])"));
    }
}
