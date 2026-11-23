"""The engine of xlcells: formula calculation, dataframes, regions, stores, looks.

Nothing here is meant to be called directly: `xlcells` uses it for `wb.calculate()`,
`ws.to_polars()`, `wb.regions`, `wb.to_store()`, `cell.look` and
`xlcells.polars.read_excel`. Without this package those say so; everything else of
`xlcells` (reading, writing, the openpyxl and pyExcelerator interfaces) works
without it.

The engine itself is Rust. What it works out for formulas it runs on the Polars
installed in this environment (`_shim` is all there is in between), so there is one
Polars: the one the rest of the program uses. `xlcells-engine-bundled` is the same
engine with a Polars of its own built in.
"""
from ._engine import Book, Frame, Looker, calculate, engine_info, looks, recalculation, NOTE_KEY, read_note, recalculate_files, store_create, store_read, store_read_wide, store_add_sheet, store_patch, store_rename_region, store_tables, store_remember, store_undo, store_set_cells, store_recalculate, store_run, store_write, store_port

#: The versions of Polars the engine's results were checked against.
TESTED_POLARS = (2, 1)


def _check_polars():
    import warnings

    import polars

    try:
        major, minor = (int(x) for x in polars.__version__.split(".")[:2])
    except ValueError:
        return
    if major != TESTED_POLARS[0]:
        warnings.warn(
            "xlcells-engine was checked against Polars {0}.x; this is Polars {1}. Formula results may differ; "
            "xlcells-engine-bundled calculates with the Polars it was built with.".format(TESTED_POLARS[0], polars.__version__),
            stacklevel=3,
        )


_check_polars()

__all__ = ["Book", "Frame", "Looker", "calculate", "engine_info", "looks", "recalculation", "NOTE_KEY", "read_note", "recalculate_files", "store_create", "store_read", "store_read_wide", "store_add_sheet", "store_patch", "store_rename_region", "store_tables", "store_remember", "store_undo", "store_set_cells", "store_recalculate", "store_run", "store_write", "store_port"]
