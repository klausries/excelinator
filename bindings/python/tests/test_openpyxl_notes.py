"""The logic of a workbook travels with its data: notes in the Parquet files of a store,
and the calculation done again from them alone."""
import json
import os

import pytest

pl = pytest.importorskip("polars")
import xlcells.openpyxl as xo  # noqa: E402

from xlcells._needs_engine import engine  # noqa: E402

try:  # any build of the engine: built for this machine, or the WebAssembly module
    engine("The tests of notes")
except ImportError as e:
    pytest.skip(str(e), allow_module_level=True)


def model(tmp_path):
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Sales"
    ws.append(["item", "qty", "price", "amount", "big"])
    for k in range(3):
        row = k + 2
        ws.append(["item{0}".format(k), k + 1, 2.0, "=ROUND(B{0}*C{0}*(1+Model!$B$1),2)".format(row), '=IF(D{0}>3,"yes","no")'.format(row)])
    m = wb.create_sheet("Model")
    m.append(["Tax rate", 0.19])
    m.append(["Gross", "=SUM(Sales!D2:D4)"])
    m.append(["Average", "=B2/COUNT(Sales!B2:B4)"])
    m.append(["Today", "=TODAY()"])
    path = str(tmp_path / "model.xlsx")
    wb.save(path)
    wb = xo.load_workbook(path)
    wb.calculate()
    wb.save(path)
    return xo.load_workbook(path)


def test_the_parquet_files_of_a_store_say_how_they_are_calculated(tmp_path):
    store = model(tmp_path).to_store(tmp_path / "store")
    note = xo.read_note(os.path.join(store.path, "data", "Sales.parquet"))
    assert (note["region"], note["kind"], note["sheet"], note["ref"]) == ("Sales", "table", "Sales", "A1:E4")
    amount = {c["name"]: c for c in note["columns"]}["amount"]
    assert amount["formula"] == "ROUND(B2*C2*(1+Model!$B$1),2)"
    assert amount["logic"] == "ROUND([@qty]*[@price]*(1+Tax_rate),2)"
    assert [i.get("column") or i.get("cell") for i in amount["inputs"]] == ["qty", "price", ["Model", "Tax rate"]]
    assert note["plans"][amount["plan"]]["outputs"][0][0] == "_r"
    # Any reader of Parquet finds it: it is plain file metadata.
    assert json.loads(pl.read_parquet_metadata(os.path.join(store.path, "data", "Sales.parquet"))[xo.NOTE_KEY]) == note
    # What the engine has no plan for says why.
    today = {c["name"]: c for c in store.notes["Model"]["columns"]}["Today"]
    assert "plan" not in today and "volatile" in today["why"]


def test_a_store_is_calculated_again_without_its_workbook(tmp_path):
    store = model(tmp_path).to_store(tmp_path / "store")
    # Other data: one more order, another tax rate.
    sales = pl.DataFrame({"item": ["a", "b", "c", "d"], "qty": [1, 2, 3, 10], "price": [2.0, 2.0, 2.5, 1.0]})
    store.write("Sales", sales)
    # (Single cells are rows of the store: name, cell, dtype, number, text.)
    model_rows = store.read("Model")
    assert model_rows.columns == ["name", "cell", "dtype", "number", "text"]
    assert model_rows.filter(pl.col("name") == "Tax rate").select("cell", "dtype", "number").row(0) == ("B1", "Float64", 0.19)
    store.write("Model", pl.DataFrame({"name": ["Tax rate"], "number": [0.07]}))
    os.rename(os.path.join(store.path, "workbook.xlsx"), os.path.join(store.path, "away.xlsx"))
    report = store.recalculate()
    assert report["done"] == [("Sales", "amount"), ("Sales", "big"), ("Model", "Gross"), ("Model", "Average")]
    assert [(k[0], k[1]) for k in report["kept"]] == [("Model", "Today")]
    values = store.values("Model")
    by_notes = (store.read("Sales"), {name: values[name] for name in ("Tax rate", "Gross", "Average")})
    assert by_notes[0]["amount"].to_list() == pytest.approx([2.14, 4.28, 8.03, 10.7])
    assert by_notes[0]["big"].to_list() == ["no", "yes", "yes", "yes"]
    assert by_notes[1] == {"Tax rate": 0.07, "Gross": pytest.approx(25.15), "Average": pytest.approx(25.15 / 4)}
    # The workbook's own calculation of the same data agrees.
    os.rename(os.path.join(store.path, "away.xlsx"), os.path.join(store.path, "workbook.xlsx"))
    store.run()
    assert store.read("Sales").equals(by_notes[0])
    values = store.values("Model")
    assert {name: values[name] for name in by_notes[1]} == by_notes[1]
    # A cell without a value (the engine does not work out TODAY) is None, and a row without one.
    assert values["Today"] is None
    assert store.read("Model").filter(pl.col("name") == "Today").select("dtype", "number", "text").row(0) == ("Null", None, None)


def test_parquet_files_outside_a_store_are_calculated_from_their_notes(tmp_path):
    store = model(tmp_path).to_store(tmp_path / "store")
    files = {}
    for name in ("Sales", "Model"):
        files[name] = str(tmp_path / (name + ".parquet"))
        os.rename(os.path.join(store.path, "data", name + ".parquet"), files[name])
    # A file changed with Polars keeps its note if it is written with it.
    note = xo.read_note(files["Sales"])
    pl.read_parquet(files["Sales"]).with_columns(pl.col("qty") * 10).write_parquet(files["Sales"], metadata={xo.NOTE_KEY: json.dumps(note)})
    report = xo.recalculate(files.values())
    assert ("Sales", "amount") in report["done"] and ("Model", "Gross") in report["done"]
    assert pl.read_parquet(files["Sales"])["amount"].to_list() == [23.8, 47.6, 71.4]
    assert pl.read_parquet(files["Model"]).filter(pl.col("name") == "Gross")["number"][0] == pytest.approx(142.8)
    assert xo.read_note(files["Sales"]) == note
    # Without its note a file cannot be calculated, and that is said.
    pl.read_parquet(files["Sales"]).write_parquet(files["Sales"])
    with pytest.raises(ValueError, match="carries no note"):
        xo.recalculate(files.values())


def test_a_plan_that_no_longer_fits_the_data_keeps_the_values(tmp_path):
    store = model(tmp_path).to_store(tmp_path / "store")
    before = store.read("Sales")["amount"].to_list()
    # The quantities become text: the plan was made for numbers.
    store.write("Sales", store.read("Sales").drop("amount", "big").with_columns(pl.col("qty").cast(pl.String)))
    report = store.recalculate()
    kept = {(k[0], k[1]): k[3] for k in report["kept"]}
    assert "held numbers and holds String now" in kept[("Sales", "amount")]
    assert store.read("Sales")["amount"].to_list() == before
    # What reads the kept column is calculated, and named as resting on it.
    assert ("Model", "Gross", "Sales[amount]") in report["from_kept"]


def with_exceptions(tmp_path):
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Sales"
    ws.append(["item", "qty", "price", "amount"])
    for k in range(6):
        row = k + 2
        amount = "=B{0}*C{0}".format(row)
        if k == 2:
            amount = "=B{0}*C{0}*2".format(row)     # a formula of its own
        if k == 4:
            amount = 99.0                            # a plain value
        ws.append(["item{0}".format(k), "=B2*2" if k == 1 else k + 1, 2.0, amount])
    path = str(tmp_path / "exceptions.xlsx")
    wb.save(path)
    wb = xo.load_workbook(path)
    wb.calculate()
    wb.save(path)
    return xo.load_workbook(path)


def test_rows_that_are_not_as_their_column_are_its_exceptions(tmp_path):
    wb = with_exceptions(tmp_path)
    columns = {c.name: c for c in wb.regions["Sales"].columns}
    assert columns["amount"].logic == "[@qty]*[@price]"
    assert [(e.row, e.coordinate, e.logic) for e in columns["amount"].exceptions] == [(2, "D4", "[@qty]*[@price]*2"), (4, "D6", None)]
    assert (columns["qty"].formula, [(e.row, e.coordinate, e.formula) for e in columns["qty"].exceptions]) == (None, [(1, "B3", "B2*2")])

    store = wb.to_store(tmp_path / "store")
    noted = {c["name"]: c for c in store.notes["Sales"]["columns"]}
    assert [e["row"] for e in noted["amount"]["exceptions"]] == [2, 4]
    assert "plan" in noted["amount"]["exceptions"][0] and "formula" not in noted["amount"]["exceptions"][1]

    # Other data: the exceptions stay what they are, calculated from the files alone.
    store.write("Sales", store.read("Sales").with_columns(pl.Series("qty", [10, 0, 30, 40, 50, 60])))
    report = store.recalculate()
    assert report["kept"] == []
    assert report["done"] == [("Sales", "qty (row 2)"), ("Sales", "amount"), ("Sales", "amount (row 3)")]
    by_notes = store.read("Sales")
    assert by_notes["qty"].to_list() == [10, 20, 30, 40, 50, 60]
    assert by_notes["amount"].to_list() == [20, 40, 120, 80, 99, 120]
    # The workbook's own calculation gives the same, and keeps the rows as they were.
    store.run()
    assert store.read("Sales").equals(by_notes)
    ws = xo.load_workbook(os.path.join(store.path, "workbook.xlsx"))["Sales"]
    assert (ws["B3"].value, ws["D4"].value, ws["D6"].value, ws["D7"].value) == ("=B2*2", "=B4*C4*2", 99, "=B7*C7")
