"""Inserting, deleting and moving rows, columns and ranges: the same operations with
openpyxl and the facade give the same cells (and the same sheet size)."""
import io

import openpyxl
from hypothesis import given, settings
from hypothesis import strategies as st

import xlcells.openpyxl as facade

OPS = st.lists(st.tuples(st.sampled_from(["insert_rows", "insert_cols", "delete_rows", "delete_cols", "move", "move_translate"]),
                         st.integers(1, 7), st.integers(1, 3), st.integers(-3, 4), st.integers(-3, 4)), min_size=1, max_size=5)


def filled(m):
    wb = m.Workbook()
    ws = wb.active
    for r in range(1, 9):
        for c in range(1, 7):
            if (r * c) % 3:
                ws.cell(r, c, value="=A{0}+$B{1}+C$2".format(r, c) if (r + c) % 4 == 0 else r * 10 + c)
    return wb


def apply(m, ops):
    wb = filled(m)
    ws = wb.active
    try:
        for name, a, b, dr, dc in ops:
            if name.startswith("move"):
                ws.move_range("B{0}:C{1}".format(a, a + b), rows=dr, cols=dc, translate=name.endswith("translate"))
            else:
                getattr(ws, name)(a, b)
    except Exception:  # leaving the sheet (ValueError, or openpyxl's TranslatorError)
        return "ValueError"
    cells = {k: c.value for k, c in ws._cells.items()}
    return cells, (ws.min_row, ws.max_row, ws.min_column, ws.max_column), wb


@settings(max_examples=400, deadline=None)
@given(OPS)
def test_structure_operations_match_openpyxl(ops):
    theirs, ours = apply(openpyxl, ops), apply(facade, ops)
    if theirs == "ValueError" or ours == "ValueError":
        assert theirs == ours
        return
    assert ours[:2] == theirs[:2]
    # and what is saved reads the same
    out = io.BytesIO()
    ours[2].save(out)
    back = openpyxl.load_workbook(io.BytesIO(out.getvalue())).active
    assert {k: c.value for k, c in back._cells.items() if c.value is not None} == {k: v for k, v in theirs[0].items() if v is not None}


def test_moving_on_a_loaded_sheet_keeps_formats_and_links():
    wb = openpyxl.Workbook()
    ws = wb.active
    ws["B2"] = "fett"
    ws["B2"].font = openpyxl.styles.Font(b=True)
    ws["B3"] = "Link"
    ws["B3"].hyperlink = "https://example.com"
    ws["C3"] = "=B2&B3"
    data = io.BytesIO()
    wb.save(data)
    ours = facade.load_workbook(io.BytesIO(data.getvalue()))
    ours.active.insert_rows(1, 2)
    ours.active.move_range("C5", cols=2, translate=True)
    out = io.BytesIO()
    ours.save(out)
    got = openpyxl.load_workbook(io.BytesIO(out.getvalue())).active
    assert got["B4"].value == "fett" and got["B4"].font.b
    assert got["B5"].hyperlink.target == "https://example.com" and got["B5"].hyperlink.ref == "B5"
    assert got["E5"].value == "=D2&D3"
