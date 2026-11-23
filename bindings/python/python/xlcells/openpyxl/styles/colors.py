"""Colours: `Color(rgb="FF0000")`, and the indexed palette of the file format."""
from .._styles import Color  # noqa: F401

__all__ = ["Color", "COLOR_INDEX", "BLACK", "WHITE", "BLUE", "RGB", "ColorDescriptor", "RgbColor", "ColorList"]

# The 64 indexed colours (ECMA-376 part 1, 18.8.27), as ARGB with alpha 00.
COLOR_INDEX = (
    "00000000", "00FFFFFF", "00FF0000", "0000FF00", "000000FF", "00FFFF00", "00FF00FF", "0000FFFF",
    "00000000", "00FFFFFF", "00FF0000", "0000FF00", "000000FF", "00FFFF00", "00FF00FF", "0000FFFF",
    "00800000", "00008000", "00000080", "00808000", "00800080", "00008080", "00C0C0C0", "00808080",
    "009999FF", "00993366", "00FFFFCC", "00CCFFFF", "00660066", "00FF8080", "000066CC", "00CCCCFF",
    "00000080", "00FF00FF", "00FFFF00", "0000FFFF", "00800080", "00800000", "00008080", "000000FF",
    "0000CCFF", "00CCFFFF", "00CCFFCC", "00FFFF99", "0099CCFF", "00FF99CC", "00CC99FF", "00FFCC99",
    "003366FF", "0033CCCC", "0099CC00", "00FFCC00", "00FF9900", "00FF6600", "00666699", "00969696",
    "00003366", "00339966", "00003300", "00333300", "00993300", "00993366", "00333399", "00333333",
)
BLACK = COLOR_INDEX[0]
WHITE = COLOR_INDEX[1]
BLUE = COLOR_INDEX[4]
RGB = "^(?:[A-Fa-f0-9]{2})?[A-Fa-f0-9]{6}$"
RgbColor = Color
ColorDescriptor = Color


class ColorList:
    """The custom palette of a workbook (`indexedColors`, `mruColors`)."""

    def __init__(self, indexedColors=(), mruColors=()):
        self.indexedColors = list(indexedColors)
        self.mruColors = list(mruColors)

    @property
    def index(self):
        return [c.rgb if hasattr(c, "rgb") else c for c in self.indexedColors]

    def __bool__(self):
        return bool(self.indexedColors or self.mruColors)
