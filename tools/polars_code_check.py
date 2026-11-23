#!/usr/bin/env python3
"""Run the Polars program of real workbooks and count what it gets right.

    tools/polars_code_check.py DIR [STEP] [MAX_CELLS] [plain|auto]

For every STEP-th workbook under DIR: `wb.to_polars_code(accuracy=...)` is executed;
its own check compares every calculated column and cell with the value the workbook
stores (Excel's). Counted: programs that run, items written as Polars (`polars`) or as
the engine's translation (`plan`), and of those the ones whose every value is the
workbook's; and why the rest is left to the engine. Needs xlcells and polars.
"""
import collections
import os
import sys
import time
import traceback

import xlcells.openpyxl as xo


def main():
    root = sys.argv[1]
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 100
    max_cells = int(sys.argv[3]) if len(sys.argv) > 3 else 20000
    accuracy = sys.argv[4] if len(sys.argv) > 4 else "plain"
    paths = sorted(os.path.join(d, f) for d, _s, fs in os.walk(root) for f in fs if f.lower().endswith((".xls", ".xlsx", ".xlsm")))
    n = collections.Counter()
    errors = collections.Counter()
    left = collections.Counter()
    differ = []
    start = time.time()
    for path in paths[::step]:
        try:
            wb = xo.load_workbook(path)
        except Exception:
            n["unreadable"] += 1
            continue
        if sum(len(ws._cells) for ws in wb.worksheets if hasattr(ws, "_cells")) > max_cells:
            n["skipped (large)"] += 1
            continue
        n["workbooks"] += 1
        try:
            code = wb.to_polars_code(path, accuracy=accuracy)
        except Exception as e:
            n["no program"] += 1
            errors["to_polars_code: " + type(e).__name__ + ": " + str(e)[:80]] += 1
            continue
        polars_items = [(r, name) for r, name, _f, how, _w in code.items if how == "polars"]
        plan_items = [(r, name) for r, name, _f, how, _w in code.items if how == "plan"]
        n["items"] += len(code.items)
        n["items as polars"] += len(polars_items)
        n["items as the engine's plan"] += len(plan_items)
        n["items by the engine's values"] += len(code.items) - len(polars_items) - len(plan_items)
        for _r, _n, _f, how, why in code.items:
            if how == "engine":
                left[(why or "").split("no translation by the engine to write out: ")[-1][:90]] += 1
        if not code.items:
            n["workbooks without formulas"] += 1
            continue
        lines = ["" if line.startswith("wb = xo.load_workbook(") else line for line in code.splitlines()]
        scope = {"wb": wb, "print": lambda *a, **k: None}
        try:
            exec(compile("\n".join(lines), "<program>", "exec"), scope)
        except Exception as e:
            n["programs that fail"] += 1
            n["items as polars in failed programs"] += len(polars_items)
            tb = traceback.extract_tb(e.__traceback__)
            line = lines[tb[-1].lineno - 1] if tb and tb[-1].filename == "<program>" else ""
            where = next((lines[f.lineno - 1] for f in reversed(tb) if f.filename == "<program>"), line)
            errors[type(e).__name__ + ": " + str(e).splitlines()[0][:70] + "  |  " + where.strip()[:90]] += 1
            continue
        n["programs that run"] += 1
        problems = scope["problems"]
        wrong = set(zip(problems["region"].to_list(), problems["column"].to_list()))
        n["items as polars, every value right"] += sum(1 for item in polars_items if item not in wrong)
        n["items as polars, a value wrong"] += sum(1 for item in polars_items if item in wrong)
        n["items as plan, every value right"] += sum(1 for item in plan_items if item not in wrong)
        n["items as plan, a value wrong"] += sum(1 for item in plan_items if item in wrong)
        # The plan is the engine's translation: it must give what the engine itself works out.
        if plan_items:
            try:
                engine_values = wb.evaluate({})
                results = scope["results"]
                for region, name in plan_items:
                    # Cells hold serial numbers in the program, the engine's frames dates.
                    norm = scope.get("_value", lambda x: x)
                    got = [norm(x) if x is not None else None for x in results[region][name].to_list()]
                    want = [norm(x) if x is not None else None for x in engine_values[region][name].to_list()]
                    same = len(got) == len(want) and all(
                        g == w or (isinstance(g, (int, float)) and isinstance(w, (int, float)) and abs(g - w) <= 1e-9 * max(abs(g), abs(w), 1.0))
                        or (g in (None, 0, "") and w in (None, 0, ""))
                        for g, w in zip(got, want)
                    )
                    n["plan items equal to the engine's values"] += same
                    if not same:
                        n["plan items different from the engine's values"] += 1
                        if len(differ) < 12:
                            differ.append("%s %s[%s] plan %r engine %r" % (os.path.basename(path)[-28:], region, name, [g for g, w in zip(got, want) if g != w][:2], [w for g, w in zip(got, want) if g != w][:2]))
            except Exception as e:
                n["workbooks whose engine values could not be compared"] += 1
        if not wrong:
            n["programs with every value right"] += 1
    print("%s, every %d. workbook, accuracy %s, %.0f s" % (root, step, accuracy, time.time() - start))
    for key, value in n.items():
        print("  %-42s %d" % (key, value))
    for line in differ:
        print("  plan differs from the engine:", line)
    print("why the engine's values are asked for (no plan could be written):")
    for why, count in left.most_common(25):
        print("  %5d  %s" % (count, why))
    print("why programs fail:")
    for why, count in errors.most_common(25):
        print("  %4d  %s" % (count, why))


if __name__ == "__main__":
    main()
