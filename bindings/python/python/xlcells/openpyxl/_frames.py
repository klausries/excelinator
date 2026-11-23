"""Tables of a workbook as dataframes (not in openpyxl).

The columns are typed and the table is built in Rust; it reaches Polars or
pyarrow through the Arrow stream interface, not through Python objects.
"""
from .._needs_engine import engine
from .utils import range_boundaries


def area_of(book, sheet_index, cell_range):
    """A range (``"A1:F40"``, whole columns ``"A:C"``, whole rows ``"2:9"``) as the
    0-based area of the sheet; ``None`` for the used block."""
    if cell_range is None:
        return None
    min_col, min_row, max_col, max_row = range_boundaries(cell_range)
    if min_row is None or min_col is None:
        used = book.used_area(sheet_index)
        if used is None:
            return (1, 1, 0, 0)  # nothing: an empty frame
        if min_row is None:
            min_row, max_row = used[0] + 1, used[2] + 1
        if min_col is None:
            min_col, max_col = used[1] + 1, used[3] + 1
    return (min_row - 1, min_col - 1, max_row - 1, max_col - 1)


def frame_of(book, sheet_index, cell_range=None, header=True):
    return book.frame(sheet_index, area_of(book, sheet_index, cell_range), header)


def sheet_frame(worksheet, cell_range=None, header=True):
    """The frame of a sheet of the openpyxl facade, as the workbook is now."""
    workbook = worksheet.parent
    book = engine("A dataframe of a sheet").Book(workbook.to_bytes())
    return frame_of(book, workbook._sheets.index(worksheet), cell_range, header)


def as_polars(frame):
    try:
        import polars
    except ImportError:
        raise ImportError("to_polars needs the polars package; xlcells-engine installs it (pip install xlcells-engine)") from None
    return polars.DataFrame(frame)


def as_arrow(frame):
    try:
        import pyarrow
    except ImportError:
        raise ImportError("to_arrow needs the pyarrow package (pip install pyarrow)") from None
    return pyarrow.table(frame)
