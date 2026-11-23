"""Rows and columns: height, width, hidden, outline levels (grouping), collapsed, thick
borders, best fit and the format of whole rows and columns (`ws.row_dimensions`,
`ws.column_dimensions`, `group`).

Golden reference: openpyxl (same scripts, same corpus files)."""
import glob
import importlib
import io
import os
import warnings

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS


def font(f):
    return None if f is None else (bool(f.b), bool(f.i), f.sz, f.name, f.color.value if f.color is not None else None)


def fmt(d):
    return (font(d.font), d.fill.fill_type if d.fill is not None else None, d.number_format,
            d.border.left.style if d.border is not None and d.border.left is not None else None)


def view(ws, styled=None):
    """Rows and columns as plain values. The format of a row or column counts only where
    openpyxl says it has one (`styled`: keys from openpyxl's side): for those without,
    openpyxl shows the file's first font and fill, Excel and the module the Normal style."""
    rows, cols = {}, {}
    for k in list(ws.row_dimensions):
        d = ws.row_dimensions[k]
        rows[int(k)] = (d.ht, bool(d.customHeight), bool(d.hidden), d.outlineLevel or 0, bool(d.collapsed), bool(d.thickTop), bool(d.thickBot),
                        fmt(d) if styled is None or ("r", int(k)) in styled else None)
    for k in list(ws.column_dimensions):
        d = ws.column_dimensions[k]
        d.reindex()
        cols[k] = (d.min, d.max, round(d.width, 6) if d.customWidth else None, bool(d.bestFit), bool(d.hidden), d.outlineLevel or 0,
                   bool(d.collapsed), fmt(d) if styled is None or ("c", k) in styled else None)
    return rows, cols


def styled_keys(ws):
    return {("r", int(k)) for k, d in ws.row_dimensions.items() if d.has_style} | {("c", k) for k, d in ws.column_dimensions.items() if d.has_style}


def build(m):
    st = importlib.import_module(m.__name__ + ".styles")
    wb = m.Workbook()
    ws = wb.active
    for r in range(1, 30):
        ws.append([r, r * 2, r * 3, "x"])
    ws.row_dimensions.group(3, 7, outline_level=1, hidden=True)
    ws.row_dimensions.group(4, 5, outline_level=2, hidden=True)
    ws.row_dimensions[8].collapsed = True
    ws.row_dimensions[10].height = 30
    ws.row_dimensions[11].hidden = True
    ws.row_dimensions[12].font = st.Font(b=True, color="FF0000")
    ws.row_dimensions[12].fill = st.PatternFill("solid", fgColor="FFFF00")
    ws.row_dimensions[13].thickBot = True
    ws.row_dimensions[40].height = 5  # a row without cells
    ws.column_dimensions.group("B", "C", outline_level=1, hidden=True)
    ws.column_dimensions["D"].width = 25.5
    ws.column_dimensions["E"].bestFit = True
    ws.column_dimensions["E"].width = 9
    ws.column_dimensions["F"].font = st.Font(i=True)
    ws.column_dimensions["F"].width = 12
    ws.column_dimensions["G"].collapsed = True
    ws.column_dimensions["G"].width = 4
    buf = io.BytesIO()
    wb.save(buf)
    return buf.getvalue()


def read(data, module=openpyxl):
    return module.load_workbook(io.BytesIO(data)).active


def test_built_rows_and_columns_read_back_as_openpyxls():
    assert view(read(build(x))) == view(read(build(openpyxl)))


def test_rows_and_columns_are_read_as_openpyxl_reads_them():
    data = build(openpyxl)
    assert view(read(data, x)) == view(read(data))


def test_outline_levels_reach_the_sheet_format():
    # Excel shows the outline buttons up to these levels; openpyxl leaves them out.
    ws = read(build(x))
    assert (ws.sheet_format.outlineLevelRow, ws.sheet_format.outlineLevelCol) == (2, 1)


def test_editing_dimensions_of_a_loaded_workbook_keeps_the_rest():
    data = build(openpyxl)
    ours, theirs = read(data, x).parent, read(data).parent
    for wb in (ours, theirs):
        ws = wb.active
        ws.row_dimensions[10].hidden = True
        ws.column_dimensions["D"].width = 7
        ws.row_dimensions.group(20, 22, outline_level=3)
    buf = io.BytesIO()
    ours.save(buf)
    obuf = io.BytesIO()
    theirs.save(obuf)
    assert view(read(buf.getvalue())) == view(read(obuf.getvalue()))


def test_dimension_objects_behave_like_openpyxls():
    for module in (openpyxl, x):
        ws = module.Workbook().active
        col = ws.column_dimensions["C"]
        assert (col.width, col.index) == (13, "C")
        # openpyxl counts the default width as set and writes width 13 for every column it
        # touched (hiding one widens it); the module writes a width only when one is given.
        assert col.customWidth is (module is openpyxl)
        col.width = 20
        col.reindex()
        assert (col.min, col.max, col.range, col.customWidth) == (3, 3, "C:C", True)
        row = ws.row_dimensions[4]
        assert (row.ht, row.customHeight, row.customFormat, row.index) == (None, False, False, 4)
        row.height = 22
        assert (row.ht, row.customHeight) == (22, True)


FILES = sorted(f for f in glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)
               if os.path.getsize(f) < 1_000_000 and "OOM" not in f.upper())


@pytest.mark.skipif(not FILES, reason="corpus not present")
def test_corpus_rows_and_columns_match_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in FILES:
        try:
            a, b = openpyxl.load_workbook(path), x.load_workbook(path)
        except Exception:
            continue
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            styled = styled_keys(wa)
            va, vb = view(wa, styled), view(wb_, styled)
            # A dimension only one side lists is compared with a fresh one of the other.
            for k in set(va[0]) ^ set(vb[0]):
                wa.row_dimensions[k], wb_.row_dimensions[k]
            for k in set(va[1]) ^ set(vb[1]):
                wa.column_dimensions[k], wb_.column_dimensions[k]
            va, vb = view(wa, styled), view(wb_, styled)
            compared += len(va[0]) + len(va[1])
            if va != vb:
                differing.append((path, wa.title))
    assert compared > 10000
    assert not differing, differing[:5]
