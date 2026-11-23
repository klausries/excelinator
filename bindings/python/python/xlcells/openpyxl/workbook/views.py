"""Workbook windows (`wb.views`, a list of `BookView`): size, position, scroll bars, sheet
tabs, first visible and active tab."""
from .._serial import Element

__all__ = ["BookView", "CustomWorkbookView"]


class BookView(Element):
    tagname = "workbookView"
    _attrs = (
        ("visibility", str, "visible"), ("minimized", bool, False), ("showHorizontalScroll", bool, True), ("showVerticalScroll", bool, True),
        ("showSheetTabs", bool, True), ("xWindow", int, None), ("yWindow", int, None), ("windowWidth", int, None), ("windowHeight", int, None),
        ("tabRatio", int, 600), ("firstSheet", int, 0), ("activeTab", int, 0), ("autoFilterDateGrouping", bool, True),
    )


class CustomWorkbookView(Element):
    tagname = "customWorkbookView"
    _attrs = (("name", str, None), ("guid", str, None), ("autoUpdate", bool, None), ("mergeInterval", int, None), ("changesSavedWin", bool, None),
              ("onlySync", bool, None), ("personalView", bool, None), ("includePrintSettings", bool, None), ("includeHiddenRowCol", bool, None),
              ("maximized", bool, None), ("minimized", bool, None), ("showHorizontalScroll", bool, None), ("showVerticalScroll", bool, None),
              ("showSheetTabs", bool, None), ("xWindow", int, None), ("yWindow", int, None), ("windowWidth", int, None), ("windowHeight", int, None),
              ("tabRatio", int, None), ("activeSheetId", int, None), ("showFormulaBar", bool, None), ("showStatusbar", bool, None),
              ("showComments", str, None), ("showObjects", str, None))


class _BookViews(Element):
    tagname = "bookViews"
    _children = (("workbookView", BookView),)
    _lists = ("workbookView",)
