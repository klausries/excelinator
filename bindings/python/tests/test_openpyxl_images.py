"""Pictures through the openpyxl interface, judged by openpyxl; charts read from a
file must survive loading and saving."""
import importlib
import io
import os
import struct
import warnings
import zlib

import openpyxl
import pytest

import xlcells.openpyxl as facade

from conftest import CORPUS, needs_corpus
from test_openpyxl_styles_and_keep import parts

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def png(width, height, colour):
    """A real PNG file of one colour."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    rows = b"".join(b"\x00" + bytes(colour) * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


RED, BLUE = png(120, 40, (200, 30, 30)), png(16, 16, (30, 30, 200))


def script(m):
    Image = importlib.import_module(m.__name__ + ".drawing.image").Image
    wb = m.Workbook()
    ws = wb.active
    ws["A1"] = "Bilder"
    ws.add_image(Image(io.BytesIO(RED)), "B3")
    small = Image(io.BytesIO(BLUE))
    small.width, small.height = 48, 32
    ws.add_image(small, "F10")
    other = wb.create_sheet("Zwei")
    again = Image(io.BytesIO(RED))
    again.anchor = "A1"
    other.add_image(again)
    return wb


def describe(wb):
    out = []
    for ws in wb.worksheets:
        pictures = []
        for img in ws._images:
            anchor = img.anchor
            where = anchor if isinstance(anchor, str) else "{0}{1}".format(openpyxl.utils.get_column_letter(anchor._from.col + 1), anchor._from.row + 1)
            data = img._data()
            pictures.append((where, len(data), zlib.crc32(data)))
        out.append((ws.title, sorted(pictures)))
    return out


def sizes(data):
    """Position and size of every picture as written in the drawing parts (openpyxl
    itself forgets a changed size when it loads a file)."""
    import re

    out = []
    for name, xml in sorted(parts(data).items()):
        if name.startswith("xl/drawings/drawing"):
            out.append(re.findall(r"<(?:\w+:)?col>(\d+)</(?:\w+:)?col>.*?<(?:\w+:)?row>(\d+)</(?:\w+:)?row>.*?<(?:\w+:)?ext cx=\"(\d+)\" cy=\"(\d+)\"", xml.decode(), re.S))
    return out


def saved(m):
    buf = io.BytesIO()
    script(m).save(buf)
    return buf.getvalue()


def test_script_places_the_same_pictures():
    a = describe(openpyxl.load_workbook(io.BytesIO(saved(facade))))
    assert a == describe(openpyxl.load_workbook(io.BytesIO(saved(openpyxl))))
    assert a[0][1][0][0] == "B3"
    assert sizes(saved(facade)) == sizes(saved(openpyxl)) == [[("1", "2", "1143000", "381000"), ("5", "9", "457200", "304800")], [("0", "0", "1143000", "381000")]]


@pytest.mark.parametrize("writer", MODULES)
def test_both_load_the_pictures_the_same_way(writer):
    data = saved(MODULES[writer])
    assert describe(facade.load_workbook(io.BytesIO(data))) == describe(openpyxl.load_workbook(io.BytesIO(data)))


def test_pictures_can_be_changed_on_a_loaded_workbook():
    data = saved(openpyxl)
    wb = facade.load_workbook(io.BytesIO(data))
    assert parts(wb.to_bytes()) == parts(data)
    ws = wb.active
    ws._images.pop(0)
    ws.add_image(facade.Image(BLUE), "H2")
    wb["Zwei"]._images.clear()
    out = wb.to_bytes()
    assert wb._kept_original
    got = describe(openpyxl.load_workbook(io.BytesIO(out)))
    assert [(t, [p[0] for p in ps]) for t, ps in got] == [("Sheet", ["F10", "H2"]), ("Zwei", [])]
    assert sizes(out) == [[("5", "9", "457200", "304800"), ("7", "1", "152400", "152400")]]
    again = facade.load_workbook(io.BytesIO(out)).active._images
    assert [(i.anchor, i.width, i.height) for i in again] == [("F10", 48, 32), ("H2", 16, 16)]
    assert not [n for n in parts(out) if "drawing2" in n]


def chart_files():
    found = []
    for d, _, names in os.walk(CORPUS):
        if "/src-" in d + "/" and "src-enron" not in d and "/lo-" not in d + "/":
            for n in sorted(names):
                p = os.path.join(d, n)
                if n.lower().endswith(".xlsx") and os.path.getsize(p) < 300000 and "chart" in n.lower():
                    found.append(p)
    return sorted(found)[:40]


@needs_corpus
def test_charts_from_a_file_survive_changes_to_the_sheet():
    """Add a picture next to existing charts: openpyxl still finds every chart."""
    warnings.simplefilter("ignore")
    checked = 0
    for path in chart_files():
        data = open(path, "rb").read()
        try:
            ref = openpyxl.load_workbook(io.BytesIO(data))
            wb = facade.load_workbook(io.BytesIO(data))
        except Exception:
            continue
        counts = [len(ws._charts) for ws in ref.worksheets]
        target = next((ws for ws in wb.worksheets if ws._charts), None)
        if target is None or len(ref.worksheets) != len(wb.worksheets) or not sum(counts):
            continue  # openpyxl itself cannot read these charts
        target.add_image(facade.Image(BLUE), "K1")
        try:
            out = wb.to_bytes()
        except ValueError as e:
            assert "shapes" in str(e)
            continue
        got = openpyxl.load_workbook(io.BytesIO(out))
        assert [len(ws._charts) for ws in got.worksheets] == counts, path
        assert sum(len(ws._images) for ws in got.worksheets) == sum(len(ws._images) for ws in ref.worksheets) + 1, path
        checked += 1
    assert checked >= 5, checked
