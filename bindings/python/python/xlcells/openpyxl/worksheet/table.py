"""Tables (list objects): `Table`, `TableColumn`, `TableStyleInfo`, `ws.add_table`,
`ws.tables`, as openpyxl's `worksheet.table`. Attributes and children the classes do not
know are kept as read; a table left unchanged is written as it was read."""
import warnings

from .._serial import Element, TextElement, _esc
from .._xstring import escape_controls, unescape
from ..utils import range_boundaries
from .filters import AutoFilter, SortState

__all__ = ["Table", "TableColumn", "TableStyleInfo", "TableFormula", "XMLColumnProps", "TableList", "TablePartList", "TABLESTYLES", "PIVOTSTYLES"]

TABLESTYLES = tuple(["TableStyleMedium{0}".format(i) for i in range(1, 29)] + ["TableStyleLight{0}".format(i) for i in range(1, 22)]
                    + ["TableStyleDark{0}".format(i) for i in range(1, 12)])
PIVOTSTYLES = tuple(["PivotStyleMedium{0}".format(i) for i in range(1, 29)] + ["PivotStyleLight{0}".format(i) for i in range(1, 29)]
                    + ["PivotStyleDark{0}".format(i) for i in range(1, 29)])

_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"


class TableStyleInfo(Element):
    tagname = "tableStyleInfo"
    _attrs = (("name", str, None), ("showFirstColumn", bool, None), ("showLastColumn", bool, None), ("showRowStripes", bool, None),
              ("showColumnStripes", bool, None))


class XMLColumnProps(Element):
    tagname = "xmlColumnPr"
    _attrs = (("mapId", int, None), ("xpath", str, None), ("denormalized", bool, None), ("xmlDataType", str, None))


class TableFormula(TextElement):
    """A formula of a column (`calculatedColumnFormula`, `totalsRowFormula`); `text` (alias
    `attr_text`) without the leading `=`."""

    tagname = "tableFormula"
    _attrs = (("array", bool, None),)

    def __init__(self, array=None, attr_text=None):
        TextElement.__init__(self, array=array)
        self.text = attr_text

    attr_text = property(lambda self: self.text, lambda self, v: setattr(self, "text", v))


class TableColumn(Element):
    tagname = "tableColumn"
    _attrs = (("id", int, None), ("uniqueName", str, None), ("name", str, None), ("totalsRowFunction", str, None), ("totalsRowLabel", str, None),
              ("queryTableFieldId", int, None), ("headerRowDxfId", int, None), ("dataDxfId", int, None), ("totalsRowDxfId", int, None),
              ("headerRowCellStyle", str, None), ("dataCellStyle", str, None), ("totalsRowCellStyle", str, None))
    _children = (("calculatedColumnFormula", TableFormula), ("totalsRowFormula", TableFormula), ("xmlColumnPr", XMLColumnProps))
    _FUNCTIONS = ("sum", "min", "max", "average", "count", "countNums", "stdDev", "var", "custom")

    def __init__(self, *args, **kw):
        kw.pop("extLst", None)
        Element.__init__(self, *args, **kw)
        if self.totalsRowFunction is not None and self.totalsRowFunction not in self._FUNCTIONS:
            raise ValueError("Value must be one of {0}".format(set(self._FUNCTIONS)))

    def _attr_xml(self):
        # Control characters in the name go as _xHHHH_ (an attribute would lose line breaks).
        name = self.name
        try:
            if name is not None:
                self.name = escape_controls(name)
            return Element._attr_xml(self)
        finally:
            self.name = name

    def _loaded(self):
        if self.name is not None:
            self.name = unescape(self.name)


class _TableColumns(Element):
    tagname = "tableColumns"
    _children = (("tableColumn", TableColumn),)
    _lists = ("tableColumn",)


class Table(Element):
    """A table over `ref` (such as `A1:D10`) with a display name (no spaces), its columns,
    filter, sort state and style."""

    tagname = "table"
    mime_type = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"
    _path = "/tables/table{0}.xml"
    _attrs = (("id", int, 1), ("displayName", str, None), ("ref", str, None), ("name", str, None), ("comment", str, None), ("tableType", str, None),
              ("headerRowCount", int, 1), ("insertRow", bool, None), ("insertRowShift", bool, None), ("totalsRowCount", int, None),
              ("totalsRowShown", bool, None), ("published", bool, None), ("headerRowDxfId", int, None), ("dataDxfId", int, None),
              ("totalsRowDxfId", int, None), ("headerRowBorderDxfId", int, None), ("tableBorderDxfId", int, None), ("totalsRowBorderDxfId", int, None),
              ("headerRowCellStyle", str, None), ("dataCellStyle", str, None), ("totalsRowCellStyle", str, None), ("connectionId", int, None))
    _children = (("autoFilter", AutoFilter), ("sortState", SortState), ("tableColumns", _TableColumns), ("tableStyleInfo", TableStyleInfo))

    def __init__(self, id=1, displayName=None, ref=None, name=None, comment=None, tableType=None, headerRowCount=1, insertRow=None,
                 insertRowShift=None, totalsRowCount=None, totalsRowShown=None, published=None, headerRowDxfId=None, dataDxfId=None,
                 totalsRowDxfId=None, headerRowBorderDxfId=None, tableBorderDxfId=None, totalsRowBorderDxfId=None, headerRowCellStyle=None,
                 dataCellStyle=None, totalsRowCellStyle=None, connectionId=None, autoFilter=None, sortState=None, tableColumns=(),
                 tableStyleInfo=None, extLst=None):
        Element.__init__(self, id=id, ref=ref, comment=comment, tableType=tableType, headerRowCount=headerRowCount, insertRow=insertRow,
                         insertRowShift=insertRowShift, totalsRowCount=totalsRowCount, totalsRowShown=totalsRowShown, published=published,
                         headerRowDxfId=headerRowDxfId, dataDxfId=dataDxfId, totalsRowDxfId=totalsRowDxfId, headerRowBorderDxfId=headerRowBorderDxfId,
                         tableBorderDxfId=tableBorderDxfId, totalsRowBorderDxfId=totalsRowBorderDxfId, headerRowCellStyle=headerRowCellStyle,
                         dataCellStyle=dataCellStyle, totalsRowCellStyle=totalsRowCellStyle, connectionId=connectionId, autoFilter=autoFilter,
                         sortState=sortState, tableStyleInfo=tableStyleInfo)
        if tableType is not None and tableType not in ("worksheet", "xml", "queryTable"):
            raise ValueError("Value must be one of {'worksheet', 'xml', 'queryTable'}")
        self.displayName = displayName
        self.name = displayName if name is None else name
        self.tableColumns = list(tableColumns)
        self._as_read = None

    def __setattr__(self, key, value):
        if key == "displayName" and value is not None and " " in value:
            raise ValueError("Table names cannot have spaces")
        object.__setattr__(self, key, value)

    def _loaded(self):
        columns = self.tableColumns
        self.tableColumns = list(columns.tableColumn or []) if columns is not None else []

    def _inner_xml(self):
        out = []
        for name in ("autoFilter", "sortState"):
            value = getattr(self, name)
            if value is not None:
                out.append(value._xml())
        if self.tableColumns:
            out.append('<tableColumns count="{0}">{1}</tableColumns>'.format(len(self.tableColumns), "".join(c._xml() for c in self.tableColumns)))
        if self.tableStyleInfo is not None:
            out.append(self.tableStyleInfo._xml())
        out.extend(self._other_children)
        return "".join(out)

    def _xml(self, tag=None):
        x = Element._xml(self, tag)
        return x.replace("<table", '<table xmlns="{0}"'.format(_NS), 1)

    @property
    def path(self):
        return "/xl" + self._path.format(self.id)

    def _initialise_columns(self):
        """Columns `Column1`, `Column2`, ... over the range, and a filter over it when
        the table has a header row."""
        min_col, min_row, max_col, max_row = range_boundaries(self.ref)
        for idx in range(min_col, max_col + 1):
            self.tableColumns.append(TableColumn(id=idx, name="Column{0}".format(idx)))
        if self.headerRowCount and not self.autoFilter:
            self.autoFilter = AutoFilter(ref=self.ref)

    @property
    def column_names(self):
        return [column.name for column in self.tableColumns]

    def _prepare(self, ws):
        """As openpyxl does when saving: columns named after the header cells."""
        if self.tableColumns:
            return
        self._initialise_columns()
        if self.headerRowCount:
            try:
                row = ws[self.ref][0]
                for cell, col in zip(row, self.tableColumns):
                    if cell.data_type != "s":
                        warnings.warn("File may not be readable: column headings must be strings.")
                    col.name = str(cell.value)
            except TypeError:
                warnings.warn("Column headings are missing, file may not be readable")

    @classmethod
    def _from_table_xml(cls, xml):
        table = cls._from_xml(xml)
        object.__setattr__(table, "_as_read", (table._xml(), xml))
        return table

    def _spec(self):
        x = self._xml()
        as_read = getattr(self, "_as_read", None)
        return as_read[1] if as_read is not None and as_read[0] == x else x


class TablePartList:
    """Kept for openpyxl's name; the module writes the table parts itself."""

    tagname = "tableParts"

    def __init__(self, count=None, tablePart=()):
        self.tablePart = list(tablePart)

    def append(self, part):
        self.tablePart.append(part)

    @property
    def count(self):
        return len(self.tablePart)

    def __bool__(self):
        return bool(self.tablePart)


class TableList(dict):
    """The tables of a sheet by name."""

    def add(self, table):
        if not isinstance(table, Table):
            raise TypeError("You can only add tables")
        self[table.name] = table

    def get(self, name=None, table_range=None):
        if name is not None:
            return dict.get(self, name)
        for table in self.values():
            if table_range == table.ref:
                return table
        return None

    def items(self):
        return [(name, table.ref) for name, table in dict.items(self)]
