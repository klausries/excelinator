"""Runs the port: reads the inputs, works out every calculated column and cell, and
writes the results.

    python run.py [--data DIR] [--out DIR] [--excel FILE.xlsx]

--data   the inputs, as data/ (a Parquet file per region; default data/)
--out    where the results go, a Parquet file per region (default out/)
--excel  also the workbook with the results, in its old layout (needs xlcells): the
         way back to Excel; its formulas are calculated anew there.
"""
import argparse
import sys
from pathlib import Path

# The project's folder, also where Python does not add the script's own (the embeddable one).
sys.path.insert(0, str(Path(__file__).resolve().parent))

from port import xl  # noqa: E402
from port.calc import calculate  # noqa: E402


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--data", default=str(xl.HOME / "data"))
    p.add_argument("--out", default=str(xl.HOME / "out"))
    p.add_argument("--excel")
    a = p.parse_args()
    results = calculate(a.data)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    files = {r["name"]: r["file"] for r in xl.logic()["regions"]}
    for region, frame in results.items():
        frame.write_parquet(out / files[region])
    print("written: {0} ({1} regions)".format(out, len(results)))
    if xl.KEPT:
        print("not worked out (values of expected/): " + ", ".join("{0}[{1}]".format(*k) for k in sorted(xl.KEPT)))
    if a.excel:
        import xlcells.openpyxl as real

        wb = real.load_workbook(xl.HOME / xl.logic()["workbook"])
        for region, frame in results.items():
            wb.regions[region].write(frame)
        wb.calculate()
        wb.save(a.excel)
        print("written: " + a.excel)
    return 0


if __name__ == "__main__":
    sys.exit(main())
