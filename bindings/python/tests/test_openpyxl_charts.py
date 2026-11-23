"""Charts through the openpyxl interface: the same script with both modules, judged
by openpyxl; our chart parts must be valid against the chart schema."""
import importlib
import io

import openpyxl
import pytest
from lxml import etree

import xlcells.openpyxl as facade

from conftest import CORPUS
from test_openpyxl_styles_and_keep import parts


def script(m):
    c = importlib.import_module(m.__name__ + ".chart")
    wb = m.Workbook()
    ws = wb.active
    ws.title = "Daten & Co"
    ws.append(["Monat", "Nord", "Süd", "x"])
    for i, month in enumerate(["Jan", "Feb", "Mär", "Apr", "Mai", "Jun"], 1):
        ws.append([month, i * 10, 70 - i * 8, i * 1.5])
    data = c.Reference(ws, min_col=2, min_row=1, max_col=3, max_row=7)
    cats = c.Reference(ws, min_col=1, min_row=2, max_row=7)

    bar = c.BarChart()
    bar.type = "bar"
    bar.grouping = "stacked"
    bar.overlap = 100
    bar.title = "Umsatz <Nord & Süd>"
    bar.x_axis.title = "Monat"
    bar.y_axis.title = "Euro"
    bar.add_data(data, titles_from_data=True)
    bar.set_categories(cats)
    bar.width, bar.height = 18, 9
    ws.add_chart(bar, "F2")

    line = c.LineChart()
    line.title = "Verlauf"
    line.add_data(data, titles_from_data=True)
    line.set_categories(cats)
    ws.add_chart(line, "F22")

    pie = c.PieChart()
    pie.add_data(c.Reference(ws, min_col=2, min_row=1, max_row=7), titles_from_data=True)
    pie.set_categories(cats)
    ws.add_chart(pie, "P2")

    scatter = c.ScatterChart()
    scatter.title = "Punkte"
    scatter.x_axis.title = "x"
    xs = c.Reference(ws, min_col=4, min_row=2, max_row=7)
    for col in (2, 3):
        scatter.series.append(c.Series(c.Reference(ws, min_col=col, min_row=1, max_row=7), xs, title_from_data=True))
    scatter.series.append(c.Series(c.Reference(ws, min_col=2, min_row=2, max_row=7), xs, title="fester Name"))
    ws.add_chart(scatter, "P22")

    rows = wb.create_sheet("Zeilen")
    rows.append(["", "a", "b", "c"])
    rows.append(["eins", 1, 2, 3])
    rows.append(["zwei", 4, 5, 6])
    by_row = c.BarChart()
    by_row.add_data(c.Reference(rows, min_col=1, min_row=2, max_col=4, max_row=3), from_rows=True, titles_from_data=True)
    by_row.set_categories(c.Reference(rows, min_col=2, min_row=1, max_col=4))
    rows.add_chart(by_row)
    return wb


def text(title):
    if title is None or title.tx is None or title.tx.rich is None:
        return None
    return "".join(r.t for p in title.tx.rich.p for r in (p.r or []))


def ref(source):
    if source is None:
        return None
    for kind in ("numRef", "strRef"):
        r = getattr(source, kind, None)
        if r is not None:
            return r.f
    return getattr(source, "v", None)


def describe(data):
    out = []
    for ws in openpyxl.load_workbook(io.BytesIO(data)).worksheets:
        for ch in ws._charts:
            a = ch.anchor
            series = [(ref(s.tx), ref(s.cat), ref(s.val), ref(s.xVal), ref(s.yVal)) for s in ch.series]
            axes = (text(ch.x_axis.title), text(ch.y_axis.title)) if hasattr(ch, "x_axis") else None
            out.append((ws.title, type(ch).__name__, text(ch.title), series, axes, getattr(ch, "type", None), getattr(ch, "grouping", None), getattr(ch, "overlap", None),
                        (a._from.col, a._from.row, a.ext.width, a.ext.height), None if ch.legend is None else ch.legend.position))
    return sorted(out, key=lambda c: (c[0], c[8]))


def saved(m):
    buf = io.BytesIO()
    script(m).save(buf)
    return buf.getvalue()


def test_script_draws_the_same_charts():
    ours, theirs = describe(saved(facade)), describe(saved(openpyxl))
    assert len(theirs) == 5
    for a, b in zip(ours, theirs):
        assert a == b
    assert ours == theirs


def test_chart_parts_are_schema_valid():
    xsd = CORPUS / "ref-xsd" / "dml-chart.xsd"
    if not xsd.is_file():
        pytest.skip("schemas not fetched")
    schema = etree.XMLSchema(etree.parse(str(xsd)))
    checked = 0
    for name, content in parts(saved(facade)).items():
        if name.startswith("xl/charts/chart"):
            doc = etree.fromstring(content)
            assert schema.validate(doc), (name, [e.message for e in schema.error_log][:3])
            # Every series carries its colour: without a theme part LibreOffice
            # would otherwise draw the series invisibly.
            assert content.count(b"<c:ser>") >= 1 and content.count(b"srgbClr") >= content.count(b"<c:ser>")
            checked += 1
    assert checked == 5


def test_charts_survive_our_own_load_and_save():
    data = saved(facade)
    wb = facade.load_workbook(io.BytesIO(data))
    assert parts(wb.to_bytes()) == parts(data)
    wb.active["B2"] = 99
    c = importlib.import_module("xlcells.openpyxl.chart")
    extra = c.LineChart()
    extra.add_data(c.Reference(wb.active, min_col=2, min_row=1, max_row=7), titles_from_data=True)
    wb["Zeilen"].add_chart(extra, "H10")
    out = wb.to_bytes()
    assert wb._kept_original
    after = describe(out)
    assert [c for c in after if c[0] == "Daten & Co"] == [c for c in describe(data) if c[0] == "Daten & Co"]
    assert [(c[1], c[8][:2]) for c in after if c[0] == "Zeilen"] == [("BarChart", (4, 14)), ("LineChart", (7, 9))]


def test_loaded_charts_move_and_go():
    data = saved(facade)
    before = [c for c in describe(data) if c[0] == "Zeilen"]
    wb = facade.load_workbook(io.BytesIO(data))
    ws = wb["Zeilen"]
    chart = ws._charts[0]
    chart.anchor = "K30"
    moved = [c for c in describe(wb.to_bytes()) if c[0] == "Zeilen"]
    assert [c[8][:2] for c in moved] == [(10, 29)]
    assert [c[:8] for c in moved] == [c[:8] for c in before]
    ws._charts.remove(chart)
    assert [c for c in describe(wb.to_bytes()) if c[0] == "Zeilen"] == []
    # The other sheet's charts stay as they were.
    assert [c for c in describe(wb.to_bytes()) if c[0] != "Zeilen"] == [c for c in describe(data) if c[0] != "Zeilen"]
