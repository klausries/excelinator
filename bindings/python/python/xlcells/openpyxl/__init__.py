"""The interface of openpyxl on top of xlcells.

    from xlcells.openpyxl import Workbook, load_workbook
    wb = load_workbook("in.xlsx")
    ws = wb.active
    ws["A1"] = 42
    ws.append([1, 2, 3])
    wb.save("out.xlsx")

Covered: workbooks and sheets, cell values (numbers, text, booleans, dates and times,
formulas, array formulas, errors), number formats, fonts, solid fills, borders,
alignment, merged cells, column widths, row heights, frozen panes, `data_only`.

A loaded workbook is saved onto its original file: charts, images, comments, defined
names, conditional formats, data validation, pivot tables, macros and properties stay
as they were, and sheets that were not touched stay byte for byte. These things
cannot be created or changed through this module yet. Sheets can be added, removed,
renamed and reordered; the others keep their content.

This is a re-creation of the interface and shares no code with openpyxl.
"""
import builtins as _builtins
import datetime as _dt
import re as _re

from .. import _xlcells
from . import utils
from .utils import column_index_from_string, coordinate_to_tuple, get_column_letter, range_boundaries
from .worksheet.cell_range import CellRange, MultiCellRange
from ._rich import CellRichText, _from_xml as _rich_from_xml

__all__ = ["Workbook", "load_workbook", "open_store", "recalculate", "read_note", "NOTE_KEY", "Image", "Comment", "DefinedName", "DataValidation", "Hyperlink", "Font", "PatternFill", "Border", "Side", "Alignment", "Color", "utils"]

_open_file = _builtins.open  # `open` below is openpyxl's alias of load_workbook
_EPOCH_1900 = _dt.datetime(1899, 12, 30)
_EPOCH_1904 = _dt.datetime(1904, 1, 1)
# openpyxl's rules for "this number format shows a date" and "... a duration": only the
# first section counts; quoted text and brackets other than [h], [m], [s] are ignored.
_STRIP = _re.compile(r'".*?"|\[(?!hh?\]|mm?\]|ss?\])[^\]]*\]')
_DATE_CHAR = _re.compile(r"(?<![_\\])[dmhysDMHYS]")
_TIMEDELTA_FORMAT = _re.compile(r"\[hh?\](:mm(:ss(\.0*)?)?)?|\[mm?\](:ss(\.0*)?)?|\[ss?\](\.0*)?", _re.I)


def _is_date_format(fmt):
    return _DATE_CHAR.search(_STRIP.sub("", fmt.split(";")[0])) is not None
FORMAT_GENERAL = "General"
FORMAT_DATE_DATETIME = "yyyy-mm-dd h:mm:ss"
FORMAT_DATE_YYYYMMDD2 = "yyyy-mm-dd"
FORMAT_DATE_TIME6 = "h:mm:ss"
FORMAT_DATE_TIMEDELTA = "[hh]:mm:ss"


from ._styles import (  # noqa: E402,F401
    _Value, _argb, Color, Font, Fill, PatternFill, GradientFill, Stop, Side, Border, Alignment, Protection,
    DEFAULT_FONT as _DEFAULT_FONT, DEFAULT_FILL as _DEFAULT_FILL, DEFAULT_BORDER as _DEFAULT_BORDER,
    DEFAULT_ALIGNMENT as _DEFAULT_ALIGNMENT, DEFAULT_PROTECTION as _DEFAULT_PROTECTION, _element,
)

_PLAIN = (FORMAT_GENERAL, _DEFAULT_FONT, _DEFAULT_FILL, _DEFAULT_BORDER, _DEFAULT_ALIGNMENT, _DEFAULT_PROTECTION, None, False, False)
_SEP = "\x1f"


def _spec(number_format, font, fill, border, alignment, protection=None, named=None, quote_prefix=False, pivot_button=False):
    """The style of a cell in the key=value form the core takes: the fields the core
    knows, and the XML of each part that says more than they do."""
    f = ["fmt=" + (number_format or FORMAT_GENERAL)]
    font = font or _DEFAULT_FONT
    if font.b:
        f.append("b=1")
    if font.i:
        f.append("i=1")
    if font.u:
        f.append("u=" + str(font.u))
    if font.strike:
        f.append("strike=1")
    if font.name and (font.name, font.sz) != ("Calibri", 11):
        f.append("font=" + font.name)
    if font.sz and (font.name or "Calibri", font.sz) != ("Calibri", 11):
        f.append("sz={0!r}".format(float(font.sz)))
        if not font.name:
            f.append("font=Calibri")
    if font.color is not None and font.color.type == "rgb" and font.color.rgb:
        f.append("color=" + font.color.rgb)
    if _beyond_fields(font, theme_default=not any(x.startswith(("font=", "color=")) for x in f)):
        f.append("fontxml=" + font._xml())
    if isinstance(fill, PatternFill) and fill.patternType == "solid" and fill.fgColor.type == "rgb":
        f.append("fill=" + fill.fgColor.rgb)
    plain_solid = isinstance(fill, PatternFill) and fill.patternType == "solid" and fill.fgColor.type == "rgb" and not fill.fgColor.tint \
        and fill.bgColor == Color(indexed=64)
    if fill is not None and fill != _DEFAULT_FILL and not plain_solid:
        f.append("fillxml=" + fill._xml())
    if border is not None:
        for key, side in (("bl", border.left), ("br", border.right), ("bt", border.top), ("bb", border.bottom)):
            if side is not None and side.style:
                f.append(key + "=" + side.style + (":" + side.color.rgb if side.color is not None and side.color.type == "rgb" else ""))
        if border != _fields_border(border):
            f.append("borderxml=" + border._xml())
    if alignment is not None:
        if alignment.horizontal:
            f.append("h=" + alignment.horizontal)
        if alignment.vertical:
            f.append("v=" + alignment.vertical)
        if alignment.wrapText:
            f.append("wrap=1")
        if alignment.textRotation:
            f.append("rot={0}".format(int(alignment.textRotation)))
        if alignment.indent:
            f.append("indent={0}".format(int(alignment.indent)))
        plain = Alignment(horizontal=alignment.horizontal, vertical=alignment.vertical, wrapText=True if alignment.wrapText else None,
                          textRotation=alignment.textRotation, indent=int(alignment.indent))
        if alignment != plain and alignment != _DEFAULT_ALIGNMENT:
            f.append("alignxml=" + alignment._xml())
    if protection is not None and protection != _DEFAULT_PROTECTION:
        f.append("protxml=" + protection._xml())
    if quote_prefix:
        f.append("xf.quotePrefix=1")
    if pivot_button:
        f.append("xf.pivotButton=1")
    if named and named != "Normal":
        f.append("named=" + named)
    return _SEP.join(f)


def _beyond_fields(font, theme_default=False):
    """Whether a font holds more than the core's fields say (theme or indexed colour, tint,
    vertical position, effects, family other than Swiss, scheme, charset, no name or size).
    `theme_default`: the fields say nothing about name and colour, so the core writes
    Excel's default font (Calibri 11, theme colour 1, minor scheme)."""
    colour = font.color
    if font.name is None or font.sz is None:
        return True
    if theme_default:
        return not (colour == Color(theme=1) and font.scheme == "minor" and font.family == 2 and not font.vertAlign
                    and font.charset is None and font.strike is not False
                    and all(getattr(font, n) is None for n in ("outline", "shadow", "condense", "extend")))
    return bool((colour is not None and (colour.type != "rgb" or colour.tint)) or font.vertAlign or font.scheme or font.charset is not None
                or any(getattr(font, n) is not None for n in ("outline", "shadow", "condense", "extend"))
                or font.family not in (None, 2) or font.strike is False)


def _fields_border(border):
    def plain(side):
        if side is None or not side.style:
            return Side()
        colour = side.color if side.color is not None and side.color.type == "rgb" and not side.color.tint else None
        return Side(style=side.style, color=colour)
    return Border(plain(border.left), plain(border.right), plain(border.top), plain(border.bottom))


def _objects(spec):
    """(number format, Font, Fill, Border, Alignment, Protection, named style, quotePrefix,
    pivotButton) of a style description; the XML of a part where the description has it."""
    d = dict(field.split("=", 1) for field in spec.split(_SEP) if "=" in field)

    def side(key):
        if key not in d:
            return Side()
        style, _, colour = d[key].partition(":")
        return Side(style=style, color=colour or None)

    if "fontxml" in d:
        font = Font._from(_element(d["fontxml"]))
    elif not any(k in d for k in ("b", "i", "u", "strike", "font", "sz", "color")):
        font = _DEFAULT_FONT
    else:
        # As the core writes it: without name and colour, Excel's default (theme) font.
        theme = "font" not in d and "color" not in d
        font = Font(name=d.get("font", "Calibri"), sz=float(d["sz"]) if "sz" in d else 11, b="b" in d, i="i" in d,
                    u=d.get("u"), strike=True if "strike" in d else None,
                    color=Color(theme=1) if theme else d.get("color"), family=2 if "font" not in d else None,
                    scheme="minor" if theme else None)
    if "fillxml" in d:
        fill = Fill._from(_element(d["fillxml"]))
    else:
        fill = PatternFill(fill_type="solid", fgColor=d["fill"], bgColor=Color(indexed=64)) if "fill" in d else _DEFAULT_FILL
    if "borderxml" in d:
        border = Border._from(_element(d["borderxml"]))
    else:
        border = Border(side("bl"), side("br"), side("bt"), side("bb")) if any(k in d for k in ("bl", "br", "bt", "bb")) else _DEFAULT_BORDER
    if "alignxml" in d:
        alignment = Alignment._from(_element(d["alignxml"]))
    elif any(k in d for k in ("h", "v", "wrap", "rot", "indent")):
        alignment = Alignment(horizontal=d.get("h"), vertical=d.get("v"), wrap_text=True if "wrap" in d else None,
                              text_rotation=int(d.get("rot", 0)), indent=int(d.get("indent", 0)))
    else:
        alignment = _DEFAULT_ALIGNMENT
    protection = Protection._from(_element(d["protxml"])) if "protxml" in d else _DEFAULT_PROTECTION
    return (d.get("fmt", FORMAT_GENERAL), font, fill, border, alignment, protection, d.get("named"),
            d.get("xf.quotePrefix") in ("1", "true"), d.get("xf.pivotButton") in ("1", "true"))


def _from_serial(value, date1904, number_format):
    """A date serial as datetime, time or timedelta, the way openpyxl converts it."""
    if _TIMEDELTA_FORMAT.search(number_format.split(";")[0]):
        td = _dt.timedelta(days=value)
        if td.microseconds:  # openpyxl rounds durations to milliseconds
            td = _dt.timedelta(seconds=td.total_seconds() // 1, microseconds=round(td.microseconds, -3))
        return td
    day, fraction = divmod(value, 1)
    diff = _dt.timedelta(milliseconds=round(fraction * 86400000))
    if 0 <= value < 1 and diff.days == 0:
        mins, seconds = divmod(diff.seconds, 60)
        hours, mins = divmod(mins, 60)
        return _dt.time(hours, mins, seconds, diff.microseconds)
    if not date1904 and 0 < value < 60:
        day += 1
    return (_EPOCH_1904 if date1904 else _EPOCH_1900) + _dt.timedelta(days=day) + diff


def _to_serial(value, date1904):
    """Serial number of a datetime, date, time or timedelta."""
    if isinstance(value, _dt.timedelta):
        return value.total_seconds() / 86400
    if isinstance(value, _dt.time):
        return (value.hour * 3600 + value.minute * 60 + value.second + value.microsecond / 1e6) / 86400
    if not isinstance(value, _dt.datetime):
        value = _dt.datetime(value.year, value.month, value.day)
    delta = value.replace(tzinfo=None) - (_EPOCH_1904 if date1904 else _EPOCH_1900)
    serial = delta.days + (delta.seconds + delta.microseconds / 1e6) / 86400
    if not date1904 and 0 < serial <= 60:
        serial -= 1
    return serial


_EMU_PER_PIXEL = 9525


def _image_info(data):
    """(extension, width, height) of a PNG, JPEG, GIF or BMP file, read from its header."""
    if data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) >= 24:
        return "png", int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")
    if data[:6] in (b"GIF87a", b"GIF89a"):
        return "gif", int.from_bytes(data[6:8], "little"), int.from_bytes(data[8:10], "little")
    if data[:2] == b"BM" and len(data) >= 26:
        return "bmp", int.from_bytes(data[18:22], "little"), abs(int.from_bytes(data[22:26], "little", signed=True))
    if data[:2] == b"\xff\xd8":
        i = 2
        while i + 9 < len(data):
            if data[i] != 0xFF:
                i += 1
                continue
            marker = data[i + 1]
            if marker in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
                return "jpeg", int.from_bytes(data[i + 7:i + 9], "big"), int.from_bytes(data[i + 5:i + 7], "big")
            i += 2 + int.from_bytes(data[i + 2:i + 4], "big")
    raise ValueError("not a PNG, JPEG, GIF or BMP picture")


class Image:
    """A picture to place on a sheet: a file name, a file object or the file content.
    `width` and `height` are in pixels and can be changed before saving."""

    def __init__(self, img):
        if isinstance(img, (bytes, bytearray)):
            self._bytes = bytes(img)
        elif hasattr(img, "read"):
            self._bytes = img.read()
        else:
            with _open_file(img, "rb") as f:
                self._bytes = f.read()
        self.format, self.width, self.height = _image_info(self._bytes)
        self.anchor = "A1"
        self._loaded = None

    def _data(self):
        return self._bytes

    def _drawing(self):
        row, column = coordinate_to_tuple(self.anchor if isinstance(self.anchor, str) else self.anchor.coordinate)
        spec = ((column - 1, 0, row - 1, 0), None, (int(self.width * _EMU_PER_PIXEL), int(self.height * _EMU_PER_PIXEL)), (self.format, self._bytes), None)
        if self._loaded is not None and self._loaded[0] == spec:
            return self._loaded[1]  # untouched: exactly as it was in the file
        return spec


class _LoadedChart:
    """A chart read from a file: written back unchanged, except where it sits. Setting
    `anchor` to another cell moves it (size and offsets within the cell kept);
    removing it from `ws._charts` removes it. Its content (series, titles, axes) is
    not modelled and cannot be changed."""

    def __init__(self, spec):
        self._spec = spec
        self.anchor = self._anchor_at_load = "{0}{1}".format(get_column_letter(spec[0][0] + 1), spec[0][2] + 1)

    def _drawing(self):
        if self.anchor == self._anchor_at_load:
            return self._spec
        row, col = coordinate_to_tuple(self.anchor)
        start, end, size, picture, chart = self._spec
        dc, dr = col - 1 - start[0], row - 1 - start[2]
        start = (start[0] + dc, start[1], start[2] + dr, start[3])
        if end is not None:
            end = (end[0] + dc, end[1], end[2] + dr, end[3])
        return (start, end, size, picture, chart)


class Comment:
    """A note on a cell. `width` and `height` are accepted; boxes get a standard size."""

    def __init__(self, text, author, height=79, width=144):
        self.text, self.author, self.height, self.width = text, author, height, width

    content = property(lambda self: self.text)

    def __eq__(self, other):
        return isinstance(other, Comment) and (self.text, self.author) == (other.text, other.author)

    def __repr__(self):
        return "Comment: {0} by {1}".format(self.text, self.author)


class Hyperlink:
    """A link on a cell: `target` is an address outside the workbook, `location` a
    place inside it."""

    def __init__(self, ref=None, location=None, tooltip=None, display=None, id=None, target=None):
        self.ref, self.location, self.tooltip, self.display, self.target = ref, location, tooltip, display, target

    def _key(self):
        return (self.ref, self.target, self.location, self.tooltip, self.display)


class DefinedName:
    """A name for a range or formula. `attr_text` is what it stands for, such as
    `Sheet1!$A$1:$B$5`."""

    def __init__(self, name=None, comment=None, localSheetId=None, hidden=None, attr_text=None, **other):
        self.name = name
        self.attr_text = attr_text
        self.localSheetId = localSheetId
        self.hidden = bool(hidden)

    value = property(lambda self: self.attr_text)

    @property
    def destinations(self):
        """(sheet title, range) pairs the name points to."""
        for piece in (self.attr_text or "").split(","):
            sheet, _, cells = piece.rpartition("!")
            if sheet:
                yield sheet.strip("'").replace("''", "'"), cells


class _DefinedNames(dict):
    def add(self, defn):
        self[defn.name] = defn


class DataValidation:
    """A rule for what may be entered into cells."""

    _ATTRS = (("type", "type"), ("errorStyle", "errorStyle"), ("operator", "operator"), ("allowBlank", "allowBlank"), ("showDropDown", "showDropDown"),
              ("showInputMessage", "showInputMessage"), ("showErrorMessage", "showErrorMessage"), ("errorTitle", "errorTitle"), ("error", "error"),
              ("promptTitle", "promptTitle"), ("prompt", "prompt"))

    def __init__(self, type=None, formula1=None, formula2=None, showErrorMessage=False, showInputMessage=False, showDropDown=False, allowBlank=False,
                 sqref=(), promptTitle=None, errorStyle=None, error=None, prompt=None, errorTitle=None, imeMode=None, operator=None, allow_blank=None):
        self.type, self.formula1, self.formula2, self.operator, self.errorStyle = type, formula1, formula2, operator, errorStyle
        self.showErrorMessage, self.showInputMessage, self.showDropDown = showErrorMessage, showInputMessage, showDropDown
        self.allowBlank = allowBlank if allow_blank is None else allow_blank
        self.promptTitle, self.prompt, self.errorTitle, self.error = promptTitle, prompt, errorTitle, error
        self._ranges = [sqref] if isinstance(sqref, str) and sqref else list(sqref.split() if isinstance(sqref, str) else sqref)
        self._extra = []

    allow_blank = property(lambda self: self.allowBlank, lambda self, v: setattr(self, "allowBlank", v))
    hide_drop_down = property(lambda self: self.showDropDown)

    @property
    def sqref(self):
        return " ".join(self._ranges)

    def add(self, cell):
        self._ranges.append(cell.coordinate if hasattr(cell, "coordinate") else str(cell))

    def _attrs(self):
        out = []
        for attr, key in self._ATTRS:
            v = getattr(self, attr)
            if isinstance(v, bool):
                if v:
                    out.append((key, "1"))
            elif v is not None:
                out.append((key, str(v)))
        return out + self._extra + [("sqref", self.sqref)]

    @classmethod
    def _load(cls, attrs, formula1, formula2):
        d = dict(attrs)
        flag = lambda k: d.get(k) in ("1", "true")  # noqa: E731
        dv = cls(type=d.get("type"), formula1=formula1, formula2=formula2, operator=d.get("operator"), errorStyle=d.get("errorStyle"),
                 showErrorMessage=flag("showErrorMessage"), showInputMessage=flag("showInputMessage"), showDropDown=flag("showDropDown"),
                 allowBlank=flag("allowBlank"), promptTitle=d.get("promptTitle"), prompt=d.get("prompt"), errorTitle=d.get("errorTitle"),
                 error=d.get("error"), sqref=d.get("sqref", ""))
        known = {k for _, k in cls._ATTRS} | {"sqref"}
        dv._extra = [(k, v) for k, v in attrs if k not in known]
        dv._loaded = (list(attrs), formula1, formula2)
        dv._snapshot = (dv._attrs(), dv.formula1, dv.formula2)
        return dv

    def _spec(self):
        """(attributes, formula1, formula2); for an untouched loaded rule exactly as read."""
        spec = (self._attrs(), self.formula1, self.formula2)
        if getattr(self, "_snapshot", None) == spec:
            return self._loaded
        return spec


class _DataValidations:
    def __init__(self):
        self.dataValidation = []

    count = property(lambda self: len(self.dataValidation))


def _xml_text(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def _unxml(value):
    return value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", '"').replace("&amp;", "&")


def _escape_attr(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


class DifferentialStyle:
    """The format a conditional rule applies: font, fill, border (and number format,
    alignment, protection)."""

    def __init__(self, font=None, numFmt=None, fill=None, alignment=None, border=None, protection=None):
        self.font, self.fill, self.border = font, fill, border
        self.numFmt, self.alignment, self.protection = numFmt, alignment, protection

    def _xml(self):
        x = ""
        if self.font is not None:
            x += self.font._xml()
        if self.numFmt is not None:
            x += '<numFmt numFmtId="{0}" formatCode="{1}"/>'.format(self.numFmt.numFmtId, _escape_attr(self.numFmt.formatCode))
        if self.fill is not None:
            x += self.fill._xml()
        if self.alignment is not None:
            x += self.alignment._xml()
        if self.border is not None:
            x += self.border._xml()
        if self.protection is not None:
            x += self.protection._xml()
        return x

    @classmethod
    def _load(cls, xml):
        from .styles.numbers import NumberFormat
        root = _element("<dxf>{0}</dxf>".format(xml))
        kw = {}
        for child in root:
            name = child.tag.rsplit("}", 1)[-1]
            if name == "font":
                kw["font"] = Font._from(child)
            elif name == "fill":
                kw["fill"] = Fill._from(child)
            elif name == "border":
                kw["border"] = Border._from(child)
            elif name == "alignment":
                kw["alignment"] = Alignment._from(child)
            elif name == "protection":
                kw["protection"] = Protection._from(child)
            elif name == "numFmt":
                kw["numFmt"] = NumberFormat(int(child.attrib.get("numFmtId", 0)), child.attrib.get("formatCode"))
        return cls(**kw)

class ArrayFormula:
    """An array formula: the range it fills and its text with the leading '='."""

    def __init__(self, ref, text=None):
        self.ref = ref
        self.text = text

    def __eq__(self, other):
        return isinstance(other, ArrayFormula) and (self.ref, self.text) == (other.ref, other.text)

    def __repr__(self):
        return "ArrayFormula({0!r}, {1!r})".format(self.ref, self.text)


_ILLEGAL = _re.compile(r"[\000-\010]|[\013-\014]|[\016-\037]")


class IllegalCharacterError(ValueError):
    """Text with control characters a cell cannot hold."""


class Cell:
    __slots__ = ("parent", "row", "column", "_value", "number_format", "font", "fill", "border", "alignment", "protection", "_style_name", "quotePrefix",
                 "pivotButton", "_cached", "_error", "_literal", "_raw", "_dirty", "_hyperlink", "comment")

    @property
    def look(self):
        """What the cell looks like in Excel, every attribute resolved through named
        styles, row and column formats, table styles and conditional formats, each with
        its source (not in openpyxl). Each call works on the workbook as it is now, which
        costs writing and reading it: for many cells use ``Worksheet.looks``, for cell
        after cell ``Workbook.looker()``."""
        return self.parent.looks([self.coordinate])[self.coordinate]

    def __init__(self, worksheet, row, column, value=None):
        self.parent = worksheet
        self.row = row
        self.column = column
        # A new cell has the workbook's first format (that of the file it was loaded from).
        (self.number_format, self.font, self.fill, self.border, self.alignment, self.protection, self._style_name, self.quotePrefix,
         self.pivotButton) = getattr(worksheet.parent, "_defaults", _PLAIN) if worksheet is not None else _PLAIN
        self._hyperlink = None
        self.comment = None
        self._raw = None  # (kind, num, text, formula, style index, array range, float spelling, runs XML) as read
        self._dirty = True
        self._cached = None
        self._error = False
        self._literal = False  # text read from a file, even if it starts with "="
        self._value = None
        if value is not None:
            self.value = value

    @property
    def value(self):
        return self._value

    @value.setter
    def value(self, value):
        if value is not None and not isinstance(value, (str, bool, int, float, _dt.datetime, _dt.date, _dt.time, _dt.timedelta, ArrayFormula, CellRichText)):
            try:
                value = float(value)  # Decimal, numpy scalars
            except (TypeError, ValueError):
                raise ValueError("Cannot convert {0!r} to Excel".format(value))
        if self.number_format == FORMAT_GENERAL:
            if isinstance(value, _dt.datetime):
                self.number_format = FORMAT_DATE_DATETIME
            elif isinstance(value, _dt.date):
                self.number_format = FORMAT_DATE_YYYYMMDD2
            elif isinstance(value, _dt.time):
                self.number_format = FORMAT_DATE_TIME6
            elif isinstance(value, _dt.timedelta):
                self.number_format = FORMAT_DATE_TIMEDELTA
        self._value = value
        self._cached = None
        self._error = False
        self._literal = False
        self._dirty = True

    @property
    def hyperlink(self):
        return self._hyperlink

    @hyperlink.setter
    def hyperlink(self, value):
        if value is None:
            self._hyperlink = None
            return
        if not isinstance(value, Hyperlink):
            value = Hyperlink(ref="", target=value)
        value.ref = self.coordinate
        self._hyperlink = value
        if self._value is None:
            self.value = value.target or value.location

    @property
    def column_letter(self):
        return get_column_letter(self.column)

    @property
    def col_idx(self):
        return self.column

    @property
    def base_date(self):
        return self.parent.parent.epoch

    @property
    def internal_value(self):
        return self._value

    @property
    def encoding(self):
        return "utf-8"

    def _style_key(self):
        return (self.number_format or FORMAT_GENERAL, self.font, self.fill, self.border, self.alignment, self.protection, self._style_name,
                bool(self.quotePrefix), bool(self.pivotButton))

    @property
    def has_style(self):
        return self._style_key() != getattr(self.parent.parent, "_defaults", _PLAIN)

    @property
    def style(self):
        """The name of the named style the cell's format is based on."""
        if self._style_name:
            return self._style_name
        named = self.parent.parent._named_styles
        return named[0].name if len(named) else "Normal"

    @style.setter
    def style(self, value):
        """Apply a named style (its name, or a NamedStyle): the cell takes its format."""
        wb = self.parent.parent
        named = wb._named_style(value)
        self.number_format, self.font, self.fill, self.border, self.alignment, self.protection = (
            named.number_format, named.font, named.fill, named.border, named.alignment, named.protection)
        self._style_name = None if named.name == "Normal" else named.name

    @property
    def style_id(self):
        return self._raw[4] if self._raw is not None and not self._dirty else 0

    def check_string(self, value):
        """Text as a cell takes it: at most 32,767 characters, no control characters."""
        if value is None:
            return None
        if isinstance(value, bytes):
            value = value.decode("utf-8")
        value = str(value)[:32767]
        if _ILLEGAL.search(value):
            raise IllegalCharacterError("{0} cannot be used in worksheets.".format(value))
        return value

    def check_error(self, value):
        try:
            return str(value)
        except UnicodeDecodeError:
            return "#N/A"

    @property
    def coordinate(self):
        return "{0}{1}".format(self.column_letter, self.row)

    @property
    def data_type(self):
        v = self._value
        if self._error:
            return "e"
        if isinstance(v, bool):
            return "b"
        if isinstance(v, ArrayFormula):
            return "f"
        if isinstance(v, CellRichText):
            return "s"
        if isinstance(v, str):
            return "f" if len(v) > 1 and v.startswith("=") and not self._literal else "s"
        if isinstance(v, (_dt.datetime, _dt.date, _dt.time, _dt.timedelta)):
            return "d"
        return "n"

    @property
    def is_date(self):
        """A date value, or a number (or nothing) with a date format, as openpyxl says."""
        return self.data_type == "d" or (self.data_type == "n" and _is_date_format(self.number_format))

    def offset(self, row=0, column=0):
        return self.parent.cell(self.row + row, self.column + column)

    def __repr__(self):
        return "<Cell {0!r}.{1}>".format(self.parent.title, self.coordinate)


class _Range(CellRange):
    """A merged range (openpyxl: MergedCellRange)."""

    def __init__(self, min_col, min_row, max_col, max_row):
        CellRange.__init__(self, min_col=min_col, min_row=min_row, max_col=max_col, max_row=max_row)

    def __repr__(self):
        return "<MergedCellRange {0}>".format(self.coord)


MergedCellRange = _Range


class _MergedCells:
    def __init__(self):
        self.ranges = []

    def __iter__(self):
        return iter(self.ranges)

    def __len__(self):
        return len(self.ranges)

    def __contains__(self, coordinate):
        row, col = coordinate_to_tuple(coordinate)
        return any(r.min_row <= row <= r.max_row and r.min_col <= col <= r.max_col for r in self.ranges)


class Worksheet:
    def looks(self, cells):
        """The looks of cells (a range such as ``"A1:C10"``, ``"A:C"`` or ``"2:4"``, or
        coordinates), by coordinate; worked out together, which is much cheaper than
        cell by cell."""
        return self.parent.looker().looks(self, cells)

    @property
    def regions(self):
        """The tables, groups of single cells and blocks of this sheet by name (not in
        openpyxl); see ``Workbook.regions``."""
        from ._regions import regions_of
        return regions_of(self.parent, self)

    def to_polars(self, cell_range=None, *, header=True):
        """The sheet, or a range of it (``"A1:F40"``, ``"A:C"``, ``"2:9"``), as a Polars
        DataFrame (not in openpyxl). One data type per column: whole numbers Int64, other
        numbers Float64, booleans, dates, times, text; a column of mixed kinds is text;
        empty cells and errors are null. With ``header`` the first row names the columns.
        Without a range: the block of cells that hold something."""
        from . import _frames
        return _frames.as_polars(_frames.sheet_frame(self, cell_range, header))

    def to_arrow(self, cell_range=None, *, header=True):
        """As ``to_polars``, as a pyarrow Table (not in openpyxl)."""
        from . import _frames
        return _frames.as_arrow(_frames.sheet_frame(self, cell_range, header))

    def __init__(self, parent, title):
        self.parent = parent
        self.title = title
        self._cells = {}
        self.merged_cells = _MergedCells()
        self.column_dimensions = DimensionHolder(self, default_factory=ColumnDimension)
        self.row_dimensions = DimensionHolder(self, default_factory=RowDimension)
        self.auto_filter = AutoFilter()
        self.data_validations = _DataValidations()
        self.conditional_formatting = ConditionalFormattingList()
        self.defined_names = _DefinedNames()
        self._images = []
        self._charts = []
        self._drawing_order = None  # pictures and charts in the order of the file
        self._range_links = []  # hyperlinks on ranges, kept as loaded
        self.sheet_properties = WorksheetProperties()
        self.views = SheetViewList()
        self.sheet_format = SheetFormatProperties()
        self.protection = SheetProtection()
        self.print_options = PrintOptions()
        self.page_margins = PageMargins()
        self.page_setup = PrintPageSetup(worksheet=self)
        self.HeaderFooter = HeaderFooter()
        self.row_breaks = RowBreak()
        self.col_breaks = ColBreak()
        self._print_area = None
        self._print_rows = None
        self._print_cols = None
        self._loaded_extra = {}  # element name -> (XML as read or None, XML of the objects right after loading)
        self._loaded_print = {}  # reserved name -> (text as read, order, text right after loading)
        self._tables = TableList()
        self._other_extra = []  # (element name, XML) as read, of the elements no object here stands for
        self._kind = 0
        self._origin = None  # position in the file this sheet was loaded from
        self._current_row = 0
        self.sheet_state = "visible"

    # --- views, printing, protection -----------------------------------------
    @property
    def sheet_view(self):
        return self.views.sheetView[0]

    @property
    def show_gridlines(self):
        return self.sheet_view.showGridLines

    @property
    def active_cell(self):
        return self.sheet_view.selection[0].activeCell if self.sheet_view.selection else None

    @property
    def selected_cell(self):
        return self.sheet_view.selection[0].sqref if self.sheet_view.selection else None

    @property
    def freeze_panes(self):
        pane = self.sheet_view.pane
        return pane.topLeftCell if pane is not None else None

    @freeze_panes.setter
    def freeze_panes(self, cell):
        """Freeze the rows above and the columns left of `cell` ("B2", a cell, None)."""
        if cell is not None and not isinstance(cell, str):
            cell = cell.coordinate
        view = self.sheet_view
        if not cell or cell == "A1":
            view.pane = None
            return
        row, column = coordinate_to_tuple(cell)
        if row > 1 and column > 1:
            where = "bottomRight"
        elif row > 1:
            where = "bottomLeft"
        else:
            where = "topRight"
        view.pane = Pane(xSplit=column - 1 if column > 1 else None, ySplit=row - 1 if row > 1 else None, topLeftCell=cell, activePane=where, state="frozen")
        current = view.selection[-1] if view.selection else Selection()
        current.pane = where
        view.selection = ([Selection(pane="topRight", activeCell=None, sqref=None), Selection(pane="bottomLeft", activeCell=None, sqref=None)] if where == "bottomRight" else []) + [current]

    def _header(name):
        return property(lambda self: getattr(self.HeaderFooter, name), lambda self, v: setattr(self.HeaderFooter, name, v))

    oddHeader = _header("oddHeader")
    oddFooter = _header("oddFooter")
    evenHeader = _header("evenHeader")
    evenFooter = _header("evenFooter")
    firstHeader = _header("firstHeader")
    firstFooter = _header("firstFooter")
    del _header

    @property
    def page_breaks(self):
        return (self.row_breaks, self.col_breaks)

    def set_printer_settings(self, paper_size, orientation):
        """Paper size (an Excel number, 9 is A4) and orientation ("portrait", "landscape")."""
        self.page_setup.paperSize = paper_size
        self.page_setup.orientation = orientation

    def _sheet_ref(self, ref):
        return "{0}!{1}".format(quote_sheetname(self.title), ref)

    @property
    def print_area(self):
        if not self._print_area:
            return None
        return ",".join(self._sheet_ref(absolute_coordinate(r)) for r in self._print_area)

    @print_area.setter
    def print_area(self, value):
        if value is None:
            self._print_area = None
            return
        if isinstance(value, str):
            value = value.split(",")
        self._print_area = [v.split("!")[-1].replace("$", "") for v in value]

    @property
    def print_title_rows(self):
        return self._print_rows

    @print_title_rows.setter
    def print_title_rows(self, value):
        if value is not None and not _re.match(r"^\$?\d+:\$?\d+$", value):
            raise ValueError("Print title rows must be the form 1:3")
        self._print_rows = None if value is None else ":".join("$" + p.replace("$", "") for p in value.split(":"))

    @property
    def print_title_cols(self):
        return self._print_cols

    @print_title_cols.setter
    def print_title_cols(self, value):
        if value is not None and not _re.match(r"^\$?[A-Za-z]{1,3}:\$?[A-Za-z]{1,3}$", value):
            raise ValueError("Print title columns must be the form A:C")
        self._print_cols = None if value is None else ":".join("$" + p.replace("$", "").upper() for p in value.split(":"))

    @property
    def print_titles(self):
        parts = [self._sheet_ref(t) for t in (self._print_rows, self._print_cols) if t]
        return ",".join(parts) if parts else None

    def _read_print_name(self, name, value):
        """Take a reserved print name as read; False if it is not of a form we know."""
        parts = [p.strip() for p in _split_refs(value)]
        refs = [p.split("!")[-1] for p in parts]
        if name == "_xlnm.Print_Area":
            if not all(_re.match(r"^\$?[A-Za-z]{1,3}\$?\d+(:\$?[A-Za-z]{1,3}\$?\d+)?$", r) for r in refs):
                return False
            self._print_area = [r.replace("$", "") for r in refs]
            return True
        rows = [r for r in refs if _re.match(r"^\$?\d+:\$?\d+$", r)]
        cols = [r for r in refs if _re.match(r"^\$?[A-Za-z]{1,3}:\$?[A-Za-z]{1,3}$", r)]
        if len(rows) + len(cols) != len(refs) or len(rows) > 1 or len(cols) > 1:
            return False
        self._print_rows = rows[0] if rows else None
        self._print_cols = cols[0] if cols else None
        return True

    def _extra_objects(self):
        return (("sheetPr", self.sheet_properties), ("sheetViews", self.views), ("sheetFormatPr", self.sheet_format), ("sheetProtection", self.protection),
                ("printOptions", self.print_options), ("pageMargins", self.page_margins), ("pageSetup", self.page_setup), ("headerFooter", self.HeaderFooter),
                ("rowBreaks", self.row_breaks), ("colBreaks", self.col_breaks), ("autoFilter", self.auto_filter))

    def _print_names(self):
        """The reserved names for print area and titles: (name, text)."""
        out = []
        if self.print_titles:
            out.append(("_xlnm.Print_Titles", self.print_titles))
        if self.print_area:
            out.append(("_xlnm.Print_Area", self.print_area))
        return out

    # --- single cells -------------------------------------------------------
    def cell(self, row, column, value=None):
        if row < 1 or column < 1:
            raise ValueError("Row or column values must be at least 1")
        cell = self._cells.get((row, column))
        if cell is None:
            cell = self._cells[(row, column)] = Cell(self, row, column)
            if row > self._current_row:
                self._current_row = row
        if value is not None:
            cell.value = value
        return cell

    def __getitem__(self, key):
        """A cell (`"A1"`), a range (`"A1:B2"`), a column (`"A"`, `"A:C"`) or a row (`5`,
        `"5"`, `"2:4"`, `2:4`), as openpyxl gives them."""
        if isinstance(key, slice):
            if not all([key.start, key.stop]):
                raise IndexError("{0} is not a valid coordinate or range".format(key))
            key = "{0}:{1}".format(key.start, key.stop)
        if isinstance(key, int):
            key = str(key)
        min_col, min_row, max_col, max_row = range_boundaries(key)
        if not any([min_col, min_row, max_col, max_row]):
            raise IndexError("{0} is not a valid coordinate or range".format(key))
        if min_row is None:
            cols = tuple(self.iter_cols(min_col, max_col))
            return cols[0] if min_col == max_col else cols
        if min_col is None:
            rows = tuple(self.iter_rows(min_col=min_col, min_row=min_row, max_col=self.max_column, max_row=max_row))
            return rows[0] if min_row == max_row else rows
        if ":" not in key:
            return self.cell(min_row, min_col)
        return tuple(self.iter_rows(min_row=min_row, min_col=min_col, max_row=max_row, max_col=max_col))

    def __setitem__(self, key, value):
        self[key].value = value

    def __delitem__(self, key):
        self._cells.pop(coordinate_to_tuple(key), None)

    # --- extent ------------------------------------------------------------
    @property
    def min_row(self):
        return min((r for r, _ in self._cells), default=1)

    @property
    def max_row(self):
        return max((r for r, _ in self._cells), default=1)

    @property
    def min_column(self):
        return min((c for _, c in self._cells), default=1)

    @property
    def max_column(self):
        return max((c for _, c in self._cells), default=1)

    @property
    def dimensions(self):
        return "{0}{1}:{2}{3}".format(get_column_letter(self.min_column), self.min_row, get_column_letter(self.max_column), self.max_row)

    def calculate_dimension(self):
        return self.dimensions

    # --- iteration ---------------------------------------------------------
    def iter_rows(self, min_row=None, max_row=None, min_col=None, max_col=None, values_only=False):
        if not self._cells and all(v is None for v in (min_row, max_row, min_col, max_col)):
            return
        min_row, max_row = min_row or 1, max_row or self.max_row
        min_col, max_col = min_col or 1, max_col or self.max_column
        get = self._cells.get
        for r in range(min_row, max_row + 1):
            if values_only:
                yield tuple((lambda c: None if c is None else c.value)(get((r, c))) for c in range(min_col, max_col + 1))
            else:
                yield tuple(self.cell(r, c) for c in range(min_col, max_col + 1))

    def iter_cols(self, min_col=None, max_col=None, min_row=None, max_row=None, values_only=False):
        if not self._cells and all(v is None for v in (min_row, max_row, min_col, max_col)):
            return
        min_row, max_row = min_row or 1, max_row or self.max_row
        min_col, max_col = min_col or 1, max_col or self.max_column
        for c in range(min_col, max_col + 1):
            column = tuple(self.cell(r, c) for r in range(min_row, max_row + 1))
            yield tuple(x.value for x in column) if values_only else column

    @property
    def rows(self):
        return self.iter_rows()

    @property
    def columns(self):
        return self.iter_cols()

    @property
    def values(self):
        return self.iter_rows(values_only=True)

    def __iter__(self):
        return self.iter_rows()

    def append(self, iterable):
        """Add a row below the last one written; a dict maps columns to values."""
        row = self._current_row + 1
        if isinstance(iterable, dict):
            for key, value in iterable.items():
                column = column_index_from_string(key) if isinstance(key, str) else key
                self.cell(row, column).value = value
        else:
            if isinstance(iterable, (str, bytes)) or not hasattr(iterable, "__iter__"):
                raise TypeError("Value must be a list, tuple, range or generator, or a dict. Supplied value is {0}".format(type(iterable)))
            for offset, value in enumerate(iterable):
                cell = self.cell(row, offset + 1)
                if value is not None:
                    cell.value = value.value if isinstance(value, Cell) else value
        self._current_row = row

    # --- layout ------------------------------------------------------------
    def merge_cells(self, range_string=None, start_row=None, start_column=None, end_row=None, end_column=None):
        if range_string is None:
            bounds = (start_column, start_row, end_column, end_row)
        else:
            bounds = range_boundaries(range_string)
        merged = _Range(*bounds)
        if merged not in self.merged_cells.ranges:
            self.merged_cells.ranges.append(merged)
        self._fill_merged(merged, clear=True)

    def _fill_merged(self, merged, clear):
        """As in openpyxl, every cell of a merged range exists and only the top-left
        one keeps its value."""
        if (merged.max_row - merged.min_row + 1) * (merged.max_col - merged.min_col + 1) > 1000000:
            return
        for r in range(merged.min_row, merged.max_row + 1):
            for c in range(merged.min_col, merged.max_col + 1):
                cell = self.cell(r, c)
                if clear and (r, c) != (merged.min_row, merged.min_col):
                    cell.value = None

    def add_image(self, img, anchor=None):
        if anchor is not None:
            img.anchor = anchor
        self._images.append(img)

    def add_chart(self, chart, anchor=None):
        if anchor is not None:
            chart.anchor = anchor
        self._charts.append(chart)

    def _drawings(self):
        """Pictures and charts: those from the file in their order, then new ones."""
        items = self._images + self._charts
        first = [d for d in (self._drawing_order or []) if any(d is i for i in items)]
        return first + [i for i in items if not any(i is d for d in first)]

    def add_data_validation(self, data_validation):
        self.data_validations.dataValidation.append(data_validation)

    @property
    def merged_cell_ranges(self):
        return list(self.merged_cells.ranges)

    @property
    def array_formulae(self):
        """Ranges of array formulas by their first cell."""
        return {c.coordinate: c._value.ref for c in self._cells.values() if isinstance(c._value, ArrayFormula)}

    @property
    def tables(self):
        return self._tables

    def add_table(self, table):
        """Add a table; its name must not be taken by another table or a defined name,
        nor read as a cell (``T1``, ``ABC12``) or in R1C1 notation (``R``, ``C``, ``R1C1``):
        Excel repairs a file with such a table name (openpyxl writes it nevertheless)."""
        if _reads_as_cell(table.name):
            raise ValueError("Table name {0!r} reads as a cell reference, which Excel does not allow; use another name (e.g. {0!r} + 'Table')".format(table.name))
        if self.parent._duplicate_name(table.name):
            raise ValueError("Table with name {0} already exists".format(table.name))
        self._tables.add(table)

    # --- moving cells ---------------------------------------------------------
    # As in openpyxl: only cells move (formulas, merged ranges, row heights and column
    # widths stay), and the cells of the area involved come into being, empty ones too.
    def _move_cell(self, row, column, row_offset, col_offset, translate=False):
        cell = self.cell(row, column)
        del self._cells[(row, column)]
        cell.row, cell.column = row + row_offset, column + col_offset
        cell._dirty = True
        self._cells[(cell.row, cell.column)] = cell
        if cell._hyperlink is not None:
            cell._hyperlink.ref = cell.coordinate
        if translate and cell.data_type == "f":
            from .formula.translate import Translator
            if isinstance(cell._value, ArrayFormula):
                ref = CellRange(cell._value.ref)
                ref.shift(col_offset, row_offset)
                cell._value = ArrayFormula(ref.coord, Translator(cell._value.text, "A1").translate_formula(row_delta=row_offset, col_delta=col_offset))
            else:
                cell._value = Translator(cell._value, "A1").translate_formula(row_delta=row_offset, col_delta=col_offset)

    def _move_cells(self, min_row=None, min_col=None, offset=0, row_or_col="row"):
        """Move every cell from `min_row` (or `min_col`) on by `offset` rows (columns)."""
        key = 0 if row_or_col == "row" else 1
        if key == 0:
            list(self.iter_rows(min_row=min_row))
        else:
            list(self.iter_cols(min_col=min_col))
        for row, column in sorted(self._cells, key=lambda k: k[key], reverse=offset > 0):
            if (min_row and row < min_row) or (min_col and column < min_col):
                continue
            self._move_cell(row, column, offset if key == 0 else 0, offset if key == 1 else 0)

    def insert_rows(self, idx, amount=1):
        """Insert `amount` empty rows before row `idx`."""
        self._move_cells(min_row=idx, offset=amount, row_or_col="row")
        self._current_row = self.max_row

    def insert_cols(self, idx, amount=1):
        self._move_cells(min_col=idx, offset=amount, row_or_col="column")

    def delete_rows(self, idx, amount=1):
        """Remove `amount` rows from row `idx` on; the rows below move up."""
        gutter = range(max(self.max_row + 1 - amount, idx), min(idx + amount, self.max_row) + 1)
        self._move_cells(min_row=idx + amount, offset=-amount, row_or_col="row")
        first, last = self.min_column, self.max_column
        for row in gutter:
            for col in range(first, last + 1):
                self._cells.pop((row, col), None)
        self._current_row = self.max_row if self._cells else 0

    def delete_cols(self, idx, amount=1):
        gutter = range(max(self.max_column + 1 - amount, idx), min(idx + amount, self.max_column) + 1)
        self._move_cells(min_col=idx + amount, offset=-amount, row_or_col="column")
        first, last = self.min_row, self.max_row
        for col in gutter:
            for row in range(first, last + 1):
                self._cells.pop((row, col), None)

    def move_range(self, cell_range, rows=0, cols=0, translate=False):
        """Move the cells of a range by `rows` and `cols`, overwriting the cells there.
        With `translate`, the relative references of formulas move along."""
        if isinstance(cell_range, str):
            cell_range = CellRange(cell_range)
        if not isinstance(cell_range, CellRange):
            raise ValueError("Only CellRange objects can be moved")
        if not rows and not cols:
            return
        lines = sorted(cell_range.rows, reverse=rows > 0) if rows else sorted(cell_range.cols, reverse=cols > 0)
        for line in lines:
            for row, col in line:
                self._move_cell(row, col, rows, cols, translate)
        cell_range.shift(row_shift=rows, col_shift=cols)

    def unmerge_cells(self, range_string=None, start_row=None, start_column=None, end_row=None, end_column=None):
        bounds = (start_column, start_row, end_column, end_row) if range_string is None else range_boundaries(range_string)
        self.merged_cells.ranges.remove(_Range(*bounds))

    def __repr__(self):
        return '<Worksheet "{0}">'.format(self.title)


class Workbook:
    def __init__(self, write_only=False, iso_dates=False):
        self._sheets = []
        self._active = 0
        self.defined_names = _DefinedNames()
        self._original = None  # bytes of the file this workbook was loaded from
        self._specs = None  # its styles, by index
        self._style_objects = None
        self.epoch = _EPOCH_1900
        self._read_only = False
        self._data_only = False
        self._write_only = bool(write_only)
        self.template = False
        self.iso_dates = iso_dates
        self._named_styles = _default_named_styles()
        self._defaults = _PLAIN
        self._named_loaded = None  # the named styles as read (name, spec, builtin id, hidden)
        self.security = None  # WorkbookProtection
        self.properties = DocumentProperties()
        self.custom_doc_props = CustomPropertyList()
        self.calculation = CalcProperties()
        self.views = [BookView()]
        self._workbook_pr = WorkbookProperties()
        self._loaded_wb_extra = {}  # element name -> (XML as read or None, XML of the objects right after loading)
        self._other_wb_extra = []  # children of <workbook> passed through as read
        self._loaded_props = {}  # document property part -> (XML as read, XML of the object right after loading)
        if not self._write_only:
            self.create_sheet("Sheet")

    @property
    def code_name(self):
        return self._workbook_pr.codeName

    @code_name.setter
    def code_name(self, value):
        self._workbook_pr.codeName = value

    def _wb_extra_objects(self):
        views = _BookViews()
        views.workbookView = list(self.views)
        return (("workbookPr", self._workbook_pr), ("workbookProtection", self.security), ("bookViews", views), ("calcPr", self.calculation))

    # --- named styles -----------------------------------------------------------
    @property
    def named_styles(self):
        """The names of the workbook's named cell styles."""
        return self._named_styles.names

    def add_named_style(self, style):
        self._named_styles.append(style)
        style.bind(self)

    def _duplicate_name(self, name):
        """Whether a table or defined name already has this name (case ignored)."""
        name = name.lower()
        for sheet in self._sheets:
            for t in sheet.tables:
                if name == t.lower():
                    return True
        return any(name == n.lower() for n in self.defined_names)

    def _named_style(self, value):
        """The named style for `cell.style = value`; Excel's built-in styles join the workbook
        when first used."""
        if isinstance(value, NamedStyle):
            if value.name not in self._named_styles.names:
                self.add_named_style(value)
            return self._named_styles[value.name]
        if value not in self._named_styles.names:
            builtin = _builtin_style(value)
            if builtin is None:
                raise ValueError("{0} is not a known style".format(value))
            self.add_named_style(builtin)
        return self._named_styles[value]

    @property
    def style_names(self):
        return self.named_styles

    def _named_untouched(self):
        at_load = getattr(self, "_named_at_load", {})
        return {s.name for s in self._named_styles if at_load.get(s.name) == _named_spec(s)}

    @property
    def read_only(self):
        return self._read_only

    @property
    def data_only(self):
        return self._data_only

    @property
    def write_only(self):
        return self._write_only

    @property
    def excel_base_date(self):
        return self.epoch

    # --- the names openpyxl kept from earlier versions --------------------------
    def get_sheet_by_name(self, name):
        return self[name]

    def get_sheet_names(self):
        return self.sheetnames

    def remove_sheet(self, worksheet):
        self.remove(worksheet)

    def get_index(self, worksheet):
        return self.index(worksheet)

    def create_named_range(self, name, worksheet=None, value=None, scope=None):
        defn = DefinedName(name=name)
        defn.attr_text = "{0}!{1}".format(quote_sheetname(worksheet.title), value) if worksheet is not None else value
        self.defined_names[name] = defn

    def move_sheet(self, sheet, offset=0):
        """Move a sheet (or the sheet of that name) by `offset` places."""
        if not isinstance(sheet, Worksheet):
            sheet = self[sheet]
        index = self._sheets.index(sheet)
        del self._sheets[index]
        self._sheets.insert(index + offset, sheet)

    def copy_worksheet(self, from_worksheet):
        """A copy of a sheet of this workbook, as the last sheet: cells with their values
        and formats, merged ranges, row heights, column widths, frozen panes, printing and
        view settings (as openpyxl; pictures and charts are not copied)."""
        import copy as _copy
        if self._read_only or self._write_only:
            raise ValueError("Cannot copy worksheets in read-only or write-only mode")
        if from_worksheet.parent is not self:
            raise ValueError("Cannot copy between worksheets from different workbooks")
        ws = self.create_sheet(self._unique_title("{0} Copy".format(from_worksheet.title)))
        for (row, col), cell in from_worksheet._cells.items():
            new = ws.cell(row, col)
            new._value, new._cached, new._error, new._literal = cell._value, cell._cached, cell._error, cell._literal
            (new.number_format, new.font, new.fill, new.border, new.alignment, new.protection, new._style_name, new.quotePrefix,
             new.pivotButton) = (cell.number_format, cell.font, cell.fill, cell.border, cell.alignment, cell.protection, cell._style_name,
                                 cell.quotePrefix, cell.pivotButton)
            if cell._raw is not None:
                new._raw = cell._raw
            new._dirty = True
        for merged in from_worksheet.merged_cells.ranges:
            ws.merged_cells.ranges.append(_Range(*merged.bounds))
        for holder, source in ((ws.row_dimensions, from_worksheet.row_dimensions), (ws.column_dimensions, from_worksheet.column_dimensions)):
            for key, dim in source.items():
                new = _copy.copy(dim)
                new.parent, new._loaded = ws, None
                holder[key] = new
        for name in ("sheet_format", "sheet_properties", "page_margins", "page_setup", "print_options", "views", "HeaderFooter", "row_breaks", "col_breaks"):
            setattr(ws, name, _copy.deepcopy(getattr(from_worksheet, name)))
        ws.page_setup._parent = ws
        ws.page_setup.id = None  # the printer settings part belongs to the original sheet
        ws._print_area, ws._print_rows, ws._print_cols = from_worksheet._print_area, from_worksheet._print_rows, from_worksheet._print_cols
        ws._current_row = from_worksheet._current_row
        return ws

    # --- sheets ------------------------------------------------------------
    def _unique_title(self, base):
        titles = {s.title.lower() for s in self._sheets}
        if base.lower() not in titles:
            return base
        n = 1
        while "{0}{1}".format(base, n).lower() in titles:
            n += 1
        return "{0}{1}".format(base, n)

    def create_sheet(self, title=None, index=None):
        if self._read_only:
            raise ReadOnlyWorkbookException("Cannot create new sheet in a read-only workbook")
        ws = Worksheet(self, self._unique_title(title or "Sheet"))
        if self._write_only:
            _make_write_only(ws)
        if index is None:
            self._sheets.append(ws)
        else:
            self._sheets.insert(index, ws)
        return ws

    def remove(self, worksheet):
        self._sheets.remove(worksheet)
        self._active = min(self._active, max(len(self._sheets) - 1, 0))

    @property
    def active(self):
        return self._sheets[self._active] if self._sheets else None

    @active.setter
    def active(self, value):
        self._active = value if isinstance(value, int) else self._sheets.index(value)

    @property
    def worksheets(self):
        return [s for s in self._sheets if s._kind == 0]

    @property
    def chartsheets(self):
        return [s for s in self._sheets if s._kind == 1]

    @property
    def sheetnames(self):
        return [s.title for s in self._sheets]

    def index(self, worksheet):
        return self._sheets.index(worksheet)

    def __getitem__(self, name):
        for sheet in self._sheets:
            if sheet.title == name:
                return sheet
        raise KeyError("Worksheet {0} does not exist.".format(name))

    def __delitem__(self, name):
        self.remove(self[name])

    def __contains__(self, name):
        return name in self.sheetnames

    def __iter__(self):
        return iter(self._sheets)

    def close(self):
        pass

    @property
    def regions(self):
        """What the workbook holds, for moving it elsewhere (not in openpyxl): its
        tables, groups of single cells (named by their labels) and blocks, by name.
        ``print(wb.regions)`` lists them. Each gives a dataframe (``to_polars()``), takes one back into its cells (``write(df)``) and compares one
        with the workbook (``verify(df)``); calculated columns and cells show their
        formula (``formula``) and its readable form (``logic``)."""
        from ._regions import regions_of
        return regions_of(self)

    def to_store(self, path):
        """Make a store of the workbook in the directory `path` (not in openpyxl): the
        data of its regions as Parquet files, a model document with their logic, and
        the workbook as the form the data returns to. See ``xlcells.openpyxl.open_store``
        and ``help(store)``."""
        from ._store import to_store
        return to_store(self, path)

    def evaluate(self, frames=None):
        """The workbook's regions as Polars DataFrames after putting `frames` (region
        name -> dataframe) into their cells and calculating every formula with the
        engine, exactly as Excel does (not in openpyxl). The workbook itself is not
        changed. ``result.not_computed`` lists what the engine could not calculate."""
        from ._regions import evaluate
        return evaluate(self, frames)

    def to_polars_code(self, path="workbook.xlsx", accuracy="auto"):
        """The workbook as a Polars program in Python (not in openpyxl): its regions as
        dataframes, its formulas as Polars expressions in the order of their dependence,
        and a check of every result against the workbook. `path` is the file the program
        loads. `accuracy`: ``"plain"`` writes Polars wherever a translation exists,
        ``"exact"`` writes the formula engine's own translation as Polars code (what it
        reads is the program's data and cells, so other data and other values give the
        values the engine works out for them), and takes the engine's values
        (``wb.evaluate``) where that cannot be written out, ``"auto"`` (default) writes
        Polars where that reproduces the workbook's values, then the engine's translation,
        then the engine's values. ``code.items`` and ``code.summary()`` tell how each
        calculated column and cell is written: ``polars``, ``plan`` or ``engine``."""
        from ._regions import polars_code
        return polars_code(self, path, accuracy)

    def _reload(self, data):
        """Take the state of the file `data` (this workbook, changed in Rust). The
        workbook and its worksheets stay the objects they are."""
        new = load_workbook(data, data_only=self._data_only, rich_text=getattr(self, "_rich_text", False))
        old_sheets = list(self._sheets)
        self.__dict__.clear()
        self.__dict__.update(new.__dict__)
        for k, ws in enumerate(self._sheets):
            if k < len(old_sheets):
                kept = old_sheets[k]
                kept.__dict__.clear()
                kept.__dict__.update(ws.__dict__)
                self._sheets[k] = ws = kept
                for cell in getattr(ws, "_cells", {}).values():
                    cell.parent = ws
            ws.parent = self

    def looker(self):
        """The looks of the workbook as it is now (not in openpyxl): ``looker.look(cell)``,
        ``looker.looks(ws, "A1:C10")``. Written and read once, however many cells are
        asked for; changes made afterwards are not seen."""
        from ._look import Looker
        return Looker(self)

    # --- output ------------------------------------------------------------
    def to_bytes(self):
        """The xlsx file. A loaded workbook is written onto its original file, so that
        everything this module does not cover stays as it was."""
        w = _xlcells.Writer()
        date1904 = self.epoch == _EPOCH_1904
        w.set_date1904(date1904)
        styles = {}
        local = []
        loaded = self._style_objects
        if self._specs is not None:
            w.set_styles(self._specs)
            # A format equal to one of the file's keeps that one's number.
            for i, key in enumerate(loaded):
                styles.setdefault(key, i)
        named = [(n.name, _named_spec(n), -1 if n.builtinId is None else n.builtinId, n.hidden) for n in self._named_styles]
        if self._named_loaded is not None:
            # As read where unchanged, so that the file keeps its own formats for them.
            loaded_by_name = {entry[0]: entry for entry in self._named_loaded}
            named = [loaded_by_name.get(n[0], n) if n[0] in self._named_untouched() else n for n in named]
        if named != [entry for entry in ([("Normal", _named_spec(_default_named_styles()[0]), 0, False)])]:
            w.set_named_styles(named)
        for index, ws in enumerate(self._sheets):
            w.add_sheet(ws.title)
            if ws._kind:
                w.set_sheet_kind(index, ws._kind)
            if ws._origin is not None and self._original is not None:
                w.set_sheet_origin(index, ws._origin)
            for (row, column), cell in sorted(ws._cells.items()):
                raw = cell._raw
                key = cell._style_key()
                if raw is not None and not cell._dirty:
                    as_read = loaded[raw[4]]
                    if key[0] == as_read[0] and all(a is b for a, b in zip(key[1:6], as_read[1:6])) and key[6:] == as_read[6:]:
                        w.write(index, row - 1, column - 1, raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7], raw[8])
                        continue
                if raw is None and cell._value is None and key == self._defaults:
                    continue  # an empty cell without formatting is no cell
                style = styles.get(key)
                if style is None:
                    style = styles[key] = w.add_style_spec(_spec(*key))
                if raw is not None and not cell._dirty:
                    w.write(index, row - 1, column - 1, raw[0], raw[1], raw[2], raw[3], style, raw[5], raw[6], raw[7], raw[8])
                    continue
                value = cell._value
                formula = None
                if isinstance(value, CellRichText):
                    text = str(value)
                    if text:
                        w.write(index, row - 1, column - 1, 2, 0.0, text, None, style, None, False, value._xml())
                    elif style:
                        w.write(index, row - 1, column - 1, 0, 0.0, None, None, style)
                    continue
                if isinstance(value, ArrayFormula):
                    w.write(index, row - 1, column - 1, 0, 0.0, None, (value.text or "=").lstrip("="), style, value.ref)
                    continue
                if isinstance(value, str) and len(value) > 1 and value.startswith("=") and not cell._error and not cell._literal:
                    formula, value = value[1:], cell._cached
                if value is None:
                    if formula is None and style == 0:
                        continue
                    kind, num, text = 0, 0.0, None
                elif isinstance(value, bool):
                    kind, num, text = 3, float(value), None
                elif isinstance(value, str):
                    kind, num, text = (4 if cell._error or (formula and value.startswith("#")) else 2), 0.0, value
                elif isinstance(value, (int, float)):
                    kind, num, text = 1, float(value), None
                else:
                    kind, num, text = 1, _to_serial(value, date1904), None
                # As openpyxl with iso_dates: dates, times and dates with times as ISO 8601 cells.
                form = 0
                if self.iso_dates and formula is None and isinstance(value, (_dt.date, _dt.time)):
                    form = 3 if isinstance(value, _dt.time) else 2 if isinstance(value, _dt.datetime) else 1
                w.write(index, row - 1, column - 1, kind, num, text, formula, style, None, False, None, form)
            for merged in ws.merged_cells.ranges:
                w.merge(index, merged.min_row - 1, merged.max_row - 1, merged.min_col - 1, merged.max_col - 1)
            def dim_style(dim):
                if not dim.has_style:
                    return None
                key = dim._style_key()
                if key not in styles:
                    styles[key] = w.add_style_spec(_spec(*key))
                return styles[key]
            outline_cols, dims_changed = 0, False
            for dim in sorted(ws.column_dimensions.values(), key=lambda d: (d.reindex(), d.min)[1]):
                if dim._loaded is not None and dim._state() == dim._loaded[0]:
                    w.set_columns(index, *dim._loaded[1])
                elif not dim._is_plain():
                    dims_changed = True
                    w.set_columns(index, dim.min - 1, dim.max - 1, dim._width, dim.customWidth, dim.hidden, dim.bestFit, dim.outlineLevel or 0,
                                  dim.collapsed, dim_style(dim))
                else:
                    continue
                outline_cols = max(outline_cols, dim.outlineLevel or 0)
            outline_rows = 0
            for number, dim in sorted(ws.row_dimensions.items()):
                if dim._loaded is not None and dim._state() == dim._loaded[0]:
                    w.set_row(index, *dim._loaded[1])
                elif not dim._is_plain():
                    dims_changed = True
                    w.set_row(index, number - 1, dim.ht, dim.ht is not None, dim.hidden, dim.outlineLevel or 0, dim.collapsed, dim_style(dim),
                              dim.has_style, bool(dim.thickTop), bool(dim.thickBot))
                else:
                    continue
                outline_rows = max(outline_rows, dim.outlineLevel or 0)
            # Excel shows the outline buttons up to the level the sheet format names
            # (rows and columns as read stay as they were).
            fmt = ws.sheet_format
            if dims_changed and outline_rows > (fmt.outlineLevelRow or 0):
                fmt.outlineLevelRow = outline_rows
            if dims_changed and outline_cols > (fmt.outlineLevelCol or 0):
                fmt.outlineLevelCol = outline_cols
            if ws.auto_filter.ref:
                w.set_auto_filter(index, ws.auto_filter.ref)
            for dv in ws.data_validations.dataValidation:
                w.add_validation(index, *dv._spec())
            for item in ws._drawings():
                w.add_drawing(index, *item._drawing())
            noted = [(getattr(c.comment, "_order", 1 << 30), c.coordinate, c.comment) for _, c in sorted(ws._cells.items()) if c.comment is not None]
            for _, where, comment in sorted(noted, key=lambda n: n[0]):
                w.add_comment(index, where, comment.author or "", comment.text or "")
            for table in ws.tables.values():
                table._prepare(ws)
                w.add_table(index, table._spec())
            for cf in ws.conditional_formatting:
                w.add_cond_format(index, str(cf.sqref), [rule._spec() for rule in cf.rules])
            links = [c._hyperlink for _, c in sorted(ws._cells.items()) if c._hyperlink is not None] + ws._range_links
            for link in sorted(links, key=lambda h: getattr(h, "_order", 1 << 30)):
                w.add_hyperlink(index, link.ref, link.target, link.location, link.tooltip, link.display)
            for name, defn in ws.defined_names.items():
                local.append((getattr(defn, "_order", 1 << 30), name, defn.attr_text or "", index, defn.hidden))
            if ws.freeze_panes and ws.freeze_panes != "A1":
                frozen_row, frozen_col = coordinate_to_tuple(ws.freeze_panes)
                w.set_freeze(index, frozen_row - 1, frozen_col - 1)
            for name, obj in ws._extra_objects():
                xml = obj._xml()
                as_read = ws._loaded_extra.get(name)
                if as_read is not None and xml == as_read[1]:
                    # Untouched: as it was read, or (not in the file) not at all.
                    if as_read[0] is not None:
                        w.set_sheet_xml(index, name, as_read[0])
                    continue
                if as_read is None and xml == _DEFAULT_XML[name]:
                    continue
                if name == "sheetViews" and xml == _frozen_views_xml(ws.freeze_panes):
                    continue  # the frozen panes say it all
                w.set_sheet_xml(index, name, xml)
            managed = {name for name, _ in ws._extra_objects()}
            for name, xml in ws._other_extra:
                if name not in managed:
                    w.set_sheet_xml(index, name, xml)
            current = dict(ws._print_names())
            for name in sorted(set(current) | set(ws._loaded_print)):
                raw, order, at_load = ws._loaded_print.get(name, (None, None, None))
                text = current.get(name)
                if text is None:
                    continue
                if raw is not None and text == at_load:
                    text = raw
                local.append((order if order is not None else 1 << 30, name, text, index, False))
        names = local + [(getattr(d, "_order", 1 << 30), n, d.attr_text or "", None, d.hidden) for n, d in self.defined_names.items()]
        for _, name, text, sheet, hidden in sorted(names, key=lambda n: n[0]):
            w.add_defined_name(name, text, sheet, hidden)
        # Workbook settings: as read where unchanged, left out where new and default.
        if self.views and 0 <= self._active < len(self._sheets):
            self.views[0].activeTab = self._active
        for name, obj in self._wb_extra_objects():
            xml = obj._xml() if obj is not None else ""
            as_read = self._loaded_wb_extra.get(name)
            if as_read is not None and xml == as_read[1]:
                if as_read[0] is not None:
                    w.set_workbook_xml(name, as_read[0])
                continue
            if as_read is None and xml == _DEFAULT_WB_XML[name]:
                continue
            w.set_workbook_xml(name, xml)
        for name, xml in self._other_wb_extra:
            w.set_workbook_xml(name, xml)
        # Document properties; as openpyxl, the time of saving is the time of change.
        self.properties.modified = _dt.datetime.now(tz=_dt.timezone.utc).replace(tzinfo=None)
        for kind, obj in (("core", self.properties), ("custom", self.custom_doc_props)):
            xml = obj._xml() if (kind == "core" or len(obj)) else ""
            as_read = self._loaded_props.get(kind)
            if as_read is not None and xml == as_read[1]:
                xml = as_read[0]
            elif as_read is not None and kind == "core":
                xml = obj._xml_onto(as_read[0])  # usually only the time of saving changes
            elif kind == "core" and self._original is not None and obj._key(dates=False) == self._core_at_load:
                continue  # a file without document properties gets none unless some are set
            w.set_properties_xml(kind, xml)
        if "app" in self._loaded_props:
            w.set_properties_xml("app", self._loaded_props["app"][0])
        if self._original is not None:
            data, self._kept_original = w.to_bytes_onto(self._original)
            return data
        return w.to_bytes()

    def calculate(self):
        """Calculate the workbook's formulas (not in openpyxl, which leaves that to Excel).

        Formulas filled down columns become Polars expressions (see `xl_polars::lift`);
        each formula cell whose expression holds gets its value, also as the value saved
        with the file. Cells on a cycle, cells whose formula has no translation, and every
        cell that reads one of them keep the value they had; if there are any, Excel is
        asked to calculate everything when it opens the file. Returns what happened:
        formula cells, computed, changed, and the cells not computed by reason."""
        if self._data_only:
            raise ValueError("A workbook loaded with data_only=True has no formulas to calculate")
        from .._needs_engine import engine
        result = engine("Workbook.calculate").calculate(self.to_bytes())
        for sheet, row, col, kind, num, text in result["values"]:
            cell = self._sheets[sheet]._cells.get((row + 1, col + 1))
            if cell is None:
                continue
            value = _number(num, False) if kind == 1 else text if kind in (2, 4) else (num != 0.0) if kind == 3 else None
            if cell._raw is not None and not cell._dirty:
                raw = cell._raw
                cell._raw = (kind, num, text if kind in (2, 4) else None) + tuple(raw[3:7]) + (None, 0)
            if isinstance(cell._value, (str, ArrayFormula)) and (isinstance(cell._value, ArrayFormula) or cell._value.startswith("=")):
                cell._cached = value
        report = {k: v for k, v in result.items() if k != "values"}
        report["not_computed"] = dict(report["not_computed"])
        if report["not_computed"]:
            if self.calculation is None:
                self.calculation = CalcProperties()
            self.calculation.fullCalcOnLoad = True
        return report

    def save(self, filename):
        if self._read_only:
            raise TypeError("Workbook is read-only")
        if self._write_only:
            if not self.worksheets:
                self.create_sheet()
            if any(ws.closed for ws in self.worksheets):
                raise WorkbookAlreadySaved("Workbook has already been saved and cannot be modified or saved anymore.")
        data = self.to_bytes()
        if self._write_only:
            for ws in self.worksheets:
                ws.close()
        if hasattr(filename, "write"):
            filename.write(data)
        else:
            with _open_file(filename, "wb") as f:
                f.write(data)


def _number(value, float_text):
    """openpyxl reads '42' as int and '42.0', '42.5' or '1E+20' as float; so do we."""
    return int(value) if not float_text and value == int(value) and abs(value) < 1e16 else value


def _reads_as_cell(name):
    """Whether a name reads as a cell or in R1C1 notation, which Excel refuses as a name."""
    m = _re.fullmatch(r"([A-Za-z]{1,3})(\d+)", name or "")
    if m and column_index_from_string(m.group(1).upper()) <= 16384 and 1 <= int(m.group(2)) <= 1048576:
        return True
    return bool(_re.fullmatch(r"[RrCc]|[Rr]\d*[Cc]\d*|[Rr]\d+|[Cc]\d+", name or ""))


#: The key of a Parquet file's metadata that holds its note (see ``open_store``).
NOTE_KEY = "xlcells"


def recalculate(paths):
    """Calculate Parquet files again from the notes they carry (not in openpyxl): the
    calculated columns and cells of a workbook, worked out on its data alone. See
    ``xlcells.openpyxl._store``."""
    from ._store import recalculate as run
    return run(paths)


def read_note(path):
    """The note a Parquet file of a store carries (not in openpyxl): a dict, or ``None``."""
    from ._store import read_note as read
    return read(path)


def open_store(path):
    """A store made by ``Workbook.to_store`` (not in openpyxl)."""
    from ._store import Store
    return Store(path)


def load_workbook(filename, read_only=False, keep_vba=False, data_only=False, keep_links=True, rich_text=False):
    """Open a workbook file, file-like object or bytes."""
    if isinstance(filename, (bytes, bytearray)):
        data = bytes(filename)
    elif hasattr(filename, "read"):
        data = filename.read()
    else:
        with _open_file(filename, "rb") as f:
            data = f.read()
    t = _xlcells.read(data)
    wb = Workbook()
    wb._sheets = []
    wb._read_only, wb._data_only = bool(read_only), bool(data_only)
    date1904 = t["date1904"]
    wb.epoch = _EPOCH_1904 if date1904 else _EPOCH_1900
    for name, kind in zip(t["sheet_names"], t["sheet_kind"]):
        ws = Worksheet(wb, name)
        ws._kind = kind
        ws._origin = len(wb._sheets)
        wb._sheets.append(ws)
    # An .xls file (read beyond openpyxl, which refuses it) is saved as a new .xlsx.
    wb._original = None if data[:8] == b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" else data
    wb._specs = t["style_spec"]
    wb._style_objects = objects = [_objects(spec) for spec in wb._specs]
    if objects:
        wb._defaults = objects[0]
    if t["named_styles"]:
        wb._named_styles = NamedStyleList()
        for name, spec, builtin, hidden, _custom, _level in t["named_styles"]:
            fmt, font, fill, border, alignment, protection = _objects(spec)[:6]
            style = NamedStyle(name=name, font=font, fill=fill, border=border, alignment=alignment, number_format=fmt, protection=protection,
                               builtinId=None if builtin < 0 else builtin, hidden=hidden)
            if name not in wb._named_styles.names:
                wb._named_styles.append(style)
                style.bind(wb)
        wb._named_loaded = [(n, spec, b, h) for n, spec, b, h, _c, _l in t["named_styles"]]
        wb._named_at_load = {s.name: _named_spec(s) for s in wb._named_styles}
    formats = t["num_format"]
    dates = [_is_date_format(f) for f in formats]
    rich = dict(t["rich"])
    iso = dict(t["iso"])
    for index, (sheet, row, col, kind, num, float_text, text, formula, style, array_ref) in enumerate(zip(
        t["sheet"], t["row"], t["col"], t["kind"], t["num"], t["float_text"], t["text"], t["formula"], t["style"], t["array_ref"]
    )):
        ws = wb._sheets[sheet]
        cell = Cell(ws, row + 1, col + 1)
        (cell.number_format, cell.font, cell.fill, cell.border, cell.alignment, cell.protection, cell._style_name, cell.quotePrefix,
         cell.pivotButton) = objects[style]
        runs = rich.get(index)
        form = iso.get(index, 0)
        cell._raw = (kind, num, text, formula, style, array_ref, float_text, runs, form)
        cell._dirty = False
        error = False
        if kind == 1 and form and not formula:
            # An ISO date cell: a date, date and time, or time whatever its number format (as openpyxl).
            try:
                value = _from_serial(num, date1904, {1: "yyyy-mm-dd", 2: "yyyy-mm-dd h:mm:ss", 3: "h:mm:ss"}[form])
                if form == 1 and isinstance(value, _dt.datetime):
                    value = value.date()
                elif form == 3 and isinstance(value, _dt.datetime):
                    value = value.time()
            except (OverflowError, ValueError):
                value, error = "#VALUE!", True
        elif kind == 1:
            if dates[style]:
                try:
                    value = _from_serial(num, date1904, formats[style])
                except (OverflowError, ValueError):
                    value, error = "#VALUE!", True
            else:
                value = _number(num, float_text)
        elif kind == 2 and runs is not None and rich_text:
            value = _rich_from_xml(runs)
        elif kind == 2:
            value = text if (text or not formula) else None  # an empty formula result is no value in openpyxl
        elif kind == 3:
            value = num != 0.0
        elif kind == 4:
            value, error = text, True
        else:
            value = None
        if formula and not data_only:
            cell._value = ArrayFormula(array_ref, "=" + formula) if array_ref else "=" + formula
            cell._cached = value
        else:
            cell._value = value
            cell._error = error
            cell._literal = kind == 2
        ws._cells[(row + 1, col + 1)] = cell
        if row + 1 > ws._current_row:
            ws._current_row = row + 1
    for sheet, r1, c1, r2, c2 in t["merges"]:
        merged = _Range(c1 + 1, r1 + 1, c2 + 1, r2 + 1)
        wb._sheets[sheet].merged_cells.ranges.append(merged)
        wb._sheets[sheet]._fill_merged(merged, clear=False)
    for order, (name, local_sheet, value, hidden) in enumerate(t["defined_names"]):
        if local_sheet >= 0 and name in ("_xlnm.Print_Area", "_xlnm.Print_Titles") and not hidden:
            ws = wb._sheets[local_sheet]
            if ws._read_print_name(name, value):
                ws._loaded_print[name] = (value, order, None)
                continue
        defn = DefinedName(name, attr_text=value, hidden=hidden, localSheetId=None if local_sheet < 0 else local_sheet)
        defn._order = order
        (wb.defined_names if local_sheet < 0 else wb._sheets[local_sheet].defined_names)[name] = defn
    for sheet, ref in t["auto_filter"]:
        wb._sheets[sheet].auto_filter.ref = ref
    for sheet, start, end, size, picture, chart in t["drawings"]:
        ws = wb._sheets[sheet]
        if ws._drawing_order is None:
            ws._drawing_order = []
        spec = (start, end, size, picture, chart)
        if picture is not None:
            try:
                item = Image(bytes(picture[1]))
            except ValueError:
                item = _LoadedChart(spec)  # a picture type we only pass through
                ws._charts.append(item)
                ws._drawing_order.append(item)
                continue
            item.anchor = "{0}{1}".format(get_column_letter(start[0] + 1), start[2] + 1)
            if size is not None:
                item.width, item.height = size[0] / _EMU_PER_PIXEL, size[1] / _EMU_PER_PIXEL
            item._loaded = (item._drawing(), spec)
            ws._images.append(item)
        else:
            item = _LoadedChart(spec)
            ws._charts.append(item)
        ws._drawing_order.append(item)
    for order, (sheet, where, author, text) in enumerate(t["comments"]):
        comment = Comment(text, author)
        comment._order = order
        try:
            wb._sheets[sheet].cell(*coordinate_to_tuple(where)).comment = comment
        except ValueError:
            pass
    raw_wb = dict(t["workbook_extra"])
    # As openpyxl reads them: no window list and no calculation settings unless the file has them.
    wb.views, wb.calculation = [], None
    managed = {name for name, _ in wb._wb_extra_objects()}
    wb._other_wb_extra = [(n, x) for n, x in t["workbook_extra"] if n not in managed]
    for name in managed:
        xml = raw_wb.get(name)
        if xml is None:
            continue
        try:
            if name == "workbookPr":
                wb._workbook_pr = WorkbookProperties._from_xml(xml)
            elif name == "workbookProtection":
                wb.security = WorkbookProtection._from_xml(xml)
            elif name == "calcPr":
                wb.calculation = CalcProperties._from_xml(xml)
            elif name == "bookViews":
                wb.views = list(_BookViews._from_xml(xml).workbookView or [])
        except Exception:  # an element we cannot read: passed through as it is
            wb._other_wb_extra.append((name, xml))
    if wb.views and 0 <= (wb.views[0].activeTab or 0) < len(wb._sheets):
        wb._active = wb.views[0].activeTab or 0
    for name, obj in wb._wb_extra_objects():
        wb._loaded_wb_extra[name] = (raw_wb.get(name) if not any(n == name for n, _ in wb._other_wb_extra) else None, obj._xml() if obj is not None else "")
    wb._core_at_load = wb.properties._key(dates=False)
    for kind, xml in t["properties"]:
        try:
            if kind == "core":
                wb.properties = DocumentProperties._from_xml(xml)
                wb._loaded_props[kind] = (xml, wb.properties._xml())
            elif kind == "custom":
                wb.custom_doc_props = CustomPropertyList._from_xml(xml)
                wb._loaded_props[kind] = (xml, wb.custom_doc_props._xml())
            else:
                wb._loaded_props[kind] = (xml, None)
        except Exception:
            wb._loaded_props[kind] = (xml, None)
    for sheet, xml in t["tables"]:
        table = Table._from_table_xml(xml)
        wb._sheets[sheet]._tables[table.name] = table
    for sheet, sqref, rules in t["cond_formats"]:
        for r in rules:
            wb._sheets[sheet].conditional_formatting.add(sqref, Rule._load(*r))
    for sheet, attrs, formula1, formula2 in t["validations"]:
        wb._sheets[sheet].data_validations.dataValidation.append(DataValidation._load(attrs, formula1, formula2))
    for order, (sheet, ref, target, location, tooltip, display) in enumerate(t["hyperlinks"]):
        link = Hyperlink(ref=ref, target=target, location=location, tooltip=tooltip, display=display)
        link._order = order
        ws = wb._sheets[sheet]
        if ":" in ref or coordinate_to_tuple(ref) not in ws._cells:
            ws._range_links.append(link)
        else:
            ws._cells[coordinate_to_tuple(ref)]._hyperlink = link
    for sheet, row, height, custom_height, hidden, level, collapsed, style, custom_format, thick_top, thick_bot, phonetic in t["rows"]:
        ws = wb._sheets[sheet]
        dim = RowDimension(ws, index=row + 1, ht=height, hidden=hidden, outlineLevel=level, collapsed=collapsed, thickTop=thick_top or None,
                           thickBot=thick_bot or None, s=objects[style] if style is not None and custom_format else None)
        dim._loaded = (dim._state(), (row, height, custom_height, hidden, level, collapsed, style, custom_format, thick_top, thick_bot, phonetic))
        ws.row_dimensions[row + 1] = dim
    for ws in wb._sheets:
        ws._freeze_loaded = None
    for sheet, rows, cols in t["freeze"]:
        wb._sheets[sheet]._freeze_loaded = "{0}{1}".format(get_column_letter(cols + 1), rows + 1)
    for sheet, first, last, width, custom_width, hidden, best_fit, level, collapsed, style, phonetic in t["columns"]:
        ws = wb._sheets[sheet]
        letter = get_column_letter(first + 1)
        dim = ColumnDimension(ws, index=letter, width=width, bestFit=best_fit, hidden=hidden, outlineLevel=level, collapsed=collapsed,
                              style=objects[style] if style is not None else None, min=first + 1, max=last + 1)
        dim._loaded = (dim._state(), (first, last, width, custom_width, hidden, best_fit, level, collapsed, style, phonetic))
        ws.column_dimensions[letter] = dim
    raw = {}
    for ws in wb._sheets:
        ws._other_extra = []
    for sheet, name, xml in t["extra"]:
        raw[(sheet, name)] = xml
        wb._sheets[sheet]._other_extra.append((name, xml))
    for index, ws in enumerate(wb._sheets):
        for name, obj in ws._extra_objects():
            xml = raw.get((index, name))
            if xml is not None:
                try:
                    loaded = type(obj)._from_xml(xml)
                except Exception:  # an element we cannot read: kept as it is
                    loaded = None
                if loaded is not None:
                    if name == "pageSetup":
                        loaded._parent = ws
                    _set_extra(ws, name, loaded)
        if ws._freeze_loaded is not None and ws.sheet_view.pane is None:
            ws.freeze_panes = ws._freeze_loaded
        for name, obj in ws._extra_objects():
            ws._loaded_extra[name] = (raw.get((index, name)), obj._xml())
        for name, (value, order, _) in list(ws._loaded_print.items()):
            ws._loaded_print[name] = (value, order, dict(ws._print_names()).get(name))
    for sheet, r1, c1, r2, c2 in t["dimensions"]:
        wb._sheets[sheet]._file_dimension = (c1 + 1, r1 + 1, c2 + 1, r2 + 1)
    if wb._read_only:
        for ws in wb._sheets:
            if ws._kind == 0:
                _make_read_only(ws)
    return wb


def _set_extra(ws, name, obj):
    attr = {"sheetPr": "sheet_properties", "sheetViews": "views", "sheetFormatPr": "sheet_format", "sheetProtection": "protection", "printOptions": "print_options",
            "pageMargins": "page_margins", "pageSetup": "page_setup", "headerFooter": "HeaderFooter", "rowBreaks": "row_breaks", "colBreaks": "col_breaks",
            "autoFilter": "auto_filter"}[name]
    setattr(ws, attr, obj)


def _split_refs(text):
    """The references of a union `'A, B'!$A$1,Sheet!$B$2`, quotes respected."""
    out, buf, quoted = [], [], False
    for ch in text:
        if ch == "'":
            quoted = not quoted
        if ch == "," and not quoted:
            out.append("".join(buf))
            buf = []
        else:
            buf.append(ch)
    out.append("".join(buf))
    return out


def _frozen_views_xml(cell):
    """The sheet view xlcells writes for frozen panes alone (see the xl-cells writer)."""
    if not cell or cell == "A1":
        return None
    row, col = coordinate_to_tuple(cell)
    rows, cols = row - 1, col - 1
    pane = "topRight" if rows == 0 else "bottomLeft" if cols == 0 else "bottomRight"
    split = (' xSplit="{0}"'.format(cols) if cols else "") + (' ySplit="{0}"'.format(rows) if rows else "")
    return ('<sheetViews><sheetView workbookViewId="0"><pane{0} topLeftCell="{1}" activePane="{2}" state="frozen"/></sheetView></sheetViews>').format(split, cell, pane)


from .worksheet.properties import WorksheetProperties  # noqa: E402
from .worksheet.views import SheetViewList, Pane, Selection  # noqa: E402
from .worksheet.dimensions import SheetFormatProperties, ColumnDimension, RowDimension, DimensionHolder  # noqa: E402
from .worksheet.protection import SheetProtection  # noqa: E402
from .worksheet.page import PrintOptions, PageMargins, PrintPageSetup  # noqa: E402
from .worksheet.header_footer import HeaderFooter  # noqa: E402
from .worksheet.pagebreak import RowBreak, ColBreak  # noqa: E402
from .worksheet.filters import AutoFilter  # noqa: E402
from .worksheet.table import Table, TableList  # noqa: E402
from .workbook.protection import WorkbookProtection  # noqa: E402
from .workbook.properties import CalcProperties, WorkbookProperties  # noqa: E402
from .workbook.views import BookView, _BookViews  # noqa: E402
from .packaging.core import DocumentProperties  # noqa: E402
from .packaging.custom import CustomPropertyList  # noqa: E402
from .utils.exceptions import ReadOnlyWorkbookException, WorkbookAlreadySaved  # noqa: E402
from ._streaming import _ReadOnlyMixin, _WriteOnlyMixin, make_read_only as _make_read_only, make_write_only as _make_write_only  # noqa: E402
from ._streaming import EMPTY_CELL, EmptyCell, ReadOnlyCell, WriteOnlyCell  # noqa: E402,F401


class ReadOnlyWorksheet(_ReadOnlyMixin, Worksheet):
    """A sheet of a workbook loaded with `read_only=True`."""


class WriteOnlyWorksheet(_WriteOnlyMixin, Worksheet):
    """A sheet of a `Workbook(write_only=True)`: rows go in with `append`."""
from .utils import absolute_coordinate, quote_sheetname  # noqa: E402

_DEFAULT_WB_XML = {
    "workbookPr": WorkbookProperties()._xml(), "workbookProtection": "", "calcPr": CalcProperties()._xml(),
    "bookViews": (lambda v: (setattr(v, "workbookView", [BookView()]), v._xml())[1])(_BookViews()),
}

_DEFAULT_XML = {
    "sheetPr": WorksheetProperties()._xml(), "sheetViews": SheetViewList()._xml(), "sheetFormatPr": SheetFormatProperties()._xml(),
    "sheetProtection": SheetProtection()._xml(), "printOptions": PrintOptions()._xml(), "pageMargins": PageMargins()._xml(),
    "pageSetup": PrintPageSetup()._xml(), "headerFooter": HeaderFooter()._xml(), "rowBreaks": RowBreak()._xml(), "colBreaks": ColBreak()._xml(),
    "autoFilter": "",
}

from .styles.named_styles import NamedStyle, NamedStyleList  # noqa: E402


def _builtin_style(name):
    from .styles.builtins import STYLES
    for entry in STYLES:
        if entry[0] == name:
            sname, builtin, fmt, font, fill, border, align, locked, hidden = entry
            return NamedStyle(name=sname, builtinId=builtin, number_format=fmt, font=Font._from(_element(font)), fill=Fill._from(_element(fill)),
                              border=Border._from(_element(border)), alignment=Alignment._from(_element(align)) if align else Alignment(),
                              protection=Protection(locked, hidden))
    return None


def _default_named_styles():
    """Normal alone, with the format new cells have."""
    styles = NamedStyleList()
    styles.append(NamedStyle(name="Normal", builtinId=0, font=_DEFAULT_FONT, fill=_DEFAULT_FILL, border=_DEFAULT_BORDER, alignment=_DEFAULT_ALIGNMENT))
    return styles


def _named_spec(style):
    return _spec(style.number_format, style.font, style.fill, style.border, style.alignment, style.protection)


from . import styles  # noqa: E402,F401
from .formatting.rule import CellIsRule, ColorScaleRule, DataBarRule, FormulaRule, IconSetRule, Rule  # noqa: E402,F401
from .formatting.formatting import ConditionalFormatting, ConditionalFormattingList  # noqa: E402,F401

open = load_workbook  # openpyxl.open

# The subpackages openpyxl makes available after `import openpyxl`.
from . import cell, worksheet, workbook, formatting, packaging  # noqa: E402,F401
