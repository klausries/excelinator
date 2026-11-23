"""Formatted text in cells (CellRichText) through the openpyxl interface.

Golden reference: openpyxl. Workbooks built with both modules read back alike in openpyxl
(rich_text=True); corpus files with formatted text give the same values in both; random
rich strings go through unchanged."""
import glob
import io
import os
import warnings
import zipfile

import openpyxl
import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from openpyxl.cell.rich_text import CellRichText as OCellRichText, TextBlock as OTextBlock
from openpyxl.cell.text import InlineFont as OInlineFont

import xlcells.openpyxl as facade
from xlcells.openpyxl.cell.rich_text import CellRichText, TextBlock
from xlcells.openpyxl.cell.text import InlineFont

from conftest import CORPUS
from test_property import plain_text

KINDS = {
    "openpyxl": (openpyxl, OCellRichText, OTextBlock, OInlineFont),
    "xlcells": (facade, CellRichText, TextBlock, InlineFont),
}


def font_view(f):
    c = f.color
    return (f.rFont, f.charset, f.family, bool(f.b), bool(f.i), f.strike, f.outline, f.shadow, f.condense, f.extend,
            None if c is None else (c.type, c.value, round(c.tint or 0, 9)), f.sz, f.u, f.vertAlign, f.scheme)


def view(value):
    if type(value).__name__ == "CellRichText":
        return ("rich", tuple((p.text, font_view(p.font)) if hasattr(p, "font") else p for p in value))
    return value


def build(kind):
    module, Rich, Block, Font = KINDS[kind]
    wb = module.Workbook()
    ws = wb.active
    ws["A1"] = Rich(["plain ", Block(Font(b=True, color="FF0000", sz=14, rFont="Arial"), "bold red"), " tail "])
    ws["A2"] = Rich(Block(Font(i=True, u="double", vertAlign="superscript"), "x_x0041_"), "y")
    ws["A3"] = Rich([Block(Font(color=module.styles.Color(theme=4, tint=0.4), strike=True, family=2, charset=0, scheme="minor"), "Thema")])
    ws["A4"] = "plain cell"
    ws["A5"] = Rich("only plain")
    buf = io.BytesIO()
    wb.save(buf)
    return buf.getvalue()


def read_back(data, rich_text=True):
    ws = openpyxl.load_workbook(io.BytesIO(data), rich_text=rich_text).active
    return [view(ws.cell(r, 1).value) for r in range(1, 6)]


def test_built_rich_text_reads_back_as_openpyxls():
    assert read_back(build("xlcells")) == read_back(build("openpyxl"))


def test_rich_text_is_read_as_openpyxl_reads_it():
    data = build("openpyxl")
    # Row 2: openpyxl writes "x_x0041_" unescaped, so the file says "xA" (ECMA-376 22.9.2.19);
    # openpyxl reads its own text back, the module reads what the file says.
    rows = (1, 3, 4, 5)
    ours = facade.load_workbook(io.BytesIO(data), rich_text=True).active
    assert [view(ours.cell(r, 1).value) for r in rows] == [read_back(data)[r - 1] for r in rows]
    assert ours["A2"].value[0].text == "xA"
    plain = facade.load_workbook(io.BytesIO(data)).active
    assert [plain.cell(r, 1).value for r in rows] == [read_back(data, rich_text=False)[r - 1] for r in rows]


@pytest.mark.parametrize("rich_text", [False, True])
def test_runs_survive_editing_even_when_read_as_plain_text(rich_text):
    # openpyxl drops the runs when it loads without rich_text; the module keeps them.
    want = read_back(build("openpyxl"))
    wb = facade.load_workbook(io.BytesIO(build("openpyxl")), rich_text=rich_text)
    wb.active["B1"] = "new"
    wb.active["A4"].font = facade.styles.Font(b=True)
    buf = io.BytesIO()
    wb.save(buf)
    assert read_back(buf.getvalue()) == want


def test_rich_text_objects_behave_like_openpyxls():
    for Rich, Block, Font in ((OCellRichText, OTextBlock, OInlineFont), (CellRichText, TextBlock, InlineFont)):
        bold = Font(b=True)
        t = Rich("a", Block(bold, "b"), Block(bold, "c"), "")
        t += [Block(bold, "d"), "e", "f"]
        yield_ = (str(t), t.as_list(), [type(p).__name__ for p in t])
        if Rich is OCellRichText:
            want = yield_
        else:
            assert yield_ == want
        with pytest.raises(TypeError):
            Rich([object()])


fonts = st.builds(
    InlineFont,
    rFont=st.none() | st.sampled_from(["Arial", "Calibri", "Times New Roman"]),
    b=st.booleans(), i=st.booleans(),
    strike=st.none() | st.booleans(),
    color=st.none() | st.sampled_from(["FFFF0000", "FF00FF00", "00123456"]),
    sz=st.none() | st.sampled_from([8, 11, 14.5]),
    u=st.none() | st.sampled_from(["single", "double", "singleAccounting"]),
    vertAlign=st.none() | st.sampled_from(["superscript", "subscript", "baseline"]),
)
parts = st.lists(st.one_of(plain_text, st.builds(TextBlock, fonts, plain_text)), min_size=1, max_size=5)


def as_openpyxl(value):
    out = []
    for p in value:
        if isinstance(p, TextBlock):
            f = p.font
            font = OInlineFont(rFont=f.rFont, b=f.b, i=f.i, strike=f.strike, color=f.color.rgb if f.color is not None else None, sz=f.sz, u=f.u,
                               vertAlign=f.vertAlign)
            out.append(OTextBlock(font, p.text))
        else:
            out.append(p)
    return OCellRichText(out)


@settings(max_examples=60, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(parts)
def test_random_rich_text_round_trips_and_matches_openpyxl(value):
    value = CellRichText(value)
    wb = facade.Workbook()
    wb.active["A1"] = value
    buf = io.BytesIO()
    wb.save(buf)
    ours = buf.getvalue()
    owb = openpyxl.Workbook()
    owb.active["A1"] = as_openpyxl(value)
    obuf = io.BytesIO()
    owb.save(obuf)
    theirs = openpyxl.load_workbook(io.BytesIO(obuf.getvalue()), rich_text=True).active["A1"].value
    assert view(openpyxl.load_workbook(io.BytesIO(ours), rich_text=True).active["A1"].value) == view(theirs)
    back = facade.load_workbook(io.BytesIO(ours), rich_text=True).active["A1"].value
    assert view(back) == view(theirs)
    assert facade.load_workbook(io.BytesIO(ours)).active["A1"].value == str(value)


def _has_runs(path):
    try:
        with zipfile.ZipFile(path) as z:
            return any(b"<r>" in z.read(m) or b"<x:r>" in z.read(m) for m in z.namelist()
                       if m.endswith("sharedStrings.xml") or ("/worksheets/" in m and m.endswith(".xml")))
    except Exception:
        return False


RICH_FILES = sorted(f for f in glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)
                    if os.path.getsize(f) < 1_000_000 and "OOM" not in f.upper() and _has_runs(f))


@pytest.mark.skipif(not RICH_FILES, reason="corpus not present")
def test_corpus_rich_text_matches_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in RICH_FILES:
        data = open(path, "rb").read()
        try:
            a = openpyxl.load_workbook(io.BytesIO(data), rich_text=True)
            b = facade.load_workbook(io.BytesIO(data), rich_text=True)
        except Exception:
            continue
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            for key, cell in list(wa._cells.items())[:3000]:
                other = wb_._cells.get(key)
                # openpyxl does not decode _xHHHH_ (only _x005F_); the module does
                if other is None or (isinstance(cell.value, str) and "_x" in cell.value):
                    continue
                compared += 1
                if view(cell.value) != view(other.value):
                    differing.append((path, wa.title, cell.coordinate))
                    break
    assert compared > 10000
    assert not differing, differing[:5]
