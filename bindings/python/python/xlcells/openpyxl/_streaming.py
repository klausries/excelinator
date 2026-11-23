"""Read-only and write-only workbooks, as openpyxl's `load_workbook(read_only=True)` and
`Workbook(write_only=True)`.

The module holds a whole workbook in memory as columns in either case (compact enough
for large files); these classes give the behaviour of openpyxl's streaming worksheets:
read-only cells that refuse new values and fill gaps with `EMPTY_CELL`, and write-only
sheets that take whole rows with `append` and can be saved once."""
from .utils.cell import get_column_letter
from .utils.exceptions import ReadOnlyWorkbookException, WorkbookAlreadySaved

__all__ = ["ReadOnlyCell", "EmptyCell", "EMPTY_CELL", "WriteOnlyCell"]


class ReadOnlyCell:
    """A cell of a read-only sheet: value, type and format, nothing to change."""

    __slots__ = ("parent", "row", "column", "_value", "data_type", "_style_id", "_cell")

    def __init__(self, sheet, row, column, value, data_type="n", style_id=0, _cell=None):
        self.parent = sheet
        self._value = None
        self.row = row
        self.column = column
        self.data_type = data_type
        self.value = value
        self._style_id = style_id
        self._cell = _cell

    def __eq__(self, other):
        return all(getattr(self, a) == getattr(other, a, None) for a in ("parent", "row", "column", "_value", "data_type", "_style_id")) or None

    def __ne__(self, other):
        return not self.__eq__(other)

    def __repr__(self):
        return "<ReadOnlyCell {0!r}.{1}>".format(self.parent.title, self.coordinate)

    @property
    def coordinate(self):
        return "{0}{1}".format(get_column_letter(self.column), self.row)

    @property
    def column_letter(self):
        return get_column_letter(self.column)

    @property
    def has_style(self):
        return self._style_id != 0

    number_format = property(lambda self: self._cell.number_format)
    font = property(lambda self: self._cell.font)
    fill = property(lambda self: self._cell.fill)
    border = property(lambda self: self._cell.border)
    alignment = property(lambda self: self._cell.alignment)
    protection = property(lambda self: self._cell.protection)

    @property
    def is_date(self):
        from . import _is_date_format
        return self.data_type == "d" or (self.data_type == "n" and _is_date_format(self.number_format))

    @property
    def internal_value(self):
        return self._value

    @property
    def value(self):
        return self._value

    @value.setter
    def value(self, value):
        if self._value is not None:
            raise AttributeError("Cell is read only")
        self._value = value


class EmptyCell:
    """A cell a read-only sheet does not have."""

    __slots__ = ()
    value = None
    is_date = False
    font = None
    border = None
    fill = None
    number_format = None
    alignment = None
    data_type = "n"

    def __repr__(self):
        return "<EmptyCell>"


EMPTY_CELL = EmptyCell()


def _readonly(ws, cell):
    raw = cell._raw
    return ReadOnlyCell(ws, cell.row, cell.column, cell.value, cell.data_type, raw[4] if raw is not None else 0, _cell=cell)


def make_read_only(ws):
    """Turn a loaded worksheet into a read-only one."""
    from . import ReadOnlyWorksheet
    ws.__class__ = ReadOnlyWorksheet
    ws._ro_max = None
    return ws


class _ReadOnlyMixin:
    """Mixed into a loaded worksheet: cells and rows come as `ReadOnlyCell` (gaps as
    `EMPTY_CELL`, or None with `values_only`), and nothing can be set."""

    # The extent as the file states it (`<dimension>`), as openpyxl reads it; without one
    # the sheet is unsized until calculate_dimension(force=True).
    def _extent(self):
        if self._ro_max is False:
            return None
        if self._ro_max is not None:
            return self._ro_max
        return getattr(self, "_file_dimension", None)

    max_row = property(lambda self: self._extent()[3] if self._extent() else None)
    max_column = property(lambda self: self._extent()[2] if self._extent() else None)
    min_row = property(lambda self: self._extent()[1] if self._extent() else 1)
    min_column = property(lambda self: self._extent()[0] if self._extent() else 1)

    def calculate_dimension(self, force=False):
        if not all([self.max_column, self.max_row]):
            if not force:
                raise ValueError("Worksheet is unsized, use calculate_dimension(force=True)")
            self._ro_max = (min((c for _, c in self._cells), default=1), min((r for r, _ in self._cells), default=1),
                            max((c for _, c in self._cells), default=1), max((r for r, _ in self._cells), default=1))
        return "{0}{1}:{2}{3}".format(get_column_letter(self.min_column), self.min_row, get_column_letter(self.max_column), self.max_row)

    def reset_dimensions(self):
        """Forget the extent: rows then end at their last cell."""
        self._ro_max = False

    def cell(self, row, column, value=None):
        if row < 1 or column < 1:
            raise ValueError("Row or column values must be at least 1")
        found = self._cells.get((row, column))
        cell = _readonly(self, found) if found is not None and found._raw is not None else EMPTY_CELL
        if value is not None:
            cell.value = value
        return cell

    def __setitem__(self, key, value):
        raise TypeError("'ReadOnlyWorksheet' object does not support item assignment")

    def iter_rows(self, min_row=None, max_row=None, min_col=None, max_col=None, values_only=False):
        filler = None if values_only else EMPTY_CELL
        min_col, min_row = min_col or 1, min_row or 1
        max_col = max_col or self.max_column
        max_row = max_row or self.max_row
        by_row = {}
        for (r, c), cell in self._cells.items():
            if cell._raw is not None:  # cells of the file (not made for a comment or a merged range)
                by_row.setdefault(r, []).append((c, cell))
        # As openpyxl: rows end with the last row the file has (with cells or attributes).
        # (Trailing rows of empty cells without format, which the module does not keep, are
        # not given.)
        file_last = max(list(by_row) + [int(k) for k in self.row_dimensions], default=0)
        last = min(max_row, file_last) if max_row is not None else max(by_row, default=0)
        for r in range(min_row, last + 1):
            cells = dict(by_row.get(r, ()))
            if max_col is None:
                if not cells:
                    yield ()
                    continue
                width = max(cells) + 1 - min_col
            else:
                width = max_col + 1 - min_col
            out = []
            for c in range(min_col, min_col + width):
                cell = cells.get(c)
                if cell is None:
                    out.append(filler)
                else:
                    out.append(cell.value if values_only else _readonly(self, cell))
            yield tuple(out)

    @property
    def rows(self):
        return self.iter_rows()

    @property
    def values(self):
        return self.iter_rows(values_only=True)

    def __iter__(self):
        return self.iter_rows()

    def append(self, iterable):
        raise AttributeError("'ReadOnlyWorksheet' object has no attribute 'append'")


class _WriteOnlyMixin:
    """Mixed into a new worksheet: rows go in with `append` only, and the sheet is saved
    once."""

    def __getattribute__(self, name):
        if name in ("cell", "iter_rows", "iter_cols", "rows", "columns", "values", "insert_rows", "delete_rows", "insert_cols", "delete_cols",
                    "move_range", "merge_cells", "unmerge_cells"):
            raise AttributeError("'WriteOnlyWorksheet' object has no attribute {0!r}".format(name))
        return object.__getattribute__(self, name)

    def __getitem__(self, key):
        raise TypeError("'WriteOnlyWorksheet' object is not subscriptable")

    def __setitem__(self, key, value):
        raise TypeError("'WriteOnlyWorksheet' object does not support item assignment")

    @property
    def closed(self):
        return getattr(self, "_wo_saved", False)

    def append(self, row):
        """Add a row: a list, tuple, range or generator of values or cells
        (`WriteOnlyCell` for a value with format, comment or link)."""
        import inspect
        from . import Cell, Worksheet
        if self.closed:
            raise WorkbookAlreadySaved("Workbook has already been saved and cannot be modified or saved anymore.")
        if not inspect.isgenerator(row) and not isinstance(row, (list, tuple, range)):
            raise TypeError("Value must be a list, tuple, range or a generator Supplied value is {0}".format(type(row)))
        index = self._current_row + 1
        get = Worksheet.cell.__get__(self)
        for column, value in enumerate(row, 1):
            if value is None:
                continue
            target = get(index, column)
            if isinstance(value, Cell):
                target.value = value.value
                for name in ("number_format", "font", "fill", "border", "alignment", "protection", "_style_name", "quotePrefix", "pivotButton"):
                    setattr(target, name, getattr(value, name))
                target.comment = value.comment
                if value.hyperlink is not None:
                    target.hyperlink = value.hyperlink
            else:
                target.value = value
        self._current_row = index

    def close(self):
        if self.closed:
            raise WorkbookAlreadySaved("Workbook has already been saved and cannot be modified or saved anymore.")
        self._wo_saved = True


def make_write_only(ws):
    from . import WriteOnlyWorksheet
    ws.__class__ = WriteOnlyWorksheet
    return ws


def WriteOnlyCell(ws=None, value=None):
    """A cell for `append` on a write-only sheet, with format, comment or link."""
    from . import Cell
    return Cell(worksheet=ws, row=1, column=1, value=value)


def check_writable(wb):
    if wb.read_only:
        raise ReadOnlyWorkbookException("Workbook is read-only")
