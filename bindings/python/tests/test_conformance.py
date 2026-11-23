"""The shared conformance cases: the Python binding must produce the same file bytes
and read back the same table as the Rust reference (tests/conformance/expected.txt)."""
import struct

import xlcells

from conftest import ROOT

DIR = ROOT / "tests" / "conformance"


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def text(field):
    return None if field == "-" else "" if field == "=" else bytes.fromhex(field).decode("utf-8")


def run(lines):
    w = xlcells.Writer()
    styles = [0]
    sheet = 0
    for line in lines:
        f = line.split(" ")
        if f[0] == "date1904":
            w.set_date1904(f[1] == "1")
        elif f[0] == "style":
            styles.append(w.add_style(text(f[3]), f[1] == "1", f[2] == "1"))
        elif f[0] == "sheet":
            sheet = w.add_sheet(text(f[1]))
        elif f[0] == "merge":
            w.merge(sheet, *map(int, f[1:5]))
        elif f[0] == "width":
            w.set_col_width(sheet, int(f[1]), int(f[2]))
        elif f[0] == "cell":
            num = struct.unpack(">d", bytes.fromhex(f[4]))[0]
            w.write(sheet, int(f[1]), int(f[2]), int(f[3]), num, text(f[6]), text(f[7]), styles[int(f[5])])
    data = w.to_bytes()
    t = xlcells.read(data)
    dump = "".join(f"S\t{s}\n" for s in t["sheet_names"]) + f"D\t{int(t['date1904'])}\n" + "".join(f"N\t{s}\n" for s in t["num_format"])
    for i in range(len(t["row"])):
        bits = struct.pack(">d", t["num"][i]).hex()
        txt = t["text"][i] if t["kind"][i] in (2, 4) else "\\N"
        formula = t["formula"][i] or "\\N"
        dump += f"C\t{t['sheet'][i]}\t{t['row'][i]}\t{t['col'][i]}\t{t['kind'][i]}\t{bits}\t{int(t['is_date'][i])}\t{t['style'][i]}\t{txt}\t{formula}\n"
    return len(data), fnv(data), fnv(dump.encode("utf-8"))


def test_conformance():
    got = []
    for block in (DIR / "cases.txt").read_text(encoding="utf-8").split("\nend\n"):
        lines = block.split("\n")
        if not lines[0].startswith("case "):
            continue
        size, file_hash, table_hash = run(lines[1:])
        got.append(f"{lines[0][5:]}\t{size}\t{file_hash:016x}\t{table_hash:016x}")
    expected = (DIR / "expected.txt").read_text().splitlines()
    assert len(got) == len(expected) >= 45
    assert got == expected
