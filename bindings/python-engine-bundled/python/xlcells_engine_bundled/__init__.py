"""The engine of xlcells with a Polars of its own built in.

The same engine as `xlcells-engine`, which runs its formula expressions on the Polars
installed in the environment. This one runs them on the Polars it was built with: the
reference its results are checked against, and the choice where no Polars 2 is
installed. `xlcells` uses `xlcells-engine` when both are there; set the environment
variable `XLCELLS_ENGINE=bundled` to use this one.
"""
from ._engine import Book, Frame, Looker, calculate, engine_info, looks, recalculation, NOTE_KEY, read_note, recalculate_files, store_create, store_read, store_read_wide, store_add_sheet, store_patch, store_rename_region, store_tables, store_remember, store_undo, store_set_cells, store_recalculate, store_run, store_write

__all__ = ["Book", "Frame", "Looker", "calculate", "engine_info", "looks", "recalculation", "NOTE_KEY", "read_note", "recalculate_files", "store_create", "store_read", "store_read_wide", "store_add_sheet", "store_patch", "store_rename_region", "store_tables", "store_remember", "store_undo", "store_set_cells", "store_recalculate", "store_run", "store_write"]
