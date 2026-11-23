"""Text inside a cell: the font of a formatted run (`InlineFont`), runs, phonetic guides,
and formatted text (`CellRichText` of `str` and `TextBlock` parts).

Behaves like openpyxl's classes of the same names (same constructor forms, merging of
neighbours with equal formatting, `as_list`, `+`, `+=`)."""
import xml.etree.ElementTree as _ET
from copy import copy

from ._styles import Font, _Value, _color, _esc, _flag, _local, _num, Color, _NS  # noqa: F401
from ._xstring import t_element, unescape


_FLAGS = ("b", "i", "strike", "outline", "shadow", "condense", "extend")


class InlineFont(_Value):
    """The font of a run (`<rPr>`): like `Font`, with the name in `rFont`. Unset
    attributes are None and come from the cell's font."""

    _fields = ("rFont", "charset", "family", "b", "i", "strike", "outline", "shadow", "condense", "extend", "color", "sz", "u", "vertAlign", "scheme")
    tagname = "RPrElt"

    def __init__(self, rFont=None, charset=None, family=None, b=None, i=None, strike=None, outline=None, shadow=None, condense=None,
                 extend=None, color=None, sz=None, u=None, vertAlign=None, scheme=None):
        self.rFont = rFont
        self.charset = None if charset is None else int(charset)
        self.family = None if family is None else float(family)
        self.b, self.i = bool(b), bool(i)
        self.strike, self.outline, self.shadow, self.condense, self.extend = strike, outline, shadow, condense, extend
        self.color = _color(color)
        self.sz = None if sz is None else float(sz)
        self.u, self.vertAlign, self.scheme = u, vertAlign, scheme

    name = property(lambda self: self.rFont)
    bold = property(lambda self: self.b)
    italic = property(lambda self: self.i)
    size = property(lambda self: self.sz)
    underline = property(lambda self: self.u)

    def _xml(self, tag="rPr"):
        x = []
        if self.rFont is not None:
            x.append('<rFont val="{0}"/>'.format(_esc(self.rFont)))
        if self.charset is not None:
            x.append('<charset val="{0}"/>'.format(self.charset))
        if self.family is not None:
            x.append('<family val="{0}"/>'.format(_num(self.family)))
        for name in _FLAGS:
            value = getattr(self, name)
            if value:
                x.append("<{0}/>".format(name))
            elif value is not None and name not in ("b", "i"):
                x.append('<{0} val="0"/>'.format(name))
        if self.color is not None:
            x.append(self.color._xml())
        if self.sz is not None:
            x.append('<sz val="{0}"/>'.format(_num(self.sz)))
        if self.u:
            x.append("<u/>" if self.u == "single" else '<u val="{0}"/>'.format(_esc(self.u)))
        if self.vertAlign:
            x.append('<vertAlign val="{0}"/>'.format(_esc(self.vertAlign)))
        if self.scheme:
            x.append('<scheme val="{0}"/>'.format(_esc(self.scheme)))
        return "<{0}>{1}</{0}>".format(tag, "".join(x))

    @classmethod
    def _from(cls, el):
        kw = {}
        for child in el:
            name, val = _local(child.tag), child.attrib.get("val")
            if name in _FLAGS:
                kw[name] = _flag(val) if val is not None else True
            elif name == "u":
                kw["u"] = None if val == "none" else (val or "single")
            elif name == "color":
                kw["color"] = Color._from(child)
            elif name in ("rFont", "vertAlign", "scheme"):
                kw[name] = val
            elif name == "name":  # some writers use the element of <font>
                kw["rFont"] = val
            elif name in ("sz", "family", "charset") and val is not None:
                kw[name] = float(val) if name != "charset" else int(val)
        return cls(**kw)


class RichText(_Value):
    """One run: its font (`rPr`, alias `font`) and text (`t`, alias `text`)."""

    _fields = ("rPr", "t")
    tagname = "RElt"

    def __init__(self, rPr=None, t=None):
        self.rPr, self.t = rPr, t

    font = property(lambda self: self.rPr)
    text = property(lambda self: self.t)


class PhoneticText(_Value):
    """A phonetic guide (`<rPh>`) over the characters sb to eb."""

    _fields = ("sb", "eb", "t")
    tagname = "rPh"

    def __init__(self, sb=None, eb=None, t=None):
        self.sb, self.eb, self.t = sb, eb, t

    text = property(lambda self: self.t)


class PhoneticProperties(_Value):
    _fields = ("fontId", "type", "alignment")
    tagname = "phoneticPr"

    def __init__(self, fontId=None, type=None, alignment=None):
        self.fontId, self.type, self.alignment = fontId, type, alignment


class Text(_Value):
    """A string item: plain text (`t`) or runs (`r`), with phonetic guides."""

    _fields = ("t", "r", "rPh", "phoneticPr")
    tagname = "text"

    def __init__(self, t=None, r=(), rPh=(), phoneticPr=None):
        self.t, self.r, self.rPh, self.phoneticPr = t, list(r), list(rPh), phoneticPr

    plain = property(lambda self: self.t)
    formatted = property(lambda self: self.r)
    phonetic = property(lambda self: self.rPh)

    @property
    def content(self):
        """The text without formatting."""
        return (self.t or "") + "".join(r.t or "" for r in self.r)

    @classmethod
    def _from(cls, el):
        t, runs, guides, props = None, [], [], None
        for child in el:
            name = _local(child.tag)
            if name == "t":
                t = unescape(child.text or "")
            elif name == "r":
                font, text = None, None
                for part in child:
                    if _local(part.tag) == "rPr":
                        font = InlineFont._from(part)
                    elif _local(part.tag) == "t":
                        text = unescape(part.text or "")
                runs.append(RichText(font, text))
            elif name == "rPh":
                text = next((unescape(p.text or "") for p in child if _local(p.tag) == "t"), None)
                guides.append(PhoneticText(int(child.attrib.get("sb", 0)), int(child.attrib.get("eb", 0)), text))
            elif name == "phoneticPr":
                a = child.attrib
                props = PhoneticProperties(int(a["fontId"]) if "fontId" in a else None, a.get("type"), a.get("alignment"))
        return cls(t, runs, guides, props)


def _run_xml(font, text):
    return "<r>{0}{1}</r>".format(font._xml() if font is not None else "", t_element(text))


_NUMERIC = (int, float)


class TextBlock:
    """Text in one format."""

    def __init__(self, font, text):
        if not isinstance(font, InlineFont):
            raise TypeError("expected InlineFont, got {0!r}".format(font))
        self.font = font
        self.text = str(text)

    def __eq__(self, other):
        return isinstance(other, TextBlock) and self.text == other.text and self.font == other.font

    def __hash__(self):
        return hash((self.text, self.font))

    def __str__(self):
        return self.text

    def __repr__(self):
        font = self.font if self.font != InlineFont() else "default"
        return "{0} text={1}, font={2}".format(type(self).__name__, self.text, font)

    def __copy__(self):
        return TextBlock(self.font, self.text)


class CellRichText(list):
    """A rich text string: a list of `str` and `TextBlock` parts. Built from a list, a
    tuple, several arguments, or one part."""

    def __init__(self, *args):
        if len(args) == 1:
            args = args[0]
            if isinstance(args, (list, tuple)):
                CellRichText._check_rich_text(args)
            else:
                CellRichText._check_element(args)
                args = [args]
        else:
            CellRichText._check_rich_text(args)
        super().__init__(args)

    @classmethod
    def _check_element(cls, value):
        if not isinstance(value, (str, TextBlock) + _NUMERIC):
            raise TypeError("Illegal CellRichText element {0}".format(value))

    @classmethod
    def _check_rich_text(cls, rich_text):
        for t in rich_text:
            CellRichText._check_element(t)

    @classmethod
    def from_tree(cls, node):
        """From a string item (`<si>` or `<is>` element)."""
        text = Text._from(node)
        if text.t:
            return (text.t,)
        parts = []
        for r in text.r:
            t = r.t or ""
            parts.append(TextBlock(r.rPr, t) if r.rPr is not None else t)
        return cls(parts)

    def _opt(self):
        """Merge neighbours with the same format; drop empty parts."""
        out = []
        for t in self:
            if (isinstance(t, str) and not t) or (isinstance(t, TextBlock) and not t.text):
                continue
            if out and type(out[-1]) is type(t):
                if isinstance(t, str):
                    out[-1] += t
                    continue
                if isinstance(t, TextBlock) and out[-1].font == t.font:
                    out[-1] = TextBlock(t.font, out[-1].text + t.text)
                    continue
            out.append(t)
        super().__setitem__(slice(None), out)
        return self

    def __iadd__(self, arg):
        CellRichText._check_rich_text(arg)
        super().__iadd__([copy(e) for e in list(arg)])
        return self._opt()

    def __add__(self, arg):
        return CellRichText([copy(e) for e in list(self) + list(arg)])._opt()

    def __setitem__(self, index, value):
        CellRichText._check_element(value)
        super().__setitem__(index, value)
        self._opt()

    def append(self, arg):
        CellRichText._check_element(arg)
        super().append(arg)

    def extend(self, arg):
        CellRichText._check_rich_text(arg)
        super().extend(arg)

    def __repr__(self):
        return "CellRichText([{0}])".format(", ".join(repr(s) for s in self))

    def __str__(self):
        return "".join(str(s) for s in self)

    def as_list(self):
        """The parts as plain strings."""
        return [str(s) for s in self]

    def _xml(self):
        """The content of the string item: one run per part."""
        return "".join(_run_xml(p.font if isinstance(p, TextBlock) else None, str(p)) for p in self)


def _from_xml(xml):
    """The value openpyxl gives a string item with `rich_text=True`: a plain `str` when
    it holds no runs or one unformatted run, else `CellRichText`."""
    node = _ET.fromstring('<si xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{0}</si>'.format(xml))
    value = CellRichText.from_tree(node)
    if len(value) == 0:
        return ""
    if len(value) == 1 and isinstance(value[0], str):
        return value[0]
    return value
