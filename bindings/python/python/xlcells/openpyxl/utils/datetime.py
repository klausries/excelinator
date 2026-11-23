"""Excel serial numbers and ISO 8601 texts ⇄ Python dates, as openpyxl.utils.datetime."""
import datetime
import math
import re

__all__ = ["to_excel", "from_excel", "to_ISO8601", "from_ISO8601", "time_to_days", "timedelta_to_days", "days_to_time",
           "MAC_EPOCH", "WINDOWS_EPOCH", "CALENDAR_WINDOWS_1900", "CALENDAR_MAC_1904", "SECS_PER_DAY", "ISO_FORMAT"]

MAC_EPOCH = datetime.datetime(1904, 1, 1)
WINDOWS_EPOCH = datetime.datetime(1899, 12, 30)
CALENDAR_WINDOWS_1900 = WINDOWS_EPOCH
CALENDAR_MAC_1904 = MAC_EPOCH
SECS_PER_DAY = 86400
ISO_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
ISO_REGEX = re.compile(r"""
(?P<date>(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2}))?T?
(?P<time>(?P<hour>\d{2}):(?P<minute>\d{2})(:(?P<second>\d{2})(?P<microsecond>\.\d{1,3})?)?)?Z?""", re.VERBOSE)
ISO_DURATION = re.compile(r"PT((?P<hours>\d+)H)?((?P<minutes>\d+)M)?((?P<seconds>\d+(\.\d{1,3})?)S)?")


def to_ISO8601(dt):
    if getattr(dt, "microsecond", 0):
        return dt.isoformat(timespec="milliseconds")
    return dt.isoformat()


def from_ISO8601(formatted_string):
    """A date, time, datetime or (`PT1H30M`) duration from ISO 8601 text."""
    if not formatted_string:
        return None
    m = ISO_REGEX.match(formatted_string)
    if m and any(m.groups()):
        g = m.groupdict()
        num = lambda k: int(g[k]) if g[k] else 0  # noqa: E731
        micro = int(float(g["microsecond"]) * 1_000_000) if g["microsecond"] else 0
        if not g["date"]:
            return datetime.time(num("hour"), num("minute"), num("second"), micro)
        if not g["time"]:
            return datetime.date(num("year"), num("month"), num("day"))
        return datetime.datetime(num("year"), num("month"), num("day"), num("hour"), num("minute"), num("second"), micro)
    m = ISO_DURATION.match(formatted_string)
    if m and any(m.groups()):
        return datetime.timedelta(**{k: float(v) for k, v in m.groupdict().items() if v})
    raise ValueError("Invalid datetime value {}".format(formatted_string))


def time_to_days(value):
    return (value.hour * 3600 + value.minute * 60 + value.second + value.microsecond / 10 ** 6) / SECS_PER_DAY


def timedelta_to_days(value):
    return value.total_seconds() / SECS_PER_DAY


def days_to_time(value):
    minutes, seconds = divmod(value.seconds, 60)
    hours, minutes = divmod(minutes, 60)
    return datetime.time(hours, minutes, seconds, value.microseconds)


def to_excel(dt, epoch=WINDOWS_EPOCH):
    """The serial number of a date, datetime, time or timedelta. In the 1900 system the
    days before 1 March 1900 count one less (Excel's non-existent 29 February 1900)."""
    if isinstance(dt, datetime.time):
        return time_to_days(dt)
    if isinstance(dt, datetime.timedelta):
        return timedelta_to_days(dt)
    if isinstance(dt.year, float) and math.isnan(dt.year):
        return None
    if not hasattr(dt, "date"):
        dt = datetime.datetime.combine(dt, datetime.time())
    days = (dt - epoch).days
    if 0 < days <= 60 and epoch == WINDOWS_EPOCH:
        days -= 1
    return days + time_to_days(dt)


def from_excel(value, epoch=WINDOWS_EPOCH, timedelta=False):
    """The date (or, below 1, the time; with `timedelta` the duration) of a serial number,
    to the millisecond."""
    if value is None:
        return None
    if timedelta:
        td = datetime.timedelta(days=value)
        if td.microseconds:
            td = datetime.timedelta(seconds=td.total_seconds() // 1, microseconds=round(td.microseconds, -3))
        return td
    day, fraction = divmod(value, 1)
    diff = datetime.timedelta(milliseconds=round(fraction * SECS_PER_DAY * 1000))
    if 0 <= value < 1 and diff.days == 0:
        return days_to_time(diff)
    if 0 < value < 60 and epoch == WINDOWS_EPOCH:
        day += 1
    return epoch + datetime.timedelta(days=day) + diff
