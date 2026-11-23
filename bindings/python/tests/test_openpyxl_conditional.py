"""Conditional formatting through the openpyxl interface, judged by openpyxl."""
import importlib
import io

import openpyxl
import pytest

import xlcells.openpyxl as facade

from test_openpyxl_styles_and_keep import parts

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def script(m):
    rule = importlib.import_module(m.__name__ + ".formatting.rule")
    st = m.styles
    wb = m.Workbook()
    ws = wb.active
    ws.append(["Name", "Menge", "Preis", "Art"])
    for i in range(2, 8):
        ws.append([f"Artikel {i}", i, i * 1.5, "a" if i % 2 else "b"])
    red = st.PatternFill(start_color="FFFFC7CE", end_color="FFFFC7CE", fill_type="solid")
    ws.conditional_formatting.add("B2:B7", rule.CellIsRule(operator="greaterThan", formula=["4"], font=st.Font(bold=True, color="FF9C0006"), fill=red))
    ws.conditional_formatting.add("C2:C7", rule.ColorScaleRule(start_type="min", start_color="FFFFFFFF", end_type="max", end_color="FF00B050"))
    ws.conditional_formatting.add("A2:A7", rule.FormulaRule(formula=['AND($D2="a",$B2<5)'], stopIfTrue=True, fill=red))
    ws.conditional_formatting.add("B2:B7", rule.CellIsRule(operator="lessThan", formula=["2"], font=st.Font(italic=True)))
    return wb


def rgb(c):
    v = getattr(c, "rgb", None)
    return v if isinstance(v, str) else None


def describe(ws):
    out = []
    for cf in ws.conditional_formatting:
        rules = []
        for r in cf.rules:
            d = r.dxf
            font = None if d is None or d.font is None else (bool(d.font.b), bool(d.font.i), rgb(d.font.color))
            fill = None if d is None or d.fill is None else (d.fill.fill_type, rgb(d.fill.fgColor), rgb(d.fill.bgColor))
            scale = None if r.colorScale is None else ([v.type for v in r.colorScale.cfvo], [rgb(c) for c in r.colorScale.color])
            rules.append((r.type, r.operator, list(r.formula), r.priority, bool(r.stopIfTrue), font, fill, scale))
        out.append((str(cf.sqref), rules))
    return sorted(out)


def saved(m):
    buf = io.BytesIO()
    script(m).save(buf)
    return buf.getvalue()


def test_script_writes_the_same_rules():
    a = describe(openpyxl.load_workbook(io.BytesIO(saved(facade))).active)
    b = describe(openpyxl.load_workbook(io.BytesIO(saved(openpyxl))).active)
    assert a == b


@pytest.mark.parametrize("writer", MODULES)
def test_both_load_the_rules_the_same_way(writer):
    data = saved(MODULES[writer])
    assert describe(facade.load_workbook(io.BytesIO(data)).active) == describe(openpyxl.load_workbook(io.BytesIO(data)).active)


def test_rules_can_be_changed_on_a_loaded_workbook():
    rule = importlib.import_module("xlcells.openpyxl.formatting.rule")
    data = saved(openpyxl)
    wb = facade.load_workbook(io.BytesIO(data))
    assert parts(wb.to_bytes()) == parts(data)
    ws = wb.active
    ws.conditional_formatting.add("D2:D7", rule.CellIsRule(operator="equal", formula=['"b"'], font=facade.Font(bold=True, color="FF0000FF")))
    out = wb.to_bytes()
    assert wb._kept_original
    before = describe(openpyxl.load_workbook(io.BytesIO(data)).active)
    after = describe(openpyxl.load_workbook(io.BytesIO(out)).active)
    assert [c for c in after if c[0] != "D2:D7"] == before
    assert [c for c in after if c[0] == "D2:D7"] == [("D2:D7", [("cellIs", "equal", ['"b"'], 5, False, (True, False, "FF0000FF"), None, None)])]
    a, b = parts(data), parts(out)
    assert {n for n in a if b.get(n) != a[n]} <= {"xl/styles.xml", "xl/worksheets/sheet1.xml"}
