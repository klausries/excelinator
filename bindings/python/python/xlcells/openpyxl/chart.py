"""Charts in the shape of openpyxl.chart: BarChart, LineChart, PieChart, ScatterChart
with Reference and Series.

    from xlcells.openpyxl.chart import BarChart, Reference
    chart = BarChart()
    chart.title = "Umsatz"
    chart.add_data(Reference(ws, min_col=2, min_row=1, max_col=3, max_row=7), titles_from_data=True)
    chart.set_categories(Reference(ws, min_col=1, min_row=2, max_row=7))
    ws.add_chart(chart, "E2")

Written: chart type, series with their cell references and names, categories, chart
title, axis titles, bar direction and grouping, legend, position and size. Colours,
data labels, trend lines, secondary axes, 3D and the other chart types are not
written yet. A chart read from a file is kept as it is and cannot be edited.
"""
from .utils import coordinate_to_tuple, get_column_letter

__all__ = ["BarChart", "LineChart", "PieChart", "ScatterChart", "Reference", "Series"]

_NS = ('xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
       'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"')
_EMU_PER_CM = 360000
# Series colours are written out, so that a chart does not depend on a theme part
# (without one, some programs draw the series invisibly). The Office palette.
_PALETTE = ("4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47")


def _fill(index):
    return '<a:solidFill><a:srgbClr val="{0}"/></a:solidFill>'.format(_PALETTE[index % len(_PALETTE)])


def _esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


class Reference:
    """A block of cells on a sheet, 1-based and inclusive."""

    def __init__(self, worksheet=None, min_col=None, min_row=None, max_col=None, max_row=None, range_string=None):
        if range_string is not None:
            sheet, _, cells = range_string.rpartition("!")
            first, _, last = cells.replace("$", "").partition(":")
            min_row, min_col = coordinate_to_tuple(first)
            max_row, max_col = coordinate_to_tuple(last or first)
            self._title = sheet.strip("'").replace("''", "'")
        else:
            self._title = worksheet.title
        self.worksheet = worksheet
        self.min_col, self.min_row = min_col, min_row
        self.max_col = max_col if max_col is not None else min_col
        self.max_row = max_row if max_row is not None else min_row

    def _copy(self, **changes):
        new = Reference.__new__(Reference)
        new.__dict__.update(self.__dict__)
        new.__dict__.update(changes)
        return new

    def __str__(self):
        sheet = "'{0}'".format(self._title.replace("'", "''"))
        first = "${0}${1}".format(get_column_letter(self.min_col), self.min_row)
        if (self.min_col, self.min_row) == (self.max_col, self.max_row):
            return "{0}!{1}".format(sheet, first)
        return "{0}!{1}:${2}${3}".format(sheet, first, get_column_letter(self.max_col), self.max_row)

    def __len__(self):
        return (self.max_col - self.min_col + 1) if self.min_row == self.max_row else (self.max_row - self.min_row + 1)

    @property
    def rows(self):
        for row in range(self.min_row, self.max_row + 1):
            yield self._copy(min_row=row, max_row=row)

    @property
    def cols(self):
        for col in range(self.min_col, self.max_col + 1):
            yield self._copy(min_col=col, max_col=col)

    @property
    def sheetname(self):
        return "'{0}'".format(self._title.replace("'", "''"))

    def pop(self):
        """Take the first cell off the block and return its coordinate, as openpyxl does."""
        first = "{0}{1}".format(get_column_letter(self.min_col), self.min_row)
        if self.min_row == self.max_row:
            self.min_col += 1
        else:
            self.min_row += 1
        return first


class _Series:
    def __init__(self, values, xvalues=None, title=None, title_ref=None):
        self.values, self.xvalues, self.title, self.title_ref = values, xvalues, title, title_ref
        self.categories = None


def Series(values, xvalues=None, zvalues=None, title=None, title_from_data=False):
    """A data series; with `title_from_data` the first cell of `values` is its name."""
    title_ref = None
    if title_from_data:
        values = values._copy()
        title_ref = "{0}!{1}".format(values.sheetname, values.pop())
    return _Series(values, xvalues, title, title_ref)


class _Axis:
    def __init__(self):
        self.title = None


class _Legend:
    def __init__(self):
        self.position = "r"


class _Chart:
    _tag = None

    def __init__(self):
        self.title = None
        self.series = []
        self.x_axis, self.y_axis = _Axis(), _Axis()
        self.legend = _Legend()
        self.width, self.height = 15, 7.5  # centimetres
        self.anchor = "E15"
        self.style = None

    ser = property(lambda self: self.series)

    def append(self, series):
        self.series.append(series)

    def add_data(self, data, from_rows=False, titles_from_data=False):
        for block in (data.rows if from_rows else data.cols):
            self.series.append(Series(block, title_from_data=titles_from_data))

    def set_categories(self, labels):
        for series in self.series:
            series.categories = labels

    # --- XML -----------------------------------------------------------------
    @staticmethod
    def _title(text):
        if text is None:
            return ""
        return "<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>{0}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title>".format(_esc(text))

    @staticmethod
    def _source(tag, ref, kind="numRef"):
        return "" if ref is None else "<c:{0}><c:{1}><c:f>{2}</c:f></c:{1}></c:{0}>".format(tag, kind, _esc(ref))

    _outline = False  # series drawn as lines rather than areas

    def _series_head(self, index, series):
        x = '<c:ser><c:idx val="{0}"/><c:order val="{0}"/>'.format(index)
        if series.title_ref is not None:
            x += "<c:tx><c:strRef><c:f>{0}</c:f></c:strRef></c:tx>".format(_esc(series.title_ref))
        elif series.title is not None:
            x += "<c:tx><c:v>{0}</c:v></c:tx>".format(_esc(series.title))
        if self._outline:
            return x + '<c:spPr><a:ln w="28575" cap="rnd">{0}<a:round/></a:ln></c:spPr>'.format(_fill(index))
        return x + "<c:spPr>{0}</c:spPr>".format(_fill(index))

    def _series(self, index, series):
        return self._series_head(index, series) + self._source("cat", series.categories) + self._source("val", series.values) + "</c:ser>"

    def _axis(self, tag, ident, cross, position, axis, gridlines=False):
        return ('<c:{0}><c:axId val="{1}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="{2}"/>{3}{4}'
                '<c:crossAx val="{5}"/></c:{0}>').format(tag, ident, position, "<c:majorGridlines/>" if gridlines else "", self._title(axis.title), cross)

    def _axes(self):
        return self._axis("catAx", 10, 100, "b", self.x_axis) + self._axis("valAx", 100, 10, "l", self.y_axis, gridlines=True)

    def _plot(self):
        raise NotImplementedError

    def _xml(self):
        legend = "" if self.legend is None else '<c:legend><c:legendPos val="{0}"/></c:legend>'.format(self.legend.position)
        return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<c:chartSpace {0}><c:chart>{1}<c:autoTitleDeleted val="{2}"/>'
                '<c:plotArea><c:layout/>{3}</c:plotArea>{4}<c:plotVisOnly val="1"/></c:chart></c:chartSpace>').format(
                    _NS, self._title(self.title), 0 if self.title is not None else 1, self._plot(), legend)

    def _drawing(self):
        where = self.anchor if isinstance(self.anchor, str) else self.anchor.coordinate
        row, column = coordinate_to_tuple(where)
        return ((column - 1, 0, row - 1, 0), None, (int(self.width * _EMU_PER_CM), int(self.height * _EMU_PER_CM)), None, self._xml())


class BarChart(_Chart):
    def __init__(self):
        super().__init__()
        self.type = "col"  # "col" upright, "bar" lying
        self.grouping = "clustered"
        self.gapWidth = 150
        self.overlap = None

    def _axes(self):
        # Lying bars: categories run up the left side, values along the bottom.
        cat, val = ("l", "b") if self.type == "bar" else ("b", "l")
        return self._axis("catAx", 10, 100, cat, self.x_axis) + self._axis("valAx", 100, 10, val, self.y_axis, gridlines=True)

    def _plot(self):
        series = "".join(self._series(i, s) for i, s in enumerate(self.series))
        overlap = "" if self.overlap is None else '<c:overlap val="{0}"/>'.format(int(self.overlap))
        return ('<c:barChart><c:barDir val="{0}"/><c:grouping val="{1}"/><c:varyColors val="0"/>{2}<c:gapWidth val="{3}"/>{4}'
                '<c:axId val="10"/><c:axId val="100"/></c:barChart>').format(self.type, self.grouping, series, int(self.gapWidth), overlap) + self._axes()


class LineChart(_Chart):
    _outline = True

    def __init__(self):
        super().__init__()
        self.grouping = "standard"

    def _series(self, index, series):
        return (self._series_head(index, series) + '<c:marker><c:symbol val="none"/></c:marker>' + self._source("cat", series.categories)
                + self._source("val", series.values) + '<c:smooth val="0"/></c:ser>')

    def _plot(self):
        series = "".join(self._series(i, s) for i, s in enumerate(self.series))
        return '<c:lineChart><c:grouping val="{0}"/><c:varyColors val="0"/>{1}<c:axId val="10"/><c:axId val="100"/></c:lineChart>'.format(self.grouping, series) + self._axes()


class PieChart(_Chart):
    def _series(self, index, series):
        head = '<c:ser><c:idx val="{0}"/><c:order val="{0}"/>'.format(index)
        if series.title_ref is not None:
            head += "<c:tx><c:strRef><c:f>{0}</c:f></c:strRef></c:tx>".format(_esc(series.title_ref))
        elif series.title is not None:
            head += "<c:tx><c:v>{0}</c:v></c:tx>".format(_esc(series.title))
        # one colour per slice
        points = "".join('<c:dPt><c:idx val="{0}"/><c:bubble3D val="0"/><c:spPr>{1}</c:spPr></c:dPt>'.format(i, _fill(i)) for i in range(len(series.values)))
        return head + points + self._source("cat", series.categories) + self._source("val", series.values) + "</c:ser>"

    def _plot(self):
        return '<c:pieChart><c:varyColors val="1"/>{0}<c:firstSliceAng val="0"/></c:pieChart>'.format("".join(self._series(i, s) for i, s in enumerate(self.series)))


class ScatterChart(_Chart):
    _outline = True

    def __init__(self):
        super().__init__()
        self.scatterStyle = "lineMarker"

    def _series(self, index, series):
        return self._series_head(index, series) + self._source("xVal", series.xvalues) + self._source("yVal", series.values) + '<c:smooth val="0"/></c:ser>'

    def _axes(self):
        return self._axis("valAx", 10, 100, "b", self.x_axis) + self._axis("valAx", 100, 10, "l", self.y_axis, gridlines=True)

    def _plot(self):
        series = "".join(self._series(i, s) for i, s in enumerate(self.series))
        return '<c:scatterChart><c:scatterStyle val="{0}"/><c:varyColors val="0"/>{1}<c:axId val="10"/><c:axId val="100"/></c:scatterChart>'.format(self.scatterStyle, series) + self._axes()
