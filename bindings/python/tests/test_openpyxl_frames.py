"""ws.to_polars / to_arrow: sheets as typed dataframes, built in Rust."""
import datetime
import io

import polars as pl
import pytest
from polars.testing import assert_frame_equal

import xlcells.openpyxl as xo


def book():
    wb = xo.Workbook()
    ws = wb.active
    ws.append(["id", "name", "price", "when", "ok", "mixed", None, "at"])
    rows = [
        (1, "apple", 1.5, datetime.date(2026, 1, 31), True, 1, None, datetime.time(8, 30)),
        (2, "pear", 2.25, datetime.date(2026, 2, 28), False, "two", None, datetime.time(12, 0)),
        (3, None, None, None, None, 3.5, None, None),
    ]
    for r in rows:
        ws.append(r)
    ws["C5"] = "=1/0"
    return wb, ws


def test_columns_are_typed():
    wb, ws = book()
    df = ws.to_polars()
    assert df.columns == ["id", "name", "price", "when", "ok", "mixed", "column_7", "at"]
    assert df.schema == {"id": pl.Int64, "name": pl.String, "price": pl.Float64, "when": pl.Date, "ok": pl.Boolean, "mixed": pl.String, "column_7": pl.Null, "at": pl.Time}
    assert df["id"].to_list() == [1, 2, 3]
    assert df["name"].to_list() == ["apple", "pear", None]
    assert df["price"].to_list() == [1.5, 2.25, None]
    assert df["when"].to_list() == [datetime.date(2026, 1, 31), datetime.date(2026, 2, 28), None]
    assert df["ok"].to_list() == [True, False, None]
    assert df["mixed"].to_list() == ["1", "two", "3.5"]
    assert df["at"].to_list() == [datetime.time(8, 30), datetime.time(12, 0), None]


def test_ranges_and_headers():
    wb, ws = book()
    part = ws.to_polars("A1:C3")
    assert part.shape == (2, 3) and part.columns == ["id", "name", "price"]
    assert_frame_equal(ws.to_polars("A:B"), ws.to_polars().select("id", "name"))
    assert ws.to_polars("2:3", header=False)["column_2"].to_list() == ["apple", "pear"]
    raw = ws.to_polars("A1:B2", header=False)
    assert raw.columns == ["column_1", "column_2"] and raw["column_1"].to_list() == ["id", "1"]
    assert xo.Workbook().active.to_polars().shape == (0, 0)
    with pytest.raises(ValueError):
        ws.to_polars("nonsense")


def test_changes_are_seen_and_a_loaded_workbook_reads_the_same():
    wb, ws = book()
    ws["A2"] = 10
    assert ws.to_polars()["id"].to_list() == [10, 2, 3]
    again = xo.load_workbook(io.BytesIO(wb.to_bytes()))
    assert_frame_equal(again.active.to_polars(), ws.to_polars())


def test_arrow_gets_the_same_table():
    wb, ws = book()
    df = ws.to_polars()
    table = ws.to_arrow()
    assert table.column_names == df.columns and table.num_rows == 3
    assert_frame_equal(pl.DataFrame(table), df)
