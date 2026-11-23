"""cell.look: the effective formatting of a cell and where each attribute comes from."""
import io
import xlcells.openpyxl as xo
from xlcells.openpyxl.styles import PatternFill, Font, NamedStyle
from xlcells.openpyxl.formatting.rule import CellIsRule, DataBarRule
from xlcells.openpyxl.worksheet.table import Table, TableStyleInfo


def test_cell_look_sources():
    wb = xo.Workbook()
    ws = wb.active
    good = NamedStyle(name="Highlight", fill=PatternFill("solid", fgColor="FFC6EFCE"), font=Font(color="FF006100"))
    wb.add_named_style(good)
    ws["A1"] = 1
    ws["A1"].style = "Highlight"
    ws["A2"] = 2
    ws["A2"].fill = PatternFill("solid", fgColor="FFFF0000")
    for r in range(1, 6):
        ws.cell(r, 3, r)
    ws.conditional_formatting.add("C1:C5", CellIsRule(operator="greaterThan", formula=["3"], fill=PatternFill("solid", bgColor="FF0000FF")))
    ws["E1"], ws["F1"] = "a", "b"
    ws["E2"], ws["F2"] = 1, 2
    t = Table(displayName="Prices", ref="E1:F2")
    t.tableStyleInfo = TableStyleInfo(name="TableStyleMedium9", showRowStripes=True)
    ws.add_table(t)

    a1 = ws["A1"].look
    assert a1.fill_color == "C6EFCE" and a1.fill_color_source.kind == "style" and a1.fill_color_source.name == "Highlight"
    assert a1.font_name == "Calibri" and a1.font_name_source == "default"
    a2 = ws["A2"].look
    assert a2.fill_color == "FF0000" and a2.source("fill_color") == "cell"
    looks = ws.looks("C1:C5")
    assert [looks["C%d" % r].fill_color for r in range(1, 6)] == [None, None, None, "0000FF", "0000FF"]
    assert looks["C4"].fill_color_source.kind == "conditional" and looks["C4"].fill_color_source.name == "C1:C5"
    head = ws["E1"].look
    assert head.bold is True and head.font_color == "FFFFFF" and head.bold_source.kind == "table"
    assert head.bold_source.detail == ("TableStyleMedium9", "headerRow")
    assert head.font_color_source.spec == "theme 0"


def test_names_with_a_bar_and_whole_columns():
    wb = xo.Workbook()
    ws = wb.active
    wb.add_named_style(NamedStyle(name="Input|Bold", fill=PatternFill("solid", fgColor="FFC6EFCE")))
    for r in range(1, 4):
        ws.cell(r, 1, r)
        ws.cell(r, 2, r * 2)
    ws["A1"].style = "Input|Bold"
    ws.conditional_formatting.add("B1:B3", DataBarRule(start_type="min", end_type="max", color="FF638EC6"))
    src = ws["A1"].look.fill_color_source
    assert src.kind == "style" and src.name == "Input|Bold" and src.detail == ()
    cols = ws.looks("A:B")
    assert list(cols) == ["A1", "B1", "A2", "B2", "A3", "B3"]
    assert cols["B3"].data_bar.range == "B1:B3"
    assert list(ws.looks("2:3")) == ["A2", "B2", "A3", "B3"]
    assert list(ws.looks("B2")) == ["B2"]
    looker = wb.looker()
    assert looker.look(ws["A1"]).fill_color == "C6EFCE"
    assert looker.looks(ws, ["A2"])["A2"].fill_color is None
