"""Printing, views, protection and sheet properties: the same scripts with openpyxl and
the facade, read back by openpyxl; loaded settings kept or changed; the helpers that
compute (password hash, header texts, coordinates) against openpyxl's."""
import io
import zipfile

import openpyxl
import openpyxl.utils.cell as their_cells
import pytest

from conftest import needs_corpus
from hypothesis import given, settings
from hypothesis import strategies as st
from openpyxl.worksheet.header_footer import HeaderFooterItem as TheirItem
from openpyxl.worksheet.protection import hash_password as their_hash

import xlcells.openpyxl as facade
import xlcells.openpyxl.utils.cell as our_cells
from xlcells.openpyxl.worksheet.header_footer import HeaderFooterItem as OurItem
from xlcells.openpyxl.worksheet.protection import hash_password as our_hash

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def configured(m):
    wb = m.Workbook()
    ws = wb.active
    ws.title = "Daten & Co"
    for r in range(1, 30):
        ws.append([r, r * 2, "x{0}".format(r)])
    ws.page_setup.orientation = "landscape"
    ws.page_setup.paperSize = ws.PAPERSIZE_A4 if hasattr(ws, "PAPERSIZE_A4") else 9
    ws.page_setup.fitToWidth = 1
    ws.page_setup.fitToHeight = 0
    ws.page_setup.scale = 80
    ws.sheet_properties.pageSetUpPr.fitToPage = True
    ws.sheet_properties.tabColor = "1072BA"
    ws.sheet_properties.outlinePr.summaryBelow = False
    ws.page_margins.left = 0.4
    ws.page_margins.top = 0.6
    ws.print_options.gridLines = True
    ws.print_options.horizontalCentered = True
    ws.oddHeader.center.text = "Bericht &D"
    ws.oddHeader.center.size = 14
    ws.oddHeader.center.font = "Arial,Bold"
    ws.oddFooter.left.text = "&F"
    ws.oddFooter.right.text = "Seite &P von &N"
    ws.oddFooter.right.color = "FF0000"
    ws.protection.sheet = True
    ws.protection.password = "geheim"
    ws.protection.formatColumns = False
    ws.sheet_view.zoomScale = 85
    ws.sheet_view.showGridLines = False
    ws.freeze_panes = "B2"
    ws.print_area = "A1:C20"
    ws.print_title_rows = "1:1"
    ws.print_title_cols = "A:A"
    ws.sheet_format.defaultRowHeight = 18
    ws.sheet_format.customHeight = True
    second = wb.create_sheet("Zwei")
    second["A1"] = 1
    second.freeze_panes = "A3"
    second.sheet_view.view = "pageBreakPreview"
    second.set_printer_settings(9, "portrait")
    second.HeaderFooter.differentFirst = True
    second.firstHeader.center.text = "Erste Seite"
    return wb


def settings_view(data):
    """What openpyxl reads of the settings."""
    wb = openpyxl.load_workbook(io.BytesIO(data))
    out = []
    for ws in wb.worksheets:
        ps, pm, po, sp, pr, sv = ws.page_setup, ws.page_margins, ws.print_options, ws.sheet_properties, ws.protection, ws.sheet_view
        out.append((
            ws.title,
            (ps.orientation, ps.paperSize, ps.fitToWidth, ps.fitToHeight, ps.scale, sp.pageSetUpPr.fitToPage if sp.pageSetUpPr else None),
            (pm.left, pm.right, pm.top, pm.bottom, pm.header, pm.footer),
            (po.gridLines, po.horizontalCentered, po.verticalCentered, po.headings),
            (sp.tabColor.rgb if sp.tabColor is not None else None, sp.outlinePr.summaryBelow if sp.outlinePr else None),
            (pr.sheet, pr.password, pr.formatColumns, pr.objects, pr.selectLockedCells),
            (sv.zoomScale, sv.showGridLines, sv.view, ws.freeze_panes),
            tuple(str(getattr(ws, n)) for n in ("oddHeader", "oddFooter", "evenHeader", "evenFooter", "firstHeader", "firstFooter")),
            ws.HeaderFooter.differentFirst,
            (ws.print_area, ws.print_title_rows, ws.print_title_cols),
            (ws.sheet_format.defaultRowHeight, ws.sheet_format.customHeight),
        ))
    return out


def save(wb):
    out = io.BytesIO()
    wb.save(out)
    return out.getvalue()


def test_settings_written_by_both_read_the_same():
    theirs, ours = (settings_view(save(configured(m))) for m in (openpyxl, facade))
    assert ours == theirs


def test_settings_read_back_by_the_facade_are_what_openpyxl_wrote():
    data = save(configured(openpyxl))
    wb = facade.load_workbook(io.BytesIO(data))
    ws = wb["Daten & Co"]
    assert ws.page_setup.orientation == "landscape" and ws.page_setup.fitToPage
    assert ws.protection.sheet and ws.protection.password == their_hash("geheim")
    assert str(ws.oddFooter) == '&L&F&R&KFF0000Seite &P von &N'
    assert ws.freeze_panes == "B2" and ws.print_area == "'Daten & Co'!$A$1:$C$20" and ws.print_title_rows == "$1:$1"
    assert wb["Zwei"].freeze_panes == "A3" and wb["Zwei"].firstHeader.center.text == "Erste Seite"


def parts(data):
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        return {n: z.read(n) for n in z.namelist()}


def test_loaded_settings_stay_byte_for_byte_and_changes_land():
    data = save(configured(openpyxl))
    wb = facade.load_workbook(io.BytesIO(data))
    assert parts(wb.to_bytes()) == parts(data)
    ws = wb["Daten & Co"]
    ws.page_setup.orientation = "portrait"
    ws.protection.disable()
    ws.freeze_panes = "C5"  # frozen panes of a loaded sheet change too
    ws.print_area = "A1:B5"
    wb["Zwei"].page_margins.bottom = 2
    out = wb.to_bytes()
    assert wb._kept_original
    got = settings_view(out)
    want = settings_view(data)
    want[0] = (want[0][0], ("portrait",) + want[0][1][1:], want[0][2], want[0][3], want[0][4], (False, None, True, False, False),
               want[0][6][:3] + ("C5",), want[0][7], want[0][8], ("'Daten & Co'!$A$1:$B$5",) + want[0][9][1:], want[0][10])
    want[1] = (want[1][0], want[1][1], want[1][2][:3] + (2.0,) + want[1][2][4:]) + want[1][3:]
    assert got == want
    # the parts that did not change stay as they were
    before, after = parts(data), parts(out)
    assert {n for n in before if before[n] != after.get(n)} <= {"xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml", "xl/workbook.xml"}


@needs_corpus
def test_settings_survive_files_from_excel_in_the_corpus():
    """Loading and saving keeps printer settings (r:id), compatibility attributes and the rest."""
    from conftest import CORPUS
    seen = 0
    for path in sorted(CORPUS.glob("src-*/**/*.xlsx"))[:400]:
        if path.stat().st_size > 1_000_000 or "OOM" in path.name:
            continue  # large files and files built to exhaust memory
        data = path.read_bytes()
        try:
            wb = facade.load_workbook(io.BytesIO(data))
            ref = openpyxl.load_workbook(io.BytesIO(data))
        except Exception:
            continue
        if not wb.worksheets or not ref.worksheets:
            continue
        ws = wb.worksheets[0]
        ws.page_margins.left = 1.25
        try:
            out = wb.to_bytes()
        except ValueError:
            continue
        got = openpyxl.load_workbook(io.BytesIO(out)).worksheets[0]
        assert got.page_margins.left == 1.25, path
        r = ref.worksheets[0]
        assert (got.page_setup.orientation, got.page_setup.paperSize, got.freeze_panes, got.sheet_view.zoomScale) == (r.page_setup.orientation, r.page_setup.paperSize, r.freeze_panes, r.sheet_view.zoomScale), path
        seen += 1
    assert seen > 50


@settings(max_examples=300, deadline=None)
@given(st.text(max_size=20))
def test_password_hash_is_excels(password):
    assert our_hash(password) == their_hash(password)


_PIECES = st.sampled_from(["&P", "&N", "&D", "&T", "&A", "&F", "&&", "Seite ", "von", " ", "x", "&B", "&I", '&"Arial,Bold"', "&12", "&KFF0000", "ä"])
_HEADER = st.lists(st.tuples(st.sampled_from(["&L", "&C", "&R"]), st.lists(_PIECES, min_size=1, max_size=5).map("".join)), min_size=1, max_size=3, unique_by=lambda p: p[0])


@settings(max_examples=300, deadline=None)
@given(_HEADER)
def test_header_texts_are_read_like_openpyxl(parts_):
    order = {"&L": 0, "&C": 1, "&R": 2}
    text = "".join(code + body for code, body in sorted(parts_, key=lambda p: order[p[0]]))
    import xml.etree.ElementTree as ET
    theirs = TheirItem.from_tree(ET.fromstring("<h>{0}</h>".format(text.replace("&", "&amp;"))))
    ours = OurItem._from_text(text)
    for side in ("left", "center", "right"):
        a, b = getattr(theirs, side), getattr(ours, side)
        assert (a.text, a.font, a.size, a.color) == (b.text, b.font, b.size, b.color), (text, side)
    assert str(ours) == str(theirs).replace("&amp;", "&") or str(ours) == str(theirs)


_COORD = st.tuples(st.integers(1, 18278), st.integers(1, 1048576), st.booleans(), st.booleans())


def _a1(c):
    col, row, ac, ar = c
    return ("$" if ac else "") + their_cells.get_column_letter(col) + ("$" if ar else "") + str(row)


@settings(max_examples=300, deadline=None)
@given(_COORD, _COORD, st.sampled_from(["", "Daten", "Mit Leer", "O'Neil"]))
def test_coordinate_helpers_are_openpyxls(a, b, sheet):
    rng = _a1(a) + ":" + _a1(b)
    for name in ("range_boundaries", "absolute_coordinate", "coordinate_from_string", "coordinate_to_tuple"):
        arg = rng if name in ("range_boundaries", "absolute_coordinate") else _a1(a)
        try:
            want = getattr(their_cells, name)(arg)
        except ValueError:
            continue  # openpyxl rejects $ here; we read it
        assert getattr(our_cells, name)(arg) == want, (name, arg)
    assert our_cells.quote_sheetname(sheet or "x") == their_cells.quote_sheetname(sheet or "x")
    if sheet:
        ref = their_cells.quote_sheetname(sheet) + "!" + rng
        assert our_cells.range_to_tuple(ref) == their_cells.range_to_tuple(ref)
    small = their_cells.get_column_letter(min(a[0], 30)) + "1:" + their_cells.get_column_letter(min(b[0], 30)) + "3"
    assert list(our_cells.rows_from_range(small)) == list(their_cells.rows_from_range(small))
    assert list(our_cells.cols_from_range(small)) == list(their_cells.cols_from_range(small))
    assert our_cells.get_column_interval(min(a[0], 40), min(a[0], 40) + 5) == their_cells.get_column_interval(min(a[0], 40), min(a[0], 40) + 5)


@pytest.mark.parametrize("text", ["A:C", "2:4", "A1", "$A$1:$B$2", "AB12", "A1:B", "x", "A0", ""])
def test_range_boundaries_edge_cases_match(text):
    def run(m):
        try:
            return m.range_boundaries(text)
        except Exception as e:
            return type(e).__name__
    assert run(our_cells) == run(their_cells)


import datetime as _dt  # noqa: E402

import openpyxl.utils.datetime as their_dates  # noqa: E402

import xlcells.openpyxl.utils.datetime as our_dates  # noqa: E402


@settings(max_examples=400, deadline=None)
@given(st.floats(min_value=0, max_value=2958465, allow_nan=False), st.booleans(), st.booleans())
def test_serial_numbers_convert_like_openpyxl(value, mac, delta):
    epoch = (our_dates.MAC_EPOCH if mac else our_dates.WINDOWS_EPOCH)

    def run(m):
        try:
            return m.from_excel(value, epoch, delta)
        except OverflowError:
            return "OverflowError"
    assert run(our_dates) == run(their_dates)


@settings(max_examples=300, deadline=None)
@given(st.one_of(st.datetimes(min_value=_dt.datetime(1899, 12, 31), max_value=_dt.datetime(9999, 12, 31)), st.dates(min_value=_dt.date(1900, 1, 1)), st.times(), st.timedeltas(min_value=_dt.timedelta(0), max_value=_dt.timedelta(days=9000))), st.booleans())
def test_dates_convert_like_openpyxl(value, mac):
    epoch = (our_dates.MAC_EPOCH if mac else our_dates.WINDOWS_EPOCH)
    assert our_dates.to_excel(value, epoch) == their_dates.to_excel(value, epoch)
    if not isinstance(value, _dt.timedelta):
        text = our_dates.to_ISO8601(value)
        assert text == their_dates.to_ISO8601(value)
        assert our_dates.from_ISO8601(text[:23]) == their_dates.from_ISO8601(text[:23])
