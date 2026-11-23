"""Comments through the openpyxl interface, judged by openpyxl."""
import importlib
import io

import openpyxl
import pytest

import xlcells.openpyxl as facade

from test_openpyxl_styles_and_keep import parts

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def script(m):
    Comment = importlib.import_module(m.__name__ + ".comments").Comment
    wb = m.Workbook()
    ws = wb.active
    ws["A1"] = "Wert"
    ws["A1"].comment = Comment("Erste Zeile\nzweite <Zeile> & mehr", "Klaus")
    ws["C3"].comment = Comment("auf leerer Zelle", "Jemand anderes")
    ws["B2"] = 5
    other = wb.create_sheet("Zwei")
    other["D4"] = "x"
    other["D4"].comment = Comment("zweites Blatt", "Klaus")
    wb.create_sheet("Ohne")["A1"] = 1
    return wb


def describe(wb):
    return [(ws.title, sorted((c.coordinate, c.comment.text, c.comment.author) for c in ws._cells.values() if c.comment)) for ws in wb.worksheets]


def saved(m):
    buf = io.BytesIO()
    script(m).save(buf)
    return buf.getvalue()


def test_script_writes_the_same_comments():
    a = describe(openpyxl.load_workbook(io.BytesIO(saved(facade))))
    assert a == describe(openpyxl.load_workbook(io.BytesIO(saved(openpyxl))))
    assert a[0][1][0] == ("A1", "Erste Zeile\nzweite <Zeile> & mehr", "Klaus")


@pytest.mark.parametrize("writer", MODULES)
def test_both_load_the_comments_the_same_way(writer):
    data = saved(MODULES[writer])
    assert describe(facade.load_workbook(io.BytesIO(data))) == describe(openpyxl.load_workbook(io.BytesIO(data)))


def test_comments_can_be_changed_on_a_loaded_workbook():
    data = saved(openpyxl)
    wb = facade.load_workbook(io.BytesIO(data))
    assert parts(wb.to_bytes()) == parts(data)
    wb.active["B2"].comment = facade.Comment("neu", "Klaus")
    wb.active["C3"].comment = None
    wb["Zwei"]["D4"].comment = None
    wb["Ohne"]["A1"].comment = facade.Comment("erster Kommentar hier", "Klaus")
    out = wb.to_bytes()
    assert wb._kept_original
    assert describe(openpyxl.load_workbook(io.BytesIO(out))) == [
        ("Sheet", [("A1", "Erste Zeile\nzweite <Zeile> & mehr", "Klaus"), ("B2", "neu", "Klaus")]),
        ("Zwei", []),
        ("Ohne", [("A1", "erster Kommentar hier", "Klaus")]),
    ]
    # and once more through our own loader, then back to no comments at all
    again = facade.load_workbook(io.BytesIO(out))
    assert describe(again) == describe(openpyxl.load_workbook(io.BytesIO(out)))
    for ws in again.worksheets:
        for cell in ws._cells.values():
            cell.comment = None
    cleared = again.to_bytes()
    assert all(not c for _, c in describe(openpyxl.load_workbook(io.BytesIO(cleared))))
    assert not [n for n in parts(cleared) if "comment" in n.lower() or n.endswith(".vml")]
