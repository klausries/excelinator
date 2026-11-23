#!/usr/bin/env python3
"""Compare xl-look with the Excel that is installed (macOS, Microsoft Excel, AppleScript).

    tools/excel_look.py [OUTDIR]

Writes a workbook for each scenario (tints, conditional formats of every kind, priorities,
row and column formats, a table style, date rules) to OUTDIR (default: a temporary
directory; `ONLY=<scenario>` for one), opens it in Excel and reads what each cell displays (`display format`: fill,
font colour, bold, italic, conditional formats and table styles included), runs
`cargo run --example dump -p xl-look` over the same file, and prints where they differ.

Excel is left as it was found: only the workbook made here is opened, and closed again
without saving. Needs openpyxl.
"""
import datetime
import os
import subprocess
import sys
import tempfile

from openpyxl import Workbook
from openpyxl.formatting.rule import ColorScaleRule, Rule
from openpyxl.styles import Color, Font, NamedStyle, PatternFill
from openpyxl.styles.differential import DifferentialStyle
from openpyxl.utils import get_column_letter
from openpyxl.worksheet.table import Table, TableStyleInfo

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
VALUES = [5, 3, 8, 3, 9, 1, 7, 5, 2, 10]
WORDS = ["apple", "banana", "cherry", "grape", "Apple", "fig", "date", "pear", "plum", "kiwi"]


def solid(**color):
    return PatternFill(fill_type="solid", fgColor=Color(**color))


def dxf(fill=None, bold=None, italic=None, color=None):
    return DifferentialStyle(
        fill=PatternFill(bgColor=color_of(fill), fill_type="solid") if fill else None,
        font=Font(bold=bold, italic=italic, color=color_of(color) if color else None) if (bold or italic or color) else None,
    )


def color_of(rgb):
    return Color(rgb="FF" + rgb)


def rule(kind, fill=None, bold=None, italic=None, color=None, stop=False, **kw):
    return Rule(type=kind, dxf=dxf(fill, bold, italic, color), stopIfTrue=stop or None, **kw)


def column(ws, col, values):
    for r, v in enumerate(values, 1):
        ws.cell(r, col, v)


def build(path):
    wb = Workbook()
    wb.remove(wb.active)
    regions = {}

    # --- tints: theme, palette and RGB colours with tints ------------------------------
    ws = wb.create_sheet("tint")
    tints = [0, -0.5, -0.25, 0.4, 0.6, 0.8]
    for t, theme in enumerate(range(12)):
        for c, tint in enumerate(tints):
            cell = ws.cell(t + 1, c + 1, 1)
            cell.fill = solid(theme=theme, tint=tint)
            cell.font = Font(color=Color(theme=(theme + 3) % 10, tint=tint))
        ws.cell(t + 1, 7, 1).fill = solid(indexed=8 + t, tint=0.4)
        ws.cell(t + 1, 8, 1).fill = solid(rgb="FF" + ["3366CC", "FF9900", "00AA55", "888888", "AA0000", "123456", "FEDCBA", "808000", "FFFFFF", "000000", "010203", "C0FFEE"][t], tint=-0.3)
        ws.cell(t + 1, 9, 1).fill = solid(indexed=8 + t)
    regions["tint"] = "A1:I12"

    # --- cellIs, expression, text, blanks and errors ------------------------------------
    ws = wb.create_sheet("rules")
    for col in (1, 2, 3):
        column(ws, col, VALUES)
    column(ws, 4, WORDS)
    column(ws, 5, [1, None, 3, "#DIV/0!", 5, None, "x", 8, "#N/A", 10])
    cf = ws.conditional_formatting
    cf.add("A1:A10", rule("cellIs", fill="FF0000", operator="greaterThan", formula=["7"]))
    cf.add("A1:A10", rule("cellIs", bold=True, color="00B050", operator="between", formula=["3", "5"]))
    cf.add("A1:A10", rule("cellIs", italic=True, operator="equal", formula=["3"]))
    cf.add("B1:B10", rule("cellIs", fill="0000FF", stop=True, operator="greaterThan", formula=["5"]))
    cf.add("B1:B10", rule("cellIs", fill="FFFF00", bold=True, operator="greaterThan", formula=["3"]))
    cf.add("B1:B10", rule("cellIs", fill="FFA500", italic=True, operator="lessThan", formula=["3"]))
    cf.add("C1:C10", rule("expression", fill="C0C0C0", formula=["AND(A1>3,MOD(ROW(),2)=0)"]))
    cf.add("C1:C10", rule("expression", bold=True, color="800000", formula=["$A1=C1"]))
    cf.add("D1:D10", rule("containsText", fill="99CCFF", operator="containsText", text="ap", formula=['NOT(ISERROR(SEARCH("ap",D1)))']))
    cf.add("D1:D10", rule("beginsWith", bold=True, operator="beginsWith", text="p", formula=['LEFT(D1,LEN("p"))="p"']))
    cf.add("D1:D10", rule("endsWith", italic=True, operator="endsWith", text="e", formula=['RIGHT(D1,LEN("e"))="e"']))
    cf.add("E1:E10", rule("containsBlanks", fill="FFFF99", formula=["LEN(TRIM(E1))=0"]))
    cf.add("E1:E10", rule("containsErrors", bold=True, color="FF0000", formula=["ISERROR(E1)"]))
    cf.add("E1:E10", rule("notContainsBlanks", italic=True, formula=["LEN(TRIM(E1))>0"]))
    regions["rules"] = "A1:E10"

    # --- rules over the values of a range ------------------------------------------------
    ws = wb.create_sheet("range")
    for col in range(1, 9):
        column(ws, col, VALUES)
    cf = ws.conditional_formatting
    cf.add("A1:A10", rule("top10", fill="FF0000", rank=3))
    cf.add("B1:B10", rule("top10", fill="FF0000", rank=2, bottom=True))
    cf.add("C1:C10", rule("top10", fill="FF0000", rank=25, percent=True))
    cf.add("D1:D10", rule("aboveAverage", fill="00FF00"))
    cf.add("E1:E10", rule("aboveAverage", fill="00FF00", aboveAverage=False, equalAverage=True))
    cf.add("F1:F10", rule("aboveAverage", fill="00FF00", stdDev=1))
    cf.add("G1:G10", rule("duplicateValues", fill="FFC000"))
    cf.add("H1:H10", rule("uniqueValues", fill="FFC000"))
    regions["range"] = "A1:H10"

    # --- colour scales -------------------------------------------------------------------
    ws = wb.create_sheet("scale")
    for col in range(1, 6):
        column(ws, col, [1, 2, 4, 8, 16, 32, 50, 64, 80, 100])
    cf = ws.conditional_formatting
    cf.add("A1:A10", ColorScaleRule(start_type="min", start_color="FFFFFFFF", end_type="max", end_color="FFFF0000"))
    cf.add("B1:B10", ColorScaleRule(start_type="min", start_color="FFF8696B", mid_type="percentile", mid_value=50, mid_color="FFFFEB84", end_type="max", end_color="FF63BE7B"))
    cf.add("C1:C10", ColorScaleRule(start_type="num", start_value=10, start_color="FF00FF00", mid_type="num", mid_value=50, mid_color="FFFFFF00", end_type="num", end_value=90, end_color="FFFF0000"))
    cf.add("D1:D10", ColorScaleRule(start_type="percent", start_value=10, start_color="FF0000FF", end_type="percent", end_value=90, end_color="FFFFFF00"))
    cf.add("E1:E10", ColorScaleRule(start_type="percentile", start_value=20, start_color="FF000000", end_type="percentile", end_value=80, end_color="FFFFFFFF"))
    regions["scale"] = "A1:E10"

    # --- date rules, relative to today -----------------------------------------------------
    ws = wb.create_sheet("dates")
    today = (datetime.date.today() - datetime.date(1899, 12, 30)).days
    for col in range(1, 7):
        column(ws, col, [today + d for d in (-40, -8, -7, -6, -1, 0, 1, 6, 7, 40)])
        for r in range(1, 11):
            ws.cell(r, col).number_format = "yyyy-mm-dd"
    cf = ws.conditional_formatting
    cf.add("A1:A10", rule("timePeriod", fill="FF0000", timePeriod="today", formula=["FLOOR(A1,1)=TODAY()"]))
    cf.add("B1:B10", rule("timePeriod", fill="FF0000", timePeriod="yesterday", formula=["FLOOR(B1,1)=TODAY()-1"]))
    cf.add("C1:C10", rule("timePeriod", fill="FF0000", timePeriod="tomorrow", formula=["FLOOR(C1,1)=TODAY()+1"]))
    cf.add("D1:D10", rule("timePeriod", fill="FF0000", timePeriod="last7Days", formula=["AND(TODAY()-FLOOR(D1,1)<=6,FLOOR(D1,1)<=TODAY())"]))
    cf.add("E1:E10", rule("timePeriod", fill="FF0000", timePeriod="thisMonth", formula=["AND(MONTH(E1)=MONTH(TODAY()),YEAR(E1)=YEAR(TODAY()))"]))
    cf.add("F1:F10", rule("timePeriod", fill="FF0000", timePeriod="thisWeek", formula=["AND(TODAY()-ROUNDDOWN(F1,0)<=WEEKDAY(TODAY())-1,ROUNDDOWN(F1,0)-TODAY()<=7-WEEKDAY(TODAY()))"]))
    regions["dates"] = "A1:F10"

    # --- where a rule's formula is anchored, references between cells ------------------------
    ws = wb.create_sheet("anchor")
    for r in range(1, 21):
        ws.cell(r, 5, r)
        ws.cell(r, 6, 100 - r)
        ws.cell(r, 7, r % 3)
    cf = ws.conditional_formatting
    cf.add("A1:A10", rule("expression", fill="FF0000", formula=["$E1>5"]))
    cf.add("B1:B10", rule("expression", fill="00FF00", formula=["$E1+$G1=6"]))
    cf.add("C1:D10", rule("expression", fill="0000FF", formula=["OR($G1=1,E$1=1)"]))
    cf.add("I5:I10 H1:H3", rule("expression", fill="FFA500", formula=["F1<95"]))
    cf.add("A12:C14", rule("expression", bold=True, formula=["SUM($E$1:$E1)>10"]))
    cf.add("A16:A20", rule("expression", italic=True, formula=["COUNTIF($G$1:$G$20,$G16)>6"]))
    regions["anchor"] = "A1:I20"

    # --- formats: named style, row and column formats, own formats, over them a rule ----------
    ws = wb.create_sheet("formats")
    wb.add_named_style(NamedStyle(name="Loud", font=Font(bold=True, color="FFC00000"), fill=solid(rgb="FFFFF2CC")))
    ws["A1"] = "named"
    ws["A1"].style = "Loud"
    ws["B2"] = 1
    ws["B2"].fill = solid(theme=5, tint=0.6)
    ws["B2"].font = Font(italic=True, color=Color(theme=1, tint=0.35))
    ws.row_dimensions[3].fill = solid(rgb="FFDDEBF7")
    ws.row_dimensions[3].font = Font(italic=True)
    ws["C3"] = "in a formatted row"
    ws["D3"] = "own format"
    ws["D3"].font = Font(bold=True)
    ws.column_dimensions["F"].fill = solid(rgb="FFE2EFDA")
    ws.column_dimensions["F"].font = Font(color="FF7030A0")
    ws["F5"] = "in a formatted column"
    ws["F6"] = "own"
    ws["F6"].font = Font(bold=True)
    ws["H1"], ws["H2"], ws["H3"] = 1, 5, 9
    ws["H2"].fill = solid(rgb="FFBDD7EE")
    ws.conditional_formatting.add("H1:H3", rule("cellIs", fill="FF0000", operator="greaterThan", formula=["4"]))
    ws.conditional_formatting.add("A3:G3", rule("expression", bold=True, formula=["COLUMN()=3"]))
    regions["formats"] = "A1:H8"

    # --- a table style, a cell with its own fill in it, a rule over it ------------------------
    ws = wb.create_sheet("table")
    ws.append(["name", "a", "b"])
    for r in range(5):
        ws.append(["row%d" % r, r, r * r])
    ws["B3"].fill = solid(rgb="FFFFFF00")
    # A table name must not read as a cell (T1 does): Excel repairs the file.
    t = Table(displayName="Orders", ref="A1:C6")
    t.tableStyleInfo = TableStyleInfo(name="TableStyleMedium2", showRowStripes=True, showFirstColumn=True)
    ws.add_table(t)
    ws.append([])
    ws["E1"], ws["F1"] = "h1", "h2"
    for r in range(2, 6):
        ws.cell(r, 5, r)
        ws.cell(r, 6, r * 2)
    t2 = Table(displayName="Lookup", ref="E1:F5")
    t2.tableStyleInfo = TableStyleInfo(name="TableStyleLight9", showRowStripes=False, showColumnStripes=True)
    ws.add_table(t2)
    ws.conditional_formatting.add("C2:C6", rule("cellIs", fill="FF0000", operator="greaterThan", formula=["8"]))
    regions["table"] = "A1:F7"

    only = os.environ.get("ONLY")  # a sheet alone, to find which one Excel stumbles over
    if only:
        for ws in list(wb.worksheets):
            if ws.title != only:
                wb.remove(ws)
        regions = {only: regions[only]}
    wb.save(path)
    return regions


def excel_reads(path, regions):
    """{(sheet, cell): (fill, font colour, bold, italic)} as Excel displays them."""
    from openpyxl.utils import range_boundaries
    pairs = []
    for sheet, ref in regions.items():
        c0, r0, c1, r1 = range_boundaries(ref)
        pairs += ['{"%s", "%s"}' % (sheet, get_column_letter(c) + str(r)) for r in range(r0, r1 + 1) for c in range(c0, c1 + 1)]
    script = '''
on hexbyte(n)
  set digits to "0123456789ABCDEF"
  return (character (n div 16 + 1) of digits) & (character (n mod 16 + 1) of digits)
end hexbyte
on hexcolor(col)
  if col is false then return "-"
  return hexbyte(item 1 of col) & hexbyte(item 2 of col) & hexbyte(item 3 of col)
end hexcolor
set sep to tab
set nl to linefeed
with timeout of 600 seconds
tell application "Microsoft Excel"
  open POSIX file "%s"
  set wb to workbook "%s"
  set out to ""
  try
    repeat with spec in {%s}
      set ws to worksheet (item 1 of spec) of wb
      set d to display format of (range (item 2 of spec) of ws)
      set fillc to my hexcolor(color of interior object of d)
      set fontc to my hexcolor(color of font object of d)
      set b to bold of font object of d
      set i to italic of font object of d
      set out to out & (item 1 of spec) & sep & (item 2 of spec) & sep & fillc & sep & fontc & sep & b & sep & i & nl
    end repeat
  on error m
    close wb saving no
    error m
  end try
  close wb saving no
  return out
end tell
end timeout
''' % (path, os.path.basename(path), ",".join(pairs))
    out = subprocess.run(["osascript", "-"], input=script, capture_output=True, text=True, timeout=1800)
    if out.returncode:
        sys.exit("Excel: " + out.stderr)
    result = {}
    for line in out.stdout.replace("\r", "\n").split("\n"):
        f = line.split("\t")
        if len(f) == 6:
            result[(f[0], f[1])] = (f[2], f[3], f[4] == "true", f[5] == "true")
    return result


def ours(path):
    env = dict(os.environ, CARGO_PROFILE_DEV_DEBUG="0", CARGO_INCREMENTAL="0")
    out = subprocess.run(["cargo", "run", "-q", "-p", "xl-look", "--example", "dump", "--", path], cwd=ROOT, env=env, capture_output=True, text=True)
    if out.returncode:
        sys.exit(out.stderr)
    result = {}
    for line in out.stdout.splitlines():
        s, cell, fill, font, bold, italic, certain = line.split("\t")
        result[(s, cell)] = (fill, font, bold == "1", italic == "1", certain == "1")
    return result


def main():
    outdir = sys.argv[1] if len(sys.argv) > 1 else tempfile.mkdtemp()
    path = os.path.join(outdir, "excel_look.xlsx")
    # Each scenario in a workbook of its own: Excel (Microsoft 365 for Mac, 2026-11) stops
    # at a dialog when opening all of them in one workbook, and opens each alone.
    sheets = [os.environ["ONLY"]] if os.environ.get("ONLY") else ["tint", "rules", "range", "scale", "dates", "anchor", "formats", "table"]
    excel, mine = {}, {}
    for sheet in sheets:
        os.environ["ONLY"] = sheet
        regions = build(path)
        excel.update(excel_reads(path, regions))
        mine.update(ours(path))
    names = ("fill", "font colour", "bold", "italic")
    bad, uncertain, total = {}, 0, 0
    for key, e in sorted(excel.items()):
        m = mine[key]
        total += 1
        norm = lambda fill: "-" if fill in ("-", "FFFFFF") else fill  # white and no fill look alike through Excel
        diffs = [(n, a, b) for n, a, b in zip(names, (norm(e[0]),) + e[1:], (norm(m[0]),) + m[1:4]) if a != b]
        if not m[4]:
            uncertain += 1
        if diffs:
            bad.setdefault(key[0], []).append((key[1], diffs, m[4]))
    print("%d cells compared, %d differ, %d not certain" % (total, sum(map(len, bad.values())), uncertain))
    for sheet, items in bad.items():
        print("\n%s: %d cells" % (sheet, len(items)))
        for cell, diffs, certain in items[:int(os.environ.get("SHOW", "40"))]:
            print("  %-4s %s%s" % (cell, "; ".join("%s: Excel %s, xl-look %s" % d for d in diffs), "" if certain else "  (not certain)"))
    print("\nworkbook:", path)


if __name__ == "__main__":
    main()
