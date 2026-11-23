# xlcells (pre-alpha)

Excel workbooks as tables, logic and data. xlcells reads and writes `.xlsx` and `.xls`
files, calculates their formulas as Excel does, finds the tables and single cells in a
sheet, and moves them into Polars and Parquet files that carry their logic with them,
and back into the workbook in its old layout.

Its engine is one WebAssembly module inside the package: it runs on any processor and
system where Python (3.11 or later), Polars and `wasmtime` run; nothing is compiled
when it is installed.

```sh
uv tool install xlcells        # the MCP server, as the program xlcells-mcp
uv pip install xlcells         # the library, in an environment
```

```python
import xlcells.openpyxl as xo

wb = xo.load_workbook("model.xlsx")         # the interface of openpyxl
wb.calculate()                              # every formula, as Excel calculates it
for name, region in wb.regions.items():     # tables and single cells found
    print(name, region.kind, region.to_polars())
store = wb.to_store("model.store")          # Parquet files with their logic
```

`xlcells-mcp` is a Model Context Protocol server through which an agent (Claude,
Microsoft Copilot) moves a workbook into tables and logic, works on it and brings it
back (`xlcells-mcp --app DIR` shows the same as an app in the browser).

This is a pre-alpha release: the interfaces will change. See the repository for the
handbook, what is checked and what is not.
