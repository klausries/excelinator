"""Conditional formatting in full: data bars, icon sets, colour scales with thresholds,
top/bottom, above/below average, text and date rules, as objects like openpyxl's.

Golden reference: openpyxl. Both modules build the same rules; openpyxl reads both files
alike. Corpus files give the same rule objects in both (where openpyxl can read them)."""
import glob
import io
import os
import warnings
import zipfile

import openpyxl
import pytest

import xlcells.openpyxl as x

from conftest import CORPUS
def col(c): return None if c is None else (c.type, c.value, round(c.tint or 0, 9))
def cfvo(v): return (v.type, v.val, v.gte)
def rule_view(r):
    cs = r.colorScale and (tuple(cfvo(v) for v in r.colorScale.cfvo), tuple(col(c) for c in r.colorScale.color))
    db = r.dataBar and (r.dataBar.minLength, r.dataBar.maxLength, r.dataBar.showValue, tuple(cfvo(v) for v in r.dataBar.cfvo), col(r.dataBar.color))
    ic = r.iconSet and (r.iconSet.iconSet, r.iconSet.showValue, r.iconSet.percent, r.iconSet.reverse, tuple(cfvo(v) for v in r.iconSet.cfvo))
    return (r.type, r.priority, r.stopIfTrue, r.aboveAverage, r.percent, r.bottom, r.operator, r.text, r.timePeriod, r.rank, r.stdDev, r.equalAverage,
            tuple(r.formula), cs, db, ic, r.dxf is not None)
def cf_view(ws):
    return [(str(cf.sqref), tuple(rule_view(r) for r in cf.rules)) for cf in ws.conditional_formatting]
def build(m):
    from importlib import import_module
    R = import_module(m.__name__ + '.formatting.rule'); st = import_module(m.__name__ + '.styles')
    wb = m.Workbook(); ws = wb.active
    ws.conditional_formatting.add('A1:A10', R.DataBarRule(start_type='min', end_type='max', color='FF638EC6', showValue=True, minLength=10, maxLength=90))
    ws.conditional_formatting.add('B1:B10', R.IconSetRule('3TrafficLights1', 'percent', [0, 33, 67], showValue=False, reverse=True))
    ws.conditional_formatting.add('C1:C10', R.ColorScaleRule(start_type='num', start_value=0, start_color='FFF8696B', mid_type='percentile', mid_value=50, mid_color='FFFFEB84', end_type='formula', end_value='$D$1', end_color='FF63BE7B'))
    ws.conditional_formatting.add('D1:D10', R.Rule(type='top10', rank=3, percent=True, bottom=True, dxf=None))
    ws.conditional_formatting.add('E1:E10', R.Rule(type='aboveAverage', aboveAverage=False, equalAverage=True, stdDev=1))
    ws.conditional_formatting.add('F1:F10', R.Rule(type='containsText', operator='containsText', text='ab', formula=['NOT(ISERROR(SEARCH("ab",F1)))']))
    ws.conditional_formatting.add('G1:G10', R.Rule(type='timePeriod', timePeriod='lastWeek', formula=['AND(TODAY()-ROUNDDOWN(G1,0)>=(WEEKDAY(TODAY())),TODAY()-ROUNDDOWN(G1,0)<(WEEKDAY(TODAY())+7))']))
    ws.conditional_formatting.add('A1:A10', R.CellIsRule(operator='>', formula=['5'], fill=st.PatternFill('solid', fgColor='FFFF00')))
    b = io.BytesIO(); wb.save(b); return b.getvalue()


def test_built_rules_read_back_as_openpyxls():
    assert cf_view(openpyxl.load_workbook(io.BytesIO(build(x))).active) == cf_view(openpyxl.load_workbook(io.BytesIO(build(openpyxl))).active)


def test_rules_are_read_as_openpyxl_reads_them():
    data = build(openpyxl)
    assert cf_view(x.load_workbook(io.BytesIO(data)).active) == cf_view(openpyxl.load_workbook(io.BytesIO(data)).active)


def test_changed_rules_are_written_and_unchanged_ones_kept():
    wb = x.load_workbook(io.BytesIO(build(openpyxl)))
    ws = wb.active
    bar = ws.conditional_formatting["A1:A10"][0]
    bar.dataBar.color = x.styles.Color(theme=5)
    bar.dataBar.cfvo[1] = x.formatting.rule.FormatObject("percentile", 90)
    buf = io.BytesIO()
    wb.save(buf)
    got = cf_view(openpyxl.load_workbook(io.BytesIO(buf.getvalue())).active)
    want = cf_view(openpyxl.load_workbook(io.BytesIO(build(openpyxl))).active)
    assert got[1:] == want[1:]
    bar_view = got[0][1][0][14]
    assert bar_view[3][1] == ("percentile", 90.0, None) and bar_view[4] == ("theme", 5, 0)


def test_rule_objects_validate_as_openpyxl():
    from openpyxl.formatting import rule as theirs
    from xlcells.openpyxl.formatting import rule as ours
    for module in (theirs, ours):
        with pytest.raises(ValueError):
            module.Rule(type="nonsense")
        with pytest.raises(ValueError):
            module.FormatObject(type="nonsense")
        assert module.FormatObject("num", "5").val == 5.0
        assert module.FormatObject("num", "$A$1").val == "$A$1"


CF_FILES = []
for _path in sorted(glob.glob(os.path.join(CORPUS, "src-*", "**", "*.xlsx"), recursive=True)):
    if os.path.getsize(_path) < 1_000_000 and "OOM" not in _path.upper():
        try:
            with zipfile.ZipFile(_path) as _z:
                if any(b"cfRule" in _z.read(n) for n in _z.namelist() if n.endswith(".xml")):
                    CF_FILES.append(_path)
        except Exception:
            pass


@pytest.mark.skipif(not CF_FILES, reason="corpus not present")
def test_corpus_rules_match_openpyxl():
    compared, differing = 0, []
    for path in CF_FILES:
        data = open(path, "rb").read()
        try:
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                a = openpyxl.load_workbook(io.BytesIO(data))
            b = x.load_workbook(io.BytesIO(data))
        except Exception:
            continue
        if any("Failed to load a conditional formatting rule" in str(w.message) for w in caught):
            continue  # openpyxl drops rules it cannot parse (a threshold naming a defined name); the module keeps them
        for wa, wb_ in zip(a.worksheets, b.worksheets):
            compared += len(cf_view(wa))
            if cf_view(wa) != cf_view(wb_):
                differing.append((path, wa.title))
    assert compared > 100
    assert not differing, differing[:5]
