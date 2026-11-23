"""Document properties (`wb.properties`, docProps/core.xml): title, subject, author
(`creator`), keywords, description, dates and the rest of Dublin Core and OPC.

Behaves like openpyxl's `DocumentProperties` (same names and defaults: creator
"openpyxl", created and modified now; modified is set when saving). Elements the class
does not know are kept as read."""
import datetime as _dt
import re as _re
import xml.etree.ElementTree as _ET

from .._serial import _esc

__all__ = ["DocumentProperties"]

CP = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
DC = "http://purl.org/dc/elements/1.1/"
DCTERMS = "http://purl.org/dc/terms/"
XSI = "http://www.w3.org/2001/XMLSchema-instance"
_PREFIX = {CP: "cp", DC: "dc", DCTERMS: "dcterms", XSI: "xsi"}

# (attribute, namespace, element) in openpyxl's order.
_FIELDS = (("creator", DC, "creator"), ("title", DC, "title"), ("description", DC, "description"), ("subject", DC, "subject"),
           ("identifier", DC, "identifier"), ("language", DC, "language"), ("created", DCTERMS, "created"), ("modified", DCTERMS, "modified"),
           ("lastModifiedBy", CP, "lastModifiedBy"), ("category", CP, "category"), ("contentStatus", CP, "contentStatus"), ("version", CP, "version"),
           ("revision", CP, "revision"), ("keywords", CP, "keywords"), ("lastPrinted", CP, "lastPrinted"))
_DATES = ("created", "modified", "lastPrinted")


def _now():
    return _dt.datetime.now(tz=_dt.timezone.utc).replace(tzinfo=None)


def _parse_date(text):
    text = (text or "").strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1]
    # Python before 3.11 reads fractions of a second with 3 or 6 digits only; files have
    # any number (`22.00`, `22.8702135`).
    text = _re.sub(r"\.(\d+)", lambda m: "." + (m.group(1) + "000000")[:6], text, count=1)
    try:
        value = _dt.datetime.fromisoformat(text)
    except ValueError:
        return None
    if value.tzinfo is not None:
        value = value.astimezone(_dt.timezone.utc).replace(tzinfo=None)
    return value


class DocumentProperties:
    tagname = "coreProperties"

    def __init__(self, category=None, contentStatus=None, keywords=None, lastModifiedBy=None, lastPrinted=None, revision=None, version=None,
                 created=None, creator="openpyxl", description=None, identifier=None, language=None, modified=None, subject=None, title=None):
        now = _now()
        self.category, self.contentStatus, self.keywords, self.lastModifiedBy = category, contentStatus, keywords, lastModifiedBy
        self.lastPrinted, self.revision, self.version, self.creator = lastPrinted, revision, version, creator
        self.created, self.modified = created or now, modified or now
        self.description, self.identifier, self.language, self.subject, self.title = description, identifier, language, subject, title
        self._other = []  # elements this class does not know, as XML

    last_modified_by = property(lambda self: self.lastModifiedBy, lambda self, v: setattr(self, "lastModifiedBy", v))

    def __eq__(self, other):
        return type(other) is type(self) and all(getattr(self, f[0]) == getattr(other, f[0]) for f in _FIELDS) and self._other == other._other

    def __repr__(self):
        return "<DocumentProperties {0}>".format(", ".join("{0}={1!r}".format(f[0], getattr(self, f[0])) for f in _FIELDS if getattr(self, f[0]) is not None))

    def _xml(self):
        x = ['<cp:coreProperties xmlns:cp="{0}" xmlns:dc="{1}" xmlns:dcterms="{2}" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="{3}">'
             .format(CP, DC, DCTERMS, XSI)]
        for attr, ns, name in _FIELDS:
            value = getattr(self, attr)
            if value is None:
                continue
            tag = "{0}:{1}".format(_PREFIX[ns], name)
            if attr in _DATES:
                text = value.replace(tzinfo=None).isoformat(timespec="seconds") + "Z" if isinstance(value, _dt.datetime) else str(value)
                kind = ' xsi:type="dcterms:W3CDTF"' if ns == DCTERMS else ""
                x.append("<{0}{1}>{2}</{0}>".format(tag, kind, text))
            else:
                x.append("<{0}>{1}</{0}>".format(tag, _esc(value, False)))
        x.extend(self._other)
        x.append("</cp:coreProperties>")
        return "".join(x)

    @classmethod
    def _from_xml(cls, xml):
        root = _ET.fromstring(xml)
        # As openpyxl: what the file leaves out takes the defaults (author "openpyxl",
        # created and modified now).
        obj = cls()
        # By local name: some writers put cp: elements under dc: (as openpyxl, accepted).
        known = {name: attr for attr, ns, name in _FIELDS}
        for child in root:
            attr = known.get(child.tag.rsplit("}", 1)[-1])
            if attr is None:
                obj._other.append(_ET.tostring(child, encoding="unicode"))
            elif attr in _DATES:
                value = _parse_date(child.text)
                if value is not None or attr == "lastPrinted":
                    setattr(obj, attr, value)
            else:
                setattr(obj, attr, child.text or None)
        return obj

    def _key(self, dates=True):
        return tuple(getattr(self, f[0]) for f in _FIELDS if dates or f[0] not in ("created", "modified")) + tuple(self._other)

    def _xml_onto(self, raw):
        """The XML as read with only the modification time changed, if nothing else
        changed (and the file has that element); else the XML of the object."""
        import re
        try:
            before = DocumentProperties._from_xml(raw)
        except Exception:
            return self._xml()
        if any(getattr(before, f[0]) != getattr(self, f[0]) for f in _FIELDS if f[0] != "modified") or before._other != self._other:
            return self._xml()
        pattern = re.compile(r"(<(?:[A-Za-z_][\w.-]*:)?modified\b[^>]*>)[^<]*(</(?:[A-Za-z_][\w.-]*:)?modified>)")
        if self.modified is None or len(pattern.findall(raw)) != 1:
            return self._xml()
        stamp = self.modified.replace(tzinfo=None).isoformat(timespec="seconds") + "Z"
        return pattern.sub(lambda m: m.group(1) + stamp + m.group(2), raw)

    @classmethod
    def from_tree(cls, node):
        return cls._from_xml(_ET.tostring(node, encoding="unicode"))
