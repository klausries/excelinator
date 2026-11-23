---
name: excel-port
description: Port an Excel workbook (.xlsx, .xls) to a Python program on Polars that is checked against the workbook, and grow it into a small application with the way back to Excel. Use when asked to port, migrate, replace or rebuild a spreadsheet as Python, to move its logic into a program or database application, or to check that such a program calculates what the workbook does. Needs the xlcells MCP server (xlcells-mcp) or the Python package xlcells.
---

# Porting a workbook to Python and Polars

The result is a Python project whose program works out what the workbook calculates,
proved against the workbook value by value, with Polars alone, and that can put its
results back into the workbook. xlcells makes the first version; your work is to finish
the port and build the application around it. Never claim more than `check.py` shows.

## The way

1. **Open the workbook** with the MCP tool `open_workbook` (a path, or a name from
   `list_workbooks`). Read `describe` (all regions) and `describe` with each `region`:
   tables, single cells and blocks, their columns and the logic of every calculated one
   (`[@qty]*[@price]` is a column of the same row, `Orders[amount]` a whole column,
   `Tax_rate` a single cell). Rows listed under a column as "but D4: …" are exceptions.
2. **Make the project** with `export_port` (the store from step 1). It writes a folder
   (and, on a remote server, a zip with a download link: fetch and unpack it where you
   run Python). Without the MCP server: `xlcells.openpyxl.load_workbook(path).to_store(dir).to_port(folder)`.
3. **Check it at once**: in the project, `uv run check.py` (or `python check.py`; it
   needs Polars 2.x only). It reports how every calculated column and cell is ported and
   whether its values are right, and writes `status.json`:
   * `polars`: readable Polars code, worked out by the program;
   * `plan`: the formula engine's translation written out as Polars code: exact, but
     hard to read (make it readable if you like, `check.py` tells whether you kept it right);
   * `rest`: not ported yet; its values come from `expected/` (or from xlcells' engine
     where xlcells is installed). This is your work.
4. **Port every rest**, one at a time. Its line is in `port/calc.py` (check.py prints
   it): replace the statement that takes `exact[...]` with your own Polars code, run
   `check.py`, and go on only when it says 0 different and the item is no longer "not
   worked out". Then set its `"how"` in `logic.json` to `"polars"`. A value that depends
   on the moment (TODAY, NOW, RAND) is never compared: port it with what it means
   (`reference/excel-to-polars.md`). If you cannot port one, say so and why.
5. **Build the application** the person asked for around `port.calc.calculate(folder)`:
   reading new data (CSV, Parquet, a database) into the form of `data/`
   (`port.xl.read_inputs` checks it), the outputs, a command line, tests in `tests/`
   (pytest). Keep `calculate` and `check.py` working.
6. **Prove it on other data**, not only the workbook's own: change inputs (more rows,
   other values, edge cases such as empty cells and zero) and compare with the formula
   engine: `uv run --with xlcells check.py --exact --data other/` (xlcells installs from
   a wheel or PyPI; it runs anywhere, its engine is WebAssembly). Where xlcells cannot be
   installed: write the inputs into the store with `write_region`, call `run`, and
   compare your results with `read_region`.
7. **Show the way back**: `uv run --with xlcells run.py --excel back.xlsx` writes the
   results into the workbook in its old layout (or `write_region` + `export_workbook`).
8. **Report**: how far the port is (from `status.json`: standalone, right, different,
   not worked out), what you checked on which data, what is left and why, and the files.

## Rules

* Never change `expected/`, `cells.json` or the tolerance in `port/xl.py` (`same`):
  they are the measure. A port that only passes because the measure moved is wrong.
* Polars 2.x only; no pandas.
* Keep `calculate(folder)` and its result (a frame per region, the workbook's columns),
  and the comment line above each column and cell (`# Region[name]  =logic`, a cell
  `# Region: name`): check.py and the app find each one there.
* Excel's results are the truth, even where they look odd (rounding, empty cells as 0,
  text compared without case): port the behaviour, not your idea of it.
* Do not hide a rest: a value taken from `expected/` is not a port.

More: `reference/layout.md` (the files of the project), `reference/excel-to-polars.md`
(how Excel's behaviour looks in Polars), `reference/hosts.md` (Claude, Copilot, a
laptop). Over MCP the same texts come from the tool `port_guide`.
