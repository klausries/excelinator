"""Headers and footers (`ws.oddHeader.center.text = "Seite &P"`, `ws.HeaderFooter`).

The text of a header uses Excel's codes: `&L`, `&C`, `&R` start the left, centre and right
part, `&"Font,Style"`, `&12` and `&KRRGGBB` set font, size and colour, `&P`, `&N`, `&D`,
`&T`, `&A`, `&F` insert page, pages, date, time, sheet and file name, `&&` is an ampersand.
"""
import re

from .._serial import Element, _esc

__all__ = ["HeaderFooter", "HeaderFooterItem", "FONT_PATTERN", "COLOR_PATTERN", "SIZE_REGEX"]

FONT_PATTERN = '&"(?P<font>.+)"'
COLOR_PATTERN = "&K(?P<color>[A-F0-9]{6})"
SIZE_REGEX = r"&(?P<size>\d+\s?)"
_FORMAT = re.compile("{0}|{1}|{2}".format(FONT_PATTERN, COLOR_PATTERN, SIZE_REGEX))
_LONG_CODES = {"&[Tab]": "&A", "&[Pages]": "&N", "&[Date]": "&D", "&[Path]": "&Z", "&[Page]": "&P", "&[Time]": "&T", "&[File]": "&F", "&[Picture]": "&G"}
_LONG = re.compile("|".join(re.escape(k) for k in _LONG_CODES))


def _unescape_x(text):
    return re.sub(r"_x([0-9A-Fa-f]{4})_", lambda m: chr(int(m.group(1), 16)), text)


class _HeaderFooterPart:
    """One of the left, centre and right parts: text with font, size and colour."""

    def __init__(self, text=None, font=None, size=None, color=None):
        self.text, self.font, self.size, self.color = text, font, size, color

    def __str__(self):
        codes = []
        if self.font:
            codes.append('&"{0}"'.format(self.font))
        if self.size:
            codes.append("&{0} ".format(self.size))
        if self.color:
            codes.append("&K{0}".format(self.color))
        return "".join(codes) + (self.text or "")

    def __bool__(self):
        return bool(self.text)

    def __eq__(self, other):
        return isinstance(other, _HeaderFooterPart) and (self.text, self.font, self.size, self.color) == (other.text, other.font, other.size, other.color)

    def __repr__(self):
        return "<_HeaderFooterPart text={0!r}>".format(self.text)

    @classmethod
    def from_str(cls, text):
        found = {}
        for font, color, size in _FORMAT.findall(text):
            if font:
                found["font"] = font
            if color:
                found["color"] = color
            if size:
                found["size"] = int(size)
        return cls(text=_FORMAT.sub("", text), **found)


def _split(text):
    """Left, centre and right parts of a header text; text before any part code is the centre."""
    parts = {}
    current, buf, i = None, [], 0
    while i < len(text):
        if text.startswith("&&", i):
            buf.append("&&")
            i += 2
            continue
        code = text[i:i + 2]
        if code in ("&L", "&C", "&R"):
            if current is not None or buf:
                parts[current or "C"] = parts.get(current or "C", "") + "".join(buf)
            current, buf = code[1], []
            i += 2
            continue
        buf.append(text[i])
        i += 1
    if current is not None or buf:
        parts[current or "C"] = parts.get(current or "C", "") + "".join(buf)
    return parts


class HeaderFooterItem:
    """A header or footer: three parts."""

    def __init__(self, left=None, right=None, center=None):
        self.left = left if left is not None else _HeaderFooterPart()
        self.center = center if center is not None else _HeaderFooterPart()
        self.right = right if right is not None else _HeaderFooterPart()

    @property
    def centre(self):
        return self.center

    @centre.setter
    def centre(self, value):
        self.center = value

    def __str__(self):
        out = []
        for key, part in (("L", self.left), ("C", self.center), ("R", self.right)):
            if part.text is not None:
                out.append("&{0}{1}".format(key, part))
        return _LONG.sub(lambda m: _LONG_CODES[m.group(0)], "".join(out))

    def __bool__(self):
        return any([self.left, self.center, self.right])

    def __eq__(self, other):
        return isinstance(other, HeaderFooterItem) and (self.left, self.center, self.right) == (other.left, other.center, other.right)

    @classmethod
    def _from_text(cls, text):
        parts = _split(_unescape_x(text))
        names = {"L": "left", "C": "center", "R": "right"}
        return cls(**{names[k]: _HeaderFooterPart.from_str(v) for k, v in parts.items()})


class HeaderFooter(Element):
    tagname = "headerFooter"
    _attrs = (("differentOddEven", bool, None), ("differentFirst", bool, None), ("scaleWithDoc", bool, None), ("alignWithMargins", bool, None))
    _ITEMS = ("oddHeader", "oddFooter", "evenHeader", "evenFooter", "firstHeader", "firstFooter")

    def __init__(self, differentOddEven=None, differentFirst=None, scaleWithDoc=None, alignWithMargins=None, **items):
        Element.__init__(self, differentOddEven=differentOddEven, differentFirst=differentFirst, scaleWithDoc=scaleWithDoc, alignWithMargins=alignWithMargins)
        for name in self._ITEMS:
            setattr(self, name, items.pop(name, None) or HeaderFooterItem())
        if items:
            raise TypeError("unexpected keyword argument {0!r}".format(next(iter(items))))

    def _key(self):
        return Element._key(self) + tuple(str(getattr(self, n)) for n in self._ITEMS)

    def _inner_xml(self):
        out = []
        for name in self._ITEMS:
            text = str(getattr(self, name))
            if text:
                out.append("<{0}>{1}</{0}>".format(name, _esc(text, False)))
        return "".join(out) + "".join(self._other_children)

    def __bool__(self):
        return any(getattr(self, a[0]) for a in self._attrs) or any(getattr(self, n) for n in self._ITEMS)

    def _is_empty(self):
        return not self and not self._other

    @classmethod
    def _from_element(cls, el):
        obj = super(HeaderFooter, cls)._from_element(el)
        for name in cls._ITEMS:
            setattr(obj, name, HeaderFooterItem())
        keep = []
        import xml.etree.ElementTree as ET
        for child in el:
            local = child.tag.rsplit("}", 1)[-1]
            if local in cls._ITEMS:
                setattr(obj, local, HeaderFooterItem._from_text(child.text or ""))
            else:
                keep.append(ET.tostring(child, encoding="unicode"))
        obj._other_children = keep
        return obj
