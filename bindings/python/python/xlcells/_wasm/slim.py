"""`xlcells._xlcells` (reading and writing workbooks) on the WebAssembly module."""
from . import Held, call, outer

# Every method of a writer and its arguments by position, as `_xlcells.Writer` has them.
_METHODS = {
    "add_sheet": ("name",),
    "add_style": ("num_format", "bold", "italic"),
    "write": ("sheet", "row", "col", "kind", "num", "text", "formula", "style", "array_ref", "float_text", "rich", "iso"),
    "merge": ("sheet", "r1", "r2", "c1", "c2"),
    "set_col_width": ("sheet", "col", "width"),
    "set_col_width_chars": ("sheet", "first", "last", "width"),
    "set_named_styles": ("styles",),
    "add_style_spec": ("spec",),
    "set_styles": ("specs",),
    "set_sheet_kind": ("sheet", "kind"),
    "add_defined_name": ("name", "value", "local_sheet", "hidden"),
    "add_hyperlink": ("sheet", "cell", "target", "location", "tooltip", "display"),
    "set_auto_filter": ("sheet", "range"),
    "add_validation": ("sheet", "attrs", "formula1", "formula2"),
    "add_cond_format": ("sheet", "sqref", "rules"),
    "add_drawing": ("sheet", "from", "to", "size", "image", "chart"),
    "add_comment": ("sheet", "cell", "author", "text"),
    "set_workbook_xml": ("name", "xml"),
    "set_properties_xml": ("kind", "xml"),
    "add_table": ("sheet", "xml"),
    "set_sheet_origin": ("sheet", "origin"),
    "set_row_height": ("sheet", "row", "height"),
    "set_row": ("sheet", "row", "height", "custom_height", "hidden", "outline_level", "collapsed", "style", "custom_format", "thick_top", "thick_bot", "phonetic"),
    "set_columns": ("sheet", "first", "last", "width", "custom_width", "hidden", "best_fit", "outline_level", "collapsed", "style", "phonetic"),
    "set_sheet_xml": ("sheet", "name", "xml"),
    "set_freeze": ("sheet", "rows", "cols"),
    "set_date1904": ("on",),
    "to_bytes_onto": ("original",),
    "to_bytes": (),
}

# What gives something back is sent at once (with what was gathered before it).
_ANSWERING = {"add_sheet", "add_style", "add_style_spec", "to_bytes", "to_bytes_onto"}

# Calls gathered before they are sent together.
_BATCH = 20000


class Writer(Held):
    """Builds a workbook cell by cell. The calls are gathered and sent to the module
    together: one crossing for many cells."""

    kind = "writer"

    def __init__(self):
        super().__init__()
        self._calls = []

    def _send(self):
        calls, self._calls = self._calls, []
        return self._call(calls) if calls else []

    def _method(self, name, args, kwargs):
        names = _METHODS[name]
        if len(args) > len(names):
            raise TypeError("{0}() takes {1} arguments".format(name, len(names)))
        values = list(args) + [None] * (len(names) - len(args))
        for k, v in kwargs.items():
            if k not in names:
                raise TypeError("{0}() has no argument {1!r}".format(name, k))
            values[names.index(k)] = v
        while values and values[-1] is None:
            values.pop()
        self._calls.append([name, values])
        if name in _ANSWERING:
            result = self._send()[-1]
            if name == "to_bytes_onto":
                return (result[0], bool(result[1]))
            return result
        if len(self._calls) >= _BATCH:
            self._send()
        return None

    def __getattr__(self, name):
        if name not in _METHODS:
            raise AttributeError(name)
        return lambda *args, **kwargs: self._method(name, args, kwargs)


def read(data):
    """A workbook as equally long columns, one entry per cell."""
    t = call("read", bytes(data))
    return {k: outer(v) for k, v in t.items()}


def date_serial(year, month, day, hour=0, minute=0, second=0, microsecond=0):
    return call("date_serial", year, month, day, hour, minute, second, microsecond)


def translate_formula(formula, rows, cols):
    return call("translate_formula", formula, rows, cols)
