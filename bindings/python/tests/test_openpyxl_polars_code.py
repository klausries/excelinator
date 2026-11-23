"""wb.to_polars_code: the workbook as a Polars program, and wb.evaluate, the exact engine."""
import io

import polars as pl
import pytest

import xlcells.openpyxl as xo


def book():
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Sales"
    ws.append(["Orders"])
    ws.append(["item", "product", "qty", "price", "amount", "share", "label", "code"])
    rows = [("a", "P1", 2, None), ("b", "P2", 1, None), ("c", "P1", 4, None), ("d", "P3", 3, None)]
    for r, (item, product, qty, _) in enumerate(rows, 3):
        ws.append([item, product, qty,
                   "=VLOOKUP(B{0},Products!$A$2:$C$4,3,FALSE)".format(r),
                   "=ROUND(C{0}*D{0}*(1+Model!$B$1),2)".format(r),
                   "=E{0}/Model!$B$3".format(r),
                   '=IF(AND(C{0}>=2,E{0}>5),"big",LEFT(A{0},1)&"-small")'.format(r),
                   '=TEXT(C{0},"000")'.format(r)])
    products = wb.create_sheet("Products")
    products.append(["id", "name", "price"])
    for row in [("P1", "pen", 1.25), ("P2", "ink", 7.5), ("P3", "pad", 2.0)]:
        products.append(row)
    model = wb.create_sheet("Model")
    model["A1"], model["B1"] = "Tax rate", 0.19
    model["A2"], model["B2"] = "Pens sold", '=SUMIF(Sales!B3:B6,"P1",Sales!C3:C6)'
    model["A3"], model["B3"] = "Gross", "=SUM(Sales!E3:E6)"
    model["A4"], model["B4"] = "Average", "=IF(B2>0,B3/COUNT(Sales!C3:C6),0)"
    result = wb.calculate()
    assert not result.get("not_computed"), result
    return wb


def run(code, wb):
    out = []
    lines = ["" if line.startswith("wb = xo.load_workbook(") else line for line in code.splitlines()]
    scope = {"wb": wb, "print": lambda *a: out.append(a[0])}
    exec("\n".join(lines), scope)
    return scope, out


def test_plain_code_reproduces_the_workbook_where_polars_agrees():
    wb = book()
    code = wb.to_polars_code("sales.xlsx", accuracy="plain")
    how = {(r, n): h for r, n, _f, h, _w in code.items}
    assert how[("Orders", "price")] == "polars" and how[("Orders", "amount")] == "polars" and how[("Orders", "share")] == "polars"
    assert how[("Model", "Gross")] == "polars" and how[("Model", "Pens sold")] == "polars" and how[("Model", "Average")] == "polars"
    # TEXT has no Polars form here: it is the engine's translation, written out.
    assert how[("Orders", "code")] == "plan"
    assert 'wb = xo.load_workbook("sales.xlsx")' in code
    scope, out = run(code, wb)
    assert scope["orders"]["price"].to_list() == [1.25, 7.5, 1.25, 2.0]
    assert scope["orders"]["amount"].to_list() == [2.98, 8.93, 5.95, 7.14]
    assert scope["gross"] == pytest.approx(25.0) and scope["pens_sold"] == 6
    assert scope["orders"]["code"].to_list() == ["002", "001", "004", "003"]


def test_auto_leaves_to_the_engine_what_polars_gets_differently():
    wb = book()
    code = wb.to_polars_code(accuracy="auto")
    scope, out = run(code, wb)
    assert out == ["every value as in the workbook"], (out, code)
    assert "Orders[code]" in code.summary()
    assert sum(1 for i in code.items if i[3] == "polars") >= 5, code.summary()


def test_exact_code_and_evaluate_follow_new_inputs():
    wb = book()
    code = wb.to_polars_code(accuracy="exact")
    # No Polars forms: the engine's translation where it can be written out, else its values.
    assert all(i[3] in ("plan", "engine") for i in code.items) and any(i[3] == "plan" for i in code.items)
    scope, out = run(code, wb)
    assert out == ["every value as in the workbook"]
    orders = wb.regions["Orders"].to_polars().select("item", "product", "qty").with_columns(pl.col("qty") * 10)
    result = wb.evaluate({"Orders": orders, "Model": pl.DataFrame({"Tax rate": [0.0]})})
    assert result["Orders"]["amount"].to_list() == [25.0, 75.0, 50.0, 60.0]
    assert result["Model"]["Gross"].to_list() == [210.0] and result.not_computed == []
    # The workbook itself is as it was.
    assert wb["Model"]["B1"].value == 0.19


def test_what_cannot_be_asked_for_says_what_can():
    with pytest.raises(ValueError, match="'auto'"):
        book().to_polars_code(accuracy="fast")


def changed():
    """The workbook of `book()` with another tax rate and other quantities, calculated."""
    wb = book()
    wb["Model"]["B1"] = 0.07
    for row, qty in zip(range(3, 7), (5, 2, 7, 1)):
        wb["Sales"]["C{0}".format(row)] = qty
    result = wb.calculate()
    assert not result.get("not_computed"), result
    return wb


def test_the_engines_translation_is_written_out_and_holds_for_other_data():
    """What the Polars form lacks (TEXT) is written as the engine's own translation: it
    reads the data and the tax rate of the program, so other data and another rate give the
    values the engine works out for them."""
    code = book().to_polars_code("sales.xlsx", accuracy="exact")
    how = {(r, n): h for r, n, _f, h, _w in code.items}
    assert how[("Orders", "code")] == "plan" and how[("Orders", "amount")] == "plan" and how[("Model", "Gross")] == "plan"
    wb = changed()
    scope, out = run(code, wb)
    assert out == ["every value as in the workbook"], out
    assert scope["orders"]["code"].to_list() == ["005", "002", "007", "001"]
    stored = wb.regions["Orders"].to_polars()["amount"].to_list()
    assert scope["orders"]["amount"].to_list() == stored and stored == [6.69, 16.05, 9.36, 2.14]
    assert scope["gross"] == pytest.approx(sum(stored))


def test_auto_checks_the_plan_items_too():
    code = book().to_polars_code("sales.xlsx")
    how = {(r, n): h for r, n, _f, h, _w in code.items}
    assert how[("Orders", "code")] == "plan"
    scope, out = run(code, changed())
    assert out == ["every value as in the workbook"], out


def test_a_plan_checks_what_it_took_for_granted():
    """The translation was made for quantities that are numbers. With text among them the
    program takes the engine's values for what reads them instead of a wrong number."""
    code = book().to_polars_code("sales.xlsx", accuracy="exact")
    assert "# what the translation was made for" in code
    wb = book()
    wb["Sales"]["C4"] = "n/a"
    scope, _out = run(code, wb)
    # The engine does not tell which error text in a product is, and leaves such a cell as
    # it was: not a number worked out from the text.
    assert scope["orders"]["qty"].dtype == pl.String
    assert scope["orders"]["amount"].to_list() == pytest.approx([2.98, 8.93, 5.95, 7.14], rel=1e-12)
    # Without the text the plan gives what it gave.
    scope, out = run(code, book())
    assert out == ["every value as in the workbook"]


def test_a_plan_column_of_numbers_and_empty_text_is_numbers():
    """=IF(C3>2, C3*2, "") gives numbers and empty text: a column of numbers with gaps, as
    the workbook shows it, not text."""
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Data"
    ws.append(["Table"])
    ws.append(["qty", "double", "cheap"])
    for r, qty in enumerate([1, 3, 5, 2], 3):
        ws.append([qty, '=IF(A{0}>2,A{0}*2,"")'.format(r), '=IF(A{0}>2,"",TEXT(A{0},"00"))'.format(r)])
    wb.calculate()
    code = wb.to_polars_code("data.xlsx", accuracy="exact")
    scope, out = run(code, wb)
    assert out == ["every value as in the workbook"], out
    column = scope["table"]["double"]
    assert column.dtype in (pl.Int64, pl.Float64) and column.to_list() == [None, 6, 10, None]
