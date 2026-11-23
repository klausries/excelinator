# Excel's behaviour in Polars

Where Excel and Polars differ, the port must do what Excel does. `check.py` finds the
difference; this list says what it usually is.

| Excel | In the port |
|---|---|
| An empty cell in arithmetic is 0; in text it is "" | `pl.col(x).fill_null(0)` for numbers; single cells through `_value(x)` |
| ROUND rounds halves away from zero, on the decimal it shows (2.675 → 2.68) | `expr.round_sig_figs(15).round(n, mode="half_away_from_zero")` |
| Numbers are compared to 15 significant digits | round both sides with `round_sig_figs(15)` before `==` |
| `=` on text ignores case; `EXACT` does not | `a.str.to_lowercase() == b.str.to_lowercase()` |
| Dates are serial numbers: days since 1899-12-30, counting a 29 February 1900 that never was (serial 1 is 1900-01-01, 61 is 1900-03-01) | to a date: `pl.date(1899, 12, 30) + pl.duration(days=serial + (serial < 61).cast(pl.Int64))`; from a date: `_value(d)` |
| TODAY() / NOW() | `_value(datetime.date.today())` / `_value(datetime.datetime.now())`: marked "of the moment", not compared |
| VLOOKUP(x, table, k, FALSE), INDEX/MATCH exact | a left join on the key (first match: `unique(subset=key, keep="first")` on the lookup table first) |
| VLOOKUP(x, table, k, TRUE) (sorted, approximate) | `join_asof(…, strategy="backward")` on the sorted key |
| SUMIF / COUNTIF / AVERAGEIF on a table | filter and aggregate, or `group_by` and join back; criteria like ">10" or "a*" as the comparison or pattern they mean (wildcards `*`, `?` as regex `.*`, `.`) |
| SUMPRODUCT | `(a * b).sum()` |
| A column with some rows of their own (exceptions) | `pl.when(pl.int_range(pl.len()) == row).then(value).otherwise(expr)`; rows counted from 0 below the header |
| #DIV/0!, #N/A and other errors | null in the port (the workbook's error values are null in `expected/` too) |
| IFERROR(x, y) | y where Excel's x is an error: `pl.when(b != 0).then(a / b).otherwise(y)` for a division, `.fill_null(y)` after a lookup join that found nothing |
| A whole column of a table (`SUM(Orders[amount])`) | the frame's column: `orders["amount"].sum()` |
| A single cell by name (`Tax_rate`) | the Python variable of that cell in `calc.py` |
| Text joined with `&` | `pl.concat_str([...])`; numbers as Excel shows them (whole numbers without `.0`) |
