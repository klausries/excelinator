# The files of a port

| File | What | Change it? |
|---|---|---|
| `data/<region>.parquet` | the inputs of each region (tables: their columns without the calculated ones; single cells: one row, a column per cell) | it is the data of the workbook; new data goes elsewhere |
| `expected/<region>.parquet` | the same with every calculated column and cell as the workbook calculates it | never |
| `logic.json` | `regions` (name, kind, sheet, ref, file, inputs with dtype, calculated) and `items` (region, name, formula in A1 form, logic in structured form, how, reason, line, volatile) | only `how` of an item you ported |
| `cells.json` | every cell of the sheets: [row, column, value], 0-based, formulas as `=…` | never |
| `workbook.xlsx` | the workbook: the way back, and xlcells' engine for what is not ported | never |
| `port/calc.py` | the program; `calculate(folder)` returns {region: frame} | yes: this is the port |
| `port/xl.py` | the files above served under the names the program uses (`load_workbook`, `regions`, `evaluate`, `verify`), `read_inputs`, `same`, `differences` | rarely |
| `check.py` | the check; `--exact --data DIR` against xlcells' engine on other inputs | no |
| `run.py` | runs the port: `--data`, `--out`, `--excel FILE` | as the application needs |
| `tests/test_port.py` | the check as a test, inputs refused when a column is missing | add your own |

Inside `calc.py`: the inputs are read first (`sales = regions["Sales"].to_polars()…`,
single cells as Python values: `tax_rate = _value(model["Tax rate"][0])`), then every
calculated column and cell in the order of what reads what, each under a comment with
its logic. A table's column is `frame = frame.with_columns(<expr>.alias("name"))`, a
single cell `name = <python value>`. `exact = wb.evaluate(...)` is only there while some
item is `rest`; when none is left, delete it.

Helpers written into `calc.py` as needed: `_value(x)` (a cell as Excel calculates with
it: empty is 0, a date is its serial number), `_round` (halves away from zero), and for
`plan` items `_num`, `_text`, `_cell`, `_range`, `_summary`, `_result`, `_scalar`.
