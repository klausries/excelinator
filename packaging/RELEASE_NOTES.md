Pre-alpha: the interfaces will change, and much is not checked yet (see below).

**What it is.** Excel workbooks as tables, logic and data. `xlcells` reads and writes
`.xlsx` and `.xls`, calculates formulas as Excel does, finds the tables and single
cells of a sheet, moves them into Polars and Parquet files that carry their logic, and
back into the workbook in its old layout. `xlcells-mcp` is a Model Context Protocol
server through which an agent (Claude, Microsoft Copilot) does this, and an app in the
browser or inside a chat.

**For every processor.** The engine is one WebAssembly module inside the Python
package, run by `wasmtime`; nothing is built on the machine that uses it.

**Files**

- `xlcells-…-py3-none-any.whl`: the Python package (Python 3.11 or later; needs
  `polars` and `wasmtime`). `uv tool install <wheel>` gives the program `xlcells-mcp`;
  `uv pip install <wheel>` the library.
- `excelinator-…-windows-x64.zip`, `…-windows-arm64.zip`: unpack and use, no
  installation and no administrator rights: Python, the packages and uv are inside;
  `README.txt` says how to use it with Claude Desktop.
- `excel-port-skill.zip`: the guide for agents that port a workbook to Python (a skill
  to upload into Claude; over MCP the tool `port_guide` gives the same).
- `SHA256SUMS`: checksums of the files.

**Checked for this release** (by the release workflow): the folder for Windows x64 and
ARM64 as it is unpacked, compiled and in wasmtime's interpreter; the wheel installed with
uv on Windows, Linux (x64, ARM64) and macOS; the Python tests against the wheel on
Linux (in CI).

**Not checked yet**: Microsoft Copilot; Claude Desktop showing the app inside the
chat; workbooks from Excel with Excel's own recalculation on Windows; machines behind a
proxy that blocks PyPI (the Windows folder needs no network).
