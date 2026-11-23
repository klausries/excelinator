"""xlcells for Polars.

    import xlcells.polars as xp
    cells = xp.read_cells("in.xlsx")           # one row per cell: the base table
    df = xp.read_excel("in.xlsx", sheet_name="Daten")
    xp.write_excel(df, "out.xlsx")

`read_cells` is the lossless view: sheet, row, col, kind, num, text, formula, is_date,
num_format. `read_excel` turns one sheet into a table with one data type per column:
whole numbers Int64, other numbers Float64, booleans Boolean, date serials Date or
Datetime, text String; a column with mixed kinds becomes String; empty cells and
errors are null.
"""
import polars as _pl

from . import _xlcells
from ._needs_engine import engine
from .openpyxl import _to_serial

__all__ = ["read_cells", "read_excel", "write_excel"]


def _bytes(source):
    if isinstance(source, (bytes, bytearray)):
        return bytes(source)
    if hasattr(source, "read"):
        return source.read()
    with open(source, "rb") as f:
        return f.read()


def read_cells(source):
    """The workbook as one long table, a row per cell; rows and columns are 0-based."""
    t = _xlcells.read(_bytes(source))
    names, formats = t["sheet_names"], t["num_format"]
    kind = t["kind"]
    return _pl.DataFrame(
        {
            "sheet": t["sheet"],
            "sheet_name": [names[s] for s in t["sheet"]],
            "row": t["row"],
            "col": t["col"],
            "kind": kind,
            "num": [n if k in (1, 3) else None for n, k in zip(t["num"], kind)],
            "text": t["text"],
            "formula": t["formula"],
            "is_date": t["is_date"],
            "num_format": [formats[s] for s in t["style"]],
        },
        schema={"sheet": _pl.UInt32, "sheet_name": _pl.String, "row": _pl.UInt32, "col": _pl.UInt32, "kind": _pl.UInt8,
                "num": _pl.Float64, "text": _pl.String, "formula": _pl.String, "is_date": _pl.Boolean, "num_format": _pl.String},
    )


def read_excel(source, *, sheet_id=None, sheet_name=None, has_header=True):
    """One sheet as a DataFrame. `sheet_id` counts from 1 as in Polars; default is the
    first sheet. The table is the block of non-empty cells; with `has_header` its
    first row names the columns. The frame is built in Rust and handed over as Arrow."""
    book = engine("xlcells.polars.read_excel").Book(_bytes(source))
    names = book.sheet_names
    if sheet_name is not None:
        if sheet_name not in names:
            raise ValueError("no sheet named {0!r}; sheets are {1}".format(sheet_name, names))
        index = names.index(sheet_name)
    else:
        index = (sheet_id or 1) - 1
        if not 0 <= index < len(names):
            raise ValueError("no sheet with id {0}; the workbook has {1} sheets".format(sheet_id, len(names)))
    return _pl.DataFrame(book.frame(index, None, has_header))


def write_excel(df, target=None, *, worksheet="Sheet1", include_header=True):
    """Write a DataFrame as one sheet. Returns the bytes when `target` is None.
    Dates and times get a matching number format; nulls stay empty."""
    w = _xlcells.Writer()
    w.add_sheet(worksheet)
    styles = {}

    def style(fmt, bold=False):
        key = (fmt, bold)
        if key not in styles:
            styles[key] = w.add_style(fmt, bold, False)
        return styles[key]

    row0 = 0
    if include_header:
        for c, name in enumerate(df.columns):
            w.write(0, 0, c, 2, 0.0, name, None, style("General", True))
        row0 = 1
    for c, series in enumerate(df.get_columns()):
        dtype = series.dtype
        if dtype == _pl.Boolean:
            for r, v in enumerate(series):
                if v is not None:
                    w.write(0, row0 + r, c, 3, float(v), None, None, 0)
        elif dtype.is_numeric():
            # A whole number of a float column is spelled `1.0`: the column reads back as floats.
            is_float = dtype.is_float()
            for r, v in enumerate(series):
                if v is not None and v == v and abs(v) != float("inf"):
                    w.write(0, row0 + r, c, 1, float(v), None, None, 0, None, is_float)
        elif dtype in (_pl.Date, _pl.Time) or isinstance(dtype, (_pl.Datetime, _pl.Duration)):
            fmt = {"Date": "yyyy-mm-dd", "Time": "h:mm:ss", "Datetime": "yyyy-mm-dd h:mm:ss", "Duration": "[hh]:mm:ss"}[type(dtype).__name__ if not isinstance(dtype, type) else dtype.__name__]
            st = style(fmt)
            for r, v in enumerate(series):
                if v is not None:
                    w.write(0, row0 + r, c, 1, _to_serial(v, False), None, None, st)
        else:
            for r, v in enumerate(series.cast(_pl.String, strict=False)):
                if v is not None and v != "":
                    w.write(0, row0 + r, c, 2, 0.0, v, None, 0)
    data = w.to_bytes()
    if target is None:
        return data
    if hasattr(target, "write"):
        target.write(data)
    else:
        with open(target, "wb") as f:
            f.write(data)
    return None
