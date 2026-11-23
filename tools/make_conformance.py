#!/usr/bin/env python3
"""Generate tests/conformance/cases.txt: workbooks every language binding must turn
into the same bytes and read back as the same table.

Line format (strings are hex-encoded UTF-8, "-" means none; numbers are the 16 hex
digits of the IEEE-754 bit pattern):

    case <name>
    date1904 <0|1>
    style <bold 0|1> <italic 0|1> <format>
    sheet <name>
    merge <r1> <r2> <c1> <c2>
    width <col> <width in 1/256 character>
    cell <row> <col> <kind> <num> <style> <text> <formula>
    end
"""
import random
import struct
import sys

rng = random.Random(20261112)
out = []


def hx(s):
    return "-" if s is None else (s.encode("utf-8").hex() or "00"[:0] or "=")  # "=" marks the empty string


def bits(x):
    return struct.pack(">d", x).hex()


def cell(r, c, kind, num=0.0, style=0, text=None, formula=None):
    out.append(f"cell {r} {c} {kind} {bits(num)} {style} {hx(text)} {hx(formula)}")


def case(name, date1904=False):
    out.append(f"case {name}")
    out.append(f"date1904 {int(date1904)}")


def sheet(name):
    out.append(f"sheet {hx(name)}")


TEXTS = ["a", "Größe", "日本語", "😀 astral", "  spaces  ", "line1\nline2", "tab\there", "cr\rlf", "_x000D_ literal", "\x01\x1f control",
         "<&>\"'", "x" * 300, "=not a formula", "0123", "TRUE", "#N/A", "​ ", "é" * 50]
FORMATS = ["General", "0.00", "yyyy-mm-dd", "[h]:mm:ss", "#,##0.00 \"€\"", "0%", "@", "d. mmmm yyyy", "0.00E+00", "[Red]-0.0;[Blue]0.0"]
FORMULAS = ["1+1", "SUM(A1:B2)", "IF(A1>0,\"a;b\",\"<&>\")", "A1&\" \"&B1", "'My Sheet'!A1*2", "NOW()", "SUM(A1;B1;{1;2})", "$A$1+B$2"]
NUMBERS = [0.0, -0.0, 1.0, -1.0, 0.1, 1 / 3, 1e-300, -1e300, 5e-324, 1.7976931348623157e308, 46338.25, 123456789012345.0, 2.5e-5, 1e16, 9007199254740993.0]
ERRORS = ["#DIV/0!", "#N/A", "#NAME?", "#NULL!", "#NUM!", "#REF!", "#VALUE!"]

case("minimal")
sheet("Sheet1")
out.append("end")

case("every-kind")
out.append("style 1 0 " + hx("0.00"))
out.append("style 0 1 " + hx("yyyy-mm-dd"))
sheet("Kinds")
cell(0, 0, 0, style=1)
for i, n in enumerate(NUMBERS):
    cell(1, i, 1, n)
for i, t in enumerate(TEXTS):
    cell(2, i, 2, text=t)
cell(3, 0, 3, 1.0)
cell(3, 1, 3, 0.0)
for i, e in enumerate(ERRORS):
    cell(4, i, 4, text=e)
cell(5, 0, 1, 46338.25, style=2)
out.append("end")

case("formulas")
sheet("My Sheet")
sheet("F")
for i, f in enumerate(FORMULAS):
    cell(i, 0, 0, formula=f)
    cell(i, 1, 1, 2.5, formula=f)
    cell(i, 2, 2, text="a<b ", formula=f)
    cell(i, 3, 3, 1.0, formula=f)
    cell(i, 4, 4, text="#DIV/0!", formula=f)
out.append("end")

case("layout", date1904=True)
sheet("Erste")
out.append("merge 0 1 0 3")
out.append("merge 5 5 2 4")
out.append("width 0 3328")
out.append("width 3 256")
cell(0, 0, 2, text="merged")
cell(1048575, 16383, 2, text="last cell")
cell(65536, 256, 1, 1.0)
sheet("Zweite & <Dritte>")
sheet("it's quoted")
out.append("end")

case("overwrite-and-order")
sheet("S")
cell(9, 9, 1, 1.0)
cell(0, 0, 1, 2.0)
cell(9, 9, 2, text="wins")
cell(5, 0, 1, 3.0)
cell(0, 0, 0)
out.append("end")

for n in range(40):
    case(f"random-{n:02d}", date1904=rng.random() < 0.2)
    styles = rng.randint(0, 4)
    for _ in range(styles):
        out.append(f"style {rng.randint(0, 1)} {rng.randint(0, 1)} {hx(rng.choice(FORMATS))}")
    for s in range(rng.randint(1, 4)):
        sheet(f"Blatt {s} " + rng.choice(["", "ä", "😀", "& Co", "(1)"]))
        row = 0
        for _ in range(rng.randint(0, 3)):
            height, width = rng.randint(1, 3), rng.randint(1, 3)
            c1 = rng.randint(0, 20)
            out.append(f"merge {row} {row + height - 1} {c1} {c1 + width - 1}")
            row += height
        for c in sorted(rng.sample(range(40), rng.randint(0, 3))):
            out.append(f"width {c} {rng.randint(0, 65280)}")
        for _ in range(rng.randint(0, 60)):
            r, c = rng.choice([(rng.randint(0, 200), rng.randint(0, 50)), (rng.randint(0, 1048575), rng.randint(0, 16383))])
            kind = rng.choice([0, 1, 1, 1, 2, 2, 2, 3, 4])
            num = rng.choice(NUMBERS + [rng.uniform(-1e6, 1e6), float(rng.randint(-10**6, 10**6))]) if kind == 1 else float(rng.randint(0, 1)) if kind == 3 else 0.0
            text = rng.choice(TEXTS) if kind == 2 else rng.choice(ERRORS) if kind == 4 else None
            formula = rng.choice(FORMULAS) if rng.random() < 0.25 else None
            cell(r, c, kind, num, rng.randint(0, styles), text, formula)
    out.append("end")

open(sys.argv[1], "w").write("\n".join(out) + "\n")
print(len([l for l in out if l.startswith("case ")]), "cases,", len(out), "lines")
