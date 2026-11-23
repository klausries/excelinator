#!/usr/bin/env python3
"""Does Excel take a workbook whose table got more rows through a store, and does it
calculate what the engine calculated? (macOS, Microsoft Excel, AppleScript)

    uv run python tools/excel_shape.py [OUTDIR]

Writes a workbook with openpyxl: an Excel table with a total below it, a conditional
format and a validation on its rows, a defined name, a note and a second table further
down, and a sheet that reads the first. Makes a store of it, adds three rows to the table,
runs the store and takes the workbook out. Opens that in Excel, reads cells, the range of
the table, the conditional format and the validation, lets Excel calculate, and reads
the cells again. Prints what is not as expected.

Excel is left as it was found: only the workbook made here is opened, and closed again
without saving. Needs openpyxl and the packages of this repository.
"""
import os
import shutil
import subprocess
import sys
import tempfile

from openpyxl import Workbook
from openpyxl.formatting.rule import CellIsRule
from openpyxl.styles import PatternFill
from openpyxl.workbook.defined_name import DefinedName
from openpyxl.worksheet.datavalidation import DataValidation
from openpyxl.worksheet.table import Table, TableStyleInfo

import xlcells.openpyxl as xo


def build(path):
    wb = Workbook()
    ws = wb.active
    ws.title = "Report"
    ws.append(["item", "qty", "price", "amount"])
    for r, (item, qty) in enumerate([("a", 1), ("b", 2), ("c", 3)], 2):
        ws.append([item, qty, 2.5, "=B{0}*C{0}".format(r)])
    ws.append(["Total", None, None, "=SUM(D2:D4)"])
    ws["A7"] = "Prices in EUR"
    ws.append([])
    ws["A9"], ws["B9"] = "place", "share"
    ws["A10"], ws["B10"] = "north", "=D2/$D$5"
    ws["A11"], ws["B11"] = "south", "=D3/$D$5"
    table = Table(displayName="Orders", ref="A1:D4")
    table.tableStyleInfo = TableStyleInfo(name="TableStyleMedium2", showRowStripes=True)
    ws.add_table(table)
    ws.conditional_formatting.add("D2:D4", CellIsRule(operator="greaterThan", formula=["5"], fill=PatternFill(bgColor="FFFFC7CE", fill_type="solid")))
    whole = DataValidation(type="whole", operator="greaterThanOrEqual", formula1="0")
    whole.add("B2:B4")
    ws.add_data_validation(whole)
    wb.defined_names["Total"] = DefinedName("Total", attr_text="Report!$D$5")
    other = wb.create_sheet("Other")
    other["A1"], other["B1"] = "Total again", "=Report!D5"
    other["A2"], other["B2"] = "All", "=SUM(Report!D2:D4)"
    other["A3"], other["B3"] = "By name", "=Total*2"
    wb.save(path)


CELLS = [("Report", c) for c in ["A5", "D5", "D7", "A8", "D8", "A10", "B13", "B14"]] + [("Other", c) for c in ["B1", "B2", "B3"]]


def excel(path):
    """What Excel says of the workbook: lines of `key<TAB>value`."""
    pairs = ",".join('{"%s", "%s"}' % c for c in CELLS)
    script = '''
set nl to linefeed
set sep to tab
with timeout of 180 seconds
tell application "Microsoft Excel"
  open POSIX file "%s"
  set wb to workbook "%s"
  set out to ""
  try
    repeat with pass in {"stored", "calculated"}
      repeat with spec in {%s}
        set r to range (item 2 of spec) of worksheet (item 1 of spec) of wb
        set out to out & pass & " " & (item 1 of spec) & "!" & (item 2 of spec) & sep & (value of r as text) & sep & (formula of r as text) & nl
      end repeat
      calculate full
    end repeat
    set ws to worksheet "Report" of wb
    try
      set out to out & "table" & sep & (get address of (range object of list object "Orders" of ws)) & nl
    on error m
      set out to out & "table" & sep & "error: " & m & nl
    end try
    try
      set out to out & "conditional formats on D7" & sep & (count of format conditions of range "D7" of ws) & nl
    on error m
      set out to out & "conditional formats on D7" & sep & "error: " & m & nl
    end try
    try
      set out to out & "validation on B7" & sep & (formula1 of validation of range "B7" of ws) & nl
    on error m
      set out to out & "validation on B7" & sep & "error: " & m & nl
    end try
    try
      set out to out & "name Total" & sep & (value of named item "Total" of wb) & nl
    on error m
      set out to out & "name Total" & sep & "error: " & m & nl
    end try
  on error m
    close wb saving no
    error m
  end try
  close wb saving no
  return out
end tell
end timeout
''' % (path, os.path.basename(path), pairs)
    out = subprocess.run(["osascript", "-"], input=script, capture_output=True, text=True, timeout=400)
    if out.returncode:
        sys.exit("Excel: " + out.stderr)
    return [line.split("\t") for line in out.stdout.replace("\r", "\n").split("\n") if line.strip()]


def main():
    out = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else tempfile.mkdtemp(prefix="excel-shape-")
    os.makedirs(out, exist_ok=True)
    source, back = os.path.join(out, "shape.xlsx"), os.path.join(out, "shape_more_rows.xlsx")
    build(source)
    shutil.rmtree(os.path.join(out, "store"), ignore_errors=True)
    store = xo.load_workbook(source).to_store(os.path.join(out, "store"))
    store.run()
    store.patch("Orders", insert=[{"item": "d", "qty": 4, "price": 2.5}, {"item": "e", "qty": 5, "price": 2.5}, {"item": "f", "qty": 6, "price": 2.5}])
    report = store.run()
    shutil.copy(os.path.join(out, "store", "workbook.xlsx"), back)
    print("run:", report["computed"], "of", report["formula_cells"], "formula cells;", report["not_computed"] or "all calculated")
    lines = excel(back)
    said = {line[0]: line[1:] for line in lines}
    wrong = 0
    expected = {
        "Report!A5": ("d", None), "Report!D5": ("10", "=B5*C5"), "Report!D7": ("15", "=B7*C7"), "Report!A8": ("Total", None),
        "Report!D8": ("52.5", "=SUM(D2:D7)"), "Report!A10": ("Prices in EUR", None), "Report!B13": (str(2.5 / 52.5), "=D2/$D$8"),
        "Other!B1": ("52.5", "=Report!D8"), "Other!B2": ("52.5", "=SUM(Report!D2:D7)"), "Other!B3": ("105", "=Total*2"),
    }
    for cell, (value, formula) in expected.items():
        for when in ("stored", "calculated"):
            got = said.get("{0} {1}".format(when, cell), ["(not read)", ""])
            try:
                same = got[0] == value or abs(float(got[0].replace(",", ".")) - float(value)) < 1e-9
            except ValueError:
                same = False
            if not same or (formula and got[1] != formula):
                wrong += 1
                print("NOT AS EXPECTED {0} {1}: Excel has {2}, expected {3}".format(when, cell, got, (value, formula)))
    for key, want in [("table", "$A$1:$D$7"), ("conditional formats on D7", "1"), ("validation on B7", "0"), ("name Total", "=Report!$D$8")]:
        got = said.get(key, ["(not read)"])[0]
        if got != want:
            wrong += 1
            print("NOT AS EXPECTED {0}: Excel has {1!r}, expected {2!r}".format(key, got, want))
    print("Excel opened {0}: {1} cells read twice, {2} not as expected".format(back, len(expected), wrong))
    sys.exit(1 if wrong else 0)


if __name__ == "__main__":
    main()
