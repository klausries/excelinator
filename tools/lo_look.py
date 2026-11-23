#!/usr/bin/env python3
"""LibreOffice as oracle for the look of cells.

For each workbook: load it, read the formatting LibreOffice gives each sampled cell
(its cell properties: named style, cell format, theme colours resolved), export it as
HTML (where conditional formats are applied to the background and font colours), and
write both to <out>/<relative path>.json.

LibreOffice applies no table styles (as of 24.x); the comparison leaves those out.

LibreOffice is driven by a Basic macro (see lo_basic.py): any Python 3 runs this.

usage: lo_look.py <input-root> <out-dir> [every-nth] [max-files]
"""
import json
import os
import shutil
import sys
import tempfile
from html.parser import HTMLParser

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lo_basic  # noqa: E402

HORI = ["STANDARD", "LEFT", "CENTER", "RIGHT", "BLOCK", "REPEAT"]


def rgb(v):
    return None if v is None or v < 0 else "%06X" % (v & 0xFFFFFF)


def unesc(s):
    out, i = [], 0
    while i < len(s):
        if s[i] == "\\" and i + 1 < len(s):
            out.append({"t": "\t", "n": "\n", "r": "\r", "\\": "\\"}.get(s[i + 1], s[i + 1]))
            i += 2
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


def border(text):
    if not text:
        return None
    style, width, color = text.split(",")
    return [int(style), int(width), rgb(int(color))]


def cell_props(f):
    """The properties of a cell line of the macro (see lo_basic.CellLine), by name."""
    (font_name, font_size, bold, italic, underline, strike, font_color, escapement, back,
     left, right, top, bottom, diag_down, diag_up, hori, vert, wrap, rotate, indent, shrink, locked, hidden, num_format) = f
    p = {
        "font_name": unesc(font_name), "font_size": float(font_size), "bold": bold == "1", "italic": italic == "1",
        "underline": int(underline), "strike": int(strike), "font_color": rgb(int(font_color)), "escapement": int(escapement),
        "back": None if back == "" else rgb(int(back)),
    }
    for side, text in (("left", left), ("right", right), ("top", top), ("bottom", bottom)):
        p["border_" + side] = border(text)
    p["diagonal_down"], p["diagonal_up"] = border(diag_down), border(diag_up)
    h = int(hori)
    p["hori"] = HORI[h] if 0 <= h < len(HORI) else str(h)
    p["vert"], p["wrap"], p["rotate"], p["indent"] = int(vert), wrap == "1", int(rotate), int(indent)
    p["shrink"], p["locked"], p["hidden"] = shrink == "1", locked == "1", hidden == "1"
    p["num_format"] = None if num_format == "\\N" else unesc(num_format)
    return p


def read_cells(path):
    """The sheets of a cells file of the macro: name, visibility, used area, cells."""
    sheets = []
    with open(path, encoding="utf-8") as f:
        for line in f.read().split("\n"):
            if not line:
                continue
            f_ = line.split("\t")
            if f_[0] == "S":
                _, name, visible, last_row, last_col, r0, c0, hidden = f_
                sheets.append({"name": unesc(name), "visible": visible == "1", "last": [int(last_row), int(last_col)],
                               "html_origin": [int(r0), int(c0)], "hidden_rows_or_columns": hidden == "1", "cells": [], "_seen": set()})
            elif f_[0] == "C" and sheets:
                r, c = int(f_[1]), int(f_[2])
                sh = sheets[-1]
                if (r, c) not in sh["_seen"]:
                    sh["_seen"].add((r, c))
                    sh["cells"].append([r, c, cell_props(f_[3:])])
    for sh in sheets:
        del sh["_seen"]
    return sheets


class Tables(HTMLParser):
    """The cells of each table of LibreOffice's HTML export: background and font colour."""
    def __init__(self):
        super().__init__()
        self.tables = []; self.taken = None; self.row = -1; self.col = 0; self.cell = None
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "table":
            self.tables.append({}); self.taken = set(); self.row = -1
        elif tag == "tr" and self.tables:
            self.row += 1; self.col = 0
        elif tag == "td" and self.tables:
            while (self.row, self.col) in self.taken: self.col += 1
            rs, cs = int(a.get("rowspan", 1)), int(a.get("colspan", 1))
            for r in range(self.row, self.row + rs):
                for c in range(self.col, self.col + cs):
                    self.taken.add((r, c))
            bg = a.get("bgcolor")
            self.cell = [bg[1:].upper() if bg else None, None]
            self.tables[-1][(self.row, self.col)] = self.cell
            self.col += cs
        elif tag == "font" and self.cell is not None and "color" in a:
            if self.cell[1] is None: self.cell[1] = a["color"][1:].upper()
    def handle_endtag(self, tag):
        if tag == "td": self.cell = None

def main():
    root, out = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    step = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    limit = int(sys.argv[4]) if len(sys.argv) > 4 else 10**9
    files = []
    for d, _, names in os.walk(root):
        if "/lo-" in d + "/":
            continue
        files += [os.path.join(d, n) for n in sorted(names) if n.lower().endswith((".xlsx", ".xlsm"))]
    files.sort()
    # XML bombs (POI's security tests) make LibreOffice fill the disk.
    files = [f for f in files if "xmlbomb" not in os.path.basename(f).lower()]
    files = files[::step][:limit]
    work = tempfile.mkdtemp(prefix="lo_look_")
    items, targets = [], {}
    ok = failed = 0
    for k, path in enumerate(files):
        rel = os.path.relpath(path, root)
        target = os.path.join(out, rel + ".json")
        # Resumable: a file done by this version of the oracle is not done again.
        if os.path.exists(target) and '"html_origin"' in open(target, encoding="utf-8").read(4000):
            ok += 1
            continue
        prefix = os.path.join(work, str(k))
        items.append((path, prefix, prefix + ".status"))
        targets[prefix] = (rel, target)
    result = lo_basic.run("Look", items, log=lambda m: print(m, file=sys.stderr))
    for _path, prefix, status_file in items:
        rel, target = targets[prefix]
        status = result.get(status_file, "error: no status")
        if status != "ok" or not os.path.exists(prefix + ".cells.tsv"):
            failed += 1
            if status not in ("ok", "not-a-spreadsheet"):
                print("error", rel, status[:200], file=sys.stderr)
            continue
        sheets = read_cells(prefix + ".cells.tsv")
        small = os.path.exists(prefix + ".html")
        t = Tables()
        if small:
            t.feed(open(prefix + ".html", encoding="utf-8", errors="replace").read())
        # The export leaves out hidden sheets.
        k = -1
        for sh in sheets:
            if not sh["visible"]:
                sh["html_rows"] = 0
                sh["html"] = None
                continue
            k += 1
            grid = t.tables[k] if k < len(t.tables) else {}
            rows = 1 + max((r for r, _ in grid), default=-1)
            # The export covers the used area from its first row and column; only a
            # grid whose rows match it maps cell by cell.
            r0, c0 = sh["html_origin"]
            sh["html_rows"] = rows
            fits = small and rows == sh["last"][0] - r0 + 1 and not sh["hidden_rows_or_columns"]
            sh["html"] = [[r, c, grid.get((r - r0, c - c0)) if r >= r0 and c >= c0 else None] for r, c, _ in sh["cells"]] if fits else None
        os.makedirs(os.path.dirname(target), exist_ok=True)
        json.dump({"file": rel, "sheets": sheets}, open(target, "w"))
        ok += 1
    shutil.rmtree(work, ignore_errors=True)
    print("written %d, failed %d" % (ok, failed))


if __name__ == "__main__":
    main()
