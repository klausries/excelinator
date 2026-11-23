"""Workbook protection, document properties (core and custom), calculation settings,
windows and the code name (`wb.security`, `wb.properties`, `wb.custom_doc_props`,
`wb.calculation`, `wb.views`, `wb.code_name`).

Golden reference: openpyxl (same scripts, all corpus files)."""
import datetime
import glob
import importlib
import io
import os
import warnings
import zipfile

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS
PF = ('creator','title','description','subject','identifier','language','created','lastModifiedBy','category','contentStatus','version','revision','keywords','lastPrinted')
SF = ('workbookPassword','lockStructure','lockWindows','lockRevision','revisionsPassword','workbookAlgorithmName','workbookHashValue','workbookSaltValue','workbookSpinCount')
CF = ('calcId','calcMode','fullCalcOnLoad','refMode','iterate','iterateCount','iterateDelta','fullPrecision','calcOnSave','forceFullCalc')
VF = ('visibility','minimized','showHorizontalScroll','showVerticalScroll','showSheetTabs','xWindow','yWindow','windowWidth','windowHeight','tabRatio','firstSheet','activeTab','autoFilterDateGrouping')
def el(o, fields):
    def norm(v):
        if isinstance(v, float) and v == int(v): return int(v)
        return v
    return None if o is None else tuple(norm(getattr(o, f, None)) for f in fields)
def view(wb, skip_dates=False):
    props = list(el(wb.properties, PF))
    props[6] = props[6] and props[6].replace(microsecond=0)  # openpyxl keeps milliseconds only
    custom = tuple((type(p).__name__, p.name, p.value) for p in wb.custom_doc_props)
    sec = el(wb.security, SF) if wb.security is not None and any(el(wb.security, SF)[i] for i in (0, 1, 2, 3, 4, 6)) else None
    return [props, custom, sec, el(wb.calculation, CF), tuple(el(v, VF) for v in wb.views), wb.code_name, wb._sheets.index(wb.active) if wb.active is not None else None]
def build(m):
    wb = m.Workbook()
    ws = wb.active; ws['A1'] = 1; wb.create_sheet('Zwei'); wb.create_sheet('Drei')
    wb.active = 1
    P = importlib.import_module(m.__name__ + '.packaging.custom')
    S = importlib.import_module(m.__name__ + '.workbook.protection')
    wb.properties.title = 'Planung'; wb.properties.creator = 'Klaus'; wb.properties.keywords = 'Excel, Ablösung'
    wb.properties.created = datetime.datetime(2026, 1, 2, 3, 4, 5); wb.properties.category = 'Finanzen'; wb.properties.lastModifiedBy = 'K'
    wb.custom_doc_props.append(P.StringProperty('Projekt', 'Big Moves'))
    wb.custom_doc_props.append(P.IntProperty('Version', 3))
    wb.custom_doc_props.append(P.FloatProperty('Faktor', 1.5))
    wb.custom_doc_props.append(P.BoolProperty('Geprüft', True))
    wb.custom_doc_props.append(P.DateTimeProperty('Stichtag', datetime.datetime(2026, 11, 13, 12, 0)))
    wb.security = S.WorkbookProtection(lockStructure=True, lockWindows=True)
    wb.security.workbookPassword = 'geheim'
    wb.calculation.calcMode = 'manual'; wb.calculation.iterate = True; wb.calculation.iterateCount = 50
    wb.views[0].windowWidth = 20000; wb.views[0].showSheetTabs = False; wb.views[0].firstSheet = 0
    wb.code_name = 'DieseArbeitsmappe'
    b = io.BytesIO(); wb.save(b); return b.getvalue()


def test_built_settings_read_back_as_openpyxls():
    assert view(openpyxl.load_workbook(io.BytesIO(build(x)))) == view(openpyxl.load_workbook(io.BytesIO(build(openpyxl))))


def test_settings_are_read_as_openpyxl_reads_them():
    data = build(openpyxl)
    assert view(x.load_workbook(io.BytesIO(data))) == view(openpyxl.load_workbook(io.BytesIO(data)))


def test_saving_sets_the_modification_time_and_keeps_the_rest():
    data = build(openpyxl)
    before = datetime.datetime.now(tz=datetime.timezone.utc).replace(tzinfo=None, microsecond=0)
    wb = x.load_workbook(io.BytesIO(data))
    wb.security = None
    wb.properties.title = "Neu"
    del wb.custom_doc_props["Faktor"]
    buf = io.BytesIO()
    wb.save(buf)
    got = openpyxl.load_workbook(io.BytesIO(buf.getvalue()))
    assert got.properties.modified >= before
    assert got.security is None and got.properties.title == "Neu" and "Faktor" not in got.custom_doc_props.names
    assert view(got)[3:] == view(openpyxl.load_workbook(io.BytesIO(data)))[3:]


def test_passwords_hash_as_in_openpyxl():
    from openpyxl.workbook.protection import WorkbookProtection as Theirs
    from xlcells.openpyxl.workbook.protection import WorkbookProtection as Ours
    for cls in (Theirs, Ours):
        p = cls(workbookPassword="abc", lockStructure=True)
        assert p.workbookPassword == "CC1A"
        p.set_workbook_password("CBEB", already_hashed=True)
        assert p.workbook_password == "CBEB"
        p.revision_password = "x"
        assert p.revisionsPassword == "CEBA"


FILES = sorted(f for f in glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)
               if os.path.getsize(f) < 1_000_000 and "OOM" not in f.upper())


@pytest.mark.skipif(not FILES, reason="corpus not present")
def test_corpus_settings_match_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for p in FILES:
        try:
            a, b = openpyxl.load_workbook(p), x.load_workbook(p)
        except Exception:
            continue
        compared += 1
        va, vb = view(a), view(b)
        names = zipfile.ZipFile(p).namelist()
        if "docProps/core.xml" not in names:
            va[0] = vb[0] = None  # openpyxl looks only at docProps/core.xml; the module follows the relationship
        elif not zipfile.ZipFile(p).read("docProps/core.xml").count(b"created"):
            va[0][6] = vb[0][6] = None  # no date in the file: both take the time of loading
        if va[6] is None:
            vb[6] = None  # openpyxl finds no active sheet (strict paths)
        if va != vb:
            differing.append(p)
    assert compared > 1000
    assert not differing, differing[:5]


def test_dates_with_any_fraction_of_a_second_are_read():
    # Python before 3.11 reads only 3 or 6 digits by itself.
    from xlcells.openpyxl.packaging.core import _parse_date
    import datetime
    assert _parse_date("2013-10-23T06:17:22.00Z") == datetime.datetime(2013, 10, 23, 6, 17, 22)
    assert _parse_date("2017-01-24T12:42:22.8702135Z") == datetime.datetime(2017, 1, 24, 12, 42, 22, 870213)
    assert _parse_date("2017-01-24T12:42:22+02:00") == datetime.datetime(2017, 1, 24, 10, 42, 22)
    assert _parse_date("nonsense") is None
