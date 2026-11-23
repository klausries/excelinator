"""Calculating a workbook (`wb.calculate()`) and reading .xls files.

Golden references: LibreOffice (recalculates the same file headless), xlrd (values of
.xls files) and openpyxl (reads what we save)."""
import glob
import io
import os
import shutil
import subprocess
import tempfile

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS, needs_corpus


def _model():
    wb = x.Workbook()
    ws = wb.active
    ws.title = "Model"
    ws.append(["qty", "price", "total", "rebate"])
    for q, p in [(3, 2.5), (10, 1.25), (7, 4.0), (1, 99.0)]:
        ws.append([q, p])
    for r in range(2, 6):
        ws[f"C{r}"] = f"=A{r}*B{r}"
        ws[f"D{r}"] = f'=IF(C{r}>20,C{r}*0.1,0)'
    ws["C6"] = "=SUM(C2:C5)"
    ws["D6"] = "=SUM(D2:D5)"
    ws["E6"] = "=C6-D6"
    ws["F6"] = '=ROUND(E6/COUNT(A2:A5),2)'
    other = wb.create_sheet("Other")
    other["A1"] = "=Model!E6*2"
    return wb


def _data_only(wb):
    buf = io.BytesIO()
    wb.save(buf)
    return openpyxl.load_workbook(io.BytesIO(buf.getvalue()), data_only=True)


def test_calculate_fills_the_values_that_openpyxl_reads_back():
    wb = _model()
    report = wb.calculate()
    assert report["settled"] and report["not_computed"] == {}
    assert report["computed"] == report["formula_cells"] == 13
    back = _data_only(wb)
    m = back["Model"]
    assert [m[f"C{r}"].value for r in range(2, 7)] == [7.5, 12.5, 28.0, 99.0, 147.0]
    assert m["D6"].value == pytest.approx(12.7)
    assert m["E6"].value == pytest.approx(134.3)
    assert m["F6"].value == pytest.approx(33.58)
    assert back["Other"]["A1"].value == pytest.approx(268.6)


def test_changed_inputs_are_calculated_again():
    wb = _model()
    wb.calculate()
    wb["Model"]["A2"] = 100
    report = wb.calculate()
    assert report["changed"] > 0
    assert _data_only(wb)["Model"]["C6"].value == pytest.approx(250.0 + 12.5 + 28.0 + 99.0)


def test_a_cycle_is_left_to_excel():
    wb = x.Workbook()
    ws = wb.active
    ws["A1"] = "=B1+1"
    ws["B1"] = "=A1+1"
    ws["C1"] = 5
    ws["D1"] = "=C1*2"
    report = wb.calculate()
    assert "circular reference" in report["not_computed"]
    assert wb.calculation.fullCalcOnLoad
    assert _data_only(wb).active["D1"].value == 10


@pytest.mark.skipif(shutil.which("soffice") is None, reason="LibreOffice not installed")
def test_calculated_values_match_libreoffice():
    wb = _model()
    wb.calculate()
    ours = _data_only(wb)
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "model.xlsx")
        wb.save(src)
        out = os.path.join(d, "out")
        subprocess.run(["soffice", "--headless", "--calc", "--convert-to", "xlsx:Calc MS Excel 2007 XML", "--outdir", out, src],
                       check=True, capture_output=True, timeout=180, env={**os.environ, "HOME": d})
        theirs = openpyxl.load_workbook(os.path.join(out, "model.xlsx"), data_only=True)
    for name in ("Model", "Other"):
        for row in ours[name].iter_rows():
            for c in row:
                t = theirs[name][c.coordinate].value
                if isinstance(c.value, float):
                    assert t == pytest.approx(c.value, rel=1e-12), c.coordinate
                else:
                    assert t == c.value, c.coordinate


# --- .xls --------------------------------------------------------------------------------
def _xls_files(n):
    files = sorted(glob.glob(os.path.join(CORPUS, "src-enron", "edrm", "*.xls")))
    return files[:: max(1, len(files) // n)][:n]


@pytest.mark.parametrize("path", _xls_files(40))
def test_xls_values_match_xlrd(path):
    xlrd = pytest.importorskip("xlrd")
    try:
        book = xlrd.open_workbook(path, formatting_info=False)
    except Exception:
        pytest.skip("xlrd cannot read the file")
    try:
        wb = x.load_workbook(path)
    except Exception as e:
        # Only what we refuse on purpose: Excel 95 files and encrypted ones.
        assert "BIFF5" in str(e) or "encrypted" in str(e), e
        return
    wb_values = x.load_workbook(path, data_only=True)
    for sheet in book.sheets():
        if sheet.name not in wb.sheetnames:
            continue
        ws = wb_values[sheet.name]
        for r in range(sheet.nrows):
            for c in range(sheet.ncols):
                cell = sheet.cell(r, c)
                ours = ws._cells.get((r + 1, c + 1))
                v = None if ours is None else ours.value
                if cell.ctype in (xlrd.XL_CELL_EMPTY, xlrd.XL_CELL_BLANK):
                    assert v in (None, ""), (sheet.name, r, c, v)
                elif cell.ctype == xlrd.XL_CELL_TEXT:
                    # An empty text result of a formula is no value in openpyxl.
                    assert v == cell.value or (cell.value == "" and v is None), (sheet.name, r, c)
                elif cell.ctype == xlrd.XL_CELL_NUMBER:
                    assert v == cell.value, (sheet.name, r, c)
                elif cell.ctype == xlrd.XL_CELL_DATE:
                    assert ours.is_date, (sheet.name, r, c)
                elif cell.ctype == xlrd.XL_CELL_BOOLEAN:
                    assert v is bool(cell.value), (sheet.name, r, c)
                elif cell.ctype == xlrd.XL_CELL_ERROR:
                    assert v == xlrd.error_text_from_code[cell.value], (sheet.name, r, c)


@needs_corpus
def test_xls_formulas_survive_saving_as_xlsx():
    files = [f for f in _xls_files(200) if os.path.getsize(f) < 300_000]
    seen = 0
    for path in files:
        try:
            wb = x.load_workbook(path)
        except Exception:
            continue
        formulas = {(ws.title, k): c.value for ws in wb.worksheets for k, c in ws._cells.items() if isinstance(c.value, str) and c.value.startswith("=")}
        if not formulas:
            continue
        buf = io.BytesIO()
        wb.save(buf)
        back = openpyxl.load_workbook(io.BytesIO(buf.getvalue()))
        for (title, (r, c)), f in formulas.items():
            v = back[title].cell(r, c).value
            assert (v.text if hasattr(v, "text") else v) == f, (path, title, r, c)
        seen += 1
        if seen >= 10:
            break
    assert seen >= 5


def test_xls_errors_are_clear():
    with pytest.raises(Exception, match="xls|compound"):
        x.load_workbook(io.BytesIO(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\0" * 600))
