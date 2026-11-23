"""The formula engine's expressions on the Polars of this Python environment.

The engine is written in Rust and translates formulas into plans of column expressions
(see `xl_engine::ir::wire` for their form). This module is all there is between them and
Polars: every node becomes the Polars operation of the same name. Rust calls `run` with
a plan and an input table (as an Arrow stream) and takes the result back as one.
"""
import json

import polars as pl

_DTYPES = {"Boolean": pl.Boolean, "Int64": pl.Int64, "UInt32": pl.UInt32, "Float64": pl.Float64, "String": pl.String}
_LITERALS = {"bool": pl.Boolean, "i32": pl.Int32, "i64": pl.Int64, "u32": pl.UInt32, "u64": pl.UInt64, "f64": pl.Float64, "str": pl.String}
_SERIES = {"f64": pl.Float64, "i64": pl.Int64, "str": pl.String, "bool": pl.Boolean}

_BINARY = {
    "+": lambda a, b: a + b,
    "-": lambda a, b: a - b,
    "*": lambda a, b: a * b,
    "/": lambda a, b: a / b,
    "%": lambda a, b: a % b,
    "==": lambda a, b: a.eq(b),
    "!=": lambda a, b: a.ne(b),
    "<": lambda a, b: a.lt(b),
    "<=": lambda a, b: a.le(b),
    ">": lambda a, b: a.gt(b),
    ">=": lambda a, b: a.ge(b),
    "and": lambda a, b: a.and_(b),
    "or": lambda a, b: a.or_(b),
}


def _replace_strict(e, has_default, dtype):
    kw = {"return_dtype": _DTYPES[dtype] if dtype else None}
    if has_default:
        kw["default"] = e[3]
    return e[0].replace_strict(e[1], e[2], **kw)


# name: (expressions, parameters) -> expression
_FUNCTIONS = {
    "not_": lambda e: e[0].not_(),
    "is_null": lambda e: e[0].is_null(),
    "is_not_null": lambda e: e[0].is_not_null(),
    "is_nan": lambda e: e[0].is_nan(),
    "is_finite": lambda e: e[0].is_finite(),
    "fill_null": lambda e: e[0].fill_null(e[1]),
    "abs": lambda e: e[0].abs(),
    "floor": lambda e: e[0].floor(),
    "ceil": lambda e: e[0].ceil(),
    "sqrt": lambda e: e[0].sqrt(),
    "exp": lambda e: e[0].exp(),
    "log": lambda e: e[0].log(e[1]),
    "pow": lambda e: e[0].pow(e[1]),
    "sum": lambda e: e[0].sum(),
    "min": lambda e: e[0].min(),
    "max": lambda e: e[0].max(),
    "product": lambda e: e[0].product(),
    "count": lambda e: e[0].count(),
    "first": lambda e: e[0].first(),
    "last": lambda e: e[0].last(),
    "reverse": lambda e: e[0].reverse(),
    "any": lambda e, n: e[0].any(ignore_nulls=n),
    "all": lambda e, n: e[0].all(ignore_nulls=n),
    "cum_sum": lambda e, r: e[0].cum_sum(reverse=r),
    "cum_prod": lambda e, r: e[0].cum_prod(reverse=r),
    "cum_min": lambda e, r: e[0].cum_min(reverse=r),
    "cum_max": lambda e, r: e[0].cum_max(reverse=r),
    "shift": lambda e: e[0].shift(e[1]),
    "replace_strict": _replace_strict,
    "search_sorted": lambda e, side, descending: e[0].search_sorted(e[1], side=side, descending=descending),
    "gather": lambda e, oob: e[0].gather(e[1], null_on_oob=oob),
    "get": lambda e, oob: e[0].get(e[1], null_on_oob=oob),
    "filter": lambda e: e[0].filter(e[1]),
    "slice": lambda e: e[0].slice(e[1], e[2]),
    "str.to_lowercase": lambda e: e[0].str.to_lowercase(),
    "str.to_uppercase": lambda e: e[0].str.to_uppercase(),
    "str.len_chars": lambda e: e[0].str.len_chars(),
    "str.head": lambda e: e[0].str.head(e[1]),
    "str.tail": lambda e: e[0].str.tail(e[1]),
    "str.slice": lambda e: e[0].str.slice(e[1], e[2]),
    "str.contains": lambda e, strict: e[0].str.contains(e[1], literal=False, strict=strict),
    "str.replace_all": lambda e, literal: e[0].str.replace_all(e[1], e[2], literal=literal),
    "str.strip_chars": lambda e: e[0].str.strip_chars(e[1]),
    "str.split": lambda e: e[0].str.split(e[1], literal=True),
    "str.find_literal": lambda e: e[0].str.find(e[1], literal=True),
    "str.extract": lambda e, group: e[0].str.extract(e[1], group_index=group),
    "list.first": lambda e: e[0].list.first(),
    "min_horizontal": lambda e: pl.min_horizontal(*e),
    "max_horizontal": lambda e: pl.max_horizontal(*e),
}


def build(nodes):
    """The expressions of a plan's nodes, each built once."""
    out = []
    for n in nodes:
        kind = n[0]
        if kind == "col":
            e = pl.col(n[1])
        elif kind == "lit":
            if n[1] == "null":
                e = pl.lit(None)
            else:
                # A number that is not finite is written as text ("inf", "-inf", "nan").
                e = pl.lit(float(n[2]) if n[1] == "f64" else n[2], dtype=_LITERALS[n[1]])
        elif kind == "series":
            values = n[3]
            if n[2] == "f64" and any(isinstance(v, str) for v in values):
                values = [float(v) if isinstance(v, str) else v for v in values]
            e = pl.lit(pl.Series(n[1], values, dtype=_SERIES[n[2]]))
        elif kind == "bin":
            e = _BINARY[n[1]](out[n[2]], out[n[3]])
        elif kind == "when":
            e = pl.when(out[n[1]]).then(out[n[2]]).otherwise(out[n[3]])
        elif kind == "cast":
            e = out[n[1]].cast(_DTYPES[n[2]], strict=False)
        elif kind == "fn":
            e = _FUNCTIONS[n[1]]([out[i] for i in n[2]], *n[3:])
        else:
            raise ValueError("unknown node {0!r}".format(kind))
        out.append(e)
    return out


_prepared = {}


def run(key, plan, table):
    """Run a plan on a table (anything with `__arrow_c_stream__`); `plan` (JSON) is given
    the first time a key is used, and kept for the runs that follow."""
    p = _prepared.get(key)
    if p is None:
        spec = json.loads(plan)
        exprs = build(spec["nodes"])
        p = (
            [[exprs[i].alias(n) for n, i in step] for step in spec["steps"]],
            [exprs[i].alias(n) for n, i in spec["outputs"]],
            [n for n, _ in spec["outputs"]],
            spec["broadcast"],
        )
        _prepared.clear()
        _prepared[key] = p
    steps, outputs, names, broadcast = p
    lf = pl.DataFrame(table).lazy()
    for step in steps:
        lf = lf.with_columns(step)
    lf = lf.with_columns(outputs).select(names) if broadcast else lf.select(outputs)
    return lf.collect()


def write_parquet(path, table, key=None, note=None):
    """Write a table as a Parquet file; `note` goes into the metadata of the file under `key`."""
    pl.DataFrame(table).write_parquet(path, metadata=None if note is None else {key: note})


def read_note(path, key):
    return pl.read_parquet_metadata(path).get(key)


def read_parquet(path):
    return pl.read_parquet(path)


def version():
    return pl.__version__
