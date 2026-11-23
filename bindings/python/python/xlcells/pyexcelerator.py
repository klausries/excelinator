"""The interface of pyExcelerator on top of xlcells.

    from xlcells.pyexcelerator import *
    w = Workbook()
    ws = w.add_sheet('Hey, Dude')
    ws.write(0, 0, 'text')
    w.save('mini.xlsx')
    for sheet_name, values in parse_xls('mini.xlsx'):
        ...                       # values: {(row, col): value}

This is a re-creation of the interface; it shares no code with pyExcelerator. The
file format is xlsx. Written to the file: values, formulas, number formats, bold and
italic, merged cells, column widths. The other style attributes of pyExcelerator
(colours, borders, alignment, patterns, protection, panes, outlines, bitmaps) are
accepted so that existing scripts run, but are not written yet.
"""
import datetime as _dt

from . import _xlcells

__all__ = [
    "Workbook", "Worksheet", "Row", "Column", "XFStyle", "Font", "Alignment", "Borders",
    "Pattern", "Protection", "Formula", "ErrorCode", "parse_xls", "NoCalcs", "RecalcAlways",
    "CalcOnOpen",
]

NoCalcs, RecalcAlways, CalcOnOpen = 0x00, 0x01, 0x02


class _Bag:
    """Attributes of pyExcelerator that are accepted but not written."""

    def __init__(self, **defaults):
        self.__dict__.update(defaults)


class Font(_Bag):
    ESCAPEMENT_NONE, ESCAPEMENT_SUPERSCRIPT, ESCAPEMENT_SUBSCRIPT = 0, 1, 2
    UNDERLINE_NONE, UNDERLINE_SINGLE, UNDERLINE_SINGLE_ACC, UNDERLINE_DOUBLE, UNDERLINE_DOUBLE_ACC = 0x00, 0x01, 0x21, 0x02, 0x22

    def __init__(self):
        super().__init__(name="Arial", height=200, bold=False, italic=False, struck_out=False,
                         outline=False, shadow=False, colour_index=0x7FFF, underline=0,
                         escapement=0, family=0, charset=0)


class Alignment(_Bag):
    HORZ_GENERAL, HORZ_LEFT, HORZ_CENTER, HORZ_RIGHT, HORZ_FILLED, HORZ_JUSTIFIED = range(6)
    VERT_TOP, VERT_CENTER, VERT_BOTTOM, VERT_JUSTIFIED = range(4)

    def __init__(self):
        super().__init__(horz=0, vert=2, wrap=0, rota=0, shri=0, inde=0, merg=0)


class Borders(_Bag):
    NO_LINE, THIN, MEDIUM, DASHED, DOTTED, THICK, DOUBLE, HAIR = range(8)

    def __init__(self):
        super().__init__(left=0, right=0, top=0, bottom=0, diag=0, left_colour=0x40,
                         right_colour=0x40, top_colour=0x40, bottom_colour=0x40)


class Pattern(_Bag):
    NO_PATTERN, SOLID_PATTERN = 0, 1

    def __init__(self):
        super().__init__(pattern=0, pattern_fore_colour=0x40, pattern_back_colour=0x41)


class Protection(_Bag):
    def __init__(self):
        super().__init__(cell_locked=1, formula_hidden=0)


class XFStyle:
    def __init__(self):
        self.num_format_str = "general"
        self.font = Font()
        self.alignment = Alignment()
        self.borders = Borders()
        self.pattern = Pattern()
        self.protection = Protection()


class ErrorCode:
    def __init__(self, text):
        self.text = text


class Formula:
    """A formula without the leading '='. Arguments may be separated by ';' or ','.

    ``value`` is the result to store with the formula (optional); without it the
    spreadsheet program calculates the cell when it opens the file.
    """

    def __init__(self, text, value=None, opts=None):
        self.text_value = text
        self.value = value
        self.opts = opts

    def text(self):
        return self.text_value


def _excel_date(value):
    if isinstance(value, _dt.datetime):
        serial = _xlcells.date_serial(value.year, value.month, value.day, value.hour, value.minute,
                                      value.second, value.microsecond)
    elif isinstance(value, _dt.date):
        serial = _xlcells.date_serial(value.year, value.month, value.day)
    else:
        return (value.hour * 3600 + value.minute * 60 + value.second + value.microsecond / 1e6) / 86400
    if serial is None:
        raise ValueError("date %r cannot be stored in a workbook" % (value,))
    return serial


def _cell(label):
    """(kind, num, text, formula) of a label."""
    if isinstance(label, Formula):
        kind, num, text, _ = _cell(label.value) if label.value is not None else (0, 0.0, None, None)
        return kind, num, text, label.text_value
    if isinstance(label, ErrorCode):
        return 4, 0.0, label.text, None
    if label is None:
        return 0, 0.0, None, None
    if isinstance(label, bool):
        return 3, float(label), None, None
    if isinstance(label, str):
        return (2, 0.0, label, None) if label else (0, 0.0, None, None)
    if isinstance(label, (int, float)):
        return 1, float(label), None, None
    if isinstance(label, (_dt.datetime, _dt.date, _dt.time)):
        return 1, _excel_date(label), None, None
    raise TypeError("cannot write %r into a cell" % (label,))


class Column:
    def __init__(self, sheet, index):
        self.__dict__["_sheet"] = sheet
        self.__dict__["_index"] = index
        self.__dict__["width"] = 0x0B92

    def __setattr__(self, name, value):
        self.__dict__[name] = value
        if name == "width":
            self._sheet._w.set_col_width(self._sheet._index, self._index, int(value))


class Row:
    def __init__(self, sheet, index):
        self._sheet = sheet
        self._index = index
        self.height = 0x00FF

    def write(self, col, label, style=None):
        self._sheet.write(self._index, col, label, style)

    def set_style(self, style):
        """Accepted; row default styles are not written yet."""
        self.style = style


_DEFAULT_STYLE = XFStyle()


class Worksheet:
    def __init__(self, name, workbook, index):
        self.name = name
        self._wb = workbook
        self._w = workbook._w
        self._index = index
        self._cols = {}
        self._rows = {}
        self.frmla_opts = RecalcAlways | CalcOnOpen

    def get_name(self):
        return self.name

    def write(self, r, c, label="", style=None):
        kind, num, text, formula = _cell(label)
        self._w.write(self._index, r, c, kind, num, text, formula, self._wb._style(style or _DEFAULT_STYLE))

    def merge(self, r1, r2, c1, c2, style=None):
        self._w.merge(self._index, r1, r2, c1, c2)

    def write_merge(self, r1, r2, c1, c2, label="", style=None):
        self.merge(r1, r2, c1, c2, style)
        self.write(r1, c1, label, style)

    def write_cols(self, r, cstart, cdata, style=None):
        for offset, label in enumerate(cdata):
            self.write(r, cstart + offset, label, style)

    def set_link(self, r, c, url, target=None, description=None):
        """Accepted; hyperlinks are not written yet (the cell keeps its content)."""

    def col(self, index):
        return self._cols.setdefault(index, Column(self, index))

    def row(self, index):
        return self._rows.setdefault(index, Row(self, index))


class Workbook:
    def __init__(self):
        self._w = _xlcells.Writer()
        self._sheets = []
        self._styles = {}

    def _style(self, style):
        fmt = style.num_format_str
        if fmt.lower() == "general":
            fmt = "General"
        key = (fmt, bool(style.font.bold), bool(style.font.italic))
        if key not in self._styles:
            self._styles[key] = self._w.add_style(*key)
        return self._styles[key]

    def add_sheet(self, sheetname):
        ws = Worksheet(sheetname, self, self._w.add_sheet(sheetname))
        self._sheets.append(ws)
        return ws

    def get_sheet(self, index):
        return self._sheets[index]

    def to_bytes(self):
        return self._w.to_bytes()

    def save(self, filename):
        data = self.to_bytes()
        if hasattr(filename, "write"):
            filename.write(data)
        else:
            with open(filename, "wb") as f:
                f.write(data)


def parse_xls(filename, encoding=None):
    """[(sheet name, {(row, col): value})] of a workbook file (or its bytes).

    Dates arrive as numbers and errors as their text, as in pyExcelerator.
    """
    if isinstance(filename, (bytes, bytearray)):
        data = bytes(filename)
    else:
        with open(filename, "rb") as f:
            data = f.read()
    t = _xlcells.read(data)
    sheets = [(name, {}) for name in t["sheet_names"]]
    for s, r, c, kind, num, text in zip(t["sheet"], t["row"], t["col"], t["kind"], t["num"], t["text"]):
        if kind == 1:
            sheets[s][1][(r, c)] = num
        elif kind == 2 or kind == 4:
            sheets[s][1][(r, c)] = text
        elif kind == 3:
            sheets[s][1][(r, c)] = num != 0.0
    return sheets
