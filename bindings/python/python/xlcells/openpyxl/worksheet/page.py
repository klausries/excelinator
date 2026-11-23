"""Page margins, print options and page setup of a worksheet (`ws.page_margins`,
`ws.print_options`, `ws.page_setup`)."""
from .._serial import Element

__all__ = ["PageMargins", "PrintOptions", "PrintPageSetup"]


class PageMargins(Element):
    """Margins in inches; the defaults are openpyxl's."""
    tagname = "pageMargins"
    _attrs = (("left", float, 0.75), ("right", float, 0.75), ("top", float, 1.0), ("bottom", float, 1.0), ("header", float, 0.5), ("footer", float, 0.5))


class PrintOptions(Element):
    tagname = "printOptions"
    _attrs = (("horizontalCentered", bool, None), ("verticalCentered", bool, None), ("headings", bool, None), ("gridLines", bool, None), ("gridLinesSet", bool, None))


class PrintPageSetup(Element):
    tagname = "pageSetup"
    _attrs = (
        ("orientation", str, None), ("paperSize", int, None), ("scale", int, None), ("fitToHeight", int, None), ("fitToWidth", int, None),
        ("firstPageNumber", int, None), ("useFirstPageNumber", bool, None), ("paperHeight", str, None), ("paperWidth", str, None),
        ("pageOrder", str, None), ("usePrinterDefaults", bool, None), ("blackAndWhite", bool, None), ("draft", bool, None),
        ("cellComments", str, None), ("errors", str, None), ("horizontalDpi", int, None), ("verticalDpi", int, None), ("copies", int, None),
        ("id", str, None),
    )
    _xml_names = {"id": "r:id"}

    def __init__(self, worksheet=None, *args, **kw):
        self._parent = worksheet
        Element.__init__(self, *args, **kw)

    @property
    def sheet_properties(self):
        return self._parent.sheet_properties if self._parent is not None else None

    @property
    def fitToPage(self):
        props = self.sheet_properties
        return bool(props is not None and props.pageSetUpPr is not None and props.pageSetUpPr.fitToPage)

    @fitToPage.setter
    def fitToPage(self, value):
        from .properties import PageSetupProperties
        props = self.sheet_properties
        if props.pageSetUpPr is None:
            props.pageSetUpPr = PageSetupProperties()
        props.pageSetUpPr.fitToPage = value

    @property
    def autoPageBreaks(self):
        props = self.sheet_properties
        return bool(props is not None and props.pageSetUpPr is not None and props.pageSetUpPr.autoPageBreaks)

    @autoPageBreaks.setter
    def autoPageBreaks(self, value):
        from .properties import PageSetupProperties
        props = self.sheet_properties
        if props.pageSetUpPr is None:
            props.pageSetUpPr = PageSetupProperties()
        props.pageSetUpPr.autoPageBreaks = value
