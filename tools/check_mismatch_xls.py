#!/usr/bin/env python3
"""Classify the cells the lift computed differently from the converted file.

The corpus files in corpus/lo-enron are LibreOffice conversions of the Enron .xls files.
For each mismatch line of lift_corpus's output this compares the computed value with
the value Excel itself stored in the original .xls (read with xlrd).

usage: check_mismatch_xls.py lift-output.txt
"""
import re, sys, os
import xlrd

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "corpus")
line_re = re.compile(r"^mismatch: (\S+?\.xls)\.xlsx: (.+?)!R(\d+)C(\d+) =(.*): stored (.*), computed (.*)$")
books = {}
counts = {}
for line in open(sys.argv[1], encoding="utf-8"):
    m = line_re.match(line.rstrip("\n"))
    if not m:
        continue
    name, sheet, row, col, formula, stored, computed = m.groups()
    path = os.path.join(ROOT, "src-enron", "edrm", name)
    try:
        book = books.get(path) or xlrd.open_workbook(path)
        books[path] = book
        value = book.sheet_by_name(sheet).cell(int(row) - 1, int(col) - 1)
    except Exception as e:
        verdict = f"original not readable ({type(e).__name__})"
    else:
        num = re.match(r"Some\(([-0-9.e+]+)\)", computed)
        text = re.search(r'Some\("(.*)"\)$', computed)
        if value.ctype == xlrd.XL_CELL_ERROR:
            verdict = "original holds an error, computed a value" if (num or text) else "original holds an error too"
        elif num and value.ctype in (xlrd.XL_CELL_NUMBER, xlrd.XL_CELL_DATE):
            a, b = float(num.group(1)), float(value.value)
            verdict = "computed = Excel's original value" if abs(a - b) <= 1e-9 * max(1.0, abs(b)) else "computed differs from the original too"
        elif text and value.ctype == xlrd.XL_CELL_TEXT:
            verdict = "computed = Excel's original value" if text.group(1) == value.value else "computed differs from the original too"
        else:
            verdict = f"computed differs from the original too"
    counts[verdict] = counts.get(verdict, 0) + 1
    print(f"{verdict}\t{name[:40]}\t{sheet}!R{row}C{col}\t={formula[:80]}\tstored {stored}\tcomputed {computed}\toriginal {value.value if 'value' in dir() else '?'}")
print("\n".join(f"{v:4d}  {k}" for k, v in sorted(counts.items(), key=lambda x: -x[1])), file=sys.stderr)
