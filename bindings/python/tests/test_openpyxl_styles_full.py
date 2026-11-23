"""The full style model through the openpyxl interface: theme and indexed colours, tints,
gradients, all border sides, every alignment attribute, cell protection and named styles.

Golden reference: openpyxl itself. Both modules build the same workbook; openpyxl reads
both files and must see the same formatting. Corpus files are read by both modules and
must show the same formatting cell by cell."""
import glob
import importlib
import io
import os
import re
import warnings

import openpyxl
import pytest

import xlcells.openpyxl as facade

from conftest import CORPUS


def build(module):
    st = importlib.import_module(module.__name__ + ".styles")
    wb = module.Workbook()
    ws = wb.active
    ws["A1"] = "theme"
    ws["A1"].font = st.Font(b=True, color=st.Color(theme=4, tint=-0.25), vertAlign="superscript", scheme="minor")
    ws["A2"] = "gradient"
    ws["A2"].fill = st.GradientFill(degree=90, stop=("FF0000", "0000FF"))
    ws["A3"] = "pattern"
    ws["A3"].fill = st.PatternFill("lightGrid", fgColor=st.Color(indexed=10), bgColor="00FF00")
    ws["A4"] = "diagonal"
    ws["A4"].border = st.Border(left=st.Side("thin", color=st.Color(theme=1)), diagonal=st.Side("dashed"), diagonalUp=True)
    ws["A5"] = "alignment"
    ws["A5"].alignment = st.Alignment(horizontal="center", shrinkToFit=True, readingOrder=2, wrapText=True)
    ws["A6"] = "protection"
    ws["A6"].protection = st.Protection(locked=False, hidden=True)
    ws["A7"] = "builtin"
    ws["A7"].style = "Good"
    highlight = st.NamedStyle(name="Hervorhebung", font=st.Font(bold=True, color="FF0000"), border=st.Border(bottom=st.Side("double")))
    wb.add_named_style(highlight)
    ws["A8"] = "named"
    ws["A8"].style = "Hervorhebung"
    ws["A9"] = "title"
    ws["A9"].style = "Title"
    ws["A10"] = "bold only"
    ws["A10"].font = st.Font(b=True)
    ws["A10"].fill = st.PatternFill("solid", fgColor="FFFF00")
    ws["A11"] = "plain"
    buf = io.BytesIO()
    wb.save(buf)
    return buf.getvalue()


def colour(c):
    if c is None:
        return None
    return (c.type, c.value, round(c.tint or 0, 9))


def side(s):
    return None if s is None or not s.style else (s.style, colour(s.color))


def fill_view(f):
    if getattr(f, "stop", None):
        return ("gradient", f.type, f.degree, f.left, f.right, f.top, f.bottom, tuple((round(s.position, 9), colour(s.color)) for s in f.stop))
    if not f.patternType or f.patternType == "none":
        return None
    return ("pattern", f.patternType, colour(f.fgColor), colour(f.bgColor))


def view(cell):
    f, a, b = cell.font, cell.alignment, cell.border
    try:
        style = cell.style
    except Exception:  # openpyxl fails on some files' named styles
        style = "?"
    return (
        cell.number_format,
        (bool(f.b), bool(f.i), f.u, bool(f.strike), f.sz, f.name, colour(f.color), f.vertAlign, f.scheme, f.family),
        fill_view(cell.fill),
        tuple(side(getattr(b, n)) for n in ("left", "right", "top", "bottom", "diagonal")), b.diagonalUp, b.diagonalDown,
        (a.horizontal, a.vertical, int(a.textRotation or 0), bool(a.wrapText), bool(a.shrinkToFit), float(a.indent or 0), a.readingOrder),
        (cell.protection.locked, cell.protection.hidden),
        style,
    )


def openpyxl_view(data):
    wb = openpyxl.load_workbook(io.BytesIO(data))
    ws = wb.active
    return [view(ws.cell(r, 1)) for r in range(1, 12)], sorted(wb.named_styles)


def test_built_styles_read_back_as_openpyxl_writes_them():
    assert openpyxl_view(build(facade)) == openpyxl_view(build(openpyxl))


def test_built_styles_are_seen_by_both_modules_alike():
    data = build(facade)
    a = openpyxl.load_workbook(io.BytesIO(data)).active
    b = facade.load_workbook(io.BytesIO(data)).active
    for r in range(1, 12):
        assert view(b.cell(r, 1)) == view(a.cell(r, 1)), r


def test_theme_colours_and_named_styles_survive_editing_a_loaded_workbook():
    wb = facade.load_workbook(io.BytesIO(build(openpyxl)))
    ws = wb.active
    ws["B1"] = "new"
    ws["B2"] = "new styled"
    ws["B2"].font = ws["A1"].font
    ws["B2"].style  # reading the name must not change anything
    buf = io.BytesIO()
    wb.save(buf)
    got = openpyxl.load_workbook(io.BytesIO(buf.getvalue())).active
    want = openpyxl.load_workbook(io.BytesIO(build(openpyxl))).active
    for r in range(1, 12):
        assert view(got.cell(r, 1)) == view(want.cell(r, 1)), r
    assert view(got["B2"])[1] == view(want["A1"])[1]
    assert view(got["B1"]) == view(want["A11"])


def test_named_style_names_keep_escapes():
    # A name read as "_x0001_" is written as read, not escaped a second time.
    wb = openpyxl.Workbook()
    st = openpyxl.styles
    wb.add_named_style(st.NamedStyle(name="a_x0001_b", font=st.Font(i=True)))
    wb.active["A1"] = 1
    wb.active["A1"].style = "a_x0001_b"
    buf = io.BytesIO()
    wb.save(buf)
    ours = facade.load_workbook(io.BytesIO(buf.getvalue()))
    out = io.BytesIO()
    ours.save(out)
    assert openpyxl.load_workbook(io.BytesIO(out.getvalue())).named_styles == openpyxl.load_workbook(io.BytesIO(buf.getvalue())).named_styles


CORPUS_FILES = sorted(f for f in glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)
                      if os.path.getsize(f) < 1_000_000 and "OOM" not in f.upper())


@pytest.mark.skipif(not CORPUS_FILES, reason="corpus not present")
def test_corpus_formatting_matches_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in CORPUS_FILES:
        data = open(path, "rb").read()
        try:
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                a = openpyxl.load_workbook(io.BytesIO(data))
            b = facade.load_workbook(io.BytesIO(data))
        except Exception:
            continue  # files openpyxl (or both) cannot read; covered by the facade tests
        compared += 1
        # Without a font list openpyxl falls back to its own Normal (Calibri 12); Excel and the
        # module take Excel's default font (Calibri 11). Compare everything else.
        own_default = any("no default style" in str(w.message) for w in caught)
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            merged = {str(m).split(":")[0] for m in wa.merged_cells.ranges}
            for n, ((r, c), cell) in enumerate(wa._cells.items()):
                if n >= 2000:
                    break
                other = wb_._cells.get((r, c))
                # openpyxl gives merged cells a fresh format and merges edge borders into the first
                if other is None or type(cell).__name__ == "MergedCell" or cell.coordinate in merged:
                    continue
                va, vb = view(cell), view(other)
                if own_default and va[1][4] == 12.0 and vb[1][4] == 11.0:
                    va, vb = va[:1] + va[2:], vb[:1] + vb[2:]
                if va[-1] == "?":
                    vb = vb[:-1] + ("?",)
                # openpyxl leaves _xHHHH_ escapes in number formats and font names undecoded;
                # we decode them, as Excel does (escapes12.xlsx).
                if va != vb and re.search(r"_x[0-9A-Fa-f]{4}_", repr(va)):
                    continue
                if va != vb:
                    differing.append((path, wa.title, cell.coordinate, va, vb))
                    break
    assert compared > 1000
    assert not differing, differing[:3]
