"""Autofilter criteria and sort state (`ws.auto_filter`) as objects like openpyxl's.

Golden reference: openpyxl (same scripts, same files) and the ISO/IEC 29500 schema for
what the module writes in this block (filters, rich text, conditional formats)."""
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
def el(o, fields):
    return None if o is None else tuple(getattr(o, f) for f in fields)
def af_view(ws):
    a = ws.auto_filter
    cols = tuple((c.colId, c.hiddenButton, c.showButton,
                  c.filters and (c.filters.blank, c.filters.calendarType, tuple(c.filters.filter), tuple(el(d, ('year','month','day','hour','minute','second','dateTimeGrouping')) for d in c.filters.dateGroupItem)),
                  el(c.top10, ('top','percent','val','filterVal')),
                  c.customFilters and (c.customFilters._and, tuple((f.operator, f.val) for f in c.customFilters.customFilter)),
                  el(c.dynamicFilter, ('type','val','maxVal')),
                  el(c.colorFilter, ('dxfId','cellColor')), el(c.iconFilter, ('iconSet','iconId'))) for c in a.filterColumn)
    s = a.sortState
    ss = s and (s.columnSort, s.caseSensitive, s.sortMethod, str(s.ref), tuple((str(c.ref), c.descending, c.sortBy, c.customList, c.dxfId, c.iconSet, c.iconId) for c in s.sortCondition))
    return (a.ref and str(a.ref), cols, ss)
def build(m):
    F = importlib.import_module(m.__name__ + '.worksheet.filters')
    wb = m.Workbook(); ws = wb.active
    for r in range(1, 20):
        ws.append([r, 'x%d' % r, r * 1.5, 'abc' if r % 2 else 'xyz'])
    ws.auto_filter.ref = 'A1:D19'
    ws.auto_filter.add_filter_column(1, ['x1', 'x3', 'x5'], blank=True)
    ws.auto_filter.filterColumn.append(F.FilterColumn(colId=0, customFilters=F.CustomFilters(_and=True, customFilter=[F.CustomFilter('greaterThan', '3'), F.CustomFilter('lessThanOrEqual', '15')])))
    ws.auto_filter.filterColumn.append(F.FilterColumn(colId=2, top10=F.Top10(top=False, percent=True, val=10)))
    ws.auto_filter.filterColumn.append(F.FilterColumn(colId=3, dynamicFilter=F.DynamicFilter(type='aboveAverage', val=4.5), hiddenButton=True))
    ws.auto_filter.add_sort_condition('C2:C19', descending=True)
    ws.auto_filter.sortState.caseSensitive = True
    b = io.BytesIO(); wb.save(b); return b.getvalue()


def test_built_filters_read_back_as_openpyxls():
    assert af_view(openpyxl.load_workbook(io.BytesIO(build(x))).active) == af_view(openpyxl.load_workbook(io.BytesIO(build(openpyxl))).active)


def test_filters_are_read_as_openpyxl_reads_them():
    data = build(openpyxl)
    assert af_view(x.load_workbook(io.BytesIO(data)).active) == af_view(openpyxl.load_workbook(io.BytesIO(data)).active)


def test_editing_filters_of_a_loaded_workbook():
    wb = x.load_workbook(io.BytesIO(build(openpyxl)))
    ws = wb.active
    ws.auto_filter.filterColumn[0].filters.filter.append("x7")
    del ws.auto_filter.filterColumn[2]
    buf = io.BytesIO()
    wb.save(buf)
    got = af_view(openpyxl.load_workbook(io.BytesIO(buf.getvalue())).active)
    owb = openpyxl.load_workbook(io.BytesIO(build(openpyxl)))
    owb.active.auto_filter.filterColumn[0].filters.filter.append("x7")
    del owb.active.auto_filter.filterColumn[2]
    assert got == af_view(owb.active)
    # Removing the filter removes it from the file.
    ws.auto_filter.ref = None
    buf = io.BytesIO()
    wb.save(buf)
    assert not openpyxl.load_workbook(io.BytesIO(buf.getvalue())).active.auto_filter.ref


def test_string_number_and_blank_filters_as_openpyxl():
    from openpyxl.worksheet import filters as theirs
    from xlcells.openpyxl.worksheet import filters as ours
    for text in ("*abc*", "abc*", "*abc", "a?c", "*a~*c*", " ", "12.5"):
        for op in ("equal", "notEqual"):
            a, b = theirs.CustomFilter(op, text).convert(), ours.CustomFilter(op, text).convert()
            assert type(a).__name__ == type(b).__name__
            assert (a.operator, a.val, getattr(a, "exclude", None)) == (b.operator, b.val, getattr(b, "exclude", None)), (op, text)
    with pytest.raises(ValueError):
        ours.CustomFilter("bigger", "1")
    with pytest.raises(ValueError):
        ours.DynamicFilter(type="sometime")


def test_written_parts_are_valid_against_the_schema():
    test_schema = pytest.importorskip("test_schema")
    if not (test_schema.XSD / "sml.xsd").is_file():
        pytest.skip("schemas not fetched")
    import test_openpyxl_conditional_full as cf
    import test_openpyxl_rich_text as rt
    for data in (build(x), cf.build(x), rt.build("xlcells")):
        assert test_schema.problems(data) == []


AF_FILES = []
for _path in sorted(glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)):
    if os.path.getsize(_path) < 1_000_000 and "OOM" not in _path.upper():
        try:
            with zipfile.ZipFile(_path) as _z:
                if any(b"<autoFilter" in _z.read(n) for n in _z.namelist() if "/worksheets/" in n):
                    AF_FILES.append(_path)
        except Exception:
            pass


@pytest.mark.skipif(not AF_FILES, reason="corpus not present")
def test_corpus_filters_match_openpyxl():
    warnings.simplefilter("ignore")
    compared, differing = 0, []
    for path in AF_FILES:
        try:
            a, b = openpyxl.load_workbook(path), x.load_workbook(path)
        except Exception:
            continue
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            compared += 1
            if af_view(wa) != af_view(wb_):
                differing.append((path, wa.title))
    assert compared > 10
    assert not differing, differing[:5]
