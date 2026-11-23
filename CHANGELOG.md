# Changelog

## 0.1.0.dev1 (pre-alpha)

The first public version.

- Reading and writing `.xlsx` and `.xls`, with the interface of openpyxl (`xlcells.openpyxl`).
- A formula engine in Rust that calculates as Excel does; regions (tables, single cells,
  grids, sheets built alike) found in the sheets.
- Stores: Parquet files that carry their logic, recalculation without the workbook, the
  way back into the workbook in its layout, undo.
- Ports: a workbook as a Python project on Polars, checked against the workbook
  (`export_port`, `store.to_port`), and a guide for agents (`skills/excel-port`).
- The MCP server (`xlcells-mcp`, `xl-mcp`) for agents on the same machine and elsewhere,
  with the app as an MCP App.
- The app in the browser, in chats and on the desktop (macOS, Apple Silicon).
- One Python wheel for every processor: the engine as WebAssembly, run by wasmtime;
  folders for Windows x64 and ARM64 that need no installation.
