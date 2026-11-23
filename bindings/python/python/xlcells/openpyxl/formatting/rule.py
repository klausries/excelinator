"""Conditional formatting rules: cell comparisons, formulas, colour scales, data bars,
icon sets and the other rule types of `<cfRule>`.

Behaves like openpyxl's `formatting.rule` (same classes, constructors and helper
functions). A rule read from a file and left unchanged is written as it was read."""
import re as _re
import xml.etree.ElementTree as _ET

from .._styles import Color, _esc, _local, _num

__all__ = ["FormatObject", "RuleType", "IconSet", "DataBar", "ColorScale", "Rule", "ColorScaleRule", "FormulaRule", "CellIsRule", "IconSetRule",
           "DataBarRule"]

_COORD = _re.compile(r"^\$?[A-Z]{1,3}\$?[0-9]+$")
_OPERATORS = {">": "greaterThan", ">=": "greaterThanOrEqual", "<": "lessThan", "<=": "lessThanOrEqual", "=": "equal", "==": "equal", "!=": "notEqual"}
_ICON_SETS = ("3Arrows", "3ArrowsGray", "3Flags", "3TrafficLights1", "3TrafficLights2", "3Signs", "3Symbols", "3Symbols2", "4Arrows", "4ArrowsGray",
              "4RedToBlack", "4Rating", "4TrafficLights", "5Arrows", "5ArrowsGray", "5Rating", "5Quarters")
_RULE_TYPES = ("expression", "cellIs", "colorScale", "dataBar", "iconSet", "top10", "uniqueValues", "duplicateValues", "containsText", "notContainsText",
               "beginsWith", "endsWith", "containsBlanks", "notContainsBlanks", "containsErrors", "notContainsErrors", "timePeriod", "aboveAverage")


def _bool(value):
    return "1" if value else "0"


def _flag(attrs, key):
    return None if key not in attrs else attrs[key] in ("1", "true")


def _int(attrs, key):
    try:
        return int(attrs[key])
    except (KeyError, ValueError):
        return None


def _color(value):
    if value is None or isinstance(value, Color):
        return value
    return Color(value)


class _Object:
    _fields = ()

    def __eq__(self, other):
        return type(other) is type(self) and all(getattr(self, f) == getattr(other, f) for f in self._fields)

    def __ne__(self, other):
        return not self == other

    def __hash__(self):
        return hash(tuple(repr(getattr(self, f)) for f in self._fields))

    def __repr__(self):
        return "{0}({1})".format(type(self).__name__, ", ".join("{0}={1!r}".format(f, getattr(self, f)) for f in self._fields))


class FormatObject(_Object):
    """A threshold (`<cfvo>`): its type (num, percent, max, min, formula, percentile), value
    and whether it counts as greater-or-equal."""

    tagname = "cfvo"
    _fields = ("type", "val", "gte")

    def __init__(self, type, val=None, gte=None, extLst=None):
        if type not in ("num", "percent", "max", "min", "formula", "percentile"):
            raise ValueError("Value must be one of {'num', 'percent', 'max', 'min', 'formula', 'percentile'}")
        self.type = type
        self.gte = gte
        self.val = val

    @property
    def val(self):
        return self._val

    @val.setter
    def val(self, value):
        # As openpyxl: a number unless the type is formula or the value a cell reference.
        if value is None or self.type == "formula" or (isinstance(value, str) and _COORD.match(value)):
            self._val = value
        else:
            self._val = float(value)

    def _xml(self):
        x = '<cfvo type="{0}"'.format(self.type)
        if self.val is not None:
            x += ' val="{0}"'.format(_esc(_num(self.val) if isinstance(self.val, float) else self.val))
        if self.gte is not None:
            x += ' gte="{0}"'.format(_bool(self.gte))
        return x + "/>"

    @classmethod
    def _from(cls, el):
        a = el.attrib
        obj = cls.__new__(cls)
        obj.type, obj.gte = a.get("type"), _flag(a, "gte")
        val = a.get("val")
        try:
            obj._val = val if val is None or obj.type == "formula" or _COORD.match(val) else float(val)
        except ValueError:
            obj._val = val
        return obj


class RuleType(_Object):
    pass


class IconSet(RuleType):
    tagname = "iconSet"
    _fields = ("iconSet", "showValue", "percent", "reverse", "cfvo")

    def __init__(self, iconSet=None, showValue=None, percent=None, reverse=None, cfvo=None):
        if iconSet is not None and iconSet not in _ICON_SETS:
            raise ValueError("Value must be one of {0}".format(set(_ICON_SETS)))
        self.iconSet, self.showValue, self.percent, self.reverse = iconSet, showValue, percent, reverse
        self.cfvo = list(cfvo or [])

    def _xml(self):
        x = "<iconSet"
        if self.iconSet is not None:
            x += ' iconSet="{0}"'.format(self.iconSet)
        for key in ("showValue", "percent", "reverse"):
            if getattr(self, key) is not None:
                x += ' {0}="{1}"'.format(key, _bool(getattr(self, key)))
        return x + ">" + "".join(c._xml() for c in self.cfvo) + "</iconSet>"

    @classmethod
    def _from(cls, el):
        a = el.attrib
        obj = cls.__new__(cls)
        obj.iconSet = a.get("iconSet")
        obj.showValue, obj.percent, obj.reverse = _flag(a, "showValue"), _flag(a, "percent"), _flag(a, "reverse")
        obj.cfvo = [FormatObject._from(c) for c in el if _local(c.tag) == "cfvo"]
        return obj


class DataBar(RuleType):
    tagname = "dataBar"
    _fields = ("minLength", "maxLength", "showValue", "cfvo", "color")

    def __init__(self, minLength=None, maxLength=None, showValue=None, cfvo=None, color=None):
        self.minLength, self.maxLength, self.showValue = minLength, maxLength, showValue
        self.cfvo = list(cfvo or [])
        self.color = _color(color)

    def _xml(self):
        x = "<dataBar"
        for key in ("minLength", "maxLength"):
            if getattr(self, key) is not None:
                x += ' {0}="{1}"'.format(key, int(getattr(self, key)))
        if self.showValue is not None:
            x += ' showValue="{0}"'.format(_bool(self.showValue))
        x += ">" + "".join(c._xml() for c in self.cfvo)
        if self.color is not None:
            x += self.color._xml()
        return x + "</dataBar>"

    @classmethod
    def _from(cls, el):
        a = el.attrib
        obj = cls.__new__(cls)
        obj.minLength, obj.maxLength, obj.showValue = _int(a, "minLength"), _int(a, "maxLength"), _flag(a, "showValue")
        obj.cfvo = [FormatObject._from(c) for c in el if _local(c.tag) == "cfvo"]
        colours = [Color._from(c) for c in el if _local(c.tag) == "color"]
        obj.color = colours[0] if colours else None
        return obj


class ColorScale(RuleType):
    tagname = "colorScale"
    _fields = ("cfvo", "color")

    def __init__(self, cfvo=None, color=None):
        self.cfvo = list(cfvo or [])
        self.color = [_color(c) for c in (color or [])]

    def _xml(self):
        return "<colorScale>" + "".join(c._xml() for c in self.cfvo) + "".join(c._xml() for c in self.color) + "</colorScale>"

    @classmethod
    def _from(cls, el):
        obj = cls.__new__(cls)
        obj.cfvo = [FormatObject._from(c) for c in el if _local(c.tag) == "cfvo"]
        obj.color = [Color._from(c) for c in el if _local(c.tag) == "color"]
        return obj


class Rule(_Object):
    """A conditional formatting rule (`<cfRule>`)."""

    tagname = "cfRule"
    _fields = ("type", "dxfId", "priority", "stopIfTrue", "aboveAverage", "percent", "bottom", "operator", "text", "timePeriod", "rank", "stdDev",
               "equalAverage", "formula", "colorScale", "dataBar", "iconSet", "dxf")
    # In the order openpyxl writes them.
    _ATTRS = ("type", "rank", "priority", "equalAverage", "operator", "aboveAverage", "stdDev", "stopIfTrue", "timePeriod", "text", "percent", "bottom")

    def __init__(self, type, dxfId=None, priority=0, stopIfTrue=None, aboveAverage=None, percent=None, bottom=None, operator=None, text=None,
                 timePeriod=None, rank=None, stdDev=None, equalAverage=None, formula=(), colorScale=None, dataBar=None, iconSet=None, extLst=None,
                 dxf=None):
        if type not in _RULE_TYPES:
            raise ValueError("Value must be one of {0}".format(set(_RULE_TYPES)))
        self.type, self.dxfId, self.priority, self.stopIfTrue = type, dxfId, priority, stopIfTrue
        self.aboveAverage, self.percent, self.bottom, self.operator, self.text = aboveAverage, percent, bottom, operator, text
        self.timePeriod, self.rank, self.stdDev, self.equalAverage = timePeriod, rank, stdDev, equalAverage
        self.formula = list(formula or ())
        self.colorScale, self.dataBar, self.iconSet, self.dxf = colorScale, dataBar, iconSet, dxf
        self._extra = []  # attributes this module does not know, kept as read
        self._loaded = None

    def _content(self):
        x = "".join("<formula>{0}</formula>".format(_esc(f)) for f in self.formula)
        for part in (self.colorScale, self.dataBar, self.iconSet):
            if part is not None:
                x += part._xml()
        return x

    def _attrs(self):
        out = []
        for key in self._ATTRS:
            v = getattr(self, key)
            if v is None:
                continue
            out.append((key, _bool(v) if isinstance(v, bool) else str(v)))
        return out + self._extra

    def _spec(self):
        """(attributes, inner XML, differential format XML) for the core."""
        spec = (self._attrs(), self._content(), self.dxf._xml() if self.dxf is not None else None)
        if self._loaded is not None and self._loaded[0] == spec:
            return self._loaded[1]  # unchanged: as read, with what this module does not model
        return spec

    @classmethod
    def _load(cls, attrs, inner, dxf):
        from .. import DifferentialStyle
        a = dict(attrs)
        rule = cls.__new__(cls)
        rule.type, rule.operator, rule.text, rule.timePeriod = a.get("type"), a.get("operator"), a.get("text"), a.get("timePeriod")
        rule.priority, rule.rank, rule.stdDev, rule.dxfId = _int(a, "priority"), _int(a, "rank"), _int(a, "stdDev"), None
        for key in ("stopIfTrue", "aboveAverage", "percent", "bottom", "equalAverage"):
            setattr(rule, key, _flag(a, key))
        root = _ET.fromstring('<cfRule xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{0}</cfRule>'.format(inner))
        rule.formula, rule.colorScale, rule.dataBar, rule.iconSet = [], None, None, None
        for child in root:
            name = _local(child.tag)
            if name == "formula":
                rule.formula.append(child.text or "")
            elif name == "colorScale":
                rule.colorScale = ColorScale._from(child)
            elif name == "dataBar":
                rule.dataBar = DataBar._from(child)
            elif name == "iconSet":
                rule.iconSet = IconSet._from(child)
        rule.dxf = DifferentialStyle._load(dxf) if dxf is not None else None
        rule._extra = [(k, v) for k, v in attrs if k not in cls._ATTRS]
        rule._loaded = None
        rule._loaded = ((rule._attrs(), rule._content(), rule.dxf._xml() if rule.dxf is not None else None), (list(attrs), inner, dxf))
        return rule


def ColorScaleRule(start_type=None, start_value=None, start_color=None, mid_type=None, mid_value=None, mid_color=None, end_type=None, end_value=None,
                   end_color=None):
    formats = []
    if start_type is not None:
        formats.append(FormatObject(type=start_type, val=start_value))
    if mid_type is not None:
        formats.append(FormatObject(type=mid_type, val=mid_value))
    if end_type is not None:
        formats.append(FormatObject(type=end_type, val=end_value))
    colors = [_color(v) for v in (start_color, mid_color, end_color) if v is not None]
    return Rule(type="colorScale", colorScale=ColorScale(cfvo=formats, color=colors))


def FormulaRule(formula=None, stopIfTrue=None, font=None, border=None, fill=None):
    from .. import DifferentialStyle
    rule = Rule(type="expression", formula=formula, stopIfTrue=stopIfTrue)
    rule.dxf = DifferentialStyle(font=font, border=border, fill=fill)
    return rule


def CellIsRule(operator=None, formula=None, stopIfTrue=None, font=None, border=None, fill=None):
    from .. import DifferentialStyle
    rule = Rule(type="cellIs", operator=_OPERATORS.get(operator, operator), formula=formula, stopIfTrue=stopIfTrue)
    rule.dxf = DifferentialStyle(font=font, border=border, fill=fill)
    return rule


def IconSetRule(icon_style=None, type=None, values=None, showValue=None, percent=None, reverse=None):
    cfvo = [FormatObject(type, val) for val in values]
    return Rule(type="iconSet", iconSet=IconSet(iconSet=icon_style, cfvo=cfvo, showValue=showValue, percent=percent, reverse=reverse))


def DataBarRule(start_type=None, start_value=None, end_type=None, end_value=None, color=None, showValue=None, minLength=None, maxLength=None):
    data_bar = DataBar(cfvo=[FormatObject(start_type, start_value), FormatObject(end_type, end_value)], color=color, showValue=showValue,
                       minLength=minLength, maxLength=maxLength)
    return Rule(type="dataBar", dataBar=data_bar)
