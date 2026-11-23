"""A workbook as a store (not in openpyxl): a directory with the data of its regions as
Parquet files, a model document, and the workbook as the form the data returns to.

    store = wb.to_store("orders_store")          # or xo.open_store("orders_store")
    print(store)                                 # the regions, their columns and logic
    orders = store.read("Orders")                # a Polars DataFrame (data/Orders.parquet)
    store.write("Orders", orders.filter(pl.col("qty") > 0))
    store.set_logic("Orders", "amount", "[@qty]*[@price]*(1+Tax_rate)")
    report = store.run()                         # recalculated: Parquet, workbook, model
    wb = store.workbook()                        # the workbook, layout kept

A table is a Parquet file with its columns. A group of single cells (the inputs and
results of a model) is one with a row per cell: ``name``, ``cell``, ``dtype``, ``number``,
``text``.

    store.read("Model")                          # the rows
    store.values("Model")["Tax rate"]            # the cells by name: 0.19
    store.set_values("Model", {"Tax rate": 0.07})
    store.write("Model", pl.DataFrame({"name": ["Tax rate"], "number": [0.07]}))   # the same

Tables built alike in several sheets (a sheet per month, say) are also a *stack*: one
table with the column ``sheet`` in front. A stack is read and written by its name.

    store.stacks                                 # {"Months": {"sheets": [...], ...}}
    months = store.read("Months")                # all sheets, one below the other
    store.write("Months", months.filter(...))    # every sheet's rows go to its table
    store.add_sheet("Jan", "Apr")                # one more sheet, a copy of Jan

Rows are changed in place, and a change can be taken back; the workbook stays built
as it was when a table gets more or fewer rows (what lies below it moves with them).

    store.patch("Orders", set=[{"row": 0, "column": "qty", "value": 5}], delete=[3])
    store.patch("Orders", insert=[{"item": "new", "qty": 1}])
    store.undo()                                 # the change before is back
    store.rename("Sheet1_3", "Positions")        # a name that says what it is
    store.set_cells("Sales", {"D4": "=B4*100"})  # typed into the workbook, and run
    store.tables["Orders"]["columns"]            # logic, and what needs the workbook

A plain directory of Parquet files with their notes (no workbook) opens as a store too:
``read``, ``write``, ``patch``, ``recalculate`` and ``tables`` work on it.

The Parquet files are the database; ``run`` makes the workbook and the calculated
columns follow them. Everything is worked out in Rust.

The logic travels with the data: every Parquet file carries a note in its file metadata
(key ``xlcells``, JSON) that says which region it is and, for every calculated column and
cell, its formula and the formula engine's translation of it as a plan over the data.

    report = store.recalculate()                 # the calculated columns again, from the
                                                 # files alone: no workbook is involved
    report["kept"]                               # what has no plan and kept its values
    store.notes["Orders"]                        # the note of a region

Files outside a store work the same, as long as they carry their notes:

    xo.recalculate(["Orders.parquet", "Model.parquet"])
    note = xo.read_note("Orders.parquet")        # a dict, or None
    df.write_parquet(path, metadata={xo.NOTE_KEY: json.dumps(note)})   # keep it on writing
"""
import json
import os

from .._needs_engine import engine
from . import _frames


class Store:
    def __init__(self, path):
        self.path = str(path)
        plain = os.path.isdir(self.path) and any(n.endswith(".parquet") for n in os.listdir(self.path))
        if not os.path.exists(os.path.join(self.path, "model.json")) and not plain:
            raise FileNotFoundError("{0} is not a store (no model.json) and holds no Parquet files; make a store with Workbook.to_store(path)".format(self.path))

    @property
    def model(self):
        """The model document (``model.json``) as a dict."""
        with open(os.path.join(self.path, "model.json"), encoding="utf-8") as f:
            return json.load(f)

    def save_model(self, model):
        """Write a changed model document; ``run`` carries its logic into the workbook."""
        with open(os.path.join(self.path, "model.json"), "w", encoding="utf-8") as f:
            json.dump(model, f, indent=2, ensure_ascii=False)

    @property
    def regions(self):
        """The regions by name, each a dict of the model (kind, sheet, ref, columns or cells)."""
        return {r["name"]: r for r in self.model["regions"]}

    @property
    def stacks(self):
        """The stacks by name: tables of several sheets that are built alike. Each is a
        dict with ``columns``, ``members`` (sheet and region of every table) and
        ``differs`` (columns whose logic is not the same in every sheet). ``read``,
        ``write`` and ``set_logic`` take a stack's name as they take a region's."""
        return {s["name"]: s for s in self.model.get("stacks", [])}

    def add_sheet(self, like, name):
        """Add a sheet to the store's workbook as a copy of the sheet `like` (layout,
        formulas, and its data, to be replaced): one more sheet of a stack. Returns the
        names of the new sheet's regions. Excel tables, pictures and charts of the
        sheet are not copied."""
        return engine("A store").store_add_sheet(self.path, like, name)

    def read(self, region):
        """A region's or a stack's data as a Polars DataFrame. A table or block has its
        columns; a stack those of its tables with ``sheet`` in front. A
        group of single cells has a row per cell: ``name``, ``cell`` (the address),
        ``dtype``, ``number`` (the value of a number; of a boolean 1 or 0) and ``text``
        (the value of a text; a date or time as text). ``values`` gives single cells as
        a dict."""
        return _frames.as_polars(engine("A store").store_read(self.path, region))

    def values(self, region):
        """The single cells of a region by name: a dict of numbers, texts, booleans,
        dates and times (``None`` for a cell without a value)."""
        row = _frames.as_polars(engine("A store").store_read_wide(self.path, region))
        if row.height != 1:
            raise ValueError("{0!r} is a table of {1} rows, not single cells: use read".format(region, row.height))
        return {name: row[name][0] for name in row.columns}

    def write(self, region, frame):
        """Replace a region's data. A stack: the rows of every sheet named in ``sheet``
        go to that sheet's table (a sheet without rows in the frame stays as it is). A
        table: all its rows; calculated columns need not be
        given, ``run`` works them out. Single cells: the rows given, by ``name``, with
        their ``number`` or ``text`` (the other cells keep their value); one row with a
        column per cell is taken as well."""
        engine("A store").store_write(self.path, region, frame)
        return self

    def patch(self, region, set=None, delete=None, insert=None, at=None):
        """Change some rows of a region without writing it whole: `set` is a list of
        ``{"row": 0, "column": "qty", "value": 5}`` (rows counted from 0; single cells:
        ``{"name": "Tax rate", "value": 0.07}``), `delete` a list of rows, `insert` a list
        of rows as dicts (before the row `at`, else at the end). The state before is kept
        (``undo``). ``recalculate`` or ``run`` works the calculated columns out again."""
        patch = {"set": set or [], "delete": delete or []}
        if insert:
            patch["insert"] = {"rows": insert, **({} if at is None else {"at": at})}
        e = engine("A store")
        e.store_remember(self.path, "rows of {0}".format(region))
        try:
            e.store_patch(self.path, region, json.dumps(patch, default=str))
        except Exception:
            e.store_undo(self.path)
            raise
        return self

    def rename(self, region, name):
        """Give a region another name (found regions are often named after their sheet with
        a number). Its data and the logic that names it follow; the store is run, the
        workbook itself is not changed. The state before is kept (``undo``)."""
        e = engine("A store")
        e.store_remember(self.path, "name of {0}".format(region))
        try:
            e.store_rename_region(self.path, region, name)
        except Exception:
            e.store_undo(self.path)
            raise
        return self

    def undo(self):
        """Take back the last change kept (``patch``, ``set_cells``): data, model and
        workbook are as before it. Returns what was undone, or ``None``."""
        return engine("A store").store_undo(self.path)

    @property
    def tables(self):
        """The tables as the notes of the Parquet files describe them (also of a plain
        directory of such files): for each its columns with ``dtype`` and, if calculated,
        ``logic``, what it ``reads``, and ``state``: ``calculated`` (worked out from the
        data alone) or ``kept`` (only the workbook can; ``why`` says why)."""
        return {t["name"]: t for t in json.loads(engine("A store").store_tables(self.path))}

    def set_cells(self, sheet, cells):
        """Type into cells of the store's workbook, ``{"B7": 5, "D4": "=B4*2"}``, and run
        the store: data, model and workbook agree afterwards. Returns what ``run`` does."""
        e = engine("A store")
        e.store_remember(self.path, "cells of {0}".format(sheet))
        try:
            result = e.store_set_cells(self.path, json.dumps([[sheet, at, value] for at, value in cells.items()], default=str))
        except Exception:
            e.store_undo(self.path)
            raise
        cells_, computed, not_computed, regions, changed = result
        return {"formula_cells": cells_, "computed": computed, "not_computed": dict(not_computed), "regions": dict(regions), "logic_changed": [tuple(c) for c in changed]}

    def set_values(self, region, values=None, **more):
        """Set single cells of a region by name:
        ``store.set_values("Model", {"Tax rate": 0.07})``. The other cells keep their
        value."""
        import polars as pl
        values = dict(values or {}, **more)
        return self.write(region, pl.DataFrame({name: [value] for name, value in values.items()}, strict=False))

    def set_logic(self, region, name, logic):
        """Give a column or cell its logic: a formula in the structured form
        (``[@qty]*[@price]``, ``SUM(Orders[amount])``, names of single cells); ``None``
        makes it plain data again. Takes effect with ``run``."""
        if region in self.stacks:
            for member in self.stacks[region]["members"]:
                self.set_logic(member["region"], name, logic)
            return self
        model = self.model
        regions = {r["name"]: r for r in model["regions"]}
        if region not in regions:
            raise KeyError("no region named {0!r}; the regions are {1}".format(region, list(regions)))
        items = regions[region].get("columns") or regions[region].get("cells") or []
        for item in items:
            if item["name"] == name:
                if logic is None:
                    item.pop("logic", None)
                else:
                    item["logic"] = logic.lstrip("=")
                self.save_model(model)
                return self
        raise KeyError("{0!r} has no column or cell {1!r}; it has {2}".format(region, name, [i["name"] for i in items]))

    def run(self):
        """Bring the store up to date: the data of the Parquet files and the logic of the
        model go into the workbook, every formula is calculated, and Parquet files,
        workbook and model are written anew. Returns a dict: formula_cells, computed,
        not_computed (reason -> cells), regions (name -> rows), logic_changed."""
        cells, computed, not_computed, regions, changed = engine("A store").store_run(self.path)
        return {"formula_cells": cells, "computed": computed, "not_computed": dict(not_computed), "regions": dict(regions), "logic_changed": [tuple(c) for c in changed]}

    def recalculate(self):
        """Calculate the calculated columns and cells again from the notes of the Parquet
        files alone, in the order of what reads what, with Polars: the workbook is not
        involved (``run`` brings it in line afterwards). Returns a dict:

        * ``done``: (region, name) calculated again;
        * ``kept``: (region, name, formula, why) left with the values they had, because
          the engine has no plan for them or the plan does not hold for the data any more;
        * ``from_kept``: (region, name, what) calculated again, but from something kept.
        """
        return _report(engine("A store").store_recalculate(self.path))

    def to_port(self, path, accuracy="auto", replace=False):
        """The store as a Python project on Polars (a port of the workbook) in the folder
        `path`: its inputs, what the workbook calculates from them, the program
        (port/calc.py), run.py, check.py and tests (see the project's README.md).
        `accuracy`: ``auto`` (readable Polars where it gives the workbook's values, the
        engine's translation where it does not: the project's check.py is run to find
        out), ``plain`` (readable Polars wherever there is a translation, unchecked) or
        ``exact`` (the engine's translation everywhere). A folder that is not empty is
        only replaced with `replace`. Returns how every calculated column and cell is
        ported: a dict with ``items`` (region, name, formula, logic, how, reason, line)
        and ``summary``."""
        if accuracy not in ("auto", "plain", "exact"):
            raise ValueError("accuracy is 'auto', 'plain' or 'exact', not {0!r}".format(accuracy))
        items, summary = engine("A port").store_port(self.path, os.fspath(path), accuracy == "exact", replace, accuracy == "auto")
        keys = ("region", "name", "formula", "logic", "how", "reason", "line")
        return {"summary": summary, "items": [dict(zip(keys, i)) for i in items]}

    @property
    def notes(self):
        """The notes of the regions' Parquet files, by region."""
        out = {}
        for r in self.model["regions"]:
            note = read_note(os.path.join(self.path, r["data"]))
            if note is None:
                # Written anew by a tool that dropped it: the store has it once more.
                with open(os.path.join(self.path, "notes.json"), encoding="utf-8") as f:
                    note = json.load(f).get(r["data"])
            out[r["name"]] = note
        return out

    def workbook(self):
        """The store's workbook through the openpyxl facade."""
        from . import load_workbook
        return load_workbook(os.path.join(self.path, "workbook.xlsx"))

    def __repr__(self):
        lines = ["<Store {0}>".format(self.path)]
        for r in self.model["regions"]:
            lines.append("  {0} ({1} {2}!{3})".format(r["name"], r["kind"], r["sheet"], r["ref"]))
            for item in r.get("columns") or r.get("cells") or []:
                lines.append("    {0}: {1}{2}".format(item["name"], item["dtype"], "  =" + item["logic"] if "logic" in item else ""))
        for s in self.model.get("stacks", []):
            lines.append("  {0} (stack of {1})".format(s["name"], ", ".join(m["sheet"] for m in s["members"])))
        return "\n".join(lines)


def to_store(workbook, path):
    engine("A store").store_create(str(path), workbook.to_bytes())
    return Store(path)


def _report(result):
    done, kept, from_kept = result
    return {"done": [tuple(x) for x in done], "kept": [tuple(x) for x in kept], "from_kept": [tuple(x) for x in from_kept]}


def read_note(path):
    """The note a Parquet file carries (see the module): a dict, or ``None``."""
    text = engine("A note").read_note(str(path))
    return None if text is None else json.loads(text)


def recalculate(paths):
    """Calculate Parquet files again from the notes they carry (see ``Store.recalculate``);
    `paths` are the files of all regions the calculations read. The files are written
    anew, with their notes."""
    return _report(engine("A recalculation").recalculate_files([str(p) for p in paths]))
