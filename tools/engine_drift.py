#!/usr/bin/env python3
"""Do the two engines agree?  tools/engine_drift.py BASELINE.tsv [OUT.tsv]

BASELINE.tsv is what `cargo run --release -p xl-polars --example baseline` wrote: every
formula value of a set of workbooks removed and calculated again by the engine on the
Polars built into Rust. This runs the same calculation with `xlcells-engine` (the engine
on the Polars of this Python environment) for the same files and compares cell by cell.

Reported: cells with the same bits, numbers that differ by at most 1e-13 of their size
(the last bits: two versions of Polars do not round every operation alike), numbers
that differ more, and cells one engine calculated and the other did not. OUT.tsv gets
every differing line of both.
"""
import collections
import multiprocessing
import struct
import sys
import time


def number(value):
    return struct.unpack(">d", bytes.fromhex(value[2:]))[0] if value.startswith("n ") else None


def one(path):
    import xlcells_engine

    t = time.time()
    try:
        with open(path, "rb") as f:
            text = xlcells_engine.recalculation(f.read())
    except Exception as e:  # the file could not be read or the engine failed
        return path, None, "{0}: {1}".format(type(e).__name__, e), time.time() - t
    return path, text.splitlines(), None, time.time() - t


def main():
    baseline = collections.OrderedDict()
    skipped = 0
    with open(sys.argv[1], encoding="utf-8") as f:
        for line in f:
            name, rest = line.rstrip("\n").split("\t", 1)
            if rest in ("unreadable", "too large", "panic"):
                skipped += 1
                continue
            if rest.startswith(("lift\t", "program\t")):
                continue
            baseline.setdefault(name, []).append(rest)
    import xlcells_engine

    print("engine:", xlcells_engine.engine_info(), "files:", len(baseline), "left out by the baseline:", skipped)
    out = open(sys.argv[2], "w", encoding="utf-8") if len(sys.argv) > 2 else None
    count = collections.Counter()
    reasons = collections.Counter()
    worst = []
    seconds = 0.0
    start = time.time()
    with multiprocessing.Pool(max(1, multiprocessing.cpu_count() - 2)) as pool:
        for path, lines, error, took in pool.imap_unordered(one, list(baseline), chunksize=4):
            seconds += took
            want = baseline[path]
            if error:
                count["files failed"] += 1
                reasons[error[:100]] += 1
                continue
            count["files"] += 1
            if len(lines) != len(want):
                count["files with another number of cells"] += 1
                continue
            file_differs = False
            for mine, theirs in zip(lines[1:], want[1:]):
                count["cells"] += 1
                if mine == theirs:
                    count["same"] += 1
                    continue
                cell, value, why = (mine.split("\t") + ["", ""])[:3]
                _, value0, why0 = (theirs.split("\t") + ["", ""])[:3]
                a, b = number(value), number(value0)
                if a is not None and b is not None:
                    rel = abs(a - b) / max(abs(a), abs(b), 1e-300)
                    kind = "numbers differ in the last bits" if rel <= 1e-13 else "numbers differ more"
                    worst.append((rel, path, cell, a, b))
                elif value == "blank" and value0 != "blank":
                    kind = "only the built-in Polars calculated"
                    reasons[why[:100]] += 1
                elif value0 == "blank" and value != "blank":
                    kind = "only the environment's Polars calculated"
                elif value == value0:
                    kind = "same value, another reason"
                else:
                    kind = "values of different kinds"
                count[kind] += 1
                file_differs = True
                if out:
                    out.write("{0}\t{1}\t{2}\t{3}\t|\t{4}\t{5}\n".format(path, cell, value, why, value0, why0))
            if file_differs:
                count["files with any difference"] += 1
    print("wall seconds %.0f, engine seconds %.0f" % (time.time() - start, seconds))
    for k in ["files", "files failed", "files with another number of cells", "files with any difference", "cells", "same", "numbers differ in the last bits", "numbers differ more",
              "only the built-in Polars calculated", "only the environment's Polars calculated", "same value, another reason", "values of different kinds"]:
        print("%-45s %d" % (k, count[k]))
    for why, n in reasons.most_common(15):
        print("   %6d  %s" % (n, why))
    for rel, path, cell, a, b in sorted(worst, reverse=True)[:15]:
        print("   rel %.2e  %s %s  environment %r  built-in %r" % (rel, path[-50:], cell, a, b))


if __name__ == "__main__":
    main()
