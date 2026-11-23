"""`xlcells_engine._engine` (the formula engine, regions, stores, looks) on the
WebAssembly module. Plans run on the Polars installed here; frames come as Polars
DataFrames."""
import os

from . import Held, call, outer, tuples

NOTE_KEY = "xlcells"


def _path(p):
    """A path as the module takes it: absolute (it knows no working directory of later)."""
    return os.path.abspath(os.fspath(p))


def engine_info():
    return tuple(call("engine_info"))


class Book(Held):
    """A workbook file (xlsx or xls) read once and held in the module."""

    kind = "book"

    @property
    def sheet_names(self):
        return self._call("sheet_names")

    def used_area(self, sheet):
        a = self._call("used_area", sheet)
        return None if a is None else tuple(a)

    def frame(self, sheet, area=None, has_header=True):
        return self._call("frame", sheet, None if area is None else list(area), has_header)

    def regions(self):
        return [{k: (outer(v) if k in ("columns", "cells") else v) for k, v in r.items()} for r in self._call("regions")]

    def region_frame(self, name):
        return self._call("region_frame", name)

    def write(self, name, frame):
        return self._call("write", name, frame)

    def verify(self, name, frame):
        return self._call("verify", name, frame)

    def calculate(self):
        cells, computed, not_computed = self._call("calculate")
        return cells, computed, outer(not_computed)

    def polars_program(self, file, exact=False, engine=()):
        source, items, check_line, exact_statement = self._call("polars_program", file, exact, [list(p) for p in engine])
        return source, outer(items), check_line, exact_statement

    def to_bytes(self):
        return self._call("to_bytes")


def _looks(raw):
    return [{k: (tuples(v) if isinstance(v, list) else v) for k, v in look.items()} for look in raw]


class Looker(Held):
    """The looks of the cells of one workbook file, read once."""

    kind = "looker"

    def looks(self, cells):
        return _looks(self._call([list(c) for c in cells]))


def looks(data, cells):
    return _looks(call("looks", bytes(data), [list(c) for c in cells]))


def calculate(data):
    d = call("calculate", bytes(data))
    d["values"] = outer(d["values"])
    d["not_computed"] = outer(d["not_computed"])
    return d


def recalculation(data):
    return call("recalculation", bytes(data))


def _report(r):
    cells, computed, not_computed, regions, changed = r
    return cells, computed, outer(not_computed), outer(regions), outer(changed)


def store_create(dir, data):
    return outer(call("store_create", _path(dir), bytes(data)))


def store_run(dir):
    return _report(call("store_run", _path(dir)))


def store_read(dir, name):
    return call("store_read", _path(dir), name)


def store_read_wide(dir, name):
    return call("store_read_wide", _path(dir), name)


def store_write(dir, name, frame):
    call("store_write", _path(dir), name, frame)


def store_add_sheet(dir, like, name):
    return call("store_add_sheet", _path(dir), like, name)


def store_patch(dir, name, patch):
    return call("store_patch", _path(dir), name, patch)


def store_tables(dir):
    return call("store_tables", _path(dir))


def store_rename_region(dir, region, name):
    return _report(call("store_rename_region", _path(dir), region, name))


def store_remember(dir, what):
    call("store_remember", _path(dir), what)


def store_undo(dir):
    return call("store_undo", _path(dir))


def store_set_cells(dir, cells):
    return _report(call("store_set_cells", _path(dir), cells))


def _recalculated(r):
    return tuple(outer(x) for x in r)


def store_recalculate(dir):
    return _recalculated(call("store_recalculate", _path(dir)))


def recalculate_files(paths):
    return _recalculated(call("recalculate_files", [_path(p) for p in paths]))


def read_note(path):
    return call("read_note", _path(path))


def store_port(dir, dest, exact=False, replace=False, check=False):
    items, summary = call("store_port", _path(dir), _path(dest), exact, replace, check)
    return outer(items), summary
