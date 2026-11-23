"""A store as a Python project on Polars (a port): made, checked, carried on, and back
into the workbook. The project's own programs run as a person runs them (subprocess)."""
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

pl = pytest.importorskip("polars")
import xlcells.openpyxl as xo  # noqa: E402
from xlcells._needs_engine import engine  # noqa: E402

try:
    engine("The tests of ports")
except ImportError as e:
    pytest.skip(str(e), allow_module_level=True)

ROOT = Path(__file__).resolve().parents[3]
DATA = ROOT / "app" / "core" / "test" / "data"

# A Python with Polars and without xlcells, as an agent's sandbox has it: XL_POLARS_PYTHON,
# else the one tools make in target/polars-only (if it is there).
POLARS_ONLY = os.environ.get("XL_POLARS_PYTHON") or next((str(p) for p in [ROOT / "target/polars-only/bin/python", ROOT / "target/polars-only/Scripts/python.exe"] if p.exists()), None)


def port(tmp_path, workbook):
    shutil.copy(DATA / workbook, tmp_path / workbook)
    store = xo.load_workbook(tmp_path / workbook).to_store(tmp_path / (workbook + ".store"))
    report = store.to_port(tmp_path / "port")
    return store, tmp_path / "port", report


def run(folder, *args, python=sys.executable):
    return subprocess.run([python, *args], cwd=folder, capture_output=True, text=True, encoding="utf-8")


def test_a_workbook_becomes_a_project_whose_check_passes(tmp_path):
    _, folder, report = port(tmp_path, "model.xlsx")
    how = {(i["region"], i["name"]): i["how"] for i in report["items"]}
    # A column whose one row holds a plain value is still Polars: that row, the formula in the others.
    assert how == {("Sales", "amount"): "polars", ("Model", "Gross"): "polars", ("Model", "Today"): "rest"}
    for name in ["README.md", "pyproject.toml", "logic.json", "cells.json", "workbook.xlsx", "port/calc.py", "port/xl.py", "check.py", "run.py", "tests/test_port.py", "data/Sales.parquet", "expected/Sales.parquet"]:
        assert (folder / name).exists(), name
    assert "amount" not in pl.read_parquet(folder / "data/Sales.parquet").columns, "the inputs hold no calculated column"
    lines = (folder / "port/calc.py").read_text(encoding="utf-8").splitlines()
    gross = next(i for i in report["items"] if i["name"] == "Gross")
    assert lines[gross["line"] - 1].startswith("    # Model: Gross"), "an item's line is its line in calc.py"
    done = run(folder, "check.py")
    assert done.returncode == 0, done.stdout + done.stderr
    assert "0 different" in done.stdout
    tests = run(folder, "-m", "pytest", "-q", "-p", "no:cacheprovider")
    assert tests.returncode == 0, tests.stdout + tests.stderr


def test_a_wrong_line_is_found_and_other_data_is_checked_against_the_engine(tmp_path):
    _, folder, _ = port(tmp_path, "model.xlsx")
    # Other inputs: the program and the formula engine agree on them too.
    other = tmp_path / "other"
    shutil.copytree(folder / "data", other)
    sales = pl.read_parquet(other / "Sales.parquet").with_columns(pl.col("qty") * 7)
    sales.write_parquet(other / "Sales.parquet")
    done = run(folder, "check.py", "--exact", "--data", str(other))
    assert done.returncode == 0, done.stdout + done.stderr
    # A port that calculates wrong is told so, with the line.
    calc = folder / "port/calc.py"
    calc.write_text(calc.read_text(encoding="utf-8").replace('gross = sales["amount"].sum()', 'gross = sales["amount"].sum() + 1'), encoding="utf-8")
    done = run(folder, "check.py")
    assert done.returncode == 1
    assert "differs       Model[Gross]" in done.stdout


def test_the_results_go_back_into_the_workbook(tmp_path):
    _, folder, _ = port(tmp_path, "model.xlsx")
    done = run(folder, "run.py", "--out", str(tmp_path / "out"), "--excel", str(tmp_path / "back.xlsx"))
    assert done.returncode == 0, done.stdout + done.stderr
    assert pl.read_parquet(tmp_path / "out/Sales.parquet")["amount"].to_list() == [2.38, 4.76, 99, 9.52]
    back = xo.load_workbook(tmp_path / "back.xlsx", data_only=True)
    assert [c.value for c in back["Sales"]["D"][1:]] == [2.38, 4.76, 99, 9.52]
    assert back["Sales"]["D2"].value is not None


def test_sheets_built_alike_are_ported_each(tmp_path):
    _, folder, report = port(tmp_path, "months.xlsx")
    assert report["items"], report
    done = run(folder, "check.py")
    assert done.returncode == 0, done.stdout + done.stderr


def test_a_folder_with_a_port_in_it_is_not_overwritten(tmp_path):
    store, folder, _ = port(tmp_path, "model.xlsx")
    with pytest.raises(ValueError, match="not empty"):
        store.to_port(folder)
    assert store.to_port(folder, replace=True)["items"]


@pytest.mark.skipif(POLARS_ONLY is None, reason="no Python with Polars alone (XL_POLARS_PYTHON)")
def test_with_polars_alone_the_rest_keeps_the_workbooks_values(tmp_path):
    _, folder, _ = port(tmp_path, "model.xlsx")
    done = run(folder, "check.py", python=POLARS_ONLY)
    assert done.returncode == 0, done.stdout + done.stderr
    assert "2 right, 0 different, 1 not worked out" in done.stdout
    assert "kept          Model[Today]" in done.stdout
