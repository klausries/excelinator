"""Sheet views: zoom, grid lines, frozen panes, selection (`ws.sheet_view`)."""
from .._serial import Element

__all__ = ["SheetView", "SheetViewList", "Pane", "Selection"]


class Pane(Element):
    tagname = "pane"
    _attrs = (("xSplit", float, None), ("ySplit", float, None), ("topLeftCell", str, None), ("activePane", str, "topLeft"), ("state", str, "split"))


class Selection(Element):
    tagname = "selection"
    _attrs = (("pane", str, None), ("activeCell", str, "A1"), ("activeCellId", int, None), ("sqref", str, "A1"))


class SheetView(Element):
    tagname = "sheetView"
    _attrs = (
        ("windowProtection", bool, None), ("showFormulas", bool, None), ("showGridLines", bool, None), ("showRowColHeaders", bool, None),
        ("showZeros", bool, None), ("rightToLeft", bool, None), ("tabSelected", bool, None), ("showRuler", bool, None),
        ("showOutlineSymbols", bool, None), ("defaultGridColor", bool, None), ("showWhiteSpace", bool, None), ("view", str, None),
        ("topLeftCell", str, None), ("colorId", int, None), ("zoomScale", int, None), ("zoomScaleNormal", int, None),
        ("zoomScaleSheetLayoutView", int, None), ("zoomScalePageLayoutView", int, None), ("zoomToFit", bool, None),
        ("workbookViewId", int, 0),
    )
    _children = (("pane", Pane), ("selection", Selection))
    _lists = ("selection",)

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        if self.selection is None:
            self.selection = [Selection()]
        elif not isinstance(self.selection, list):
            self.selection = list(self.selection)

    def _loaded(self):
        if self.selection is None:
            self.selection = []


class SheetViewList(Element):
    tagname = "sheetViews"
    _children = (("sheetView", SheetView),)
    _lists = ("sheetView",)

    def __init__(self, sheetView=None, **kw):
        Element.__init__(self, **kw)
        self.sheetView = list(sheetView) if sheetView is not None else [SheetView()]

    def _loaded(self):
        if not self.sheetView:
            self.sheetView = [SheetView()]

    @property
    def active(self):
        return self.sheetView[0]
