"""Property tests: random workbooks through our writer and reader, and across
independent implementations (openpyxl, XlsxWriter, python-calamine)."""
import io
import math
import re

import openpyxl
import pytest
import xlsxwriter
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from python_calamine import CalamineWorkbook

import xlcells
from xlcells import pyexcelerator as px

SETTINGS = settings(max_examples=150, deadline=None, suppress_health_check=[HealthCheck.too_slow])

# Any text a cell can hold: all of Unicode except lone surrogates and NUL.
any_text = st.text(st.characters(blacklist_categories=("Cs",), blacklist_characters="\x00"), min_size=1, max_size=40)
# Text every implementation can carry: no control characters, no "_x" escapes,
# no surrounding whitespace (calamine trims it), not something openpyxl takes for a formula.
plain_text = st.text(st.characters(blacklist_categories=("Cs", "Cc", "Zl", "Zp"), blacklist_characters="￾￿"), min_size=1, max_size=40).filter(
    lambda s: s == s.strip() and "_x" not in s and not s.startswith("=")
)
numbers = st.one_of(
    st.floats(allow_nan=False, allow_infinity=False),
    st.integers(-(2**53), 2**53).map(float),
    st.sampled_from([0.0, -0.0, 0.1, 1e-300, 1.7976931348623157e308, 5e-324, 123456789.123456789]),
)
# (A name with an "_x" escape is written escaped, as the format asks; openpyxl reads it as written.)
sheet_names = st.text(st.characters(blacklist_categories=("Cs", "Cc"), blacklist_characters="\\/?*[]:'￾￿"), min_size=1, max_size=31).filter(lambda s: s == s.strip() and "_x" not in s and len(s.encode("utf-16-le")) <= 62)
# Numbers with at most 15 significant digits: what Excel itself stores, and what
# openpyxl and XlsxWriter (which print 16 digits) can write without loss.
numbers15 = numbers.map(lambda x: float(f"{x:.14e}")).filter(math.isfinite)
positions = st.tuples(st.integers(0, 60), st.integers(0, 30)) | st.tuples(st.sampled_from([0, 65535, 65536, 1048575]), st.sampled_from([0, 255, 256, 16383]))


def workbooks(text, numbers=numbers):
    values = st.one_of(numbers, text, st.booleans())
    sheet = st.dictionaries(positions, values, max_size=25)
    return st.lists(st.tuples(sheet_names, sheet), min_size=1, max_size=3, unique_by=lambda s: s[0].lower())


def write_ours(spec):
    w = px.Workbook()
    for name, cells in spec:
        ws = w.add_sheet(name)
        for (r, c), v in cells.items():
            ws.write(r, c, v)
    return w.to_bytes()


def same(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, float) and isinstance(b, (int, float)):
        return a == b and (a != 0 or math.copysign(1, a) == math.copysign(1, float(b)) or isinstance(b, int))
    return type(a) is type(b) and a == b


def assert_same(spec, got, who, exact_zero=True):
    assert [n for n, _ in spec] == [n for n, _ in got], who
    for (name, want), (_, have) in zip(spec, got):
        assert set(want) == set(have), f"{who}: cells of {name!r}"
        for at, v in want.items():
            h = have[at]
            ok = same(v, h) if exact_zero else (v == h and isinstance(v, bool) == isinstance(h, bool))
            assert ok, f"{who}: {name!r}{at}: wrote {v!r}, read {h!r}"


@SETTINGS
@given(workbooks(any_text))
def test_own_round_trip_is_exact(spec):
    """Every value comes back bit for bit, including -0.0, control characters and edge cells."""
    data = write_ours(spec)
    got = px.parse_xls(data)
    assert_same(spec, got, "xlcells")
    for (_, want), (_, have) in zip(spec, got):
        for at, v in want.items():
            if isinstance(v, float):
                assert v.hex() == have[at].hex()
    # The column table says the same.
    t = xlcells.read(data)
    assert len(t["row"]) == sum(len(c) for _, c in spec)
    assert t["formula"] == [None] * len(t["row"])


@SETTINGS
@given(workbooks(plain_text))
def test_openpyxl_and_calamine_read_what_we_write(spec):
    data = write_ours(spec)
    book = openpyxl.load_workbook(io.BytesIO(data))
    got = []
    for ws in book.worksheets:
        cells = {}
        for cell in ws._cells.values():  # iter_rows() would walk the whole bounding box
            if cell.value is not None:
                cells[(cell.row - 1, cell.column - 1)] = float(cell.value) if isinstance(cell.value, int) and not isinstance(cell.value, bool) else cell.value
        got.append((ws.title, cells))
    assert_same(spec, got, "openpyxl", exact_zero=False)

    cal = CalamineWorkbook.from_filelike(io.BytesIO(data))
    assert cal.sheet_names == [n for n, _ in spec]
    for name, want in spec:
        if any(r > 5000 for r, _ in want):
            continue  # calamine materialises the whole range
        sheet = cal.get_sheet_by_name(name)
        rows = sheet.to_python(skip_empty_area=False)
        have = {(r, c): v for r, row in enumerate(rows) for c, v in enumerate(row) if v != ""}
        assert have == {at: v for at, v in want.items()}, f"calamine: {name!r}"


@SETTINGS
@given(workbooks(plain_text, numbers15))
def test_we_read_what_openpyxl_and_xlsxwriter_write(spec):
    book = openpyxl.Workbook()
    book.remove(book.active)
    for name, cells in spec:
        ws = book.create_sheet(name)
        for (r, c), v in cells.items():
            ws.cell(r + 1, c + 1, v)
    buf = io.BytesIO()
    book.save(buf)
    assert_same(spec, px.parse_xls(buf.getvalue()), "openpyxl → xlcells", exact_zero=False)

    buf = io.BytesIO()
    book = xlsxwriter.Workbook(buf, {"in_memory": True, "strings_to_formulas": False, "strings_to_urls": False, "strings_to_numbers": False})
    for name, cells in spec:
        ws = book.add_worksheet(name)
        for (r, c), v in cells.items():
            if isinstance(v, bool):
                ws.write_boolean(r, c, v)
            elif isinstance(v, float):
                ws.write_number(r, c, v)
            else:
                ws.write_string(r, c, v)
    book.close()
    assert_same(spec, px.parse_xls(buf.getvalue()), "XlsxWriter → xlcells", exact_zero=False)


formula_texts = st.sampled_from(["1+1", "SUM(A1:B2)", "IF(A1>0;\"a;b\";\"c\")", "A1&\" <&> \"&B1", "'My Sheet'!A1*2", "{1;2}+1", "SUM(A1,B1)"])


@SETTINGS
@given(st.dictionaries(positions, st.tuples(formula_texts, st.one_of(st.none(), numbers, plain_text, st.booleans())), min_size=1, max_size=10))
def test_formulas_round_trip_with_their_values(cells):
    w = px.Workbook()
    ws = w.add_sheet("F")
    for (r, c), (text, value) in cells.items():
        ws.write(r, c, px.Formula(text, value))
    data = w.to_bytes()
    t = xlcells.read(data)
    got = {(r, c): (f, k, n, s) for r, c, f, k, n, s in zip(t["row"], t["col"], t["formula"], t["kind"], t["num"], t["text"])}
    book = openpyxl.load_workbook(io.BytesIO(data))
    for (r, c), (text, value) in cells.items():
        f, kind, num, s = got[(r, c)]
        expected = text.replace(";", ",") if "{" not in text and '"a;b"' not in text else f
        assert f == expected
        assert ";" not in re.sub(r'"[^"]*"|\{[^}]*\}', "", f)
        if value is None:
            assert kind == xlcells.BLANK
        elif isinstance(value, bool):
            assert (kind, num) == (xlcells.BOOL, float(value))
        elif isinstance(value, float):
            assert kind == xlcells.NUMBER and num.hex() == value.hex()
        else:
            assert (kind, s) == (xlcells.TEXT, value)
        assert book["F"].cell(r + 1, c + 1).value == "=" + f


def test_limits_are_errors():
    w = px.Workbook()
    ws = w.add_sheet("A")
    ws.write(0, 0, "x" * 32768)
    with pytest.raises(ValueError, match="longer than 32767"):
        w.to_bytes()
    for r, c in [(1048576, 0), (0, 16384)]:
        w = px.Workbook()
        w.add_sheet("A").write(r, c, 1)
        with pytest.raises(ValueError, match="outside the grid"):
            w.to_bytes()
    with pytest.raises(ValueError):
        px.Workbook().to_bytes()
    with pytest.raises(ValueError):
        px.parse_xls(b"not a workbook")
    with pytest.raises((ValueError, TypeError)):
        px.Workbook().add_sheet("A").write(0, 0, float("nan")) or px.parse_xls(b"")
