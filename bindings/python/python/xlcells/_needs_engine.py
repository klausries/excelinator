"""Where xlcells finds its engine, and what it says without it.

The engine is there in three builds: the WebAssembly module inside this package
(`wasm`: one for every processor, run by `wasmtime`, its formula expressions on the
Polars installed here), and, built for one kind of machine, `xlcells-engine` (on the
Polars installed) and `xlcells-engine-bundled` (on a Polars of its own). A built one is
used when it is installed, else the module; the environment variable `XLCELLS_ENGINE`
(`environment`, `bundled` or `wasm`) asks for one of them.
"""
import importlib
import os

_MODULES = {"environment": "xlcells_engine", "bundled": "xlcells_engine_bundled", "wasm": "xlcells._wasm.engine"}


def engine(what):
    """The engine module; `what` names what was asked for, for the message without it."""
    wanted = os.environ.get("XLCELLS_ENGINE", "").strip().lower()
    if wanted and wanted not in _MODULES:
        raise ImportError("XLCELLS_ENGINE is {0!r}; it can be 'environment' or 'bundled'".format(wanted))
    for kind in [wanted] if wanted else ["environment", "bundled", "wasm"]:
        try:
            return importlib.import_module(_MODULES[kind])
        except ImportError as e:
            # A missing dependency of an installed engine is said as it is.
            if e.name not in (_MODULES[kind], _MODULES[kind] + "._engine") and kind != "wasm":
                raise
            if kind == "wasm" and wanted:
                raise
    if wanted:
        raise ImportError("XLCELLS_ENGINE asks for the {0} engine, which is not installed (pip install {1})".format(wanted, _MODULES[wanted].replace("_", "-")))
    raise ImportError(
        "{0} needs the packages polars and wasmtime (uv pip install polars wasmtime): the engine of xlcells runs on them. "
        "Reading and writing workbooks works without them.".format(what)
    )
