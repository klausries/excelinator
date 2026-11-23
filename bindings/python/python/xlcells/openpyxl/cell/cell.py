"""Cells and the names of cell types."""
import datetime as _dt
import decimal as _decimal

from .. import ArrayFormula, Cell, IllegalCharacterError, Hyperlink  # noqa: F401
from ..styles.numbers import is_date_format  # noqa: F401
from ..utils.cell import get_column_letter  # noqa: F401

__all__ = ["Cell", "MergedCell", "WriteOnlyCell", "ERROR_CODES", "TYPE_STRING", "TYPE_FORMULA", "TYPE_NUMERIC", "TYPE_BOOL", "TYPE_NULL",
           "TYPE_INLINE", "TYPE_ERROR", "TYPE_FORMULA_CACHE_STRING", "VALID_TYPES", "TIME_TYPES", "TIME_FORMATS", "STRING_TYPES",
           "KNOWN_TYPES", "NUMERIC_TYPES", "get_type", "get_time_format"]

MergedCell = Cell  # cells inside a merged range are plain empty cells here
from .._streaming import WriteOnlyCell  # noqa: E402,F401

NUMERIC_TYPES = (int, float, _decimal.Decimal)
TIME_TYPES = (_dt.datetime, _dt.date, _dt.time, _dt.timedelta)
TIME_FORMATS = {_dt.datetime: "yyyy-mm-dd h:mm:ss", _dt.date: "yyyy-mm-dd", _dt.time: "h:mm:ss", _dt.timedelta: "[hh]:mm:ss"}
STRING_TYPES = (str, bytes)
KNOWN_TYPES = NUMERIC_TYPES + TIME_TYPES + STRING_TYPES + (bool, type(None))
ERROR_CODES = ("#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A")
TYPE_STRING = "s"
TYPE_FORMULA = "f"
TYPE_NUMERIC = "n"
TYPE_BOOL = "b"
TYPE_NULL = "n"
TYPE_INLINE = "inlineStr"
TYPE_ERROR = "e"
TYPE_FORMULA_CACHE_STRING = "str"
VALID_TYPES = (TYPE_STRING, TYPE_FORMULA, TYPE_NUMERIC, TYPE_BOOL, TYPE_NULL, TYPE_INLINE, TYPE_ERROR, TYPE_FORMULA_CACHE_STRING)


def get_type(t, value):
    """The data type letter for a value of type `t` (None if not one of the known kinds)."""
    if isinstance(value, NUMERIC_TYPES) and not isinstance(value, bool):
        return "n"
    if isinstance(value, STRING_TYPES):
        return "s"
    if isinstance(value, TIME_TYPES):
        return "d"
    if isinstance(value, ArrayFormula):
        return "f"
    return None


def get_time_format(t):
    for base in t.mro():
        if base in TIME_FORMATS:
            return TIME_FORMATS[base]
    raise ValueError("Could not get time format for {0!r}".format(t))
