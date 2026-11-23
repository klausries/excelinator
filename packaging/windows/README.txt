excelinator @VERSION@ (pre-alpha) for Windows (@ARCH@)
===========================================================

Nothing has to be built or installed: unpack this folder anywhere (your user folder
is fine, no administrator rights are needed) and use the .cmd files in it.

  xlcells-mcp.cmd          the MCP server (for Claude Desktop, see below)
  xlcells-app.cmd FOLDER   the app in the browser, on the Excel files of FOLDER
  xlcells-python.cmd       Python with xlcells, for scripts (xlcells-python.cmd my.py)

What is inside:

  python\    Python @PYTHON@ (the embeddable distribution of python.org)
  lib\       xlcells (its engine is WebAssembly: one module for every processor),
             Polars @POLARS@ and wasmtime @WASMTIME@, unpacked
  wheels\    xlcells as a wheel file, to install it into an environment of your own
  uv\        uv @UV@, the Python package manager (optional, see below)

Claude Desktop: Settings > Developer > Edit Config, and add to "mcpServers":

  "excelinator": {
    "command": "C:\\path\\to\\this\\folder\\python\\python.exe",
    "args": ["-X", "utf8", "-m", "xlcells.mcp"]
  }

(paths written with double backslashes), then restart Claude Desktop. Claude can then
open a workbook by its path ("open C:\Users\me\Documents\plan.xlsx") and show it as
an app.

Your own environment, with uv (preferred) or pip (Polars and wasmtime come from PyPI,
or from your organisation's mirror of it):

  uv\uv.exe venv C:\path\to\env
  uv\uv.exe pip install --python C:\path\to\env wheels\xlcells-@VERSION@-py3-none-any.whl
  or:  py -m pip install wheels\xlcells-@VERSION@-py3-none-any.whl

If a policy of the machine forbids code made at run time, the engine runs in the
interpreter of wasmtime instead (slower; it does so by itself, or set
XLCELLS_WASM_TARGET=pulley64). Its compiled code is kept in your user folder
(wasmtime's cache), so later starts are fast.

Licences: LICENSE-MIT, LICENSE-APACHE (excelinator); the packages carry theirs in
lib\*.dist-info, Python in python\LICENSE.txt, uv in uv\.
