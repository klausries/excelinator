"""Coordinate helpers with the names and behaviour of openpyxl.utils.cell."""
import re

__all__ = ["get_column_letter", "column_index_from_string", "coordinate_from_string", "coordinate_to_tuple", "absolute_coordinate",
           "range_boundaries", "range_to_tuple", "rows_from_range", "cols_from_range", "get_column_interval", "quote_sheetname",
           "COL_RANGE", "ROW_RANGE", "RANGE_EXPR", "SHEET_TITLE", "CellCoordinatesException"]

COL_RANGE = "[A-Z]{1,3}:[A-Z]{1,3}:"
ROW_RANGE = r"\d+:\d+:"
RANGE_EXPR = r"""
[$]?(?P<min_col>[A-Za-z]{1,3})?
[$]?(?P<min_row>\d+)?
(:[$]?(?P<max_col>[A-Za-z]{1,3})?
[$]?(?P<max_row>\d+)?)?
"""
SHEET_TITLE = r"""
(('(?P<quoted>([^']|'')*)')|(?P<notquoted>[^'^ ^!]*))!"""
_RANGE = re.compile("^" + RANGE_EXPR + "$", re.VERBOSE)
_SHEET_RANGE = re.compile("^" + SHEET_TITLE + "(?P<cells>" + RANGE_EXPR + ")$", re.VERBOSE)
_COORD = re.compile(r"^[$]?([A-Za-z]{1,3})[$]?(\d+)$")


class CellCoordinatesException(Exception):
    """A coordinate that is not of the form A1."""


def get_column_letter(idx):
    if not 1 <= idx <= 18278:
        raise ValueError("Invalid column index {0}".format(idx))
    letters = ""
    while idx:
        idx, rem = divmod(idx - 1, 26)
        letters = chr(65 + rem) + letters
    return letters


def column_index_from_string(col):
    index = 0
    for ch in col.upper():
        if not "A" <= ch <= "Z":
            raise ValueError("{0} is not a valid column name".format(col))
        index = index * 26 + ord(ch) - 64
    if not 1 <= index <= 18278 or len(col) > 3:
        raise ValueError("{0} is not a valid column name".format(col))
    return index


def coordinate_from_string(coord_string):
    """'B12' → ('B', 12)."""
    m = _COORD.match(coord_string)
    if not m:
        raise CellCoordinatesException("Invalid cell coordinates ({0})".format(coord_string))
    column, row = m.group(1), int(m.group(2))
    if not row:
        raise CellCoordinatesException("There is no row 0 ({0})".format(coord_string))
    return column, row


def coordinate_to_tuple(coordinate):
    """'B12' → (12, 2)."""
    column, row = coordinate_from_string(coordinate)
    return row, column_index_from_string(column)


def absolute_coordinate(coord_string):
    """'B12' → '$B$12', 'A1:B2' → '$A$1:$B$2'."""
    m = _RANGE.match(coord_string)
    if not m:
        raise ValueError("{0} is not a valid coordinate range".format(coord_string))
    min_col, min_row, sep, max_col, max_row = (("$" + v) if v else "" for v in m.group("min_col", "min_row", 3, "max_col", "max_row"))
    if max_col or max_row:
        return "{0}{1}:{2}{3}".format(min_col, min_row, max_col, max_row)
    return min_col + min_row


def range_boundaries(range_string):
    """(min_col, min_row, max_col, max_row) of 'A1:B2', 'A1', 'A:C' or '2:4' (missing parts None)."""
    msg = "{0} is not a valid coordinate or range".format(range_string)
    m = _RANGE.match(range_string)
    if not m:
        raise ValueError(msg)
    min_col, min_row, sep, max_col, max_row = m.group("min_col", "min_row", 3, "max_col", "max_row")
    if sep:
        cols, rows = (min_col, max_col), (min_row, max_row)
        if not (all(cols + rows) or (all(cols) and not any(rows)) or (all(rows) and not any(cols))):
            raise ValueError(msg)
    min_col = column_index_from_string(min_col) if min_col is not None else None
    min_row = int(min_row) if min_row is not None else None
    max_col = column_index_from_string(max_col) if max_col is not None else min_col
    max_row = int(max_row) if max_row is not None else min_row
    return min_col, min_row, max_col, max_row


def range_to_tuple(range_string):
    """"'Sheet'!A1:B2" → ('Sheet', (1, 1, 2, 2)); the sheet name as written between the quotes."""
    m = _SHEET_RANGE.match(range_string)
    if m is None:
        raise ValueError("Value must be of the form sheetname!A1:E4")
    return m.group("quoted") or m.group("notquoted"), range_boundaries(m.group("cells"))


def rows_from_range(range_string):
    """Tuples of cell names, row by row."""
    min_col, min_row, max_col, max_row = range_boundaries(range_string)
    for row in range(min_row, max_row + 1):
        yield tuple("{0}{1}".format(get_column_letter(col), row) for col in range(min_col, max_col + 1))


def cols_from_range(range_string):
    """Tuples of cell names, column by column."""
    min_col, min_row, max_col, max_row = range_boundaries(range_string)
    for col in range(min_col, max_col + 1):
        yield tuple("{0}{1}".format(get_column_letter(col), row) for row in range(min_row, max_row + 1))


def get_column_interval(start, end):
    """Letters of the columns from `start` to `end` (letters or numbers), inclusive."""
    if isinstance(start, str):
        start = column_index_from_string(start)
    if isinstance(end, str):
        end = column_index_from_string(end)
    return [get_column_letter(i) for i in range(start, end + 1)]


def quote_sheetname(sheetname):
    """A sheet name as a formula writes it: in quotes, inner quotes doubled."""
    return "'{0}'".format(sheetname.replace("'", "''"))
