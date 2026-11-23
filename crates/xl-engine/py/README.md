# @TITLE@

The workbook `@WORKBOOK@` as a Python program on Polars: its data in `data/`, its logic
in `port/calc.py`, checked against what the workbook calculates (`expected/`). Made by
xlcells; from here on it is yours to change.

## Run it

With uv (or pip and a Python of 3.11 or later with `polars`):

    uv run check.py                 # is every value right? writes status.json
    uv run run.py --out results     # the results as Parquet files
    uv run --with pytest pytest     # the tests
    uv run --extra excel run.py --excel back.xlsx   # back into the workbook (needs xlcells)

## How far it is

@SUMMARY@

| Region | Column or cell | Logic | Ported as | In `port/calc.py` |
|---|---|---|---|---|
@ITEMS@

* **polars**: written with Polars' own operations, readable.
* **plan**: the formula engine's translation, written out as Polars code: exact, holds
  for other data, but not meant to be read.
* **rest**: not ported yet. With xlcells installed (`--extra excel`) its engine works it
  out exactly; without, the values of `expected/` are used (right only for the data the
  workbook had), and `check.py` reports them as not worked out.

## Carrying it on

Replace a `rest` (and, where you like, a `plan`) with your own Polars code at its line
in `port/calc.py`, then run `check.py`: it must stay at 0 different values. Do not change
`expected/`: it is what the workbook calculates, the measure of the port. For inputs
other than `data/`, `check.py --exact --data DIR` compares with the formula engine of
xlcells on those inputs.

| File | What |
|---|---|
| `data/` | the inputs: a Parquet file per region, without what is calculated |
| `expected/` | the same with everything the workbook calculates |
| `logic.json` | the regions, their inputs, every calculated column and cell |
| `cells.json` | the cells of the sheets (formulas that read cells outside the regions) |
| `workbook.xlsx` | the workbook, for the way back and for xlcells' engine |
| `port/calc.py` | the program: `calculate(folder)` gives every region as a frame |
| `port/xl.py` | what the program needs besides Polars |
