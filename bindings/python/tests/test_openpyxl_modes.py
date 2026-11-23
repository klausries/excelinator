"""Read-only and write-only workbooks, and ISO 8601 date cells.

Golden reference: openpyxl (same scripts; read-only views of corpus files)."""
import datetime as dt
import glob
import io
import os
import warnings
import zipfile

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS


# --- ISO 8601 date cells -------------------------------------------------------------
def test_iso_date_cells_read_as_openpyxl_reads_them():
    path = os.path.join(CORPUS, "src-calamine", "tests", "date_iso.xlsx")
    if not os.path.exists(path):
        pytest.skip("corpus not present")
    a, b = openpyxl.load_workbook(path).active, x.load_workbook(path).active
    for r in range(1, 4):
        assert (b.cell(r, 1).value, b.cell(r, 1).data_type, b.cell(r, 1).is_date) == (a.cell(r, 1).value, a.cell(r, 1).data_type, a.cell(r, 1).is_date)


@pytest.mark.parametrize("epoch", ["1900", "1904"])
def test_iso_dates_written_as_openpyxl_writes_them(epoch):
    values = [dt.date(2026, 11, 13), dt.datetime(2026, 11, 13, 19, 30, 5), dt.time(8, 0), dt.datetime(1999, 12, 31, 23, 59, 59, 250000)]
    outs = []
    for m in (openpyxl, x):
        wb = m.Workbook(iso_dates=True)
        if epoch == "1904":
            wb.epoch = openpyxl.utils.datetime.CALENDAR_MAC_1904 if m is openpyxl else x.utils.datetime.CALENDAR_MAC_1904
        for v in values:
            wb.active.append([v])
        buf = io.BytesIO()
        wb.save(buf)
        sheet = zipfile.ZipFile(buf).read("xl/worksheets/sheet1.xml")
        assert sheet.count(b't="d"') == 4
        ws = openpyxl.load_workbook(io.BytesIO(buf.getvalue())).active
        outs.append([ws.cell(r, 1).value for r in range(1, 5)])
    assert outs[0] == outs[1]


def test_iso_dates_survive_saving_a_loaded_workbook():
    wb = x.Workbook(iso_dates=True)
    wb.active["A1"] = dt.datetime(2021, 1, 1, 10, 10, 10)
    buf = io.BytesIO()
    wb.save(buf)
    loaded = x.load_workbook(io.BytesIO(buf.getvalue()))
    loaded.active["B1"] = 1
    out = io.BytesIO()
    loaded.save(out)
    assert b'<c r="A1" s="1" t="d"><v>2021-01-01T10:10:10</v></c>' in zipfile.ZipFile(out).read("xl/worksheets/sheet1.xml")


# --- read-only ------------------------------------------------------------------------
def cell_view(c):
    if type(c).__name__ == "EmptyCell":
        return "E"
    if c.value is None and not c.has_style:
        return "E"  # an empty cell without format: the module does not keep it
    value = c.value
    if type(value).__name__ == "ArrayFormula":
        value = ("array", value.ref, value.text)
    return (value, c.data_type, c.row, c.column, c.coordinate, c.has_style, c.number_format, c.is_date)


def trimmed(rows):
    # Trailing rows of empty cells without format: openpyxl gives them, the module does not keep them.
    while rows and all(v in ("E", None) for v in rows[-1]):
        rows.pop()
    return rows


def ro_view(ws):
    rows = trimmed([tuple(cell_view(c) for c in r) for r in ws.iter_rows()])
    vals = trimmed([tuple(("array", v.ref, v.text) if type(v).__name__ == "ArrayFormula" else v for v in r) for r in ws.iter_rows(values_only=True)])
    sub = trimmed([tuple(cell_view(c) for c in r) for r in ws.iter_rows(min_row=2, max_row=4, min_col=2, max_col=3)])
    return rows, vals, sub, cell_view(ws["B2"]), cell_view(ws.cell(1, 1)), (ws.min_row, ws.min_column, ws.max_row, ws.max_column)


def test_read_only_objects_behave_like_openpyxls():
    buf = io.BytesIO()
    wb = openpyxl.Workbook()
    wb.active.append([1, "a", None, dt.date(2020, 1, 2)])
    wb.active.append([None, None, 3])
    wb.save(buf)
    for m in (openpyxl, x):
        rwb = m.load_workbook(io.BytesIO(buf.getvalue()), read_only=True)
        ws = rwb.active
        assert type(ws).__name__ == "ReadOnlyWorksheet" and type(ws["A1"]).__name__ == "ReadOnlyCell"
        with pytest.raises(AttributeError):
            ws["A1"].value = 2
        with pytest.raises(TypeError):
            ws["A1"] = 2
        with pytest.raises(TypeError):
            rwb.save(io.BytesIO())
        with pytest.raises(m.utils.exceptions.ReadOnlyWorkbookException):
            rwb.create_sheet()
        assert ws.calculate_dimension() == "A1:D2"
        ws.reset_dimensions()
        assert ws.max_row is None
        with pytest.raises(ValueError):
            ws.calculate_dimension()
        assert [tuple(c.value for c in r) for r in ws.rows] == [(1, "a", None, dt.datetime(2020, 1, 2)), (None, None, 3)]
        rwb.close()


FILES = sorted(f for f in glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)
               if os.path.getsize(f) < 300_000 and "OOM" not in f.upper())[::5]
# Formulas whose whitespace openpyxl moves (see docs/TECHNIK.md, comparison with openpyxl).
KNOWN = {"NewlineInFormulas.xlsx", "testSharedFormulasRangeSetBlankBug.xlsx"}


@pytest.mark.skipif(not FILES, reason="corpus not present")
def test_read_only_corpus_views_match_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in FILES:
        try:
            a, b = openpyxl.load_workbook(path, read_only=True), x.load_workbook(path, read_only=True)
        except Exception:
            continue
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            try:
                if wa.max_row is not None and wa.max_row > 2000:
                    continue
                va = ro_view(wa)
            except Exception:
                continue
            if "_x" in str(va) or os.path.basename(path) in KNOWN:
                continue  # openpyxl does not decode _xHHHH_; formula whitespace (docs/TECHNIK.md)
            compared += 1
            if ro_view(wb_) != va:
                differing.append((path, wa.title))
    assert compared > 300
    assert not differing, differing[:5]


# --- write-only -----------------------------------------------------------------------
def test_write_only_behaves_like_openpyxl():
    outs = []
    for m in (openpyxl, x):
        wb = m.Workbook(write_only=True)
        assert wb.worksheets == [] and wb.active is None
        ws = wb.create_sheet("Daten")
        ws.freeze_panes = "A2"  # before the first row: openpyxl writes the sheet's head then
        assert type(ws).__name__ == "WriteOnlyWorksheet"
        with pytest.raises(AttributeError):
            ws.cell(1, 1)
        with pytest.raises(TypeError):
            ws["A1"]
        with pytest.raises(TypeError):
            ws.append({"A": 1})
        cell = m.cell.WriteOnlyCell(ws, value="fett")
        cell.font = m.styles.Font(bold=True)
        ws.append([cell, 2, None, 3.5])
        ws.append(v for v in range(3))
        buf = io.BytesIO()
        wb.save(buf)
        if m is x:  # openpyxl fails here on its own temporary file and generator instead
            with pytest.raises(m.utils.exceptions.WorkbookAlreadySaved):
                wb.save(io.BytesIO())
            with pytest.raises(m.utils.exceptions.WorkbookAlreadySaved):
                ws.append([1])
        r = openpyxl.load_workbook(io.BytesIO(buf.getvalue()))["Daten"]
        outs.append(([[c.value for c in row] for row in r.iter_rows()], r["A1"].font.b, r.freeze_panes))
    assert outs[0] == outs[1]
