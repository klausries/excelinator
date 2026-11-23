"""What a workbook holds, for moving it elsewhere (not in openpyxl).

``wb.regions`` finds the tables (columns under a header row), the groups of single
cells (inputs and results of a model, named by their labels) and the blocks (grids of
values) of a workbook. Each region gives its data as a dataframe, takes a dataframe
back into its cells, and tells where a dataframe differs from the workbook::

    orders = wb.regions["Orders"]
    df = orders.to_polars()
    orders.write(df.filter(pl.col("qty") > 0))    # the table shrinks, formulas follow
    wb.calculate()
    wb.regions["Orders"].verify(df)                # the cells that differ

Finding, typing, writing and comparing happen in Rust; dataframes cross as Arrow.
"""
from .._needs_engine import engine as _engine
from . import _frames


class RowException:
    """A row of a column that is not as the column is: ``row`` counts from 0 in the data,
    ``coordinate`` is its cell. ``formula`` (and ``logic``) is the row's own formula;
    ``None`` for a plain value in a calculated column."""

    __slots__ = ("row", "coordinate", "formula", "logic")

    def __init__(self, row, coordinate, formula, logic):
        self.row, self.coordinate, self.formula, self.logic = row, coordinate, formula, logic

    def __repr__(self):
        return "<RowException {0} {1}>".format(self.coordinate, "value" if self.formula is None else "=" + (self.logic or self.formula))


class Column:
    """A column of a table or block. ``formula`` is the formula of its first data row when
    one formula is filled down the column, ``logic`` that formula with structured
    references (``[@qty]*[@price]``). ``exceptions`` are the rows that are not as the
    column is: with a formula of their own, or (in a calculated column) a plain value."""

    __slots__ = ("name", "letter", "dtype", "formula", "logic", "exceptions")

    def __init__(self, name, col, dtype, formula, logic, exceptions=(), first_row=0):
        from .utils import get_column_letter
        self.name, self.letter, self.dtype, self.formula, self.logic = name, get_column_letter(col + 1), dtype, formula, logic
        self.exceptions = [RowException(row - first_row, self.letter + str(row + 1), f, l) for row, f, l in exceptions]

    @property
    def calculated(self):
        return self.formula is not None

    def __repr__(self):
        rows = " ({0} exceptions)".format(len(self.exceptions)) if self.exceptions else ""
        return "<Column {0!r} {1} {2}{3}{4}>".format(self.name, self.letter, self.dtype, "" if self.formula is None else " =" + (self.logic or self.formula), rows)


class NamedCell:
    """A single cell with a name (from a defined name or the label beside it)."""

    __slots__ = ("name", "coordinate", "dtype", "formula", "logic")

    def __init__(self, name, row, col, dtype, formula, logic):
        from .utils import get_column_letter
        self.name, self.coordinate, self.dtype, self.formula, self.logic = name, get_column_letter(col + 1) + str(row + 1), dtype, formula, logic

    @property
    def calculated(self):
        return self.formula is not None

    def __repr__(self):
        return "<NamedCell {0!r} {1} {2}{3}>".format(self.name, self.coordinate, self.dtype, "" if self.formula is None else " =" + (self.logic or self.formula))


class Region:
    """A table, a group of single cells or a block of a workbook.

    ``kind`` is ``table``, ``cells`` or ``block``; ``ref`` its range as in
    ``openpyxl.worksheet.table.Table`` (``"A1:F40"``); ``sheet`` the worksheet;
    ``source`` is ``excel table`` or ``detected``. A table or block has ``columns``,
    a group of single cells has ``cells``."""

    def __init__(self, workbook, data, snapshot=None):
        self._workbook = workbook
        # The workbook as it was when the regions were found, held in Rust and shared by
        # the regions of one `wb.regions`.
        self._snapshot = snapshot
        self.name, self.kind, self.ref = data["name"], data["kind"], data["ref"]
        self.has_header, self.source = data["has_header"], data["source"]
        self.sheet = workbook._sheets[data["sheet"]]
        first_row = data.get("first_row", 0)
        self.columns = [Column(*c, first_row=first_row) for c in data["columns"]]
        self.cells = [NamedCell(*c) for c in data["cells"]]

    def _book(self):
        if self._snapshot is None:
            self._snapshot = _engine("Regions of a workbook").Book(self._workbook.to_bytes())
        return self._snapshot

    def _frame(self):
        return self._book().region_frame(self.name)

    def to_polars(self):
        """The region as a Polars DataFrame: a table or block as its columns, single
        cells as one row with a column per cell. Calculated columns hold the values
        the workbook has for them. A region shows the workbook as it was when
        ``wb.regions`` was asked for (and what was written through regions since);
        after changing cells directly, ask for ``wb.regions`` again."""
        return _frames.as_polars(self._frame())

    def to_arrow(self):
        return _frames.as_arrow(self._frame())

    def write(self, frame):
        """Put a dataframe (Polars, pyarrow) into the region's cells, columns
        matched by name. Calculated columns keep their formulas and are filled down; a
        table may get more or fewer rows, and formulas that read its whole columns
        follow. The values of formulas are stale until ``wb.calculate()``. The workbook
        is changed in place; cell objects taken from it before are not."""
        book = self._book()
        self.ref = book.write(self.name, frame)
        self._workbook._reload(book.to_bytes())
        return self

    def verify(self, frame):
        """The cells whose value in the dataframe differs from the workbook's, as a
        Polars DataFrame with the columns region,
        cell, column, row, expected (the workbook's) and actual; empty when all agree."""
        return _frames.as_polars(self._book().verify(self.name, frame))

    def __repr__(self):
        what = ", ".join(c.name for c in (self.columns or self.cells))
        return "<Region {0!r} {1} {2}!{3}: {4}>".format(self.name, self.kind, self.sheet.title, self.ref, what)


class Regions(dict):
    """The regions of a workbook (or of one sheet) by name, in sheet order."""

    def __getitem__(self, name):
        try:
            return dict.__getitem__(self, name)
        except KeyError:
            raise KeyError("no region named {0!r}; the regions are {1}".format(name, list(self))) from None

    def __repr__(self):
        return "\n".join(repr(r) for r in self.values()) or "<no regions>"


def regions_of(workbook, worksheet=None):
    book = _engine("Regions of a workbook").Book(workbook.to_bytes())
    found = Regions()
    for data in book.regions():
        region = Region(workbook, data, book)
        if worksheet is None or region.sheet is worksheet:
            found[region.name] = region
    return found


class Evaluation(dict):
    """The regions of a workbook as dataframes after a calculation, by name.
    ``not_computed`` lists what the engine could not calculate, as (reason, cells)."""

    not_computed = ()


def evaluate(workbook, frames=None):
    book = _engine("Regions of a workbook").Book(workbook.to_bytes())
    for name, frame in (frames or {}).items():
        book.write(name, frame)
    _cells, _computed, not_computed = book.calculate()
    out = Evaluation()
    for data in book.regions():
        out[data["name"]] = _frames.as_polars(book.region_frame(data["name"]))
    out.not_computed = list(not_computed)
    return out


class PolarsCode(str):
    """The source of a Polars program for a workbook. ``items`` tells for every
    calculated column and cell how it is written: (region, name, formula, how, reason)
    with ``how`` being ``polars`` (Polars' own operations), ``plan`` (the formula engine's
    translation, written out as Polars code) or ``engine`` (its values are asked for)."""

    items = ()

    def summary(self):
        lines = []
        for region, name, formula, how, reason in self.items:
            lines.append("{0}[{1}] ={2}: {3}{4}".format(region, name, formula, how, "" if reason is None else " (" + reason + ")"))
        return "\n".join(lines)


def polars_code(workbook, path, accuracy):
    if accuracy not in ("auto", "plain", "exact"):
        raise ValueError("accuracy is 'auto' (Polars where it reproduces the workbook, else the engine's translation or values), 'plain' (Polars wherever a translation exists) or 'exact' (the engine's translation or values for everything), not {0!r}".format(accuracy))
    book = _engine("Regions of a workbook").Book(workbook.to_bytes())
    if accuracy != "auto":
        source, items, _check_line, _exact = book.polars_program(path, accuracy == "exact", [])
        code = PolarsCode(source)
        code.items = [item[:5] for item in items]
        return code
    # Auto: the plain program is run item by item. An item whose statements fail, or whose
    # value is not the workbook's, takes the engine's value on the spot (so that what
    # reads it goes on from the right value) and is left to the engine in the program.
    source, items, check_line, exact_statement = book.polars_program(path, False, [])
    lines = ["" if line.startswith("wb = xo.load_workbook(") else line for line in source.splitlines()]
    scope = {"wb": workbook, "print": lambda *a, **k: None}
    engine, reasons = [], {}
    starts = [item[5] for item in items] + [check_line]
    try:
        exec(compile("\n".join(lines[:starts[0] - 1]), "<xlcells polars code>", "exec"), scope)
    except Exception:
        # The data itself cannot be loaded as the program wants it: everything exact.
        return polars_code(workbook, path, "exact")
    for k, (region, name, _formula, how, _why, line, fallback, check) in enumerate(items):
        block = "\n".join(lines[line - 1:starts[k + 1] - 1])
        if how in ("polars", "plan"):
            form = "Polars form" if how == "polars" else "translation"
            try:
                exec(compile(block, "<xlcells polars code>", "exec"), scope)
                right = book.verify(region, eval(check, scope)).shape[0] == 0
                why = "its {0} does not give the workbook's value".format(form)
            except Exception as error:
                right = False
                why = "its {0} fails: {1}: {2}".format(form, type(error).__name__, (str(error).splitlines() or [""])[0][:80])
            if right:
                continue
            engine.append((region, name))
            reasons[(region, name)] = why
        if "exact" not in scope:
            exec(exact_statement, scope)
        exec(fallback, scope)
    source, items, _check_line, _exact = book.polars_program(path, False, engine)
    code = PolarsCode(source)
    code.items = [(r, n, f, how, reasons.get((r, n), why) if how == "engine" else why) for r, n, f, how, why, *_rest in items]
    return code
