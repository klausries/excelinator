"""Cell formats with everything the file format holds: colours by RGB, theme, index or
automatic with tint; fonts with family, scheme and effects; pattern and gradient fills;
borders with diagonals; alignment; protection. Each object reads itself from and writes
itself to its SpreadsheetML element.

Behaves like openpyxl's style objects (same names, defaults and aliases; immutable in
use, compared by value, hashable).
"""
import xml.etree.ElementTree as _ET

_NS = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
BLACK = "00000000"


def _esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def _num(value):
    """A number as the file writes it: 1 not 1.0."""
    value = float(value)
    return str(int(value)) if value == int(value) and abs(value) < 1e15 else repr(value)


def _local(tag):
    return tag.rsplit("}", 1)[-1]


def _argb(value):
    """A colour as openpyxl stores it: 8 hex digits; 6 digits get the prefix 00."""
    if value is None:
        return None
    if isinstance(value, Color):
        return value.rgb
    value = str(value)
    return "00" + value if len(value) == 6 else value


def _flag(text):
    return text not in ("0", "false", "False")


class _Value:
    """Base of the style objects: compared and hashed by their fields."""

    _fields = ()

    def _key(self):
        return tuple(getattr(self, f) for f in self._fields)

    def __eq__(self, other):
        return type(other) is type(self) and self._key() == other._key()

    def __ne__(self, other):
        return not self == other

    def __hash__(self):
        return hash((type(self).__name__,) + self._key())

    def __repr__(self):
        return "{0}({1})".format(type(self).__name__, ", ".join("{0}={1!r}".format(f, getattr(self, f)) for f in self._fields))

    def copy(self, **changes):
        """A copy with some fields changed (as openpyxl 2.x offered)."""
        new = type(self).__new__(type(self))
        new.__dict__.update(self.__dict__)
        for name, value in changes.items():
            setattr(new, name, value)
        return new

    def __iter__(self):
        """(name, text) of the fields that are set, as openpyxl's objects give them."""
        for name in self._fields:
            value = getattr(self, name)
            if value is not None and not isinstance(value, _Value):
                yield name, str(value)

    def __copy__(self):
        return self.copy()


class Color(_Value):
    """A colour: `rgb` (ARGB), `indexed` (palette), `theme` (with `tint`) or `auto`."""

    _fields = ("rgb", "indexed", "auto", "theme", "tint", "type")

    def __init__(self, rgb=BLACK, indexed=None, auto=None, theme=None, tint=0.0, index=None, type="rgb"):
        if index is not None:
            indexed = index
        self.rgb = self.indexed = self.auto = self.theme = None
        if indexed is not None:
            self.type, self.indexed = "indexed", int(indexed)
        elif theme is not None:
            self.type, self.theme = "theme", int(theme)
        elif auto is not None:
            self.type, self.auto = "auto", bool(auto)
        else:
            self.type, self.rgb = "rgb", _argb(rgb)
        self.tint = float(tint or 0.0)

    @property
    def value(self):
        return getattr(self, self.type)

    @property
    def index(self):
        return self.value

    def _xml(self, tag="color"):
        attr = {"rgb": ' rgb="{0}"', "indexed": ' indexed="{0}"', "theme": ' theme="{0}"', "auto": ' auto="{0}"'}[self.type]
        value = self.value if self.type != "auto" else int(bool(self.auto))
        tint = ' tint="{0}"'.format(_num(self.tint)) if self.tint else ""
        return "<{0}{1}{2}/>".format(tag, attr.format(_esc(value)), tint)

    @classmethod
    def _from(cls, el):
        a = el.attrib
        tint = float(a.get("tint", 0) or 0)
        if "indexed" in a:
            return cls(indexed=int(a["indexed"]), tint=tint)
        if "theme" in a:
            return cls(theme=int(a["theme"]), tint=tint)
        if "auto" in a:
            return cls(auto=_flag(a["auto"]), tint=tint)
        return cls(rgb=a.get("rgb", BLACK), tint=tint)


def _color(value):
    return None if value is None else value if isinstance(value, Color) else Color(value)


class Font(_Value):
    """A font: name, size, bold, italic, underline, strike, colour, vertical position
    (`superscript`, `subscript`), family, scheme, charset and the outline, shadow,
    condense and extend effects."""

    _fields = ("name", "charset", "family", "b", "i", "strike", "outline", "shadow", "condense", "color", "extend", "sz", "u", "vertAlign", "scheme")
    UNDERLINE_DOUBLE = "double"
    UNDERLINE_DOUBLE_ACCOUNTING = "doubleAccounting"
    UNDERLINE_SINGLE = "single"
    UNDERLINE_SINGLE_ACCOUNTING = "singleAccounting"

    def __init__(self, name=None, sz=None, b=None, i=None, charset=None, u=None, strike=None, color=None, scheme=None, family=None,
                 size=None, bold=None, italic=None, strikethrough=None, underline=None, vertAlign=None, outline=None, shadow=None,
                 condense=None, extend=None):
        self.name = name
        self.sz = float(sz if sz is not None else size) if (sz is not None or size is not None) else None
        self.b = bool(b if b is not None else bold)
        self.i = bool(i if i is not None else italic)
        self.u = u if u is not None else underline
        self.strike = strike if strike is not None else strikethrough
        self.color = _color(color)
        self.vertAlign, self.family, self.scheme, self.charset = vertAlign, family, scheme, charset
        self.outline, self.shadow, self.condense, self.extend = outline, shadow, condense, extend
        if self.family is not None:
            self.family = float(self.family)
        if self.charset is not None:
            self.charset = int(self.charset)

    bold = property(lambda self: self.b)
    italic = property(lambda self: self.i)
    size = property(lambda self: self.sz)
    underline = property(lambda self: self.u)
    strikethrough = property(lambda self: self.strike)

    def _xml(self, tag="font"):
        x = []
        if self.b:
            x.append("<b/>")
        if self.i:
            x.append("<i/>")
        for name in ("strike", "outline", "shadow", "condense", "extend"):
            value = getattr(self, name)
            if value is not None:
                x.append("<{0}/>".format(name) if value else '<{0} val="0"/>'.format(name))
        if self.u:
            x.append("<u/>" if self.u == "single" else '<u val="{0}"/>'.format(_esc(self.u)))
        if self.vertAlign:
            x.append('<vertAlign val="{0}"/>'.format(_esc(self.vertAlign)))
        if self.sz is not None:
            x.append('<sz val="{0}"/>'.format(_num(self.sz)))
        if self.color is not None:
            x.append(self.color._xml())
        if self.name is not None:
            x.append('<name val="{0}"/>'.format(_esc(self.name)))
        if self.family is not None:
            x.append('<family val="{0}"/>'.format(_num(self.family)))
        if self.charset is not None:
            x.append('<charset val="{0}"/>'.format(self.charset))
        if self.scheme:
            x.append('<scheme val="{0}"/>'.format(_esc(self.scheme)))
        return "<{0}>{1}</{0}>".format(tag, "".join(x))

    @classmethod
    def _from(cls, el):
        kw = {}
        for child in el:
            name, val = _local(child.tag), child.attrib.get("val")
            if name in ("b", "i", "strike", "outline", "shadow", "condense", "extend"):
                kw[name] = _flag(val) if val is not None else True
            elif name == "u":
                kw["u"] = val or "single"
                if kw["u"] == "none":
                    kw["u"] = None
            elif name == "sz":
                kw["sz"] = float(val)
            elif name == "color":
                kw["color"] = Color._from(child)
            elif name in ("name", "vertAlign", "scheme"):
                kw[name] = val
            elif name in ("family", "charset"):
                kw[name] = float(val) if name == "family" else int(val)
            elif name == "rFont":
                kw["name"] = val
        return cls(**kw)


class Fill(_Value):
    """Base of the fills."""

    @classmethod
    def _from(cls, el):
        for child in el:
            if _local(child.tag) == "patternFill":
                return PatternFill._from(child)
            if _local(child.tag) == "gradientFill":
                return GradientFill._from(child)
        return PatternFill()


class PatternFill(Fill):
    """A fill with a pattern (`solid`, `gray125`, `lightGrid`, ...) in two colours."""

    _fields = ("patternType", "fgColor", "bgColor")
    tagname = "patternFill"

    def __init__(self, patternType=None, fgColor=BLACK, bgColor=BLACK, fill_type=None, start_color=None, end_color=None):
        self.patternType = fill_type if fill_type is not None else patternType
        if self.patternType == "none":
            self.patternType = None  # as openpyxl: no pattern
        self.fgColor = _color(start_color if start_color is not None else fgColor) or Color()
        self.bgColor = _color(end_color if end_color is not None else bgColor) or Color()

    fill_type = property(lambda self: self.patternType)
    start_color = property(lambda self: self.fgColor)
    end_color = property(lambda self: self.bgColor)

    def _xml(self, tag="fill"):
        inner = ""
        if self.fgColor != Color():
            inner += self.fgColor._xml("fgColor")
        if self.bgColor != Color():
            inner += self.bgColor._xml("bgColor")
        kind = ' patternType="{0}"'.format(_esc(self.patternType)) if self.patternType else ""
        body = "<patternFill{0}>{1}</patternFill>".format(kind, inner) if inner else "<patternFill{0}/>".format(kind)
        return "<{0}>{1}</{0}>".format(tag, body)

    @classmethod
    def _from(cls, el):
        kw = {"patternType": el.attrib.get("patternType")}
        for child in el:
            if _local(child.tag) in ("fgColor", "bgColor"):
                kw[_local(child.tag)] = Color._from(child)
        return cls(**kw)


class Stop(_Value):
    """A colour at a position (0 to 1) of a gradient."""

    _fields = ("color", "position")

    def __init__(self, color, position):
        self.color = _color(color)
        self.position = float(position)


class GradientFill(Fill):
    """A gradient: `linear` at an angle (`degree`) or `path` from the edges; `stop` is a
    list of colours (spread evenly) or of `Stop`s."""

    _fields = ("type", "degree", "left", "right", "top", "bottom", "stop")
    tagname = "gradientFill"

    def __init__(self, type="linear", degree=0, left=0, right=0, top=0, bottom=0, stop=(), fill_type=None):
        self.type = fill_type if fill_type is not None else type
        self.degree, self.left, self.right, self.top, self.bottom = float(degree), float(left), float(right), float(top), float(bottom)
        stops = list(stop)
        if stops and not all(isinstance(s, Stop) for s in stops):
            n = len(stops)
            stops = [s if isinstance(s, Stop) else Stop(s, i / (n - 1) if n > 1 else 0) for i, s in enumerate(stops)]
        self.stop = tuple(stops)

    fill_type = property(lambda self: self.type)

    def _xml(self, tag="fill"):
        attrs = ' type="{0}"'.format(_esc(self.type))
        for name in ("degree", "left", "right", "top", "bottom"):
            if getattr(self, name):
                attrs += ' {0}="{1}"'.format(name, _num(getattr(self, name)))
        stops = "".join('<stop position="{0}">{1}</stop>'.format(_num(s.position), s.color._xml()) for s in self.stop)
        return "<{0}><gradientFill{1}>{2}</gradientFill></{0}>".format(tag, attrs, stops)

    @classmethod
    def _from(cls, el):
        a = el.attrib
        stops = []
        for child in el:
            if _local(child.tag) == "stop":
                colour = next((c for c in child if _local(c.tag) == "color"), None)
                stops.append(Stop(Color._from(colour) if colour is not None else Color(), float(child.attrib.get("position", 0))))
        return cls(type=a.get("type", "linear"), degree=float(a.get("degree", 0)), left=float(a.get("left", 0)), right=float(a.get("right", 0)),
                   top=float(a.get("top", 0)), bottom=float(a.get("bottom", 0)), stop=stops)


class Side(_Value):
    """One line of a border: its style (`thin`, `double`, ...) and colour."""

    _fields = ("style", "color")

    def __init__(self, style=None, color=None, border_style=None):
        self.style = style if style is not None else border_style
        self.color = _color(color)

    border_style = property(lambda self: self.style)

    def _xml(self, tag):
        attrs = ' style="{0}"'.format(_esc(self.style)) if self.style else ""
        return "<{0}{1}>{2}</{0}>".format(tag, attrs, self.color._xml()) if self.color is not None else "<{0}{1}/>".format(tag, attrs)

    @classmethod
    def _from(cls, el):
        colour = next((c for c in el if _local(c.tag) == "color"), None)
        style = el.attrib.get("style")
        return cls(style=None if style == "none" else style, color=Color._from(colour) if colour is not None else None)


class Border(_Value):
    """The lines around (and across) a cell."""

    _fields = ("left", "right", "top", "bottom", "diagonal", "vertical", "horizontal", "diagonalUp", "diagonalDown", "outline", "start", "end")
    _SIDES = ("start", "end", "left", "right", "top", "bottom", "diagonal", "vertical", "horizontal")

    def __init__(self, left=None, right=None, top=None, bottom=None, diagonal=None, diagonal_direction=None, vertical=None, horizontal=None,
                 diagonalUp=False, diagonalDown=False, outline=True, start=None, end=None):
        self.left, self.right, self.top, self.bottom = left or Side(), right or Side(), top or Side(), bottom or Side()
        self.diagonal = diagonal or Side()
        self.vertical, self.horizontal, self.start, self.end = vertical, horizontal, start, end
        self.diagonalUp, self.diagonalDown, self.outline = bool(diagonalUp), bool(diagonalDown), bool(outline)
        self.diagonal_direction = diagonal_direction

    def _xml(self, tag="border"):
        attrs = ""
        if self.diagonalUp:
            attrs += ' diagonalUp="1"'
        if self.diagonalDown:
            attrs += ' diagonalDown="1"'
        if not self.outline:
            attrs += ' outline="0"'
        inner = "".join(getattr(self, s)._xml(s) for s in ("left", "right", "top", "bottom", "diagonal"))
        inner += "".join(getattr(self, s)._xml(s) for s in ("vertical", "horizontal") if getattr(self, s) is not None)
        return "<{0}{1}>{2}</{0}>".format(tag, attrs, inner)

    @classmethod
    def _from(cls, el):
        kw = {}
        for child in el:
            name = _local(child.tag)
            if name in cls._SIDES:
                kw[{"start": "left", "end": "right"}.get(name, name)] = Side._from(child)
        a = el.attrib
        return cls(diagonalUp=_flag(a.get("diagonalUp", "0")), diagonalDown=_flag(a.get("diagonalDown", "0")), outline=_flag(a.get("outline", "1")), **kw)


class Alignment(_Value):
    """Where content sits in a cell: horizontal and vertical alignment, rotation, wrap,
    shrink to fit, indent, reading order."""

    _fields = ("horizontal", "vertical", "textRotation", "wrapText", "shrinkToFit", "indent", "relativeIndent", "justifyLastLine", "readingOrder")

    def __init__(self, horizontal=None, vertical=None, textRotation=0, wrapText=None, shrinkToFit=None, indent=0, relativeIndent=0,
                 justifyLastLine=None, readingOrder=0, text_rotation=None, wrap_text=None, shrink_to_fit=None, mergeCell=None):
        self.horizontal, self.vertical = horizontal, vertical
        self.textRotation = int(text_rotation if text_rotation is not None else textRotation or 0)
        self.wrapText = wrap_text if wrap_text is not None else wrapText
        self.shrinkToFit = shrink_to_fit if shrink_to_fit is not None else shrinkToFit
        self.indent, self.relativeIndent, self.readingOrder = float(indent or 0), float(relativeIndent or 0), float(readingOrder or 0)
        self.justifyLastLine = justifyLastLine

    text_rotation = property(lambda self: self.textRotation)
    wrap_text = property(lambda self: self.wrapText)
    shrink_to_fit = property(lambda self: self.shrinkToFit)

    def _xml(self, tag="alignment"):
        attrs = ""
        for name in self._fields:
            value = getattr(self, name)
            if value is None or (name in ("textRotation", "indent", "relativeIndent", "readingOrder") and not value):
                continue
            text = ("1" if value else "0") if isinstance(value, bool) else _num(value) if isinstance(value, (int, float)) else _esc(value)
            attrs += ' {0}="{1}"'.format(name, text)
        return "<{0}{1}/>".format(tag, attrs)

    @classmethod
    def _from(cls, el):
        kw = {}
        for key, value in el.attrib.items():
            if key in ("wrapText", "shrinkToFit", "justifyLastLine"):
                kw[key] = _flag(value)
            elif key in ("textRotation",):
                kw[key] = int(float(value))
            elif key in ("indent", "relativeIndent", "readingOrder"):
                kw[key] = float(value)
            elif key in ("horizontal", "vertical"):
                kw[key] = value
        return cls(**kw)


class Protection(_Value):
    """Whether a cell is locked and its formula hidden when the sheet is protected."""

    _fields = ("locked", "hidden")

    def __init__(self, locked=True, hidden=False):
        self.locked, self.hidden = bool(locked), bool(hidden)

    def _xml(self, tag="protection"):
        return '<{0} locked="{1}" hidden="{2}"/>'.format(tag, int(self.locked), int(self.hidden))

    @classmethod
    def _from(cls, el):
        return cls(locked=_flag(el.attrib.get("locked", "1")), hidden=_flag(el.attrib.get("hidden", "0")))


def _element(xml):
    return _ET.fromstring(xml)


# Excel's (and openpyxl's) default: the theme's body font in the theme's text colour.
DEFAULT_FONT = Font(name="Calibri", sz=11, family=2, color=Color(theme=1), scheme="minor")
DEFAULT_FILL = PatternFill()
DEFAULT_BORDER = Border()
DEFAULT_ALIGNMENT = Alignment()
DEFAULT_PROTECTION = Protection()
