#!/bin/sh
# Builds the app for the browser (app/web, Jaspr) and puts it where the server takes it
# from: crates/xl-mcp/web. The built files are kept in the repository, so that building
# the server needs no Dart. Run from the root of the repository.
set -e
log=$(mktemp)
(cd app/web && dart pub get >/dev/null && jaspr build) >"$log" 2>&1 || { tr '\r' '\n' <"$log" | tail -40; exit 1; }
for f in index.html app.css main.client.dart.js chat.html; do cp "app/web/build/jaspr/$f" "crates/xl-mcp/web/$f"; done
ls -l crates/xl-mcp/web
