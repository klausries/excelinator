"""Autofilter with its criteria and sort state (`ws.auto_filter`), as openpyxl's
`worksheet.filters`: value lists, custom comparisons, top 10, dynamic (date and average)
filters, colour and icon filters, sort conditions. Attributes and children the classes
do not know are kept as read."""
import re as _re

from .._serial import Element, _esc

__all__ = ["AutoFilter", "FilterColumn", "Filters", "CustomFilters", "CustomFilter", "DynamicFilter", "ColorFilter", "IconFilter", "Top10",
           "DateGroupItem", "SortState", "SortCondition", "BlankFilter", "NumberFilter", "StringFilter"]

_ICON_SETS = ("3Arrows", "3ArrowsGray", "3Flags", "3TrafficLights1", "3TrafficLights2", "3Signs", "3Symbols", "3Symbols2", "4Arrows", "4ArrowsGray",
              "4RedToBlack", "4Rating", "4TrafficLights", "5Arrows", "5ArrowsGray", "5Rating", "5Quarters")


def _check(value, allowed, none_ok=True):
    if value is None and none_ok:
        return
    if value not in allowed:
        raise ValueError("Value must be one of {0}".format(set(allowed)))


class SortCondition(Element):
    tagname = "sortCondition"
    _attrs = (("ref", str, None), ("descending", bool, None), ("sortBy", str, None), ("customList", str, None), ("dxfId", int, None),
              ("iconSet", str, None), ("iconId", int, None))

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        _check(self.sortBy, ("value", "cellColor", "fontColor"))
        _check(self.iconSet, _ICON_SETS)


class SortState(Element):
    tagname = "sortState"
    _attrs = (("columnSort", bool, None), ("caseSensitive", bool, None), ("sortMethod", str, None), ("ref", str, None))
    _children = (("sortCondition", SortCondition),)
    _lists = ("sortCondition",)

    def __init__(self, *args, **kw):
        kw.pop("extLst", None)
        Element.__init__(self, *args, **kw)
        _check(self.sortMethod, ("stroke", "pinYin"))
        self.sortCondition = list(self.sortCondition or [])

    def __bool__(self):
        return self.ref is not None

    def _loaded(self):
        self.sortCondition = self.sortCondition or []


class IconFilter(Element):
    tagname = "iconFilter"
    _attrs = (("iconSet", str, None), ("iconId", int, None))

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        _check(self.iconSet, _ICON_SETS, none_ok=False)


class ColorFilter(Element):
    tagname = "colorFilter"
    _attrs = (("dxfId", int, None), ("cellColor", bool, None))


class DynamicFilter(Element):
    tagname = "dynamicFilter"
    _attrs = (("type", str, None), ("val", float, None), ("valIso", str, None), ("maxVal", float, None), ("maxValIso", str, None))

    _TYPES = ("null", "aboveAverage", "belowAverage", "tomorrow", "today", "yesterday", "nextWeek", "thisWeek", "lastWeek", "nextMonth", "thisMonth",
              "lastMonth", "nextQuarter", "thisQuarter", "lastQuarter", "nextYear", "thisYear", "lastYear", "yearToDate", "Q1", "Q2", "Q3", "Q4", "M1",
              "M2", "M3", "M4", "M5", "M6", "M7", "M8", "M9", "M10", "M11", "M12")

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        _check(self.type, self._TYPES, none_ok=False)
        for name in ("valIso", "maxValIso"):
            value = getattr(self, name)
            if value is not None and not isinstance(value, str):
                setattr(self, name, value.isoformat())


class CustomFilter(Element):
    tagname = "customFilter"
    _attrs = (("operator", str, "equal"), ("val", str, None))
    _OPERATORS = ("equal", "lessThan", "lessThanOrEqual", "notEqual", "greaterThanOrEqual", "greaterThan")

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        _check(self.operator, self._OPERATORS, none_ok=False)
        if self.val is not None:
            self.val = str(self.val)


    def _get_subtype(self):
        if self.val == " ":
            return BlankFilter
        try:
            float(self.val)
            return NumberFilter
        except (TypeError, ValueError):
            return StringFilter

    def convert(self):
        """The more specific filter this one is: blank, number or text."""
        kind = self._get_subtype()
        if kind is BlankFilter:
            return BlankFilter()
        if kind is NumberFilter:
            return NumberFilter(self.operator, float(self.val))
        operator, term = StringFilter._guess_operator(self.val)
        return StringFilter(operator, term, exclude=self.operator == "notEqual")


class BlankFilter(CustomFilter):
    """Excludes blanks: `notEqual " "`."""

    def __init__(self, **kw):
        CustomFilter.__init__(self, operator="notEqual", val=" ")


class NumberFilter(CustomFilter):
    """A comparison with a number."""

    def __init__(self, operator="equal", val=None):
        CustomFilter.__init__(self, operator=operator)
        self.val = None if val is None else float(val)

    def _attr_xml(self):
        return ' operator="{0}" val="{1}"'.format(self.operator, _num_text(self.val))


def _num_text(value):
    return str(int(value)) if value == int(value) and abs(value) < 1e15 else repr(value)


class StringFilter(CustomFilter):
    """A text condition: `contains`, `startswith`, `endswith` (wildcards in the term are
    escaped) or `wildcard`; `exclude` turns it around."""

    _FORMAT = {"contains": "*{0}*", "startswith": "{0}*", "endswith": "*{0}", "wildcard": "{0}"}

    def __init__(self, operator="contains", val=None, exclude=False):
        if operator not in self._FORMAT:
            raise ValueError("Value must be one of {'contains', 'startswith', 'endswith', 'wildcard'}")
        Element.__init__(self)
        self.operator, self.val, self.exclude = operator, val, bool(exclude)

    def _escape(self):
        return self.val if self.operator == "wildcard" else _re.sub(r"~|\*|\?", r"~\g<0>", self.val)

    @staticmethod
    def _unescape(value):
        return _re.sub(r"~(?P<op>[~*?])", r"\g<op>", value)

    @staticmethod
    def _guess_operator(value):
        value = StringFilter._unescape(value)
        d = {"wildcard": True, "term": value}
        for pattern in (r"^(?P<contains>\*)(?P<term>[^\*\?]*)\*$", r"^(?P<term>[^\*\?]*)(?P<startswith>\*)$", r"^(?P<endswith>\*)(?P<term>[^\*\?]*$)"):
            m = _re.match(pattern, value)
            if m:
                d = m.groupdict()
        term = d.pop("term")
        return list(d)[0], term

    def _xml(self, tag=None):
        value = self._FORMAT[self.operator].format(self._escape())
        return CustomFilter("notEqual" if self.exclude else "equal", value)._xml(tag)


class CustomFilters(Element):
    tagname = "customFilters"
    _attrs = (("_and", bool, None),)
    _xml_names = {"_and": "and"}
    _children = (("customFilter", CustomFilter),)
    _lists = ("customFilter",)

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        self.customFilter = list(self.customFilter or [])

    def _loaded(self):
        self.customFilter = self.customFilter or []


class Top10(Element):
    tagname = "top10"
    _attrs = (("top", bool, None), ("percent", bool, None), ("val", float, None), ("filterVal", float, None))


class DateGroupItem(Element):
    tagname = "dateGroupItem"
    _attrs = (("year", int, None), ("month", int, None), ("day", int, None), ("hour", int, None), ("minute", int, None), ("second", int, None),
              ("dateTimeGrouping", str, None))

    def __init__(self, *args, **kw):
        Element.__init__(self, *args, **kw)
        _check(self.dateTimeGrouping, ("year", "month", "day", "hour", "minute", "second"), none_ok=False)


class _FilterValue(Element):
    tagname = "filter"
    _attrs = (("val", str, None),)


class Filters(Element):
    """Values to show (`filter`, plain strings), whether blanks show, date groups."""

    tagname = "filters"
    _attrs = (("blank", bool, None), ("calendarType", str, None))
    _children = (("filter", _FilterValue), ("dateGroupItem", DateGroupItem))
    _lists = ("filter", "dateGroupItem")

    def __init__(self, blank=None, calendarType=None, filter=(), dateGroupItem=()):
        Element.__init__(self, blank=blank, calendarType=calendarType)
        self.filter = [str(v) for v in filter]
        self.dateGroupItem = list(dateGroupItem)

    def _loaded(self):
        self.filter = [f.val for f in (self.filter or [])]
        self.dateGroupItem = self.dateGroupItem or []

    def _inner_xml(self):
        return "".join('<filter val="{0}"/>'.format(_esc(v)) for v in self.filter) + "".join(d._xml() for d in self.dateGroupItem) + "".join(
            self._other_children)


class FilterColumn(Element):
    """The criteria of one column of the range (`colId` counts from 0, alias `col_id`)."""

    tagname = "filterColumn"
    _attrs = (("colId", int, None), ("hiddenButton", bool, False), ("showButton", bool, True))
    _children = (("filters", Filters), ("top10", Top10), ("customFilters", CustomFilters), ("dynamicFilter", DynamicFilter),
                 ("colorFilter", ColorFilter), ("iconFilter", IconFilter))

    def __init__(self, colId=None, hiddenButton=False, showButton=True, filters=None, top10=None, customFilters=None, dynamicFilter=None,
                 colorFilter=None, iconFilter=None, extLst=None, blank=None, vals=None, col_id=None):
        Element.__init__(self, colId=colId if colId is not None else col_id, hiddenButton=hiddenButton, showButton=showButton, filters=filters,
                         top10=top10, customFilters=customFilters, dynamicFilter=dynamicFilter, colorFilter=colorFilter, iconFilter=iconFilter)
        if blank is not None and self.filters:
            self.filters.blank = blank
        if vals is not None and self.filters:
            self.filters.filter = list(vals)

    col_id = property(lambda self: self.colId, lambda self, v: setattr(self, "colId", v))


class AutoFilter(Element):
    """The filtered range (`ref`), the criteria per column and the sort state."""

    tagname = "autoFilter"
    _attrs = (("ref", str, None),)
    _children = (("filterColumn", FilterColumn), ("sortState", SortState))
    _lists = ("filterColumn",)

    def __init__(self, ref=None, filterColumn=(), sortState=None, extLst=None):
        Element.__init__(self, ref=ref, sortState=sortState)
        self.filterColumn = list(filterColumn)

    def _loaded(self):
        self.filterColumn = self.filterColumn or []

    def __bool__(self):
        return self.ref is not None

    def __str__(self):
        from ..utils.cell import absolute_coordinate
        return absolute_coordinate(self.ref)

    def _xml(self, tag=None):
        return Element._xml(self, tag) if self.ref is not None else ""

    def add_filter_column(self, col_id, vals, blank=False):
        """Show only rows whose cell in column `col_id` (from 0) has one of `vals`."""
        self.filterColumn.append(FilterColumn(colId=col_id, filters=Filters(blank=blank, filter=vals)))

    def add_sort_condition(self, ref, descending=False):
        """Sort by the cells of `ref` (such as `B2:B15`)."""
        cond = SortCondition(ref, descending)
        if self.sortState is None:
            self.sortState = SortState(ref=self.ref)
        self.sortState.sortCondition.append(cond)
