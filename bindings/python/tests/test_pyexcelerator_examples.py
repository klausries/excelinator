"""The example scripts of pyExcelerator, run unchanged against two implementations.

Each script from the pyExcelerator distribution is executed twice: once with xlwt
(the maintained Python 3 descendant of pyExcelerator) standing in as `pyExcelerator`,
once with `xlcells.pyexcelerator`. The reference result (.xls) is read with xlrd, ours
with our own reader and with openpyxl. Values, number formats, bold/italic, merged
ranges and column widths must agree. Formula cells have no stored value in either
file; they are compared after LibreOffice has calculated both files.

The scripts are not part of this repository (tools/fetch_corpus.sh fetches them).
"""
import datetime
import io
import os
import pathlib
import re
import subprocess
import sys
import types

import openpyxl
import pytest
import xlrd
import xlwt

import xlcells
import xlcells.pyexcelerator as ours

from conftest import CORPUS, ROOT

EXAMPLES = CORPUS / "ref-pyexcelerator" / "examples"

# Scripts that are no workbook-writing examples, and what this layer does not write yet.
NOT_RUN = {
    "xls2csv.py": "command line tool",
    "xls2html.py": "command line tool",
    "xls2txt.py": "command line tool",
    "parse-fmla.py": "tests pyExcelerator's internal formula compiler",
    "big-35Mb.py": "same as big-16Mb, larger",
    "image.py": "bitmaps are not written yet",
    "unicode0.py": "tests pyExcelerator's internal string packing",
    "sst.py": "tests pyExcelerator's internal shared string table",
    "wsprops.py": "worksheet view and print properties are not implemented yet",
}

# Source changes needed to run Python 2 scripts deterministically on Python 3.
PATCHES = {
    "*": [(re.compile(r"^(\s*)print (.*?),?\s*$", re.M), r"\1print(\2)"), ("i/0x10", "i//0x10")],
    "dates.py": [("datetime.now()", "datetime(2005, 7, 20, 12, 34, 56)")],
    "big-16Mb.py": [("rowcount = ", "rowcount = 500 or ")],
}

# Scripts whose input no workbook can hold: (reference's message, our message).
BOTH_REJECT = {
    "unicode2.py": ("surrogates not allowed", "surrogates not allowed"),
}

# Differences that are intended, per example: reason.
KNOWN = {}


def source(name):
    text = (EXAMPLES / name).read_text(encoding="latin-1")
    for pattern, repl in PATCHES["*"] + PATCHES.get(name, []):
        text = pattern.sub(repl, text) if hasattr(pattern, "sub") else text.replace(pattern, repl)
    return text


class _RefFormula(xlwt.Formula):
    """xlwt's Formula takes the text only; pyExcelerator's also a value and options."""

    def __init__(self, text, *ignored):
        super().__init__(text)


class _RefWorkbook(xlwt.Workbook):
    """pyExcelerator lets a script write a cell twice; xlwt needs to be told."""

    def add_sheet(self, name):
        return super().add_sheet(name, cell_overwrite_ok=True)


def _excel_magic():
    """pyExcelerator's own tables (function names etc.), loaded from its source."""
    module = types.ModuleType("ExcelMagic")
    path = CORPUS / "ref-pyexcelerator" / "pyExcelerator" / "ExcelMagic.py"
    exec(compile(path.read_text(encoding="latin-1"), str(path), "exec"), module.__dict__)
    return module


def namespace(flavour):
    """A module tree that answers `from pyExcelerator import *` and `...Worksheet import *`."""
    if flavour == "ref":
        names = {n: getattr(xlwt, n) for n in ("Worksheet", "Font", "Alignment", "Borders", "Pattern", "Protection", "XFStyle")}
        names["Workbook"] = _RefWorkbook
        names["Formula"] = _RefFormula
        names["ErrorCode"] = ours.ErrorCode
    else:
        names = {n: getattr(ours, n) for n in ours.__all__}
    # The list of function names is data both implementations are driven with.
    names.update(NoCalcs=0, RecalcAlways=1, CalcOnOpen=2, ExcelMagic=_excel_magic())
    package = types.ModuleType("pyExcelerator")
    for sub in ("Worksheet", "Workbook", "Style", "Formatting", "ExcelFormula"):
        module = types.ModuleType("pyExcelerator." + sub)
        module.__dict__.update(names)
        module.__all__ = list(names)
        sys.modules["pyExcelerator." + sub] = module
        if sub not in names:  # as in the original, the classes Workbook and Worksheet shadow their modules
            setattr(package, sub, module)
    package.__dict__.update(names)
    package.__all__ = list(names) + ["Style", "Formatting", "ExcelFormula"]
    sys.modules["pyExcelerator"] = package


def run(name, flavour, outdir):
    """Execute an example; returns the files it saved."""
    namespace(flavour)
    outdir.mkdir(parents=True, exist_ok=True)
    cwd = os.getcwd()
    os.chdir(outdir)
    try:
        env = {"__name__": "__main__", "xrange": range, "unichr": chr, "unicode": str, "long": int}
        exec(compile(source(name), name, "exec"), env)
    finally:
        os.chdir(cwd)
        for key in [k for k in sys.modules if k == "pyExcelerator" or k.startswith("pyExcelerator.")]:
            del sys.modules[key]
    return sorted(p for p in outdir.iterdir() if p.suffix == ".xls")


ERRORS = {0x00: "#NULL!", 0x07: "#DIV/0!", 0x0F: "#VALUE!", 0x17: "#REF!", 0x1D: "#NAME?", 0x24: "#NUM!", 0x2A: "#N/A"}


def reference_view(path):
    """What xlrd sees in the reference file."""
    book = xlrd.open_workbook(str(path), formatting_info=True)
    view = []
    for sheet in book.sheets():
        cells = {}
        for r in range(sheet.nrows):
            for c in range(sheet.ncols):
                cell = sheet.cell(r, c)
                xf = book.xf_list[cell.xf_index]
                fmt = book.format_map[xf.format_key].format_str
                font = book.font_list[xf.font_index]
                style = (fmt, bool(font.bold), bool(font.italic))
                if cell.ctype in (xlrd.XL_CELL_EMPTY, xlrd.XL_CELL_BLANK):
                    value = None
                elif cell.ctype == xlrd.XL_CELL_BOOLEAN:
                    value = bool(cell.value)
                elif cell.ctype == xlrd.XL_CELL_ERROR:
                    value = ERRORS.get(cell.value, "#ERR")
                else:
                    value = cell.value
                cells[(r, c)] = (value, style)
        merges = sorted((rlo, clo, rhi - 1, chi - 1) for rlo, rhi, clo, chi in sheet.merged_cells)
        widths = {c: info.width for c, info in sheet.colinfo_map.items() if info.width != 2962 and c < 256}
        view.append((sheet.name, cells, merges, widths))
    return view


def our_view(path):
    """What openpyxl sees in our file, plus our own reader's values and formulas."""
    data = path.read_bytes()
    table = xlcells.read(data)
    formulas = {(s, r, c) for s, r, c, f in zip(table["sheet"], table["row"], table["col"], table["formula"]) if f}
    parsed = ours.parse_xls(data)
    book = openpyxl.load_workbook(io.BytesIO(data))
    view = []
    for index, sheet in enumerate(book.worksheets):
        cells = {}
        for row in sheet.iter_rows():
            for cell in row:
                value = cell.value
                if isinstance(value, (datetime.datetime, datetime.date, datetime.time, datetime.timedelta)):
                    value = "date"
                elif isinstance(value, (int, float)) and not isinstance(value, bool):
                    value = float(value)
                cells[(cell.row - 1, cell.column - 1)] = (value, (cell.number_format, bool(cell.font.b), bool(cell.font.i)))
        merges = sorted((m.min_row - 1, m.min_col - 1, m.max_row - 1, m.max_col - 1) for m in sheet.merged_cells.ranges)
        widths = {openpyxl.utils.column_index_from_string(k) - 1: round(d.width * 256) for k, d in sheet.column_dimensions.items() if d.customWidth}
        view.append((sheet.title, cells, merges, widths, parsed[index][1]))
    return view, formulas


# xlwt (like pyExcelerator) maps some requested format strings to a neighbouring
# built-in format id, so the reference file shows a different string than the script
# asked for. Ours stores the requested string. (reference, ours) pairs seen for that:
REFERENCE_FORMAT_QUIRKS = {
    ('#,##0_);(#,##0)', '_(#,##0_);(#,##0)'),
    ('#,##0_);[Red](#,##0)', '_(#,##0_);[Red](#,##0)'),
    ('#,##0.00_);(#,##0.00)', '_(#,##0.00_);(#,##0.00)'),
    ('#,##0.00_);[Red](#,##0.00)', '_(#,##0.00_);[Red](#,##0.00)'),
    ('_(* #,##0_);_(* (#,##0);_(* "-"_);_(@_)', '_("$"* #,##0_);_("$"* (#,##0);_("$"* "-"_);_(@_)'),
    ('_($* #,##0_);_($* (#,##0);_($* "-"_);_(@_)', '_(* #,##0_);_(* (#,##0);_(* "-"_);_(@_)'),
    ('_(* #,##0.00_);_(* (#,##0.00);_(* "-"??_);_(@_)', '_("$"* #,##0.00_);_("$"* (#,##0.00);_("$"* "-"??_);_(@_)'),
    ('_($* #,##0.00_);_($* (#,##0.00);_($* "-"??_);_(@_)', '_(* #,##0.00_);_(* (#,##0.00);_(* "-"??_);_(@_)'),
}


def same_format(a, b):
    return a.lower() == b.lower() or (a, b) in REFERENCE_FORMAT_QUIRKS


def compare(name, ref_file, our_file):
    """Differences between the reference file and ours, as readable lines."""
    diffs = []
    ref = reference_view(ref_file)
    mine, formulas = our_view(our_file)
    if [s[0] for s in ref] != [s[0] for s in mine]:
        return [f"sheet names {[s[0] for s in ref]} != {[s[0] for s in mine]}"]
    for index, ((title, rcells, rmerges, rwidths), (_, ocells, omerges, owidths, parsed)) in enumerate(zip(ref, mine)):
        if sorted(set(rmerges)) != omerges:
            diffs.append(f"{title}: merges {rmerges[:4]} != {omerges[:4]}")
        if rwidths != owidths:
            diffs.append(f"{title}: column widths {rwidths} != {owidths}")
        for at in sorted(set(rcells) | set(ocells)):
            rvalue, rstyle = rcells.get(at, (None, ("General", False, False)))
            ovalue, ostyle = ocells.get(at, (None, ("General", False, False)))
            if (index, *at) in formulas:
                continue  # compared after calculation
            own = parsed.get(at)
            if rvalue is None:
                if ovalue is not None or own is not None:
                    diffs.append(f"{title}{at}: reference empty, ours {ovalue!r}")
            else:
                if own != rvalue:
                    diffs.append(f"{title}{at}: value {rvalue!r}, our reader {own!r}")
                if ovalue != "date" and ovalue != rvalue:
                    diffs.append(f"{title}{at}: value {rvalue!r}, openpyxl reads ours as {ovalue!r}")
            styled = rvalue is not None or at in ocells and ocells[at][0] is not None
            if styled and (not same_format(rstyle[0], ostyle[0]) or rstyle[1:] != ostyle[1:]):
                diffs.append(f"{title}{at}: style {rstyle} != {ostyle}")
    return diffs


def names():
    if not EXAMPLES.is_dir():
        return []
    return sorted(p.name for p in EXAMPLES.glob("*.py") if p.name not in NOT_RUN)


@pytest.fixture(scope="session")
def results(tmp_path_factory):
    """Run every example with both implementations, then let LibreOffice calculate all files."""
    base = tmp_path_factory.mktemp("examples")
    out = {}
    for name in names():
        entry = {"error": None}
        try:
            entry["ref"] = run(name, "ref", base / "in" / "ref" / name)
        except Exception as e:  # the reference cannot run this script
            entry["error"] = f"reference: {type(e).__name__}: {e}"
            try:
                run(name, "ours", base / "rejected" / name)
            except Exception as e2:
                entry["ours_error"] = f"{type(e2).__name__}: {e2}"
            out[name] = entry
            continue
        try:
            entry["ours"] = run(name, "ours", base / "in" / "ours" / name)
        except Exception as e:
            entry["error"] = f"ours: {type(e).__name__}: {e}"
        out[name] = entry
    calc = base / "calc"
    try:
        subprocess.run(["/usr/bin/python3", str(ROOT / "tools" / "lo_oracle.py"), str(base / "in"), str(calc), "xls"], check=True, timeout=1200, capture_output=True)
        out["__calc__"] = calc
    except Exception as e:
        out["__calc__"] = None
        out["__calc_error__"] = str(e)
    return out


@pytest.mark.parametrize("name", names())
def test_example_matches_reference(name, results):
    entry = results[name]
    if entry["error"] and entry["error"].startswith("reference:"):
        if name in BOTH_REJECT:
            # Both implementations must refuse the script's input, each for the stated reason.
            assert re.search(BOTH_REJECT[name][0], entry["error"]), entry["error"]
            assert re.search(BOTH_REJECT[name][1], entry.get("ours_error") or "ours accepted it"), entry.get("ours_error")
            return
        pytest.skip(f"{name}: {entry['error']}; ours: {entry.get('ours_error') or 'ran'}")
    assert entry["error"] is None, entry["error"]
    assert [p.name for p in entry["ref"]] == [p.name for p in entry["ours"]]
    diffs = []
    for ref_file, our_file in zip(entry["ref"], entry["ours"]):
        diffs += compare(name, ref_file, our_file)
    if name in KNOWN:
        pytest.xfail(KNOWN[name])
    assert not diffs, f"{len(diffs)} differences:\n" + "\n".join(diffs[:25])


@pytest.mark.parametrize("name", names())
def test_formulas_calculate_to_the_same_values(name, results):
    entry = results[name]
    if entry["error"]:
        pytest.skip(entry["error"])
    calc = results["__calc__"]
    if calc is None:
        pytest.skip("LibreOffice not available: " + results.get("__calc_error__", ""))
    diffs = []
    checked = 0
    for ref_file, our_file in zip(entry["ref"], entry["ours"]):
        table = xlcells.read(our_file.read_bytes())
        formulas = {(s, r, c): f for s, r, c, f in zip(table["sheet"], table["row"], table["col"], table["formula"]) if f}
        if not formulas:
            continue
        ref_calc = ours.parse_xls(str(calc / "ref" / name / (ref_file.name + ".xlsx")))
        our_calc = ours.parse_xls(str(calc / "ours" / name / (our_file.name + ".xlsx")))
        for (s, r, c), f in sorted(formulas.items()):
            if re.search(r"(?i)\b(now|today|rand)\(", f):
                continue
            a, b = ref_calc[s][1].get((r, c)), our_calc[s][1].get((r, c))
            checked += 1
            if a != b and not (isinstance(a, float) and isinstance(b, float) and abs(a - b) <= 1e-9 * max(abs(a), abs(b))):
                diffs.append(f"{ref_file.name} sheet {s} ({r}, {c}) ={f}: reference {a!r}, ours {b!r}")
    if not checked:
        pytest.skip("no formulas")
    assert not diffs, f"{len(diffs)} of {checked} formulas differ:\n" + "\n".join(diffs[:25])
