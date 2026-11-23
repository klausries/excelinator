#!/usr/bin/env python3
"""xlcells.openpyxl against openpyxl itself: load every corpus workbook with both and
compare sheets, cell values and types, number formats, bold/italic, merged ranges
and the sheet extent, with and without data_only.

usage: check_openpyxl_facade.py <corpus-dir> [every-nth] [max-bytes]
"""
import collections
import datetime
import os
import sys
import warnings

import openpyxl

import xlcells.openpyxl as facade

warnings.simplefilter("ignore")
root = sys.argv[1]
step = int(sys.argv[2]) if len(sys.argv) > 2 else 1
max_bytes = int(sys.argv[3]) if len(sys.argv) > 3 else 1500000


def corpus_files():
    out = []
    for d, _, names in os.walk(root):
        if "/src-" not in d + "/" or "src-enron" in d:
            continue
        out += [os.path.join(d, n) for n in names if n.lower().endswith((".xlsx", ".xlsm")) and os.path.getsize(os.path.join(d, n)) <= max_bytes]
    return sorted(out)[::step]


import re
import xlcells.pyexcelerator


def _unescape(s):
    return re.sub(r"_x([0-9A-Fa-f]{4})_", lambda m: chr(int(m.group(1), 16)), s)


def plain(v):
    """Formula objects of openpyxl (array and data table formulas) by their text."""
    if isinstance(v, (str, int, float, bool, datetime.datetime, datetime.date, datetime.time, datetime.timedelta)) or v is None:
        return v
    if type(v).__name__ == "ArrayFormula":
        return ("array", v.ref, v.text)
    return ("object", type(v).__name__)


def main():
    stats = collections.Counter()
    examples = collections.defaultdict(list)

    def diff(kind, msg):
        stats[kind] += 1
        if len(examples[kind]) < 6 and not any(e.split(" ")[0] == msg.split(" ")[0] for e in examples[kind]):
            examples[kind].append(msg)

    for path in corpus_files():
        short = os.path.basename(path)
        for data_only in (False, True):
            try:
                ref = openpyxl.load_workbook(path, data_only=data_only)
            except Exception:
                stats["openpyxl cannot load"] += 1
                continue
            try:
                got = facade.load_workbook(path, data_only=data_only)
            except Exception as e:
                diff("only we cannot load", f"{short}: {e}")
                continue
            stats["workbooks"] += 1
            ours = {ws.title: ws for ws in got.worksheets}
            for rws in ref.worksheets:
                ows = ours.get(rws.title)
                if ows is None:
                    diff("sheet missing", f"{short}: {rws.title!r}")
                    continue
                stats["sheets"] += 1
                if sorted(str(m) for m in rws.merged_cells.ranges) != sorted(str(m) for m in ows.merged_cells.ranges):
                    diff("merged ranges", f"{short} {rws.title!r}")
                rcells = {k: c for k, c in rws._cells.items() if c.value is not None}
                ocells = {k: c for k, c in ows._cells.items() if c.value is not None}
                for at in rcells.keys() | ocells.keys():
                    r, o = rcells.get(at), ocells.get(at)
                    stats["cells"] += 1
                    if r is None or o is None:
                        diff("cell only in one", f"{short} {rws.title!r}{at}: openpyxl {getattr(r, 'value', None)!r}, ours {getattr(o, 'value', None)!r}")
                        continue
                    rv, ov = plain(r.value), plain(o.value)
                    if isinstance(rv, tuple) and rv[0] == "array":
                        if rv != ov:
                            diff("array formula", f"{short} {rws.title!r}{at}: openpyxl {rv!r}, ours {ov!r}")
                    elif isinstance(rv, tuple):
                        stats["formula objects (array, data table)"] += 1
                    elif rv != ov and isinstance(rv, str) and isinstance(ov, str) and "_x" in rv and xlcells.pyexcelerator is not None and _unescape(rv) == ov:
                        stats["escapes openpyxl leaves undecoded (_xHHHH_)"] += 1
                    elif rv != ov and isinstance(rv, str) and isinstance(ov, str) and rv.startswith("=") and "".join(rv.split()) == "".join(ov.split()):
                        diff("formula whitespace", f"{short} {rws.title!r}{at}: openpyxl {rv!r}, ours {ov!r}")
                    elif rv == "=":
                        stats["empty formula element (openpyxl: '=')"] += 1
                    elif type(rv) is not type(ov) or rv != ov:
                        kind = "formula text" if isinstance(rv, str) and rv.startswith("=") else "date conversion" if isinstance(rv, (datetime.datetime, datetime.time, datetime.timedelta)) or isinstance(ov, (datetime.datetime, datetime.time, datetime.timedelta)) else "int/float" if isinstance(rv, (int, float)) and isinstance(ov, (int, float)) and rv == ov else "value"
                        diff(kind, f"{short} {rws.title!r}{at}: openpyxl {rv!r}, ours {ov!r} (format {r.number_format!r})")
                    if r.number_format != o.number_format:
                        diff("number format", f"{short} {rws.title!r}{at}: {r.number_format!r} != {o.number_format!r}")
                    if (bool(r.font.b), bool(r.font.i)) != (bool(o.font.b), bool(o.font.i)):
                        diff("bold/italic", f"{short} {rws.title!r}{at}")
                    if r.data_type != o.data_type and not isinstance(rv, tuple) and rv != "=":
                        diff("data_type", f"{short} {rws.title!r}{at}: {r.data_type} != {o.data_type} ({rv!r})")
                if (rws.max_row, rws.max_column) != (ows.max_row, ows.max_column):
                    diff("extent", f"{short} {rws.title!r}: {(rws.max_row, rws.max_column)} != {(ows.max_row, ows.max_column)}")
    for k, v in sorted(stats.items()):
        print(f"{v:>9}  {k}")
    for k, v in examples.items():
        for line in v:
            print(f"[{k}] {line[:260]}")
    return stats, examples


if __name__ == "__main__":
    main()
