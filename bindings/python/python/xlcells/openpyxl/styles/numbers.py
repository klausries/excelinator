"""Number formats: the built-in formats, the FORMAT_ constants and the date tests."""
import re

__all__ = ["BUILTIN_FORMATS", "BUILTIN_FORMATS_MAX_SIZE", "BUILTIN_FORMATS_REVERSE", "is_builtin", "builtin_format_code", "builtin_format_id",
           "is_date_format", "is_timedelta_format", "is_datetime", "NumberFormat", "NumberFormatList", "NumberFormatDescriptor"]

# Format codes by number exactly as openpyxl lists them (ECMA-376 part 1, 18.8.30; openpyxl,
# MIT licence, writes the US dollar formats with quotes and number 44 without its section
# separators; kept so that comparisons with openpyxl agree).
BUILTIN_FORMATS = {
    0: 'General',
    1: '0',
    2: '0.00',
    3: '#,##0',
    4: '#,##0.00',
    5: '"$"#,##0_);("$"#,##0)',
    6: '"$"#,##0_);[Red]("$"#,##0)',
    7: '"$"#,##0.00_);("$"#,##0.00)',
    8: '"$"#,##0.00_);[Red]("$"#,##0.00)',
    9: '0%',
    10: '0.00%',
    11: '0.00E+00',
    12: '# ?/?',
    13: '# ??/??',
    14: 'mm-dd-yy',
    15: 'd-mmm-yy',
    16: 'd-mmm',
    17: 'mmm-yy',
    18: 'h:mm AM/PM',
    19: 'h:mm:ss AM/PM',
    20: 'h:mm',
    21: 'h:mm:ss',
    22: 'm/d/yy h:mm',
    37: '#,##0_);(#,##0)',
    38: '#,##0_);[Red](#,##0)',
    39: '#,##0.00_);(#,##0.00)',
    40: '#,##0.00_);[Red](#,##0.00)',
    41: '_(* #,##0_);_(* \\(#,##0\\);_(* "-"_);_(@_)',
    42: '_("$"* #,##0_);_("$"* \\(#,##0\\);_("$"* "-"_);_(@_)',
    43: '_(* #,##0.00_);_(* \\(#,##0.00\\);_(* "-"??_);_(@_)',
    44: '_("$"* #,##0.00_)_("$"* \\(#,##0.00\\)_("$"* "-"??_)_(@_)',
    45: 'mm:ss',
    46: '[h]:mm:ss',
    47: 'mmss.0',
    48: '##0.0E+0',
    49: '@',
}
BUILTIN_FORMATS_MAX_SIZE = 164
BUILTIN_FORMATS_REVERSE = {code: key for key, code in BUILTIN_FORMATS.items()}

FORMAT_GENERAL = BUILTIN_FORMATS[0]
FORMAT_TEXT = BUILTIN_FORMATS[49]
FORMAT_NUMBER = BUILTIN_FORMATS[1]
FORMAT_NUMBER_00 = BUILTIN_FORMATS[2]
FORMAT_NUMBER_COMMA_SEPARATED1 = BUILTIN_FORMATS[4]
FORMAT_NUMBER_COMMA_SEPARATED2 = "#,##0.00_-"
FORMAT_PERCENTAGE = BUILTIN_FORMATS[9]
FORMAT_PERCENTAGE_00 = BUILTIN_FORMATS[10]
FORMAT_DATE_YYYYMMDD2 = "yyyy-mm-dd"
FORMAT_DATE_YYMMDD = "yy-mm-dd"
FORMAT_DATE_DDMMYY = "dd/mm/yy"
FORMAT_DATE_DMYSLASH = "d/m/y"
FORMAT_DATE_DMYMINUS = "d-m-y"
FORMAT_DATE_DMMINUS = "d-m"
FORMAT_DATE_MYMINUS = "m-y"
FORMAT_DATE_XLSX14 = BUILTIN_FORMATS[14]
FORMAT_DATE_XLSX15 = BUILTIN_FORMATS[15]
FORMAT_DATE_XLSX16 = BUILTIN_FORMATS[16]
FORMAT_DATE_XLSX17 = BUILTIN_FORMATS[17]
FORMAT_DATE_XLSX22 = BUILTIN_FORMATS[22]
FORMAT_DATE_DATETIME = "yyyy-mm-dd h:mm:ss"
FORMAT_DATE_TIME1 = BUILTIN_FORMATS[18]
FORMAT_DATE_TIME2 = BUILTIN_FORMATS[19]
FORMAT_DATE_TIME3 = BUILTIN_FORMATS[20]
FORMAT_DATE_TIME4 = BUILTIN_FORMATS[21]
FORMAT_DATE_TIME5 = BUILTIN_FORMATS[45]
FORMAT_DATE_TIME6 = BUILTIN_FORMATS[21]
FORMAT_DATE_TIME7 = "i:s.S"
FORMAT_DATE_TIME8 = "h:mm:ss@"
FORMAT_DATE_TIMEDELTA = "[hh]:mm:ss"
FORMAT_DATE_YYMMDDSLASH = "yy/mm/dd@"
FORMAT_CURRENCY_USD_SIMPLE = '"$"#,##0.00_-'
FORMAT_CURRENCY_USD = "$#,##0_-"
FORMAT_CURRENCY_EUR_SIMPLE = "[$EUR ]#,##0.00_-"

COLORS = r"\[(BLACK|BLUE|CYAN|GREEN|MAGENTA|RED|WHITE|YELLOW)\]"
LITERAL_GROUP = r'".*?"'
LOCALE_GROUP = r"\[(?!hh?\]|mm?\]|ss?\])[^\]]*\]"
STRIP_RE = re.compile("{0}|{1}".format(LITERAL_GROUP, LOCALE_GROUP))
TIMEDELTA_RE = re.compile(r"\[hh?\](:mm(:ss(\.0*)?)?)?|\[mm?\](:ss(\.0*)?)?|\[ss?\](\.0*)?", re.I)
_DATE_CHAR = re.compile(r"(?<![_\\])[dmhysDMHYS]")


def is_builtin(fmt):
    return fmt in BUILTIN_FORMATS_REVERSE


def builtin_format_code(index):
    return BUILTIN_FORMATS.get(index)


def builtin_format_id(fmt):
    if fmt is None:
        return None
    return BUILTIN_FORMATS_REVERSE.get(fmt)


def is_date_format(fmt):
    """Whether a number format shows a date or time (its first section counts)."""
    if fmt is None:
        return False
    return _DATE_CHAR.search(STRIP_RE.sub("", fmt.split(";")[0])) is not None


def is_timedelta_format(fmt):
    """Whether a number format shows elapsed time (`[h]:mm`)."""
    if fmt is None:
        return False
    return TIMEDELTA_RE.search(fmt.split(";")[0]) is not None


def is_datetime(fmt):
    """'datetime', 'date' or 'time' for a date format, else None."""
    if not is_date_format(fmt):
        return None
    plain = STRIP_RE.sub("", fmt.split(";")[0])
    date, time = any(c in plain.lower() for c in "dy"), any(c in plain.lower() for c in "hs")
    if date and time:
        return "datetime"
    if date:
        return "date"
    if "m" in plain.lower() and not time:
        return "date"
    return "time"


class NumberFormat:
    def __init__(self, numFmtId=None, formatCode=None):
        self.numFmtId, self.formatCode = numFmtId, formatCode


class NumberFormatList(list):
    pass


NumberFormatDescriptor = str
