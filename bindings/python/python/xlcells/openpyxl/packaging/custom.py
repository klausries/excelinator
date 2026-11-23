"""Custom document properties (`wb.custom_doc_props`, docProps/custom.xml): named values
of type text, whole number, number, date, yes/no or link, as openpyxl's
`packaging.custom`."""
import datetime as _dt
import xml.etree.ElementTree as _ET

from .._serial import _esc
from .core import _parse_date

__all__ = ["CustomPropertyList", "StringProperty", "IntProperty", "FloatProperty", "DateTimeProperty", "BoolProperty", "LinkProperty"]

NS = "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"
VT = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"
FMTID = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}"


class _TypedProperty:
    _kind = None

    def __init__(self, name, value):
        if not isinstance(name, str):
            raise TypeError("expected <class 'str'>")
        self.name = name
        self.value = self._check(value)

    @staticmethod
    def _check(value):
        return value

    def __eq__(self, other):
        return self.name == other.name and self.value == other.value

    def __repr__(self):
        return "{0}, name={1}, value={2}".format(type(self).__name__, self.name, self.value)


class IntProperty(_TypedProperty):
    _kind = "i4"
    _check = staticmethod(lambda v: int(v))


class FloatProperty(_TypedProperty):
    _kind = "r8"
    _check = staticmethod(lambda v: float(v))


class StringProperty(_TypedProperty):
    _kind = "lpwstr"
    _check = staticmethod(lambda v: None if v is None else str(v))


class DateTimeProperty(_TypedProperty):
    _kind = "filetime"

    @staticmethod
    def _check(value):
        if not isinstance(value, _dt.datetime):
            raise TypeError("expected <class 'datetime.datetime'>")
        return value


class BoolProperty(_TypedProperty):
    _kind = "bool"
    _check = staticmethod(lambda v: v if isinstance(v, bool) else str(v).lower() in ("1", "true"))


class LinkProperty(_TypedProperty):
    """A value linked to a cell or name (`value` is the link target)."""
    _kind = "lpwstr"
    _check = staticmethod(lambda v: str(v))


_BY_KIND = {"lpwstr": StringProperty, "i4": IntProperty, "r8": FloatProperty, "filetime": DateTimeProperty, "bool": BoolProperty}


def _text(prop):
    v = prop.value
    if isinstance(prop, LinkProperty):
        return ""
    if isinstance(prop, BoolProperty):
        return "true" if v else "false"
    if isinstance(prop, DateTimeProperty):
        return v.replace(tzinfo=None).isoformat(timespec="seconds") + "Z"
    if isinstance(prop, FloatProperty):
        return repr(v)
    return "" if v is None else str(v)


class CustomPropertyList:
    def __init__(self):
        self.props = []

    def append(self, prop):
        if prop.name in self.names:
            raise ValueError("Property with name {0} already exists".format(prop.name))
        self.props.append(prop)

    def __len__(self):
        return len(self.props)

    @property
    def names(self):
        return [p.name for p in self.props]

    def __getitem__(self, name):
        for p in self.props:
            if p.name == name:
                return p
        raise KeyError("Property with name {0} not found".format(name))

    def __delitem__(self, name):
        for idx, p in enumerate(self.props):
            if p.name == name:
                self.props.pop(idx)
                return
        raise KeyError("Property with name {0} not found".format(name))

    def __iter__(self):
        return iter(self.props)

    def __repr__(self):
        return "{0} containing {1}".format(type(self).__name__, self.props)

    def __eq__(self, other):
        return isinstance(other, CustomPropertyList) and self.props == other.props

    def _xml(self):
        x = ['<Properties xmlns="{0}" xmlns:vt="{1}">'.format(NS, VT)]
        for pid, p in enumerate(self.props, 2):
            link = ' linkTarget="{0}"'.format(_esc(p.value)) if isinstance(p, LinkProperty) else ""
            x.append('<property fmtid="{0}" pid="{1}" name="{2}"{3}><vt:{4}>{5}</vt:{4}></property>'.format(
                FMTID, pid, _esc(p.name), link, p._kind, _esc(_text(p), False)))
        x.append("</Properties>")
        return "".join(x)

    @classmethod
    def _from_xml(cls, xml):
        import warnings
        out = cls()
        for node in _ET.fromstring(xml):
            if node.tag != "{%s}property" % NS:
                continue
            name = node.attrib.get("name")
            link = node.attrib.get("linkTarget")
            if link is not None:
                out.props.append(LinkProperty(name, link))
                continue
            child = next(iter(node), None)
            kind = child.tag.rsplit("}", 1)[-1] if child is not None else None
            typ = _BY_KIND.get(kind)
            if typ is None:
                warnings.warn("Unknown type for {0}".format(name))
                continue
            text = child.text
            value = _parse_date(text or "") if typ is DateTimeProperty else text
            out.props.append(typ(name, value))
        return out
