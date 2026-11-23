#!/bin/sh
# Build all language bindings: C library (used by Dart), WebAssembly (JS/TS), Python wheel.
set -e
cd "$(dirname "$0")/.."
cargo build --release -p xlcells-c
bindings/js/build.sh
# Three wheels: xlcells (reading and writing), xlcells-engine (formula engine, dataframes,
# regions, stores, looks; on the Polars of the Python environment) and
# xlcells-engine-bundled (the same engine with Polars built in).
for d in bindings/python bindings/python-engine bindings/python-engine-bundled; do
  (cd "$d" && maturin build --release)
done
pip install --force-reinstall --no-deps target/wheels/xlcells-*.whl target/wheels/xlcells_engine-0*.whl target/wheels/xlcells_engine_bundled-0*.whl
