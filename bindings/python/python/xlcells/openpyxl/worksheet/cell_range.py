"""Rectangular cell ranges and lists of them (`CellRange("A1:C3")`, `MultiCellRange("A1 B2:C4")`)."""
from ..utils.cell import get_column_letter, quote_sheetname, range_boundaries, range_to_tuple

__all__ = ["CellRange", "MultiCellRange"]


class CellRange:
    """A rectangle of cells, 1-based and inclusive, optionally on a named sheet."""

    def __init__(self, range_string=None, min_col=None, min_row=None, max_col=None, max_row=None, title=None):
        if range_string is not None:
            if "!" in range_string:
                title, (min_col, min_row, max_col, max_row) = range_to_tuple(range_string)
            else:
                min_col, min_row, max_col, max_row = range_boundaries(range_string)
        self.min_col, self.min_row, self.max_col, self.max_row, self.title = min_col, min_row, max_col, max_row, title
        if min_col > max_col:
            raise ValueError("{0} must be greater than {1}".format(max_col, min_col))
        if min_row > max_row:
            raise ValueError("{0} must be greater than {1}".format(max_row, min_row))

    # --- what it covers ----------------------------------------------------------
    @property
    def bounds(self):
        return self.min_col, self.min_row, self.max_col, self.max_row

    @property
    def coord(self):
        first = "{0}{1}".format(get_column_letter(self.min_col), self.min_row)
        if (self.min_col, self.min_row) == (self.max_col, self.max_row):
            return first
        return "{0}:{1}{2}".format(first, get_column_letter(self.max_col), self.max_row)

    @property
    def rows(self):
        for row in range(self.min_row, self.max_row + 1):
            yield [(row, col) for col in range(self.min_col, self.max_col + 1)]

    @property
    def cols(self):
        for col in range(self.min_col, self.max_col + 1):
            yield [(row, col) for row in range(self.min_row, self.max_row + 1)]

    @property
    def cells(self):
        for row in range(self.min_row, self.max_row + 1):
            for col in range(self.min_col, self.max_col + 1):
                yield row, col

    @property
    def size(self):
        return {"columns": self.max_col - self.min_col + 1, "rows": self.max_row - self.min_row + 1}

    @property
    def top(self):
        return [(self.min_row, col) for col in range(self.min_col, self.max_col + 1)]

    @property
    def bottom(self):
        return [(self.max_row, col) for col in range(self.min_col, self.max_col + 1)]

    @property
    def left(self):
        return [(row, self.min_col) for row in range(self.min_row, self.max_row + 1)]

    @property
    def right(self):
        return [(row, self.max_col) for row in range(self.min_row, self.max_row + 1)]

    # --- changing it ---------------------------------------------------------------
    def shift(self, col_shift=0, row_shift=0):
        if self.min_col + col_shift <= 0 or self.min_row + row_shift <= 0:
            raise ValueError("Invalid shift value: col_shift={0}, row_shift={1}".format(col_shift, row_shift))
        self.min_col += col_shift
        self.max_col += col_shift
        self.min_row += row_shift
        self.max_row += row_shift

    def expand(self, right=0, down=0, left=0, up=0):
        self.min_col -= left
        self.min_row -= up
        self.max_col += right
        self.max_row += down

    def shrink(self, right=0, bottom=0, left=0, top=0):
        self.min_col += left
        self.min_row += top
        self.max_col -= right
        self.max_row -= bottom

    # --- as sets of cells ------------------------------------------------------------
    def _check_title(self, other):
        if not isinstance(other, CellRange):
            raise TypeError(repr(type(other)))
        if other.title not in (None, self.title) and self.title is not None:
            raise ValueError("Cannot work with ranges from different worksheets")

    def isdisjoint(self, other):
        self._check_title(other)
        return self.max_col < other.min_col or other.max_col < self.min_col or self.max_row < other.min_row or other.max_row < self.min_row

    def issubset(self, other):
        self._check_title(other)
        return other._covers(self)

    def issuperset(self, other):
        self._check_title(other)
        return self._covers(other)

    def _covers(self, other):
        return self.min_row <= other.min_row <= other.max_row <= self.max_row and self.min_col <= other.min_col <= other.max_col <= self.max_col

    def union(self, other):
        self._check_title(other)
        return CellRange(min_col=min(self.min_col, other.min_col), min_row=min(self.min_row, other.min_row),
                         max_col=max(self.max_col, other.max_col), max_row=max(self.max_row, other.max_row), title=self.title)

    def intersection(self, other):
        if self.isdisjoint(other):
            raise ValueError("Range {0} doesn't intersect {1}".format(self, other))
        return CellRange(min_col=max(self.min_col, other.min_col), min_row=max(self.min_row, other.min_row),
                         max_col=min(self.max_col, other.max_col), max_row=min(self.max_row, other.max_row))

    __and__ = intersection
    __or__ = union
    __le__ = issubset
    __ge__ = issuperset

    def __lt__(self, other):
        return self.issubset(other) and self != other

    def __gt__(self, other):
        return self.issuperset(other) and self != other

    def __contains__(self, coord):
        return self._covers(CellRange(coord))

    def __eq__(self, other):
        if isinstance(other, str):
            other = CellRange(other)
        return isinstance(other, CellRange) and self.bounds == other.bounds and (self.title or None) == (other.title or None)

    def __ne__(self, other):
        return not self == other

    def __hash__(self):
        return hash((self.min_row, self.min_col, self.max_row, self.max_col))

    def __iter__(self):
        for name in ("min_col", "min_row", "max_col", "max_row"):
            yield name, getattr(self, name)

    def __copy__(self):
        return CellRange(min_col=self.min_col, min_row=self.min_row, max_col=self.max_col, max_row=self.max_row, title=self.title)

    def __str__(self):
        return "{0}!{1}".format(quote_sheetname(self.title), self.coord) if self.title else self.coord

    def __repr__(self):
        if self.title:
            return "<{0} {1!r}!{2}>".format(type(self).__name__, self.title, self.coord)
        return "<{0} {1}>".format(type(self).__name__, self.coord)


class MultiCellRange:
    """Several ranges, as in `sqref="A1:B2 D4"`."""

    def __init__(self, ranges=set()):
        if isinstance(ranges, str):
            ranges = [CellRange(r) for r in ranges.split()]
        self.ranges = set(ranges)

    def add(self, coord):
        if coord not in self:
            self.ranges.add(coord if isinstance(coord, CellRange) else CellRange(coord))

    def __iadd__(self, coord):
        self.add(coord)
        return self

    def remove(self, coord):
        if not isinstance(coord, CellRange):
            coord = CellRange(coord)
        self.ranges.remove(coord)

    def __contains__(self, coord):
        if isinstance(coord, str):
            coord = CellRange(coord)
        return any(coord <= r for r in self.ranges)

    def sorted(self):
        return sorted(self.ranges, key=lambda r: (r.min_col, r.min_row, r.max_col, r.max_row))

    def __iter__(self):
        return iter(self.sorted())

    def __len__(self):
        return len(self.ranges)

    def __bool__(self):
        return bool(self.ranges)

    def __eq__(self, other):
        if isinstance(other, str):
            other = MultiCellRange(other)
        return isinstance(other, MultiCellRange) and self.ranges == other.ranges

    def __ne__(self, other):
        return not self == other

    def __copy__(self):
        return MultiCellRange({r.__copy__() for r in self.ranges})

    def __str__(self):
        return " ".join(str(r) for r in self.sorted())

    def __repr__(self):
        return "<MultiCellRange [{0}]>".format(str(self))
