#!/usr/bin/env python3
"""How much of openpyxl's public interface the xlcells.openpyxl facade offers.

Compares, for the modules and classes scripts use, the public names of openpyxl with those
of the facade: a name is covered when the facade has it under the same path. Prints one
line per gap and a summary; with --markdown a table per module.

usage: openpyxl_coverage.py [--markdown]
"""
import importlib, inspect, sys

# Module paths scripts import from, and the classes whose members they use.
MODULES = [
    "", "styles", "styles.colors", "styles.fills", "styles.borders", "styles.fonts", "styles.alignment",
    "styles.protection", "styles.named_styles", "styles.numbers", "utils", "utils.cell", "utils.datetime",
    "worksheet", "worksheet.table", "worksheet.datavalidation", "worksheet.hyperlink", "worksheet.page",
    "worksheet.properties", "worksheet.protection", "worksheet.dimensions", "worksheet.filters",
    "worksheet.cell_range", "worksheet.views", "worksheet.header_footer", "worksheet.formula",
    "workbook", "workbook.defined_name", "workbook.protection", "workbook.properties",
    "formatting", "formatting.rule", "comments", "drawing", "drawing.image", "chart", "chart.reference",
    "chart.series", "chart.label", "chart.trendline", "chart.layout", "cell", "cell.cell", "cell.rich_text", "cell.text",
    "packaging.core", "formula", "formula.translate",
]

CLASSES = {
    "Workbook": ("openpyxl.workbook.workbook", "Workbook"),
    "Worksheet": ("openpyxl.worksheet.worksheet", "Worksheet"),
    "Cell": ("openpyxl.cell.cell", "Cell"),
    "ColumnDimension": ("openpyxl.worksheet.dimensions", "ColumnDimension"),
    "RowDimension": ("openpyxl.worksheet.dimensions", "RowDimension"),
    "DefinedName": ("openpyxl.workbook.defined_name", "DefinedName"),
    "DataValidation": ("openpyxl.worksheet.datavalidation", "DataValidation"),
    "Font": ("openpyxl.styles", "Font"),
    "PatternFill": ("openpyxl.styles", "PatternFill"),
    "Border": ("openpyxl.styles", "Border"),
    "Alignment": ("openpyxl.styles", "Alignment"),
}
# Members inherited from openpyxl's serialisation machinery, not part of what scripts use.
MACHINERY = {"tagname", "namespace", "idx_base", "from_tree", "to_tree", "mime_type", "path", "rel_type", "BREAK_NONE",
             "BREAK_ROW", "BREAK_COLUMN", "SHEETSTATE_VISIBLE", "SHEETSTATE_HIDDEN", "SHEETSTATE_VERYHIDDEN",
             "PAPERSIZE_LETTER", "PAPERSIZE_LETTER_SMALL", "PAPERSIZE_TABLOID", "PAPERSIZE_LEDGER", "PAPERSIZE_LEGAL",
             "PAPERSIZE_STATEMENT", "PAPERSIZE_EXECUTIVE", "PAPERSIZE_A3", "PAPERSIZE_A4", "PAPERSIZE_A4_SMALL",
             "PAPERSIZE_A5", "ORIENTATION_PORTRAIT", "ORIENTATION_LANDSCAPE"}


def owned(module, name):
    """A name a script would use from this module: a class or function defined in it (or, for
    package modules, in a submodule), or a constant; not serialisation helpers or imports."""
    obj = getattr(module, name)
    if inspect.ismodule(obj) or name in {"DEBUG", "DEFUSEDXML", "LXML", "NUMPY", "NUMERIC_TYPES"}:
        return False
    where = getattr(obj, "__module__", None)
    if inspect.isclass(obj) or inspect.isfunction(obj):
        if where is None or where.startswith(("openpyxl.descriptors", "openpyxl.xml", "openpyxl.compat")) or not where.startswith("openpyxl"):
            return False
        base = module.__name__
        return where == base or (hasattr(module, "__path__") and where.startswith(base + "."))
    return isinstance(obj, (str, int, float, tuple, frozenset, dict, list)) and name.isupper()


def public(obj):
    return {n for n in dir(obj) if not n.startswith("_")} - MACHINERY


def main():
    markdown = "--markdown" in sys.argv
    total = covered = 0
    rows = []
    for path in MODULES:
        try:
            theirs = importlib.import_module("openpyxl" + ("." + path if path else ""))
        except ImportError:
            continue
        names = {n for n in public(theirs) if owned(theirs, n)}
        try:
            ours = importlib.import_module("xlcells.openpyxl" + ("." + path if path else ""))
            have = public(ours)
        except ImportError:
            have = set()
        missing = sorted(names - have)
        total += len(names)
        covered += len(names) - len(missing)
        rows.append(("module " + (path or "openpyxl"), len(names), missing))
    for label, (module, name) in CLASSES.items():
        theirs = getattr(importlib.import_module(module), name)
        names = public(theirs)
        try:
            import xlcells.openpyxl as x
            ours = getattr(x, label, None)
            if ours is None:
                ours = {"Worksheet": getattr(x, "Worksheet", None), "ColumnDimension": getattr(x, "_ColumnDimension", None), "RowDimension": getattr(x, "_RowDimension", None)}.get(label)
            have = public(ours) if ours is not None else set()
        except ImportError:
            have = set()
        # Members set in __init__ (instance attributes) count as present when an instance has them.
        if ours is not None:
            try:
                inst = {"Workbook": lambda: ours(), "Worksheet": lambda: x.Workbook().active, "Cell": lambda: x.Workbook().active["A1"]}.get(label, lambda: ours())()
                have |= public(inst)
            except Exception:
                pass
        missing = sorted(names - have)
        total += len(names)
        covered += len(names) - len(missing)
        rows.append(("class " + label, len(names), missing))
    if markdown:
        print("| Bereich | Namen | fehlen |\n| --- | --- | --- |")
        for label, n, missing in rows:
            print("| {0} | {1} | {2} |".format(label, n, ", ".join(missing) if missing else "–"))
    else:
        for label, n, missing in rows:
            if missing:
                print("{0}: {1}/{2} missing: {3}".format(label, len(missing), n, " ".join(missing)))
    print("covered: {0} of {1} public names ({2:.1f} %)".format(covered, total, 100.0 * covered / max(total, 1)))


if __name__ == "__main__":
    main()
