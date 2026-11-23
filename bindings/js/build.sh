#!/bin/sh
# Build the WebAssembly core and its JavaScript glue for Node.js and for browsers.
# Needs: rustup target add wasm32-unknown-unknown; cargo install wasm-bindgen-cli --version 0.2.130
set -e
cd "$(dirname "$0")"
cargo build --release -p xlcells-js --target wasm32-unknown-unknown
WASM=../../target/wasm32-unknown-unknown/release/xlcells.wasm
rm -rf pkg
wasm-bindgen --target nodejs --out-dir pkg/node "$WASM"
wasm-bindgen --target web --out-dir pkg/web "$WASM"
echo '{"type":"commonjs"}' > pkg/node/package.json
