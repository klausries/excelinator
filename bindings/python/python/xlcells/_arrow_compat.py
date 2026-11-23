"""Dataframes on their way into the engine.

Polars and pyarrow write a column that holds no value (type Null) with a buffer where
the Arrow reader of the engine allows none. Such a column is handed over as a Boolean
column of nulls: for the engine the same (a cell without a value).
"""


def prepare(frame):
    """`frame` (a Polars DataFrame, a pyarrow Table, or anything else with
    ``__arrow_c_stream__``) with columns of type Null as columns of nulls of another type."""
    kind = type(frame).__module__.split(".")[0]
    if kind == "polars":
        import polars as pl

        nulls = [name for name, dtype in frame.schema.items() if dtype == pl.Null]
        if nulls:
            return frame.with_columns(pl.col(name).cast(pl.Boolean) for name in nulls)
    elif kind == "pyarrow":
        import pyarrow as pa

        for i, field in enumerate(frame.schema):
            if pa.types.is_null(field.type):
                frame = frame.set_column(i, field.name, pa.nulls(frame.num_rows, pa.bool_()))
    return frame
