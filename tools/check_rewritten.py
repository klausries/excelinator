#!/usr/bin/env python3
"""Do other implementations accept the files we write, and read the same values?

For every workbook in out/rewritten (made by `xlg rewrite` from the corpus): open it
with openpyxl and with python-calamine and compare all cell values with our reader.

usage: check_rewritten.py <dir> [max-files]
"""
import datetime
import io
import os
import sys
import warnings

import openpyxl
from python_calamine import CalamineWorkbook

from xlcells import pyexcelerator as px
import xlcells

warnings.simplefilter("ignore")
root = sys.argv[1]
limit = int(sys.argv[2]) if len(sys.argv) > 2 else 10**9
names = sorted(n for n in os.listdir(root) if n.endswith(".xlsx"))[:limit]
stats = dict(files=0, openpyxl_fail=0, openpyxl_cells=0, openpyxl_diff=0, calamine_fail=0, calamine_cells=0, calamine_diff=0)
examples = []


def note(msg):
    if len(examples) < 30:
        examples.append(msg)


def number(v):
    return float(v) if isinstance(v, (int, float)) and not isinstance(v, bool) else v


for name in names:
    data = open(os.path.join(root, name), "rb").read()
    table = xlcells.read(data)
    ours = px.parse_xls(data)
    dates = {(s, r, c) for s, r, c, d in zip(table["sheet"], table["row"], table["col"], table["is_date"]) if d}
    formulas = {(s, r, c): f for s, r, c, f in zip(table["sheet"], table["row"], table["col"], table["formula"]) if f}
    stats["files"] += 1
    # openpyxl, values as stored (formulas show as formulas, so compare those as text)
    try:
        book = openpyxl.load_workbook(io.BytesIO(data))
        assert book.sheetnames == [n for n, _ in ours], "sheet names"
        for s, ws in enumerate(book.worksheets):
            seen = {}
            for cell in ws._cells.values():
                if cell.value is not None:
                    seen[(cell.row - 1, cell.column - 1)] = cell.value
            for at, v in seen.items():
                key = (s, *at)
                if key in formulas:
                    stats["openpyxl_cells"] += 1
                    shown = v if isinstance(v, str) else getattr(v, "text", None)
                    if shown is not None and shown != "=" + formulas[key]:
                        stats["openpyxl_diff"] += 1
                        note(f"{name} openpyxl formula {key}: {shown!r} != {formulas[key]!r}")
                    continue
                want = ours[s][1].get(at)
                stats["openpyxl_cells"] += 1
                if isinstance(v, (datetime.datetime, datetime.date, datetime.time, datetime.timedelta)):
                    ok = key in dates
                elif isinstance(want, str) and want.startswith("#") and isinstance(v, str):
                    ok = v == want
                else:
                    ok = number(v) == want and isinstance(v, bool) == isinstance(want, bool)
                if not ok:
                    stats["openpyxl_diff"] += 1
                    note(f"{name} openpyxl {key}: {v!r} != ours {want!r}")
            missing = [at for at in ours[s][1] if at not in seen and (s, *at) not in formulas and ours[s][1][at] != ""]
            if missing:
                stats["openpyxl_diff"] += len(missing)
                note(f"{name} openpyxl misses {len(missing)} cells, e.g. {missing[0]} = {ours[s][1][missing[0]]!r}")
    except Exception as e:
        stats["openpyxl_fail"] += 1
        note(f"{name} openpyxl cannot load: {type(e).__name__}: {str(e)[:120]}")
    # calamine, cached values
    try:
        cal = CalamineWorkbook.from_filelike(io.BytesIO(data))
        assert cal.sheet_names == [n for n, _ in ours], "sheet names"
        for s, (sheet_name, want) in enumerate(ours):
            if not want or max(r for r, _ in want) > 20000:
                continue
            rows = cal.get_sheet_by_index(s).to_python(skip_empty_area=False)
            for (r, c), v in want.items():
                h = rows[r][c] if r < len(rows) and c < len(rows[r]) else ""
                stats["calamine_cells"] += 1
                if isinstance(h, (datetime.datetime, datetime.date, datetime.time, datetime.timedelta)):
                    ok = (s, r, c) in dates
                elif isinstance(v, str):
                    ok = h == v or (isinstance(h, str) and h == v.strip()) or v.startswith("#")
                else:
                    ok = number(h) == v
                if not ok:
                    stats["calamine_diff"] += 1
                    note(f"{name} calamine ({s}, {r}, {c}): {h!r} != ours {v!r}")
    except Exception as e:
        stats["calamine_fail"] += 1
        note(f"{name} calamine cannot load: {type(e).__name__}: {str(e)[:120]}")

print(stats)
print("\n".join(examples))
