"""wb.to_store: Parquet files as the database, the workbook following them."""
import polars as pl
import pytest

import xlcells.openpyxl as xo

from test_openpyxl_polars_code import book


def test_a_store_follows_its_parquet_files_and_its_model(tmp_path):
    wb = book()
    store = wb.to_store(tmp_path / "s")
    assert list(store.regions) == ["Orders", "Products", "Model"]
    assert "amount: Float64  =ROUND([@qty]*[@price]*(1+Tax_rate),2)" in repr(store)
    orders = store.read("Orders")
    assert orders["amount"].to_list() == pytest.approx([2.98, 8.93, 5.95, 7.14])
    # The database changes: a new product, a new order.
    products = pl.concat([store.read("Products"), pl.DataFrame({"id": ["P4"], "name": ["cap"], "price": [10.0]})])
    pl.DataFrame(products).write_parquet(tmp_path / "s" / "data" / "Products.parquet")
    more = pl.concat([orders.select("item", "product", "qty"), pl.DataFrame({"item": ["e"], "product": ["P4"], "qty": [1]})])
    store.write("Orders", more)
    report = store.run()
    assert report["not_computed"] == {} and report["regions"]["Orders"] == 5
    assert store.read("Orders")["amount"].to_list() == pytest.approx([2.98, 8.93, 5.95, 7.14, 11.9])
    assert store.values("Model")["Gross"] == pytest.approx(36.9)
    # Single cells are rows of the store; they are set by name, the others stay.
    model = store.read("Model")
    assert model.columns == ["name", "cell", "dtype", "number", "text"] and "Gross" in model["name"].to_list()
    rate = store.values("Model")["Tax rate"]
    store.set_values("Model", {"Tax rate": rate * 2})
    assert store.values("Model")["Tax rate"] == rate * 2 and store.values("Model")["Gross"] == pytest.approx(36.9)
    store.write("Model", pl.DataFrame({"name": ["Tax rate"], "number": [rate]}))
    with pytest.raises(ValueError, match="has no cell 'Nope'|has no cell \"Nope\""):
        store.set_values("Model", Nope=1)
    with pytest.raises(ValueError, match="not single cells"):
        store.values("Orders")
    # The logic changes: no rounding, another rate.
    store.set_logic("Orders", "amount", "=[@qty]*[@price]*2")
    report = store.run()
    assert report["logic_changed"] == [("Orders", "amount")]
    assert store.read("Orders")["amount"].to_list() == [5.0, 15.0, 10.0, 12.0, 20.0]
    # The workbook holds it all, in its old layout.
    wb2 = store.workbook()
    assert wb2["Sales"]["E7"].value == "=C7*D7*2" and wb2["Sales"]["A1"].value == "Orders"
    assert xo.open_store(tmp_path / "s").regions["Orders"]["ref"] == "A2:H7"
    with pytest.raises(KeyError, match="it has"):
        store.set_logic("Orders", "nope", "1")
    with pytest.raises(FileNotFoundError, match="to_store"):
        xo.open_store(tmp_path / "missing")


def months(tmp_path):
    """Three sheets built alike, a two-row header (a group above, the columns below), a
    fixed value in a calculated column, and March with another formula."""
    wb = xo.Workbook()
    for k, month in enumerate(["Jan", "Feb", "Mar"]):
        ws = wb.active if k == 0 else wb.create_sheet(month)
        ws.title = month
        ws.append([None, "Volume", None, None])
        ws.merge_cells("B1:C1")
        ws.append(["item", "plan", "actual", "difference"])
        for r in range(3):
            row = r + 3
            formula = "=ROUND(C{0}-B{0},0)" if month == "Mar" else "=C{0}-B{0}"
            ws.append(["item{0}".format(r), 10 * (k + 1) + r, 12.5, formula.format(row)])
    wb["Jan"]["D4"] = 99
    path = str(tmp_path / "months.xlsx")
    wb.save(path)
    wb = xo.load_workbook(path)
    wb.calculate()
    wb.save(path)
    return xo.load_workbook(path)


def test_sheets_built_alike_are_read_and_written_as_one_table(tmp_path):
    wb = months(tmp_path)
    # Two header rows: the group's label and the column's own make the name.
    jan = wb.regions["Jan"]
    assert [c.name for c in jan.columns] == ["item", "Volume plan", "Volume actual", "difference"]
    assert jan.columns[3].logic == "[@[Volume actual]]-[@[Volume plan]]" and jan.to_polars().height == 3
    store = wb.to_store(tmp_path / "s")
    assert list(store.stacks) == ["stack"] and "stack (stack of Jan, Feb, Mar)" in repr(store)
    stack = store.stacks["stack"]
    assert [m["sheet"] for m in stack["members"]] == ["Jan", "Feb", "Mar"]
    assert [d["column"] for d in stack["differs"]] == ["difference"]
    all_ = store.read("stack")
    assert all_.columns == ["sheet", "item", "Volume plan", "Volume actual", "difference"] and all_.height == 9
    assert all_.filter(pl.col("sheet") == "Jan")["difference"].to_list() == [2.5, 99.0, 0.5]
    # Polars over all sheets at once; February only is written back, with one more row.
    feb = pl.DataFrame({"sheet": ["Feb"] * 4, "item": ["a", "b", "c", "d"], "Volume plan": [1, 2, 3, 4], "Volume actual": [2.0, 2.0, 2.0, 2.0]})
    store.write("stack", feb)
    # Without the workbook and with it the same values come out.
    store.recalculate()
    assert store.read("stack").filter(pl.col("sheet") == "Feb")["difference"].to_list() == [1.0, 0.0, -1.0, -2.0]
    store.run()
    after = store.read("stack")
    assert after.height == 10 and after.filter(pl.col("sheet") == "Feb")["difference"].to_list() == [1.0, 0.0, -1.0, -2.0]
    assert after.filter(pl.col("sheet") == "Jan")["difference"].to_list() == [2.5, 99.0, 0.5]
    # One more month, as a copy of March.
    assert store.add_sheet("Mar", "Apr") == ["Apr"]
    store.write("stack", pl.DataFrame({"sheet": ["Apr"] * 3, "item": ["x", "y", "z"], "Volume plan": [1, 1, 1], "Volume actual": [1.4, 2.6, 1.0]}))
    store.run()
    assert store.read("stack").filter(pl.col("sheet") == "Apr")["difference"].to_list() == [0.0, 2.0, 0.0]
    book = store.workbook()
    assert book.sheetnames == ["Jan", "Feb", "Mar", "Apr"] and book["Apr"]["D3"].value == "=ROUND(C3-B3,0)"
    assert book["Apr"]["B1"].value == "Volume" and "B1:C1" in [str(m) for m in book["Apr"].merged_cells.ranges]
    with pytest.raises(ValueError, match="add_sheet"):
        store.write("stack", pl.DataFrame({"sheet": ["May"], "item": ["x"]}))


def test_a_fixed_value_in_a_calculated_column_stays_with_and_without_the_workbook(tmp_path):
    store = months(tmp_path).to_store(tmp_path / "s")
    # Other rows for January, the calculated column left out: the 99 of its second row stays.
    store.write("Jan", pl.DataFrame({"item": ["a", "b", "c", "d"], "Volume plan": [1, 1, 1, 1], "Volume actual": [3.0, 3.0, 3.0, 3.0]}))
    store.recalculate()
    without = store.read("Jan")["difference"].to_list()
    store.run()
    assert without == store.read("Jan")["difference"].to_list() == [2.0, 99.0, 2.0, 2.0]


def test_rows_change_in_place_a_change_is_taken_back_and_files_alone_are_data(tmp_path):
    import shutil
    store = book().to_store(tmp_path / "s")
    # What the notes of the files say: the logic, and whether it needs the workbook.
    columns = {c["name"]: c for c in store.tables["Orders"]["columns"]}
    assert columns["amount"]["logic"] == "ROUND([@qty]*[@price]*(1+Tax_rate),2)" and columns["amount"]["state"] == "calculated"
    assert "Model.Tax rate" in columns["amount"]["reads"] and "logic" not in columns["qty"]
    # Some rows: a value, a row less, a row more. The workbook stays built as it was.
    store.patch("Orders", set=[{"row": 0, "column": "qty", "value": 4}], delete=[3], insert=[{"item": "e", "product": "P2", "qty": 2}])
    assert store.read("Orders")["qty"].to_list() == [4, 1, 4, 2]
    store.run()
    sales = store.workbook()["Sales"]
    assert sales["A6"].value == "e" and sales["E6"].value == "=ROUND(C6*D6*(1+Model!$B$1),2)"
    gross = store.values("Model")["Gross"]
    assert gross == pytest.approx(round(4 * 1.25 * 1.19, 2) + 8.93 + 5.95 + round(2 * 7.5 * 1.19, 2))
    # Typed into the workbook, and taken back, change by change.
    store.set_cells("Sales", {"C3": 2, "E4": "=C4*1000"})
    assert store.read("Orders")["qty"].to_list()[0] == 2 and store.read("Orders")["amount"].to_list()[1] == 1000
    assert store.undo() == "cells of Sales"
    assert store.values("Model")["Gross"] == gross
    assert store.undo() == "rows of Orders"
    assert store.read("Orders")["qty"].to_list() == [2, 1, 4, 3]
    with pytest.raises(ValueError, match="the columns are"):
        store.patch("Orders", set=[{"row": 0, "column": "nope", "value": 1}])
    assert store.undo() is None
    # A region gets another name; what reads it says the new one.
    store.rename("Products", "Catalogue")
    assert list(store.regions) == ["Orders", "Catalogue", "Model"] and store.read("Catalogue").height == 3
    assert "Catalogue" in next(c for c in store.regions["Orders"]["columns"] if c["name"] == "price")["logic"]
    with pytest.raises(ValueError, match="already"):
        store.rename("Catalogue", "model")
    assert store.undo() == "name of Products" and list(store.regions) == ["Orders", "Products", "Model"]
    # The Parquet files alone, elsewhere: data to work on, calculated from their notes.
    shutil.copytree(tmp_path / "s" / "data", tmp_path / "plain")
    plain = xo.open_store(tmp_path / "plain")
    assert sorted(plain.tables) == ["Model", "Orders", "Products"]
    plain.patch("Orders", set=[{"row": 0, "column": "qty", "value": 10}])
    plain.recalculate()
    assert plain.read("Orders")["amount"].to_list()[0] == pytest.approx(14.88)
    assert store.read("Orders")["amount"].to_list()[0] == pytest.approx(2.98)
