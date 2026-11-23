"""Checks the port: works out every calculated column and cell from data/ with the
program (port/calc.py) and compares them with expected/, what the workbook calculates.
Writes status.json and says how far the port is. Polars alone is enough.

    python check.py              against expected/
    python check.py --exact      against the formula engine of xlcells on the data
                                 given (needs xlcells): for data other than data/
    python check.py --data DIR   other inputs (with --exact)

Exit code 0: every value that the program works out is right.
"""
import argparse
import json
import sys
import warnings
from pathlib import Path

import polars as pl

# The project's folder, also where Python does not add the script's own (the embeddable one).
sys.path.insert(0, str(Path(__file__).resolve().parent))

from port import xl  # noqa: E402
from port.calc import calculate  # noqa: E402


def check(data=xl.HOME / "data", exact=False):
    """The state of every calculated column and cell: ok, differs (with the first
    differences) or kept (not worked out: its values were taken from expected/)."""
    xl.KEPT.clear()
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        results = calculate(data)
    meta = xl.logic()
    if exact:
        reference = xl.engine_values(xl.read_inputs(data))
    else:
        reference = {r["name"]: xl.expected(r["name"]) for r in meta["regions"]}
    items = []
    lines = xl.item_lines()
    for item in meta["items"]:
        region, name = item["region"], item["name"]
        state = dict(item, line=lines[(region, name)])
        if (region, name) in xl.KEPT:
            state["state"] = "kept"
        elif item.get("volatile"):
            state["state"] = "of the moment"
        else:
            diff = xl.differences(region, results[region], reference[region], [name])
            state["state"] = "ok" if diff.height == 0 else "differs"
            if diff.height:
                state["differences"] = diff.to_dicts()
        items.append(state)
    return items


def summary(items):
    count = lambda **k: sum(all(i.get(a) == b for a, b in k.items()) for i in items)
    return {
        "calculated": len(items),
        "standalone": count(how="polars") + count(how="plan"),
        "polars": count(how="polars"),
        "plan": count(how="plan"),
        "rest": count(how="rest"),
        "ok": count(state="ok"),
        "differs": count(state="differs"),
        "kept": count(state="kept"),
        "moment": count(state="of the moment"),
    }


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--data", default=str(xl.HOME / "data"))
    p.add_argument("--exact", action="store_true")
    a = p.parse_args()
    items = check(a.data, a.exact)
    s = summary(items)
    (xl.HOME / "status.json").write_text(json.dumps({"summary": s, "items": items}, indent=1, ensure_ascii=False, default=str), encoding="utf-8")
    print("{calculated} calculated columns and cells: {standalone} worked out by Polars alone "
          "({polars} readable, {plan} the engine's translation), {rest} not ported yet.".format(**s))
    print("checked: {ok} right, {differs} different, {kept} not worked out (values of expected/), "
          "{moment} not compared (their value depends on the moment or on chance).".format(**s))
    for i in items:
        if i["state"] != "ok":
            where = "port/calc.py:{0}".format(i["line"])
            print("  {0:13} {1}[{2}]  ={3}  ({4}; {5})".format(i["state"], i["region"], i["name"], i.get("logic") or i["formula"], i["how"], where))
            for d in i.get("differences", [])[:3]:
                print("           row {row}: expected {expected}, got {actual}".format(**d))
    return 1 if s["differs"] else 0


if __name__ == "__main__":
    sys.exit(main())
