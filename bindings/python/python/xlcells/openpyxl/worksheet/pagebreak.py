"""Manual page breaks (`ws.row_breaks`, `ws.col_breaks`)."""
from .._serial import Element

__all__ = ["Break", "RowBreak", "ColBreak", "PageBreak"]


class Break(Element):
    tagname = "brk"
    _attrs = (("id", int, 0), ("min", int, 0), ("max", int, 16383), ("man", bool, True), ("pt", bool, None))


class RowBreak(Element):
    tagname = "rowBreaks"
    _children = (("brk", Break),)
    _lists = ("brk",)

    def __init__(self, count=None, manualBreakCount=None, brk=()):
        Element.__init__(self)
        self.brk = list(brk)

    def __len__(self):
        return len(self.brk)

    def __bool__(self):
        return bool(self.brk)

    @property
    def count(self):
        return len(self.brk)

    @property
    def manualBreakCount(self):
        return len(self.brk)

    def append(self, brk=None):
        self.brk.append(brk if brk is not None else Break(id=self.count + 1))

    def _attr_xml(self):
        return ' count="{0}" manualBreakCount="{1}"'.format(len(self.brk), sum(1 for b in self.brk if b.man)) + "".join(
            ' {0}="{1}"'.format(k, v) for k, v in self._other.items() if k not in ("count", "manualBreakCount"))

    def _loaded(self):
        self.brk = self.brk or []
        self._other.pop("count", None)
        self._other.pop("manualBreakCount", None)


PageBreak = RowBreak


class ColBreak(RowBreak):
    tagname = "colBreaks"
