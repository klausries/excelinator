"""Column and row dimensions and the sheet's default sizes (`ws.sheet_format`)."""
from .._serial import Element

__all__ = ["SheetFormatProperties", "DEFAULT_COLUMN_WIDTH", "ColumnDimension", "RowDimension", "DimensionHolder", "Dimension", "SheetDimension"]

DEFAULT_COLUMN_WIDTH = 13


class SheetFormatProperties(Element):
    tagname = "sheetFormatPr"
    _attrs = (
        ("baseColWidth", int, 8), ("defaultColWidth", float, None), ("defaultRowHeight", float, 15.0), ("customHeight", bool, None),
        ("zeroHeight", bool, None), ("thickTop", bool, None), ("thickBottom", bool, None), ("outlineLevelRow", int, None),
        ("outlineLevelCol", int, None),
    )


class SheetDimension(Element):
    tagname = "dimension"
    _attrs = (("ref", str, None),)

    @property
    def boundaries(self):
        from ..utils import range_boundaries
        return range_boundaries(self.ref)


from ..utils import column_index_from_string, get_column_letter  # noqa: E402


def _base(worksheet):
    """The format of rows and columns without one: the workbook's first fonts, fills and
    borders with the General number format (openpyxl's empty style)."""
    from .. import _PLAIN, FORMAT_GENERAL
    defaults = getattr(getattr(worksheet, "parent", None), "_defaults", _PLAIN)
    return (FORMAT_GENERAL,) + tuple(defaults[1:])


class Dimension:
    """A row or column range: hidden, outline level, collapsed, and a format for its empty
    cells (`font`, `fill`, `border`, `alignment`, `protection`, `number_format`)."""

    _fields = ("hidden", "outlineLevel", "collapsed")

    def __init__(self, index, hidden, outlineLevel, collapsed, worksheet, visible=True, style=None):
        self.parent = worksheet
        self.index = index
        self.hidden = bool(hidden)
        self.outlineLevel = outlineLevel
        self.collapsed = bool(collapsed)
        (self.number_format, self.font, self.fill, self.border, self.alignment, self.protection, self._style_name, self.quotePrefix,
         self.pivotButton) = style if style is not None else _base(worksheet)
        self._loaded = None  # (state right after loading, attributes as read)

    outline_level = property(lambda self: self.outlineLevel, lambda self, v: setattr(self, "outlineLevel", v))

    def _style_key(self):
        from .. import FORMAT_GENERAL
        return (self.number_format or FORMAT_GENERAL, self.font, self.fill, self.border, self.alignment, self.protection, self._style_name,
                bool(self.quotePrefix), bool(self.pivotButton))

    @property
    def has_style(self):
        """Whether the format differs from the workbook's default. (openpyxl looks at the
        format's numbers in the file instead: a copy of the default under another number
        counts there.)"""
        return self._style_key() != _base(self.parent)

    @property
    def style(self):
        """The name of the named style the format is based on."""
        if self._style_name:
            return self._style_name
        return "Normal"

    @style.setter
    def style(self, value):
        wb = self.parent.parent
        named = wb._named_style(value)
        self.number_format, self.font, self.fill, self.border, self.alignment, self.protection = (
            named.number_format, named.font, named.fill, named.border, named.alignment, named.protection)
        self._style_name = None if named.name == "Normal" else named.name

    @property
    def visible(self):
        return not self.hidden

    def __iter__(self):
        for key in self._fields:
            value = getattr(self, key, None)
            if value:
                yield key, str(value) if not isinstance(value, bool) else "1"

    def __repr__(self):
        return "<{0} Instance, Attributes={1}>".format(type(self).__name__, dict(self))

    def __copy__(self):
        new = type(self).__new__(type(self))
        new.__dict__.update(self.__dict__)
        return new


class RowDimension(Dimension):
    """A row (`ws.row_dimensions[5]`): height in points (`ht`, alias `height`), thick top
    and bottom borders, and what all dimensions have."""

    _fields = Dimension._fields + ("ht", "customFormat", "customHeight", "s", "thickBot", "thickTop")

    def __init__(self, worksheet, index=0, ht=None, customHeight=None, s=None, customFormat=None, hidden=False, outlineLevel=0, outline_level=None,
                 collapsed=False, visible=None, height=None, r=None, spans=None, thickBot=None, thickTop=None, **kw):
        if r is not None:
            index = r
        if height is not None:
            ht = height
        if visible is not None:
            hidden = not visible
        if outline_level is not None:
            outlineLevel = outline_level
        self.ht = None if ht is None else float(ht)
        self.thickBot, self.thickTop = thickBot, thickTop
        Dimension.__init__(self, index, hidden, outlineLevel, collapsed, worksheet, style=s)

    r = property(lambda self: self.index, lambda self, v: setattr(self, "index", v))
    height = property(lambda self: self.ht, lambda self, v: setattr(self, "ht", None if v is None else float(v)))
    customFormat = property(lambda self: self.has_style)
    customHeight = property(lambda self: self.ht is not None)

    @property
    def s(self):
        return 1 if self.has_style else None

    def _state(self):
        return (self.ht, self.hidden, self.outlineLevel or 0, self.collapsed, self._style_key(), bool(self.thickTop), bool(self.thickBot))

    def _is_plain(self):
        return self.ht is None and not self.hidden and not self.outlineLevel and not self.collapsed and not self.has_style and not self.thickTop \
            and not self.thickBot


class ColumnDimension(Dimension):
    """A column or range of columns (`ws.column_dimensions["B"]`; `min` and `max` give
    the range, from 1): width in characters, best fit (alias `auto_size`), and what all
    dimensions have. Without a width set it reads as 13, as in openpyxl, but none is
    written."""

    _fields = Dimension._fields + ("width", "bestFit", "customWidth", "style", "min", "max")

    def __init__(self, worksheet, index="A", width=None, bestFit=False, hidden=False, outlineLevel=0, outline_level=None, collapsed=False,
                 style=None, min=None, max=None, customWidth=False, visible=None, auto_size=None):
        self._width = None if width is None else float(width)
        self.min, self.max = min, max
        if visible is not None:
            hidden = not visible
        if auto_size is not None:
            bestFit = auto_size
        self.bestFit = bool(bestFit)
        if outline_level is not None:
            outlineLevel = outline_level
        Dimension.__init__(self, index, hidden, outlineLevel, collapsed, worksheet, style=style)

    @property
    def width(self):
        return DEFAULT_COLUMN_WIDTH if self._width is None else self._width

    @width.setter
    def width(self, value):
        self._width = None if value is None else float(value)

    auto_size = property(lambda self: self.bestFit, lambda self, v: setattr(self, "bestFit", bool(v)))

    @property
    def customWidth(self):
        return self._width is not None and bool(self._width)

    def reindex(self):
        """Set the range to the column of `index` if it has none."""
        if not all([self.min, self.max]):
            self.min = self.max = column_index_from_string(self.index)

    @property
    def range(self):
        return "{0}:{1}".format(get_column_letter(self.min), get_column_letter(self.max))

    def _state(self):
        return (self._width, self.bestFit, self.hidden, self.outlineLevel or 0, self.collapsed, self._style_key(), self.min, self.max)

    def _is_plain(self):
        return self._width is None and not self.bestFit and not self.hidden and not self.outlineLevel and not self.collapsed and not self.has_style


class DimensionHolder(dict):
    """The row or column dimensions of a sheet by row number or column letter; a missing
    one is made on first use. `group` puts rows or columns into an outline group."""

    def __init__(self, worksheet, reference="index", default_factory=None):
        dict.__init__(self)
        self.worksheet = worksheet
        self.max_outline = None
        self.default_factory = default_factory

    def __missing__(self, key):
        value = self.default_factory(self.worksheet, index=key)
        self[key] = value
        return value

    def group(self, start, end=None, outline_level=1, hidden=False):
        """Group the rows or columns `start` to `end` at `outline_level`."""
        if end is None:
            end = start
        if self.default_factory is ColumnDimension:
            new_dim = self[start]
            new_dim.outline_level = outline_level
            new_dim.hidden = hidden
            first, last = column_index_from_string(start), column_index_from_string(end)
            for index in range(first + 1, last + 1):
                self.pop(get_column_letter(index), None)
            new_dim.min, new_dim.max = first, last
        else:
            for row in range(start, end + 1):
                dim = self.worksheet.row_dimensions[row]
                dim.outline_level = outline_level
                dim.hidden = hidden


Dimension.__module__ = ColumnDimension.__module__
