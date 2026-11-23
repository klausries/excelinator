"""Tests of the port (run with: uv run --with pytest pytest)."""
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import polars as pl  # noqa: E402
import pytest  # noqa: E402

from check import check  # noqa: E402
from port import xl  # noqa: E402


def test_every_value_the_program_works_out_is_that_of_the_workbook():
    wrong = [i for i in check() if i["state"] == "differs"]
    assert not wrong, wrong


def test_inputs_without_a_column_are_refused(tmp_path):
    shutil.copytree(xl.HOME / "data", tmp_path / "data")
    region = next(r for r in xl.logic()["regions"] if r["inputs"])
    path = tmp_path / "data" / region["file"]
    pl.read_parquet(path).drop(region["inputs"][0]["name"]).write_parquet(path)
    with pytest.raises(xl.InputError):
        xl.read_inputs(tmp_path / "data")
