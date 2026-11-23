"""Small XML elements with declared attributes and children, as the worksheet keeps them
(page setup, margins, protection, views, ...).

A class lists its attributes as (name, type, default) and its child elements as
(name, class); `type` is bool, int, float or str. Attributes and children a class does
not know are kept as they were read and written back unchanged.
"""
import xml.etree.ElementTree as _ET

_NS_MAIN = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
_NS_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def _esc(text, quote=True):
    text = str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    return text.replace('"', "&quot;") if quote else text


def _local(tag):
    return tag.rsplit("}", 1)[-1]


def _attr_name(key):
    """`{ns-rel}id` → `r:id`; other names as they are."""
    if key.startswith("{"):
        ns, local = key[1:].split("}", 1)
        return "r:" + local if ns == _NS_REL else None
    return key


def _to_text(kind, value):
    if kind is bool:
        return "1" if value else "0"
    if kind is float:
        return repr(float(value)).rstrip("0").rstrip(".") if float(value) != int(float(value)) else str(int(float(value)))
    return str(value)


def _from_text(kind, text):
    if kind is bool:
        return text in ("1", "true", "True")
    if kind is int:
        try:
            return int(text)
        except ValueError:
            return int(float(text))
    if kind is float:
        return float(text)
    return text


def parse_fragment(xml):
    """The element of an XML fragment as written by the reader (`r:` declared on it)."""
    return _ET.fromstring(xml)


class Element:
    tagname = ""
    _attrs = ()  # (name, type, default)
    _children = ()  # (name, class); a class with `_text` holds text only
    _text = False

    def __init__(self, *args, **kw):
        names = [a[0] for a in self._attrs] + [c[0] for c in self._children]
        if len(args) > len(names):
            raise TypeError("too many arguments")
        for (name, value) in zip(names, args):
            kw.setdefault(name, value)
        for name, kind, default in self._attrs:
            object.__setattr__(self, name, kw.pop(name, default))
        for name, cls in self._children:
            object.__setattr__(self, name, kw.pop(name, None))
        self._other = {}
        self._other_children = []
        for name in list(kw):
            if name in getattr(self, "_aliases", {}):
                setattr(self, name, kw.pop(name))
        if kw:
            raise TypeError("{0}.__init__() got an unexpected keyword argument {1!r}".format(type(self).__name__, next(iter(kw))))

    # --- comparison -------------------------------------------------------------
    def _key(self):
        return tuple(getattr(self, a[0]) for a in self._attrs) + tuple(getattr(self, c[0]) for c in self._children)

    def __eq__(self, other):
        return type(other) is type(self) and self._key() == other._key()

    def __ne__(self, other):
        return not self == other

    __hash__ = None

    def __repr__(self):
        parts = ["{0}={1!r}".format(a[0], getattr(self, a[0])) for a in self._attrs if getattr(self, a[0]) is not None]
        return "<{0} {1}>".format(type(self).__name__, " ".join(parts))

    def __iter__(self):
        """(name, text) of the attributes that are set, as openpyxl's objects give them."""
        for name, kind, _ in self._attrs:
            value = getattr(self, name)
            if value is not None:
                yield name, _to_text(kind, value)

    # --- XML ----------------------------------------------------------------------
    def _attr_xml(self):
        out = []
        for name, kind, _ in self._attrs:
            value = getattr(self, name)
            if value is not None:
                out.append(' {0}="{1}"'.format(getattr(self, "_xml_names", {}).get(name, name), _esc(_to_text(kind, value))))
        for key, value in self._other.items():
            out.append(' {0}="{1}"'.format(key, _esc(value)))
        return "".join(out)

    def _inner_xml(self):
        out = []
        for name, cls in self._children:
            value = getattr(self, name)
            if value is None:
                continue
            for item in value if isinstance(value, list) else [value]:
                out.append(item._xml(getattr(self, "_child_tags", {}).get(name, name)) if isinstance(item, Element) else "")
        out.extend(self._other_children)
        return "".join(out)

    def _xml(self, tag=None):
        tag = tag or self.tagname
        if self._text:
            text = self._text_value()
            return "<{0}{1}>{2}</{0}>".format(tag, self._attr_xml(), _esc(text, False)) if text else ""
        inner = self._inner_xml()
        attrs = self._attr_xml()
        if "r:" in attrs and 'xmlns:r=' not in attrs:
            attrs += ' xmlns:r="{0}"'.format(_NS_REL)
        if not inner:
            return "<{0}{1}/>".format(tag, attrs)
        return "<{0}{1}>{2}</{0}>".format(tag, attrs, inner)

    def _is_empty(self):
        return all(getattr(self, a[0]) == a[2] for a in self._attrs) and all(getattr(self, c[0]) in (None, []) for c in self._children) and not self._other and not self._other_children

    @classmethod
    def _from_element(cls, el):
        obj = cls.__new__(cls)
        Element.__init__(obj)
        known = {getattr(cls, "_xml_names", {}).get(a[0], a[0]): a for a in cls._attrs}
        for key, value in el.attrib.items():
            name = _attr_name(key)
            if name is None:
                continue
            if name in known:
                a = known[name]
                try:
                    object.__setattr__(obj, a[0], _from_text(a[1], value))
                except ValueError:
                    obj._other[name] = value
            elif not key.startswith("{http://www.w3.org/2000/xmlns"):
                obj._other[name] = value
        children = dict(cls._children)
        tags = {v: k for k, v in getattr(cls, "_child_tags", {}).items()}
        lists = getattr(cls, "_lists", ())
        for child in el:
            local = _local(child.tag)
            name = tags.get(local, local)
            if name in children and (not child.tag.startswith("{") or child.tag.startswith("{" + _NS_MAIN + "}")):
                item = children[name]._from_element(child)
                if name in lists:
                    current = getattr(obj, name) or []
                    current.append(item)
                    object.__setattr__(obj, name, current)
                else:
                    object.__setattr__(obj, name, item)
            else:
                obj._other_children.append(_ET.tostring(child, encoding="unicode"))
        if cls._text:
            obj._set_text(el.text or "")
        obj._loaded()
        return obj

    def _loaded(self):
        pass

    @classmethod
    def _from_xml(cls, xml):
        return cls._from_element(parse_fragment(xml))


class TextElement(Element):
    """An element holding text (`<oddHeader>&amp;CSeite</oddHeader>`)."""
    _text = True

    def _text_value(self):
        return getattr(self, "text", "") or ""

    def _set_text(self, text):
        self.text = text
