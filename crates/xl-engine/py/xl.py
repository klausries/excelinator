"""What the program of this port needs besides Polars (written by xlcells; part of the
project: change it if you like).

`calc.py` was written against the interface of xlcells (`load_workbook`, the regions
as dataframes, `evaluate` for what is not ported yet). Here those names serve the files
of the project, with Polars alone:

* ``data/<region>.parquet``: the inputs, what is not calculated;
* ``expected/<region>.parquet``: what the workbook calculates from them, the measure
  the port is checked against (never change it by hand);
* ``cells.json``: the cells of the sheets, for formulas that read cells outside regions;
* ``logic.json``: the regions and every calculated column and cell, with how it is
  ported (``polars``, ``plan``, ``rest``) and where in ``calc.py``.

What is ported as ``rest`` is not worked out by this program yet: with xlcells installed
(``uv pip install xlcells``) its formula engine works it out exactly; without, its values
are those of ``expected/`` (right for the data the workbook had, not for other data),
and ``KEPT`` names them.
"""
import json
import math
import warnings
from pathlib import Path

import polars as pl

#: The folder of the project.
HOME = Path(__file__).resolve().parent.parent

#: (region, column or cell) whose values were taken from expected/, not worked out.
KEPT = set()


class InputError(ValueError):
    """Data that is not what the program was made for: a column missing, or of a type
    it cannot calculate with."""


def logic():
    return json.loads((HOME / "logic.json").read_text(encoding="utf-8"))


def _numeric(dtype):
    return dtype.is_numeric() or dtype in (pl.Boolean, pl.Null)


def read_inputs(folder):
    """The inputs of every region from a folder (as data/), checked against logic.json."""
    folder = Path(folder)
    frames = {}
    for r in logic()["regions"]:
        path = folder / r["file"]
        if not path.exists():
            raise InputError("{0}: no file {1}".format(r["name"], path))
        frame = pl.read_parquet(path)
        missing = [c["name"] for c in r["inputs"] if c["name"] not in frame.columns]
        if missing:
            raise InputError("{0}: the columns {1} are missing in {2}".format(r["name"], missing, path))
        for c in r["inputs"]:
            if c["dtype"] in ("Int64", "Float64") and not _numeric(frame.schema[c["name"]]) and frame[c["name"]].null_count() < frame.height:
                raise InputError("{0}: column {1!r} must hold numbers, it holds {2}".format(r["name"], c["name"], frame.schema[c["name"]]))
        frames[r["name"]] = frame
    return frames


def same(expected, actual):
    """Whether two values are the same as far as a workbook can tell: numbers to 1e-9 of
    their size, text and the rest exactly; empty and an empty text alike."""
    if expected in (None, "") and actual in (None, ""):
        return True
    numbers = (int, float)
    if isinstance(expected, numbers) and isinstance(actual, numbers) and not isinstance(expected, bool) and not isinstance(actual, bool):
        if math.isnan(expected) and math.isnan(actual):
            return True
        return math.isclose(expected, actual, rel_tol=1e-9, abs_tol=1e-12)
    return expected == actual


def differences(region, frame, expected, names=None, limit=20):
    """The values of `frame` that differ from `expected` (columns in both, or `names`):
    a frame of region, column, row, expected, actual (as text), at most `limit`."""
    out = []
    names = [n for n in (names or frame.columns) if n in frame.columns and n in expected.columns]
    for name in names:
        a, e = frame[name].to_list(), expected[name].to_list()
        if len(a) != len(e):
            out.append((region, name, None, "{0} rows".format(len(e)), "{0} rows".format(len(a))))
            continue
        for row, (x, y) in enumerate(zip(e, a)):
            if not same(x, y):
                out.append((region, name, row, repr(x), repr(y)))
                if len(out) >= limit:
                    break
    return pl.DataFrame(out, schema={"region": pl.String, "column": pl.String, "row": pl.Int64, "expected": pl.String, "actual": pl.String}, orient="row")


def item_lines():
    """Where each calculated column and cell is in port/calc.py: the line of its comment
    (``# Region[name]`` for a column, ``# Region: name`` for a cell), found anew each
    time, since lines move as the program is changed; logic.json's where it is gone."""
    text = (HOME / "port" / "calc.py").read_text(encoding="utf-8").splitlines()
    out = {}
    for item in logic()["items"]:
        marks = ("# {0}[{1}]".format(item["region"], item["name"]), "# {0}: {1}".format(item["region"], item["name"]))
        out[(item["region"], item["name"])] = next((k + 1 for k, line in enumerate(text) if line.strip().startswith(marks)), item["line"])
    return out


def expected(region):
    r = next(x for x in logic()["regions"] if x["name"] == region)
    return pl.read_parquet(HOME / "expected" / r["file"])


class Region:
    def __init__(self, name, frame):
        self.name = name
        self._frame = frame

    def to_polars(self):
        return self._frame.clone()

    def verify(self, frame):
        return differences(self.name, frame, expected(self.name))


class _Cell:
    def __init__(self, value):
        self.value = value


class Sheet:
    """A sheet as cells.json has it: values, formulas as ``=...``."""

    def __init__(self, cells):
        self._cells = {(r, c): v for r, c, v in cells}

    def cell(self, row, column):
        return _Cell(self._cells.get((row - 1, column - 1)))

    def iter_rows(self, min_row, max_row, min_col, max_col, values_only=True):
        for r in range(min_row - 1, max_row):
            yield tuple(self._cells.get((r, c)) for c in range(min_col - 1, max_col))


class _Kept:
    """Values of a region from expected/: each column or cell taken is named in KEPT."""

    def __init__(self, region, frame):
        self.region, self.frame = region, frame

    def __getitem__(self, name):
        KEPT.add((self.region, name))
        return self.frame[name]


class Workbook:
    """The inputs of a folder (as data/), as the program reads its workbook."""

    def __init__(self, folder):
        self.frames = read_inputs(folder)
        self.regions = {name: Region(name, frame) for name, frame in self.frames.items()}
        self._sheets = None

    def __getitem__(self, sheet):
        if self._sheets is None:
            self._sheets = json.loads((HOME / "cells.json").read_text(encoding="utf-8"))
        return Sheet(self._sheets.get(sheet, []))

    def evaluate(self, frames):
        """What is not ported yet: the formula engine of xlcells on these inputs, or
        without it the values of expected/."""
        try:
            return engine_values(frames)
        except ImportError:
            warnings.warn("xlcells is not installed: what is not ported yet keeps the values of expected/ (see xl.KEPT)", stacklevel=2)
            return {r["name"]: _Kept(r["name"], pl.read_parquet(HOME / "expected" / r["file"])) for r in logic()["regions"]}


def engine_values(frames):
    """Every region as xlcells' formula engine calculates it from these inputs, exactly as
    Excel does (needs xlcells). A region without inputs (all of it calculated) keeps the
    workbook's own."""
    import xlcells.openpyxl as real

    return real.load_workbook(HOME / logic()["workbook"]).evaluate({name: f for name, f in frames.items() if f.width})


def load_workbook(folder):
    return Workbook(folder)
