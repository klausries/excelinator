#!/bin/sh
# Run the Python tests file by file (each in its own process: the property tests take a lot of memory).
cd "$(dirname "$0")/../bindings/python"
status=0
for f in tests/test_*.py; do
  out=$(timeout 1800 python3 -m pytest -q -rf "$f" 2>&1 | tail -3)
  echo "$f: $(echo "$out" | tail -1)"
  echo "$out" | grep -q "failed\|error" && { status=1; echo "$out"; }
done
exit $status
