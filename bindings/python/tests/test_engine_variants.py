"""The two engines: on the Polars of this environment, and with Polars built in."""
import os
import struct
import subprocess
import sys

import pytest

environment = pytest.importorskip("xlcells_engine")
bundled = pytest.importorskip("xlcells_engine_bundled")

GOLDEN = os.path.join(os.path.dirname(__file__), "..", "..", "..", "crates", "xl-polars", "tests", "data", "lift-golden.xlsx")


def test_each_says_which_polars_runs_it():
    import polars

    assert environment.engine_info() == ("environment", polars.__version__)
    assert bundled.engine_info()[0] == "bundled"


def test_both_calculate_the_same_to_13_digits():
    """Every formula value of the golden workbook removed and calculated again: the same
    cells are calculated, with the same values to 13 digits (the last bits differ
    between two versions of Polars: one divides, the other multiplies by the reciprocal)."""
    with open(GOLDEN, "rb") as f:
        data = f.read()
    mine, theirs = environment.recalculation(data).splitlines(), bundled.recalculation(data).splitlines()
    assert mine[0] == theirs[0]
    assert len(mine) == len(theirs) > 3000
    number = lambda v: struct.unpack(">d", bytes.fromhex(v[2:]))[0]
    for a, b in zip(mine[1:], theirs[1:]):
        if a == b:
            continue
        (cell, va, _), (_, vb, _) = (a.split("\t") + [""])[:3], (b.split("\t") + [""])[:3]
        assert va.startswith("n ") and vb.startswith("n "), (a, b)
        x, y = number(va), number(vb)
        assert abs(x - y) <= 1e-13 * max(abs(x), abs(y)), (cell, x, y)


def test_the_plan_format_reaches_polars():
    """The shim builds Polars expressions from the engine's nodes."""
    import polars as pl
    from xlcells_engine import _shim

    plan = '{"nodes":[["col","a"],["lit","f64",1.5],["bin","+",0,1],["fn","cum_sum",[2],false],["lit","f64","inf"],["bin",">",2,4],["lit","null",null],["cast",6,"Float64"],["when",5,2,7]],"steps":[[["_t0",3]]],"outputs":[["_r",8],["_s",3]],"broadcast":true}'
    out = _shim.run(-1, plan, pl.DataFrame({"a": [1.0, 2.0]}))
    assert out.to_dict(as_series=False) == {"_r": [None, None], "_s": [2.5, 6.0]}
    # A plan known by its key is not sent again.
    assert _shim.run(-1, None, pl.DataFrame({"a": [0.5]}))["_s"].to_list() == [2.0]


@pytest.mark.parametrize("kind,module", [("environment", "xlcells_engine"), ("bundled", "xlcells_engine_bundled")])
def test_the_engine_can_be_chosen(kind, module):
    code = "from xlcells._needs_engine import engine; print(engine('x').__name__)"
    out = subprocess.run([sys.executable, "-c", code], env=dict(os.environ, XLCELLS_ENGINE=kind), capture_output=True, text=True, check=True)
    assert out.stdout.strip() == module
