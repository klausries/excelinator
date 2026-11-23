"""What a cell looks like, attribute by attribute, and where each attribute comes from.

Not in openpyxl. Every attribute of the look has its value resolved as Excel shows it
(colours as ``RRGGBB``, theme colours and tints worked out) and a source: the workbook
default, a named cell style, the cell's own format, the row's or column's format, a table
style or a conditional format. See ``xl_look`` (Rust) for the rules.
"""

ATTRIBUTES = (
    "font_name", "font_size", "bold", "italic", "underline", "strike", "font_color", "font_script",
    "fill_pattern", "fill_color", "fill_background",
    "border_left", "border_left_color", "border_right", "border_right_color", "border_top", "border_top_color",
    "border_bottom", "border_bottom_color", "border_diagonal", "border_diagonal_color", "diagonal_up", "diagonal_down",
    "horizontal", "vertical", "wrap", "rotation", "indent", "shrink", "num_format", "locked", "hidden",
)
from .._needs_engine import engine
from .utils import coordinate_to_tuple, get_column_letter, range_boundaries


class Source:
    """Where an attribute's value comes from.

    ``kind`` is ``default``, ``style``, ``cell``, ``row``, ``column``, ``table`` or
    ``conditional``; ``name`` the style's name, the row number, the column letter, the
    table's name or the conditional format's range; ``detail`` for a table its style and
    element (``TableStyleMedium9``, ``headerRow``), for a conditional format the rule's
    priority and type. ``certain`` is false where a conditional format whose rule could
    not be decided might set the attribute. ``spec`` is a colour as the file states it
    (``theme 4 tint 0.6``)."""

    __slots__ = ("kind", "name", "detail", "certain", "spec")

    def __init__(self, kind, parts, certain, spec):
        self.kind = kind
        self.name = parts[0] if parts else None
        self.detail = tuple(parts[1:])
        self.certain = certain
        self.spec = spec

    def __eq__(self, other):
        if isinstance(other, str):
            return self.kind == other
        return isinstance(other, Source) and (self.kind, self.name, self.detail) == (other.kind, other.name, other.detail)

    def __hash__(self):
        return hash((self.kind, self.name, self.detail))

    def __repr__(self):
        extra = "" if self.name is None else " " + " ".join((self.name,) + self.detail)
        return "<Source {0}{1}{2}>".format(self.kind, extra, "" if self.certain else " (uncertain)")


class DataBar:
    __slots__ = ("color", "fraction", "range")

    def __init__(self, color, fraction, range):
        self.color, self.fraction, self.range = color, fraction, range

    def __repr__(self):
        return "<DataBar {0} {1:.3f}>".format(self.color, self.fraction)


class Icon:
    __slots__ = ("set", "index", "range")

    def __init__(self, set, index, range):
        self.set, self.index, self.range = set, index, range

    def __repr__(self):
        return "<Icon {0} {1}>".format(self.set, self.index)


class Look:
    """The look of one cell: ``look.fill_color`` is the colour, ``look.fill_color_source``
    (or ``look.source("fill_color")``) where it comes from."""

    def __init__(self, data):
        self._values = {}
        self._sources = {}
        for key in ATTRIBUTES:
            value, kind, parts, certain, spec = data[key]
            self._values[key] = value
            self._sources[key] = Source(kind, parts, certain, spec)
        self.data_bar = DataBar(*data["data_bar"]) if data["data_bar"] else None
        self.icon = Icon(*data["icon"]) if data["icon"] else None
        self.undecided = [tuple(u) for u in data["undecided"]]

    def __getattr__(self, name):
        values = self.__dict__.get("_values", {})
        if name in values:
            return values[name]
        if name.endswith("_source") and name[:-7] in values:
            return self.__dict__["_sources"][name[:-7]]
        raise AttributeError(name)

    def source(self, attribute):
        return self._sources[attribute]

    def items(self):
        """(attribute, value, source) for every attribute."""
        return [(k, self._values[k], self._sources[k]) for k in ATTRIBUTES]

    def __dir__(self):
        return list(ATTRIBUTES) + [k + "_source" for k in ATTRIBUTES] + ["data_bar", "icon", "undecided", "source", "items"]

    def __repr__(self):
        return "<Look font {0} {1}{2}, fill {3}>".format(self.font_name, self.font_size, " bold" if self.bold else "", self.fill_color)


class Looker:
    """The looks of a workbook as it is when this is made (``wb.looker()``).

    The workbook is written and read once, and what is worked out for one cell serves the
    next: for looks asked one after the other (``looker.look(cell)`` in a loop) this is
    what keeps each from costing a whole workbook. Later changes to the workbook are not
    seen; make a new one after them."""

    def __init__(self, workbook):
        self._workbook = workbook
        self._inner = engine("The look of a cell").Looker(workbook.to_bytes())

    def look(self, cell):
        """The look of a cell (``ws["B3"]``)."""
        return self.looks(cell.parent, [cell.coordinate])[cell.coordinate]

    def looks(self, sheet, cells):
        """The looks of cells of a sheet by coordinate: a range (``"A1:C10"``; whole
        columns ``"A:C"`` and rows ``"2:4"`` as far as the sheet has cells) or
        coordinates."""
        if isinstance(cells, str):
            min_col, min_row, max_col, max_row = range_boundaries(cells)
            if min_row is None:
                min_row, max_row = 1, sheet.max_row
            if min_col is None:
                min_col, max_col = 1, sheet.max_column
            coords = [get_column_letter(c) + str(r) for r in range(min_row, max_row + 1) for c in range(min_col, max_col + 1)]
        else:
            coords = list(cells)
        index = self._workbook._sheets.index(sheet)
        at = []
        for c in coords:
            row, col = coordinate_to_tuple(c)
            at.append((index, row - 1, col - 1))
        return {c: Look(d) for c, d in zip(coords, self._inner.looks(at))}
