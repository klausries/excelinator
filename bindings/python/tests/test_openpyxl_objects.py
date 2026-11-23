"""Defined names, hyperlinks, auto filter and data validation through the openpyxl
interface: the same script with both modules, judged by openpyxl."""
import importlib
import io

import openpyxl
import pytest

import xlcells.openpyxl as facade

from test_openpyxl_styles_and_keep import parts

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def sub(m, name, attr):
    return getattr(importlib.import_module(m.__name__ + "." + name), attr)


def script(m):
    DefinedName = sub(m, "workbook.defined_name", "DefinedName")
    DataValidation = sub(m, "worksheet.datavalidation", "DataValidation")
    Hyperlink = sub(m, "worksheet.hyperlink", "Hyperlink")
    wb = m.Workbook()
    ws = wb.active
    ws.title = "Daten"
    ws.append(["Name", "Menge", "Preis", "Art"])
    for i in range(2, 8):
        ws.append([f"Artikel {i}", i, i * 1.5, "a"])
    other = wb.create_sheet("Zwei & mehr")
    other["A1"] = "Ziel"
    wb.defined_names["Bereich"] = DefinedName("Bereich", attr_text="Daten!$A$1:$C$7")
    wb.defined_names["Faktor"] = DefinedName("Faktor", attr_text="1.19")
    other.defined_names["Lokal"] = DefinedName("Lokal", attr_text="'Zwei & mehr'!$A$1")
    ws["F1"].hyperlink = "https://example.com/?a=1&b=2"
    ws["F2"] = "intern"
    ws["F2"].hyperlink = Hyperlink(ref="F2", location="'Zwei & mehr'!A1", tooltip="zum Ziel")
    ws.auto_filter.ref = "A1:D7"
    choice = DataValidation(type="list", formula1='"a,b,c"', allow_blank=True)
    choice.error = "Nur a, b oder c"
    choice.errorTitle = "Falsche <Art>"
    choice.showErrorMessage = True
    choice.add("D2:D7")
    choice.add("D20")
    ws.add_data_validation(choice)
    amount = DataValidation(type="whole", operator="between", formula1="0", formula2="100")
    amount.add("B2:B7")
    ws.add_data_validation(amount)
    return wb


def judge(data):
    wb = openpyxl.load_workbook(io.BytesIO(data))
    out = {"names": {n: d.attr_text for n, d in wb.defined_names.items() if not n.startswith("_xlnm")}}
    for ws in wb.worksheets:
        out[ws.title] = (
            {n: d.attr_text for n, d in ws.defined_names.items() if not n.startswith("_xlnm")},
            {c.coordinate: (c.value, c.hyperlink.target, c.hyperlink.location, c.hyperlink.tooltip) for c in ws._cells.values() if c.hyperlink},
            ws.auto_filter.ref,
            sorted((d.type, d.operator, d.formula1, d.formula2, bool(d.allowBlank), bool(d.showErrorMessage), d.error, d.errorTitle, str(d.sqref)) for d in ws.data_validations.dataValidation),
        )
    return out


def saved(m):
    buf = io.BytesIO()
    script(m).save(buf)
    return buf.getvalue()


def test_script_writes_the_same_objects():
    assert judge(saved(facade)) == judge(saved(openpyxl))


@pytest.mark.parametrize("writer", MODULES)
def test_both_load_the_objects_the_same_way(writer):
    data = saved(MODULES[writer])
    seen = {}
    for name, m in MODULES.items():
        wb = m.load_workbook(io.BytesIO(data))
        seen[name] = [{n: d.attr_text for n, d in wb.defined_names.items() if not n.startswith("_xlnm")}] + [
            (
                ws.title,
                {n: d.attr_text for n, d in ws.defined_names.items() if not n.startswith("_xlnm")},
                {c.coordinate: (c.hyperlink.target, c.hyperlink.location, c.hyperlink.tooltip) for c in ws._cells.values() if c.hyperlink},
                ws.auto_filter.ref,
                sorted((d.type, d.operator, d.formula1, d.formula2, bool(d.allowBlank), d.error, str(d.sqref)) for d in ws.data_validations.dataValidation),
            )
            for ws in wb.worksheets
        ]
    assert seen["xlcells"] == seen["openpyxl"]


def test_objects_can_be_changed_on_a_loaded_workbook():
    """Load a file written by openpyxl, change each kind of object, save onto it."""
    DefinedName, DataValidation = facade.DefinedName, facade.DataValidation
    data = saved(openpyxl)
    wb = facade.load_workbook(io.BytesIO(data))
    unchanged = wb.to_bytes()
    assert parts(unchanged) == parts(data)
    ws = wb["Daten"]
    wb.defined_names["Neu"] = DefinedName("Neu", attr_text="Daten!$B$2")
    del wb.defined_names["Faktor"]
    ws["F3"].hyperlink = "mailto:jemand@example.com"
    ws["F1"].hyperlink = None
    ws.auto_filter.ref = "A1:D5"
    ws.data_validations.dataValidation.pop()
    extra = DataValidation(type="decimal", operator="greaterThan", formula1="0.5")
    extra.add("C2:C7")
    ws.add_data_validation(extra)
    out = wb.to_bytes()
    assert wb._kept_original
    got = judge(out)
    assert got["names"] == {"Bereich": "Daten!$A$1:$C$7", "Neu": "Daten!$B$2"}
    names, links, auto, rules = got["Daten"]
    assert links == {"F2": ("intern", None, "'Zwei & mehr'!A1", "zum Ziel"), "F3": ("mailto:jemand@example.com", "mailto:jemand@example.com", None, None)}
    assert auto == "A1:D5"
    assert [(r[0], r[1], r[2], r[8]) for r in rules] == [("decimal", "greaterThan", "0.5", "C2:C7"), ("list", None, '"a,b,c"', "D2:D7 D20")]
    assert got["Zwei & mehr"][0] == {"Lokal": "'Zwei & mehr'!$A$1"}
    before, after = parts(data), parts(out)
    assert {n for n in before if after.get(n) != before[n]} <= {"xl/workbook.xml", "xl/worksheets/sheet1.xml", "xl/worksheets/_rels/sheet1.xml.rels", "[Content_Types].xml"}
