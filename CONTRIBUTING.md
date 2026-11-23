# Contributing to excelinator

Thank you for helping. excelinator is pre-alpha: interfaces change, and a short issue
before a large pull request saves both of us work.

## Reporting a problem

The most useful report is a workbook. If you can share one (or a cut-down copy without
private data) that is read, calculated or written back wrongly, attach it to the issue
with what you expected and what you got. Never attach a workbook with personal or
confidential data.

## Building and testing

You need [Rust](https://rustup.rs) (with the target `wasm32-wasip1`), [uv](https://docs.astral.sh/uv/),
and for the app [Dart](https://dart.dev) / [Flutter](https://flutter.dev).

```sh
uv sync                                              # Python packages into .venv
cargo test --profile quick -p xl-engine -p xl-mcp    # Rust tests
uv run pytest bindings/python/tests                  # Python tests (native engine)
sh tools/build_wasm.sh                               # the WebAssembly module
XLCELLS_ENGINE=wasm uv run pytest bindings/python/tests   # the same tests on the module
(cd app/core && dart test)                           # the app's flows (needs target/quick/xl-mcp)
(cd app/web && dart test -p chrome)                  # the app's page in a headless Chrome
uv run python tools/build_release.py                 # a release into dist/
```

Tests that need the corpus of real workbooks (`tools/fetch_corpus.sh`, several GB) are
skipped without it.

## Style

- Every feature lives in Rust and is reachable from Python through the openpyxl-like
  facade, and from agents through the MCP server.
- No pandas; dataframes are Polars or Arrow.
- Tests describe whole flows (a workbook opened, changed, checked, written back) rather
  than single functions where they can.
- Comments and identifiers are English; the handbook is German.

## License

By contributing you agree that your contribution is licensed under MIT OR Apache-2.0,
like the rest of the project.
