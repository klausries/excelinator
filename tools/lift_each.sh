#!/bin/sh
# Run lift_corpus on every workbook separately (memory-capped), two at a time; one output per file.
# usage: tools/lift_each.sh <corpus dir> <output dir>   (environment passed on, e.g. LIFT_MIN_RUN=1)
set -u
mkdir -p "$2"
find "$1" -name '*.xlsx' -size -3000k -print0 | sort -z | xargs -0 -P 2 -I{} sh -c '
  f="$1"; out="$2/$(echo "$f" | md5sum | cut -c1-16)"
  d=$(mktemp -d); cp "$f" "$d/"
  ( ulimit -v 2500000; POLARS_MAX_THREADS=1 LIFT_REASONS=1000 LIFT_EXAMPLES=50 timeout 300 ${LIFT_BIN:-./target/release/examples/lift_corpus} "$d" > "$out.txt" 2>"$out.err" ) || echo "FAILED $f" >> "$2/failed.txt"
  rm -rf "$d"' _ {} "$2"
