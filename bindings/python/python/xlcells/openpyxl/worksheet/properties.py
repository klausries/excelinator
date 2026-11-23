"""Worksheet properties: tab colour, outline settings, fit to page (`ws.sheet_properties`)."""
from .._serial import Element

__all__ = ["WorksheetProperties", "Outline", "PageSetupProperties"]


class _TabColor(Element):
    tagname = "tabColor"
    _attrs = (("rgb", str, None), ("indexed", int, None), ("theme", int, None), ("tint", float, None), ("auto", bool, None))


class Outline(Element):
    tagname = "outlinePr"
    _attrs = (("applyStyles", bool, None), ("summaryBelow", bool, None), ("summaryRight", bool, None), ("showOutlineSymbols", bool, None))


class PageSetupProperties(Element):
    tagname = "pageSetUpPr"
    _attrs = (("autoPageBreaks", bool, None), ("fitToPage", bool, None))


class WorksheetProperties(Element):
    tagname = "sheetPr"
    _attrs = (
        ("codeName", str, None), ("enableFormatConditionsCalculation", bool, None), ("filterMode", bool, None), ("published", bool, None),
        ("syncHorizontal", bool, None), ("syncRef", str, None), ("syncVertical", bool, None), ("transitionEvaluation", bool, None),
        ("transitionEntry", bool, None),
    )
    _children = (("tabColor", _TabColor), ("outlinePr", Outline), ("pageSetUpPr", PageSetupProperties))

    def __init__(self, *args, **kw):
        tab = kw.pop("tabColor", None)
        Element.__init__(self, *args, **kw)
        if self.outlinePr is None:
            self.outlinePr = Outline(summaryBelow=True, summaryRight=True)
        if self.pageSetUpPr is None:
            self.pageSetUpPr = PageSetupProperties()
        if tab is not None:
            self.tabColor = tab

    def __setattr__(self, name, value):
        if name == "tabColor" and value is not None and not isinstance(value, _TabColor):
            rgb = getattr(value, "rgb", value)
            rgb = str(rgb)
            value = _TabColor(rgb="00" + rgb if len(rgb) == 6 else rgb)
        object.__setattr__(self, name, value)
