"""The `_xHHHH_` escapes of ST_Xstring (ECMA-376 part 1, 22.9.2.19), as the core writes and reads them."""
import re

_ESCAPE = re.compile(r"_x[0-9A-Fa-f]{4}_")
_RUN = re.compile(r"(?:_x[0-9A-Fa-f]{4}_)+")


def escape_text(s):
    """Text for element content: XML escapes, `_x005F_` before what would read as an
    escape, `_xHHHH_` for characters XML cannot carry (and carriage returns)."""
    out = []
    for i, c in enumerate(s):
        if c == "&":
            out.append("&amp;")
        elif c == "<":
            out.append("&lt;")
        elif c == ">":
            out.append("&gt;")
        elif c == "_" and _ESCAPE.match(s, i):
            out.append("_x005F_")
        elif c in "\t\n":
            out.append(c)
        elif ord(c) < 0x20 or c in "￾￿":
            out.append("_x{0:04X}_".format(ord(c)))
        else:
            out.append(c)
    return "".join(out)


def escape_controls(s):
    """Only the ST_Xstring part of `escape_text`: control characters (line breaks and tabs
    too, for attributes) as `_xHHHH_`, `_x005F_` before what would read as an escape."""
    out = []
    for i, c in enumerate(s):
        if c == "_" and _ESCAPE.match(s, i):
            out.append("_x005F_")
        elif ord(c) < 0x20 or c in "\ufffe\uffff":
            out.append("_x{0:04X}_".format(ord(c)))
        else:
            out.append(c)
    return "".join(out)


def unescape(s):
    """Decode `_xHHHH_` (consecutive escapes may form a surrogate pair)."""
    if "_x" not in s:
        return s

    def run(m):
        units = [int(m.group(0)[i + 2:i + 6], 16) for i in range(0, len(m.group(0)), 7)]
        data = b"".join(u.to_bytes(2, "little") for u in units)
        return data.decode("utf-16-le", errors="replace")

    return _RUN.sub(run, s)


def t_element(text):
    space = ' xml:space="preserve"' if text != text.strip() else ""
    return "<t{0}>{1}</t>".format(space, escape_text(text))
