"""xlcells.polars: round trips, and agreement with Polars' own readers."""
import datetime
import io

import polars as pl
import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from polars.testing import assert_frame_equal

import xlcells.polars as xp

from test_property import plain_text

SETTINGS = settings(max_examples=80, deadline=None, suppress_health_check=[HealthCheck.too_slow])
ROWS = 5
names = st.lists(plain_text.filter(lambda s: not s.isdigit()), min_size=1, max_size=5, unique=True)


def col(strategy, dtype):
    # First and last row are never null, so that the table's extent is unambiguous.
    return st.tuples(strategy, st.lists(st.one_of(st.none(), strategy), min_size=ROWS - 2, max_size=ROWS - 2), strategy).map(lambda t: pl.Series(values=[t[0]] + t[1] + [t[2]], dtype=dtype))


column = st.one_of(
    col(st.integers(-10**9, 10**9), pl.Int64),
    # No whole numbers: a column of those comes back as integers.
    col(st.floats(-1e9, 1e9).map(lambda x: float(f"{x:.10e}") + 0.5).filter(lambda x: x != int(x)), pl.Float64),
    col(plain_text, pl.String),
    col(st.booleans(), pl.Boolean),
    col(st.dates(min_value=datetime.date(1900, 3, 1), max_value=datetime.date(2200, 1, 1)), pl.Date),
    col(st.datetimes(min_value=datetime.datetime(1900, 3, 1), max_value=datetime.datetime(2200, 1, 1)).map(lambda d: d.replace(microsecond=0, hour=d.hour or 1)), pl.Datetime("ms")),
)


@st.composite
def frames(draw):
    return pl.DataFrame([draw(column).alias(name) for name in draw(names)])


@SETTINGS
@given(frames())
def test_write_then_read_is_identity(df):
    assert_frame_equal(xp.read_excel(xp.write_excel(df)), df)


def _polars_selects_by_name(name):
    # Polars' readers select columns with pl.col(name), which reads "*" as all columns and
    # "^...$" as a regex (Polars 2.1: its own write_excel output with a column "*" fails to load).
    return name == "*" or (name.startswith("^") and name.endswith("$"))


@SETTINGS
@given(frames())
def test_polars_own_readers_see_the_same_table(df):
    if any(_polars_selects_by_name(n) for n in df.columns):
        return
    data = xp.write_excel(df)
    ours = xp.read_excel(data)
    for engine in ("calamine", "openpyxl"):
        theirs = pl.read_excel(io.BytesIO(data), engine=engine, drop_empty_rows=False, drop_empty_cols=False)
        assert theirs.columns == ours.columns, engine
        for name in ours.columns:
            a, b = ours[name], theirs[name]
            if a.dtype != b.dtype:
                # Same values; the engines pick other widths for dates, times or all-null columns.
                b = b.cast(a.dtype, strict=False)
            assert a.to_list() == b.to_list(), (engine, name, a.dtype, theirs[name].dtype)


@SETTINGS
@given(frames())
def test_we_read_what_polars_writes(df):
    buf = io.BytesIO()
    df.write_excel(buf)
    assert_frame_equal(xp.read_excel(buf.getvalue()), df)


def test_read_cells_is_the_long_table():
    df = pl.DataFrame({"a": [1, None, 3], "b": ["x", "y", None], "d": [datetime.date(2026, 11, 12), None, None]})
    cells = xp.read_cells(xp.write_excel(df))
    assert cells.columns == ["sheet", "sheet_name", "row", "col", "kind", "num", "text", "formula", "is_date", "num_format"]
    assert cells.height == 3 + 2 + 2 + 1
    assert cells.filter(pl.col("is_date"))["num"].to_list() == [46338.0]
    wide = cells.filter(pl.col("row") > 0).pivot(on="col", index="row", values="num").sort("row")
    assert wide["0"].to_list() == [1.0, None, 3.0]
    assert xp.read_excel(xp.write_excel(df), has_header=False).height == 4
    with pytest.raises(ValueError):
        xp.read_excel(xp.write_excel(df), sheet_name="nope")
