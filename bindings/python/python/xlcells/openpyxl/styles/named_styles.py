"""Named cell styles (`NamedStyle("Hervorhebung", font=Font(bold=True))`, `cell.style = "Good"`)."""
from .._styles import Alignment, Border, Font, PatternFill, Protection, _Value

__all__ = ["NamedStyle", "NamedStyleList"]


class NamedStyle(_Value):
    """A cell format with a name; `builtinId` marks Excel's own styles."""

    _fields = ("name", "font", "fill", "border", "alignment", "number_format", "protection", "builtinId", "hidden")

    def __init__(self, name="Normal", font=None, fill=None, border=None, alignment=None, number_format=None, protection=None,
                 builtinId=None, hidden=False, xfId=None):
        self.name = name
        self.font = font if font is not None else Font()
        self.fill = fill if fill is not None else PatternFill()
        self.border = border if border is not None else Border()
        self.alignment = alignment if alignment is not None else Alignment()
        self.number_format = number_format or "General"
        self.protection = protection if protection is not None else Protection()
        self.builtinId = builtinId
        self.hidden = bool(hidden)
        self._wb = None

    def bind(self, wb):
        self._wb = wb

    def __repr__(self):
        return "NamedStyle({0!r})".format(self.name)


class NamedStyleList(list):
    """The named styles of a workbook; `names` lists their names."""

    @property
    def names(self):
        return [s.name for s in self]

    def __getitem__(self, key):
        if isinstance(key, int):
            return list.__getitem__(self, key)
        for style in self:
            if style.name == key:
                return style
        raise KeyError("No named style with the name {0} exists".format(key))

    def append(self, style):
        if not isinstance(style, NamedStyle):
            raise TypeError("Only NamedStyle instances can be added")
        if style.name in self.names:
            raise ValueError("Style {0} exists already".format(style.name))
        list.append(self, style)
