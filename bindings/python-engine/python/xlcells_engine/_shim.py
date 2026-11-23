"""The formula engine's expressions on the Polars of this Python environment: the code
lives in `xlcells._polars_shim`, which the WebAssembly engine uses too."""
from xlcells._polars_shim import build, read_note, read_parquet, run, version, write_parquet  # noqa: F401
