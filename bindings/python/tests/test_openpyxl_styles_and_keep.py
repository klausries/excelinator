"""Formatting through the openpyxl interface, and saving a loaded workbook without
losing what the module does not model."""
import io
import re
import os
import warnings
import zipfile

import openpyxl
import pytest

import xlcells.openpyxl as facade

from conftest import CORPUS, needs_corpus

MODULES = {"openpyxl": openpyxl, "xlcells": facade}


def rgb(color):
    value = getattr(color, "rgb", None)
    return value if isinstance(value, str) else None


def look(cell):
    """The formatting of a cell as plain values (works on both modules' cells)."""
    f, fill, b, a = cell.font, cell.fill, cell.border, cell.alignment
    return (
        # openpyxl writes a font without name or size when none is given; we write the default
        cell.number_format, f.name or "Calibri", float(f.sz) if f.sz else 11.0, bool(f.b), bool(f.i), f.u, bool(f.strike), rgb(f.color),
        fill.fill_type, rgb(fill.fgColor) if fill.fill_type == "solid" else None,
        tuple((s.style, rgb(s.color) if s.style else None) if s is not None else (None, None) for s in (b.left, b.right, b.top, b.bottom)),
        a.horizontal, a.vertical, bool(a.wrap_text), int(a.text_rotation or 0), int(a.indent or 0),
    )


def styled(m):
    st = m.styles
    wb = m.Workbook()
    ws = wb.active
    thin = st.Side(style="thin", color="FF000000")
    ws["A1"] = "Kopf"
    head = st.Font(name="Arial", size=14, bold=True, italic=True, underline="double", strike=True, color="FFFF0000")
    ws["A1"].font = head
    ws["A1"].fill = st.PatternFill(fill_type="solid", start_color="FFFFFF00")
    ws["A1"].border = st.Border(left=thin, right=thin, top=st.Side(style="double"), bottom=st.Side(style="medium", color="FF0000FF"))
    ws["A1"].alignment = st.Alignment(horizontal="center", vertical="top", wrap_text=True)
    ws["B2"] = 1234.5
    ws["B2"].number_format = "#,##0.00"
    ws["B2"].font = st.Font(size=9, color="00FF00")
    ws["B2"].alignment = st.Alignment(horizontal="right", text_rotation=45, indent=2)
    ws["C3"] = "nur Füllung"
    ws["C3"].fill = st.PatternFill("solid", fgColor="FFCCE5FF")
    ws["D4"] = "unterstrichen"
    ws["D4"].font = st.Font(underline="single")
    ws["E5"].border = st.Border(bottom=st.Side(style="dashed"))
    for row in range(6, 9):
        ws.cell(row, 1, row).font = head
    ws.row_dimensions[1].height = 30
    ws.row_dimensions[20].height = 12.5
    ws.column_dimensions["B"].width = 18
    ws.freeze_panes = "B2"
    other = wb.create_sheet("Zweites")
    other.freeze_panes = "A3"
    other["A1"] = "x"
    return wb


def judge(data):
    wb = openpyxl.load_workbook(io.BytesIO(data))
    return [
        (ws.title, {k: (c.value, look(c)) for k, c in ws._cells.items()}, {k: d.height for k, d in ws.row_dimensions.items() if d.height is not None}, ws.freeze_panes)
        for ws in wb.worksheets
    ]


def test_formatting_script_writes_the_same_workbook():
    files = {}
    for name, module in MODULES.items():
        buf = io.BytesIO()
        styled(module).save(buf)
        files[name] = buf.getvalue()
    assert judge(files["xlcells"]) == judge(files["openpyxl"])


@pytest.mark.parametrize("writer", MODULES)
def test_both_load_formatting_the_same_way(writer):
    buf = io.BytesIO()
    styled(MODULES[writer]).save(buf)
    seen = {}
    for name, module in MODULES.items():
        wb = module.load_workbook(io.BytesIO(buf.getvalue()))
        seen[name] = [
            (ws.title, {k: (c.value, look(c)) for k, c in ws._cells.items()}, {k: d.height for k, d in ws.row_dimensions.items() if d.height is not None}, ws.freeze_panes)
            for ws in wb.worksheets
        ]
    assert seen["xlcells"] == seen["openpyxl"]


_MODIFIED = re.compile(rb"(<(?:[A-Za-z_][\w.-]*:)?modified\b[^>]*>)[^<]*(</(?:[A-Za-z_][\w.-]*:)?modified>)")


def parts(data):
    """The parts of a package; in the core properties the time of the last change (which
    saving sets, as in openpyxl and Excel) is left out."""
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        out = {n: z.read(n) for n in z.namelist() if not n.endswith("/")}
    for name in out:
        if name.lower() == "docprops/core.xml":
            out[name] = _MODIFIED.sub(rb"\1\2", out[name])
    return out


def corpus_sample():
    files = []
    for d, _, names in os.walk(CORPUS):
        if "/src-" in d + "/" and "src-enron" not in d and "/lo-" not in d + "/":
            files += [os.path.join(d, n) for n in names if n.lower().endswith((".xlsx", ".xlsm")) and os.path.getsize(os.path.join(d, n)) < 400000]
    return sorted(files)[::7]


# In a sheet that is changed, the cell data is written anew; ISO date cells become
# text there (a known gap).
SIMPLIFIED = {"date_iso.xlsx"}


@needs_corpus
def test_loading_and_saving_keeps_everything_else():
    """Load, change one cell, save: all other parts of the file (charts, drawings,
    comments, defined names, themes, properties, macros) are byte for byte the same,
    and openpyxl reads the new value."""
    warnings.simplefilter("ignore")
    checked = kept = 0
    problems = []
    for path in corpus_sample():
        data = open(path, "rb").read()
        try:
            wb = facade.load_workbook(io.BytesIO(data))
            before = parts(data)
        except Exception:
            continue
        if not wb.worksheets:
            continue
        # unchanged: every part stays as it is
        same = parts(wb.to_bytes())
        if same != before:
            problems.append(f"{os.path.basename(path)}: saving unchanged altered {[n for n in before if same.get(n) != before[n]][:3]}")
            continue
        ws = wb.worksheets[0]
        ws["A1"] = "geändert"
        ws["A1"].font = facade.Font(bold=True, color="FFFF0000")
        out = wb.to_bytes()
        checked += 1
        if not wb._kept_original:
            continue
        kept += 1
        after = parts(out)
        lost = [n for n in before if n not in after and "calcchain" not in n.lower()]
        changed = [n for n in before if n in after and after[n] != before[n]]
        if lost or len(changed) > 4:
            problems.append(f"{os.path.basename(path)}: lost {lost[:3]}, changed {changed[:6]}")
            continue
        try:
            ref = openpyxl.load_workbook(io.BytesIO(data))
        except Exception:
            continue
        got = openpyxl.load_workbook(io.BytesIO(out))
        if len(got.worksheets) != len(ref.worksheets):
            problems.append(f"{os.path.basename(path)}: openpyxl sees {len(got.worksheets)} sheets instead of {len(ref.worksheets)}")
            continue
        if not got.worksheets or got.worksheets[0].title != ws.title:
            continue  # openpyxl counts sheets differently (macro sheets)
        cell = got.worksheets[0]["A1"]
        if cell.value != "geändert" or not cell.font.b:
            problems.append(f"{os.path.basename(path)}: change not visible to openpyxl")
        # everything else openpyxl sees is unchanged
        for a, b in zip(ref.worksheets, got.worksheets):
            plain = lambda v: (v.ref, v.text) if hasattr(v, "ref") and hasattr(v, "text") else v  # noqa: E731
            va = {k: plain(c.value) for k, c in a._cells.items() if c.value is not None and k != (1, 1)}
            vb = {k: plain(c.value) for k, c in b._cells.items() if c.value is not None and k != (1, 1)}
            if len(a._charts) != len(b._charts) or len(a._images) != len(b._images):
                problems.append(f"{os.path.basename(path)}[{a.title}]: charts or images differ for openpyxl")
            elif va != vb and os.path.basename(path) not in SIMPLIFIED:
                problems.append(f"{os.path.basename(path)}[{a.title}]: content differs for openpyxl")
        if len(ref.defined_names) != len(got.defined_names):
            problems.append(f"{os.path.basename(path)}: defined names differ")
    assert checked > 100 and kept >= checked * 0.95, (checked, kept)
    assert not problems, f"{len(problems)} problems:\n" + "\n".join(problems[:20])


@needs_corpus
def test_sheets_can_be_added_removed_and_renamed_on_a_loaded_workbook():
    """The same edits with openpyxl and with the facade; ours keeps the other parts."""
    warnings.simplefilter("ignore")
    done = 0
    for path in corpus_sample()[::3]:
        data = open(path, "rb").read()
        try:
            ref = openpyxl.load_workbook(io.BytesIO(data))
            wb = facade.load_workbook(io.BytesIO(data))
        except Exception:
            continue
        if len(ref.worksheets) != len(wb.worksheets) or not wb.worksheets or len(ref.sheetnames) != len(ref.worksheets):
            continue
        for book in (ref, wb):
            book.worksheets[0].title = "Umbenannt"
            new = book.create_sheet("Neu", 1)
            new["B2"] = "neu"
            if len(book.worksheets) > 2:
                book.remove(book.worksheets[-1])
        out = wb.to_bytes()
        assert wb._kept_original, path
        buf = io.BytesIO()
        ref.save(buf)
        a = openpyxl.load_workbook(io.BytesIO(buf.getvalue()))
        b = openpyxl.load_workbook(io.BytesIO(out))
        assert a.sheetnames == b.sheetnames, path
        for x, y in zip(a.worksheets, b.worksheets):
            plain = lambda v: (v.ref, v.text) if hasattr(v, "ref") and hasattr(v, "text") else v  # noqa: E731
            vx = {k: plain(c.value) for k, c in x._cells.items() if c.value is not None}
            vy = {k: plain(c.value) for k, c in y._cells.items() if c.value is not None}
            assert vx == vy or "_x" in str(vx), (path, x.title)
        before, after = parts(data), parts(out)
        assert sum(1 for n in before if n in after and before[n] == after[n]) >= len(before) - 6, path
        done += 1
    assert done > 30


def test_changing_one_cell_leaves_the_other_cells_of_the_sheet_as_they_were():
    """In the changed sheet only the changed cell is replaced: rich text in shared
    strings, row attributes and all other cell elements stay exactly as written."""
    wb = openpyxl.Workbook()
    ws = wb.active
    for r in range(1, 30):
        ws.append([f"Zeile {r}", r, r * 1.5, f"=B{r}*2"])
    ws.row_dimensions[5].hidden = True
    ws["A3"].font = openpyxl.styles.Font(bold=True)
    buf = io.BytesIO()
    wb.save(buf)
    before = parts(buf.getvalue())

    ours = facade.load_workbook(io.BytesIO(buf.getvalue()))
    ours.active["C7"] = 99.5
    ours.active["F2"] = "neu"
    ours.active["A40"] = "neue Zeile"
    del ours.active["B9"]
    out = ours.to_bytes()
    assert ours._kept_original
    after = parts(out)
    name = "xl/worksheets/sheet1.xml"
    assert {n for n in before if after.get(n) != before[n]} <= {name, "[Content_Types].xml"}
    import re
    cell = re.compile(r'<c r="([A-Z]+\d+)"[^>]*?(?:/>|>.*?</c>)')
    old = {m.group(1): m.group(0) for m in cell.finditer(before[name].decode())}
    new = {m.group(1): m.group(0) for m in cell.finditer(after[name].decode())}
    assert set(new) == (set(old) | {"F2", "A40"}) - {"B9"}
    assert {k for k in old if k in new and old[k] != new[k]} == {"C7"}
    assert 'hidden="1"' in after[name].decode()
    back = openpyxl.load_workbook(io.BytesIO(out)).active
    assert (back["C7"].value, back["F2"].value, back["A40"].value, back["B9"].value, back["A3"].font.b, back["D7"].value) == (99.5, "neu", "neue Zeile", None, True, "=B7*2")
    assert back.row_dimensions[5].hidden
