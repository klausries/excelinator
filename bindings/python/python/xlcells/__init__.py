"""Read and write Excel workbooks (xlsx) as cell tables.

``read(data)`` returns the cells as a dict of equally long columns, the layout of a
dataframe. ``Writer`` builds a workbook cell by cell. On top of these:

- ``xlcells.openpyxl``: the interface of openpyxl
- ``xlcells.polars``: ``read_cells``, ``read_excel``, ``write_excel`` for Polars
- ``xlcells.pyexcelerator``: the interface of pyExcelerator

The work is done in Rust: built for this machine (``xlcells._xlcells``, where it is
installed) or as the WebAssembly module inside the package (``xlcells._wasm``), which
runs anywhere with ``wasmtime``. ``XLCELLS_ENGINE=wasm`` asks for the module.
"""
import os as _os
import sys as _sys


def _cells_module():
    """The module that reads and writes: the built one, else the WebAssembly one."""
    if _os.environ.get("XLCELLS_ENGINE", "").strip().lower() != "wasm":
        try:
            from . import _xlcells

            return _xlcells
        except ImportError:
            pass
    from ._wasm import slim

    _sys.modules[__name__ + "._xlcells"] = slim
    return slim


_xlcells = _cells_module()
Writer, date_serial, read = _xlcells.Writer, _xlcells.date_serial, _xlcells.read

BLANK, NUMBER, TEXT, BOOL, ERROR = range(5)

__all__ = ["Writer", "read", "date_serial", "BLANK", "NUMBER", "TEXT", "BOOL", "ERROR"]
