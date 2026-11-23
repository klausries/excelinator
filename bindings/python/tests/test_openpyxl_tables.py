"""Tables (list objects): `Table`, `TableColumn`, `TableStyleInfo`, `ws.add_table`,
`ws.tables`.

Golden references: openpyxl (same scripts, corpus files with tables, saved and read
again) and LibreOffice (it must see the tables, hidden and grouped rows and columns of
files the module writes)."""
import glob
import importlib
import io
import os
import shutil
import subprocess
import tempfile
import warnings
import zipfile

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS
def el(o, fields): return None if o is None else tuple(getattr(o, f) for f in fields)
TF = ('id','displayName','name','ref','comment','tableType','headerRowCount','insertRow','insertRowShift','totalsRowCount','totalsRowShown','published',
      'headerRowDxfId','dataDxfId','totalsRowDxfId','headerRowBorderDxfId','tableBorderDxfId','totalsRowBorderDxfId','headerRowCellStyle','dataCellStyle','totalsRowCellStyle','connectionId')
CF = ('id','uniqueName','name','totalsRowFunction','totalsRowLabel','queryTableFieldId','headerRowDxfId','dataDxfId','totalsRowDxfId','headerRowCellStyle','dataCellStyle','totalsRowCellStyle')
def tview(t):
    cols = tuple(el(c, CF) + (el(c.calculatedColumnFormula, ('array','attr_text')), el(c.totalsRowFormula, ('array','attr_text')), el(c.xmlColumnPr, ('mapId','xpath','denormalized','xmlDataType'))) for c in t.tableColumns)
    af = t.autoFilter and (t.autoFilter.ref and str(t.autoFilter.ref), len(t.autoFilter.filterColumn))
    return el(t, TF)[1:] + (cols, af, el(t.tableStyleInfo, ('name','showFirstColumn','showLastColumn','showRowStripes','showColumnStripes')))
def view(wb):
    return [sorted((t.name, tview(t)) for t in ws.tables.values()) for ws in wb.worksheets]
def build(m):
    T = importlib.import_module(m.__name__ + '.worksheet.table')
    wb = m.Workbook(); ws = wb.active
    ws.append(['Name', 'Menge', 'Preis', 'Summe'])
    for i in range(5): ws.append(['P%d' % i, i, i * 1.5, '=B%d*C%d' % (i + 2, i + 2)])
    t = T.Table(displayName='Bestellungen', ref='A1:D6')
    t.tableStyleInfo = T.TableStyleInfo(name='TableStyleMedium9', showFirstColumn=False, showLastColumn=False, showRowStripes=True, showColumnStripes=False)
    ws.add_table(t)
    ws2 = wb.create_sheet('Zwei')
    ws2.append(['a', 'b'])
    ws2.append([1, 2])
    ws2.append(['Summe', 3])
    cols = [T.TableColumn(id=1, name='a', totalsRowLabel='Summe'), T.TableColumn(id=2, name='b', totalsRowFunction='sum', calculatedColumnFormula=T.TableFormula(attr_text='Tabelle2[[#This Row],[a]]*2'))]
    t2 = T.Table(displayName='Tabelle2', ref='A1:B3', totalsRowCount=1, tableColumns=cols, comment='mit Summe')
    ws2.add_table(t2)
    b = io.BytesIO(); wb.save(b); return b.getvalue()


def test_built_tables_read_back_as_openpyxls():
    assert view(openpyxl.load_workbook(io.BytesIO(build(x)))) == view(openpyxl.load_workbook(io.BytesIO(build(openpyxl))))


def test_tables_are_read_as_openpyxl_reads_them():
    data = build(openpyxl)
    assert view(x.load_workbook(io.BytesIO(data))) == view(openpyxl.load_workbook(io.BytesIO(data)))


def test_editing_a_loaded_table():
    data = build(openpyxl)
    results = []
    for module in (openpyxl, x):
        wb = module.load_workbook(io.BytesIO(data))
        t = wb.active.tables["Bestellungen"]
        t.ref = "A1:D5"
        t.autoFilter.ref = "A1:D5"
        t.tableStyleInfo.showRowStripes = False
        t.tableColumns[3].totalsRowFunction = "sum"
        buf = io.BytesIO()
        wb.save(buf)
        results.append(view(openpyxl.load_workbook(io.BytesIO(buf.getvalue()))))
    assert results[0] == results[1]


def test_names_are_checked_as_openpyxl_checks_them():
    for module in (openpyxl, x):
        T = importlib.import_module(module.__name__ + ".worksheet.table")
        wb = module.Workbook()
        ws = wb.active
        ws.add_table(T.Table(displayName="Daten", ref="A1:B2"))
        with pytest.raises(ValueError):
            ws.add_table(T.Table(displayName="DATEN", ref="C1:D2"))
        with pytest.raises(ValueError):
            T.Table(displayName="mit Leerzeichen", ref="A1:B2")
        with pytest.raises(TypeError):
            ws.tables.add("kein Table")
        assert ws.tables.items() == [("Daten", "A1:B2")]


def test_written_tables_are_valid_against_the_schema():
    test_schema = pytest.importorskip("test_schema")
    if not (test_schema.XSD / "sml.xsd").is_file():
        pytest.skip("schemas not fetched")
    import test_openpyxl_dimensions as dims
    for data in (build(x), dims.build(x)):
        assert test_schema.problems(data) == []


@pytest.mark.skipif(shutil.which("soffice") is None, reason="LibreOffice not installed")
def test_libreoffice_sees_tables_and_grouped_rows_and_columns():
    import test_openpyxl_dimensions as dims
    with tempfile.TemporaryDirectory() as tmp:
        for name, data in (("tables.xlsx", build(x)), ("dims.xlsx", dims.build(x))):
            with open(os.path.join(tmp, name), "wb") as f:
                f.write(data)
        subprocess.run(["soffice", "--headless", "--convert-to", "xlsx:Calc MS Excel 2007 XML", "--outdir", os.path.join(tmp, "out"),
                        os.path.join(tmp, "tables.xlsx"), os.path.join(tmp, "dims.xlsx")], check=True, capture_output=True, timeout=180)
        wb = openpyxl.load_workbook(os.path.join(tmp, "out", "tables.xlsx"))
        got = [[(t.name, t.ref, [c.name for c in t.tableColumns], t.totalsRowCount or 0) for t in ws.tables.values()] for ws in wb.worksheets]
        assert got == [[("Bestellungen", "A1:D6", ["Name", "Menge", "Preis", "Summe"], 0)], [("Tabelle2", "A1:B3", ["a", "b"], 1)]]
        ws = openpyxl.load_workbook(os.path.join(tmp, "out", "dims.xlsx")).active
        rows = {k: (d.hidden, d.outlineLevel) for k, d in ws.row_dimensions.items() if d.hidden or d.outlineLevel}
        assert rows == {3: (True, 1), 4: (True, 2), 5: (True, 2), 6: (True, 1), 7: (True, 1), 11: (True, 0)}
        assert ws.row_dimensions[10].ht == 30 and ws.row_dimensions[40].ht == 5
        cols = {k: (d.min, d.max, d.hidden, d.outlineLevel) for k, d in ws.column_dimensions.items()}
        assert cols["B"] == (2, 3, True, 1) and round(ws.column_dimensions["D"].width, 1) == 25.5


TABLE_FILES = []
for _path in sorted(glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)):
    if os.path.getsize(_path) < 1_000_000 and "OOM" not in _path.upper():
        try:
            with zipfile.ZipFile(_path) as _z:
                if any("tables/" in n for n in _z.namelist()):
                    TABLE_FILES.append(_path)
        except Exception:
            pass


@pytest.mark.skipif(not TABLE_FILES, reason="corpus not present")
def test_corpus_tables_match_openpyxl_also_after_saving():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in TABLE_FILES:
        try:
            a, b = openpyxl.load_workbook(path), x.load_workbook(path)
        except Exception:
            continue
        va = view(a)
        compared += sum(len(v) for v in va)
        if view(b) != va:
            differing.append(path)
        buf = io.BytesIO()
        b.save(buf)
        # Saved and read again: the same tables (ids may be numbered anew).
        vc = view(openpyxl.load_workbook(io.BytesIO(buf.getvalue())))
        if [[(k, t[1:]) for k, t in s] for s in vc] != [[(k, t[1:]) for k, t in s] for s in va]:
            differing.append(("saved", path))
    assert compared > 100
    assert not differing, differing[:5]


def test_a_table_name_that_reads_as_a_cell_is_refused():
    # Excel repairs a file with such a name; checked with Excel (Microsoft 365 for Mac).
    from xlcells.openpyxl.worksheet.table import Table
    ws = x.Workbook().active
    for bad in ("T1", "ab12", "XFD1048576", "R", "c", "R1C1", "R2"):
        with pytest.raises(ValueError, match="reads as a cell"):
            ws.add_table(Table(displayName=bad, ref="A1:B2"))
    for good in ("Orders", "T1x", "XFE1", "Rate", "A0"):
        ws.add_table(Table(displayName=good, ref="A1:B2"))
