"""wb.regions: tables, single cells and blocks; dataframes out, back in, and compared."""
import io

import polars as pl
import pytest
from polars.testing import assert_frame_equal

import xlcells.openpyxl as xo
from xlcells.openpyxl.worksheet.table import Table


def book():
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Sales"
    ws.append(["Orders"])
    ws.append(["item", "qty", "price", "amount"])
    for r, (item, qty, price) in enumerate([("apple", 2, 1.5), ("pear", 1, 2.0), ("fig", 4, 0.5)], 3):
        ws.append([item, qty, price, "=B{0}*C{0}*(1+Model!$B$1)".format(r)])
    ws["C8"], ws["D8"] = "Total", "=SUM(D3:D5)"
    model = wb.create_sheet("Model")
    model["A1"], model["B1"] = "Tax rate:", 0.25
    model["A2"], model["B2"] = "Gross", "=Sales!D8"
    wb.calculate()
    return wb


def test_regions_are_found_and_described():
    wb = book()
    regions = wb.regions
    assert [(r.name, r.kind, r.sheet.title, r.ref) for r in regions.values()] == [
        ("Orders", "table", "Sales", "A2:D5"), ("Sales", "cells", "Sales", "C8:D8"), ("Model", "cells", "Model", "A1:B2")]
    orders = regions["Orders"]
    assert [(c.name, c.letter, c.dtype, c.calculated) for c in orders.columns] == [("item", "A", "String", False), ("qty", "B", "Int64", False), ("price", "C", "Float64", False), ("amount", "D", "Float64", True)]
    assert orders.columns[3].logic == "[@qty]*[@price]*(1+Tax_rate)" or orders.columns[3].logic == "[@qty]*[@price]*(1+Model!$B$1)"
    assert [c.name for c in regions["Model"].cells] == ["Tax rate", "Gross"]
    assert regions["Sales"].cells[0].logic == "SUM(Orders[amount])"
    assert list(wb["Model"].regions) == ["Model"]
    with pytest.raises(KeyError, match="the regions are"):
        regions["nope"]
    assert "Orders" in repr(regions)


def test_a_table_goes_out_and_comes_back_longer():
    wb = book()
    orders = wb.regions["Orders"]
    df = orders.to_polars()
    assert df["amount"].to_list() == [3.75, 2.5, 2.5]
    assert orders.verify(df).height == 0
    more = pl.concat([df.drop("amount"), pl.DataFrame({"item": ["plum"], "qty": [10], "price": [1.0]})])
    ws = wb["Sales"]
    orders.write(more)
    assert orders.ref == "A2:D6"
    # The total below the table moves down with the new row, and sums it too.
    assert ws["A6"].value == "plum" and ws["D6"].value == "=B6*C6*(1+Model!$B$1)" and ws["D9"].value == "=SUM(D3:D6)" and ws["D8"].value is None
    wb.calculate()
    again = wb.regions["Orders"].to_polars()
    assert again["amount"].to_list() == [3.75, 2.5, 2.5, 12.5]
    assert wb.regions["Model"].to_polars()["Gross"].to_list() == [21.25]
    # What differs is listed by cell.
    diff = wb.regions["Orders"].verify(again.with_columns(pl.Series("qty", [2, 1, 5, 10])))
    assert diff["cell"].to_list() == ["Sales!B5"] and diff["expected"].to_list() == ["4"] and diff["actual"].to_list() == ["5"]
    # The file holds the same.
    saved = xo.load_workbook(io.BytesIO(wb.to_bytes()))
    assert_frame_equal(saved.regions["Orders"].to_polars(), again)


def test_single_cells_are_one_row_and_inputs_can_be_set():
    wb = book()
    model = wb.regions["Model"]
    row = model.to_polars()
    assert row.columns == ["Tax rate", "Gross"] and row["Tax rate"].to_list() == [0.25]
    model.write(row.with_columns(pl.lit(0.5).alias("Tax rate")))
    assert wb["Model"]["B1"].value == 0.5 and wb["Model"]["B2"].value == "=Sales!D8"
    wb.calculate()
    assert wb.regions["Orders"].to_polars()["amount"].to_list() == [4.5, 3.0, 3.0]


def test_an_excel_table_is_a_region_and_keeps_its_range():
    wb = xo.Workbook()
    ws = wb.active
    ws.append(["k", "v"])
    for r in range(3):
        ws.append([r, r * 1.5])
    ws.add_table(Table(displayName="Data", ref="A1:B4"))
    region = wb.regions["Data"]
    assert (region.kind, region.source, region.ref) == ("table", "excel table", "A1:B4")
    region.write(pl.DataFrame({"k": [1, 2, 3, 4, 5], "v": [0.5, 1.5, 2.5, 3.5, 4.5]}))
    assert wb.regions["Data"].ref == "A1:B6" and ws.tables["Data"].ref == "A1:B6"
    assert wb.regions["Data"].to_polars()["v"].to_list() == [0.5, 1.5, 2.5, 3.5, 4.5]
