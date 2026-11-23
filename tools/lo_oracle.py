#!/usr/bin/env python3
"""LibreOffice as oracle: load each spreadsheet, recalculate everything, save as xlsx.

The saved file holds the formulas plus the values LibreOffice computed. The
recalculation check then compares an engine against these cached values.

LibreOffice is driven by a Basic macro (see lo_basic.py): any Python 3 runs this.

usage: lo_oracle.py <input-root> <out-dir> [ext,ext,...] [every-nth]
"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lo_basic  # noqa: E402


def main():
    root, out = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    exts = tuple("." + e for e in (sys.argv[3] if len(sys.argv) > 3 else "xls").split(","))
    files = []
    for d, _, names in os.walk(root):
        if "/lo-" in d + "/":
            continue
        files += [os.path.join(d, n) for n in sorted(names) if n.lower().endswith(exts)]
    files.sort()
    step = int(sys.argv[4]) if len(sys.argv) > 4 else 1
    files = files[::step]
    os.makedirs(out, exist_ok=True)
    statuses = tempfile.mkdtemp(prefix="lo_oracle_status_")
    items = []
    for k, path in enumerate(files):
        target = os.path.join(out, os.path.relpath(path, root) + ".xlsx")
        os.makedirs(os.path.dirname(target), exist_ok=True)
        items.append((path, target, os.path.join(statuses, "%d.status" % k)))
    result = lo_basic.run("Oracle", items)
    ok = failed = 0
    with open(os.path.join(out, "lo_oracle.tsv"), "w") as log:
        for path, _target, status_file in items:
            status = result.get(status_file, "error: no status").replace("\n", " ")[:160]
            ok += status == "ok"
            failed += status != "ok"
            log.write("%s\t%s\n" % (status, os.path.relpath(path, root)))
    print("converted %d, failed %d" % (ok, failed))


if __name__ == "__main__":
    main()
