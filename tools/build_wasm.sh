#!/bin/sh
# Builds the WebAssembly module of xlcells (bindings/wasm) and puts it into the Python
# package (bindings/python/python/xlcells/_wasm/xlcells.wasm). Run from the root of the
# repository. PROFILE=quick builds faster (for work on it), the default is release.
set -e
PROFILE=${PROFILE:-release}
cargo build --profile "$PROFILE" --target wasm32-wasip1 -p xl-wasm
cp "target/wasm32-wasip1/$PROFILE/xl_wasm.wasm" bindings/python/python/xlcells/_wasm/xlcells.wasm
ls -l bindings/python/python/xlcells/_wasm/xlcells.wasm
