#!/usr/bin/env python3
"""Add up the outputs of lift_corpus runs over shards (LIFT_SHARD=k/n).

usage: merge_lift.py shard0.txt shard1.txt ... > total.txt
"""
import re, sys
from collections import Counter

nums = Counter()
reasons = Counter()
mismatches = []
for path in sys.argv[1:]:
    section = None
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if m := re.match(r"workbooks read: (\d+), with column formulas: (\d+), of those lifted completely: (\d+)", line):
            nums["read"] += int(m[1]); nums["with"] += int(m[2]); nums["fully"] += int(m[3])
        elif m := re.match(r"formula cells: (\d+); in column runs: (\d+) .* in (\d+) runs", line):
            nums["formulas"] += int(m[1]); nums["run_cells"] += int(m[2]); nums["runs"] += int(m[3])
        elif m := re.match(r"lifted and verified: (\d+) cells .* in (\d+) runs", line):
            nums["lifted"] += int(m[1]); nums["lifted_runs"] += int(m[2])
        elif m := re.match(r"translated but values differ: (\d+) cells in (\d+) runs", line):
            nums["mismatch"] += int(m[1]); nums["mismatch_runs"] += int(m[2])
        elif m := re.match(r"\s+(\d+)  (.*)$", line):
            reasons[m[2]] += int(m[1])
        elif line.startswith("mismatch: "):
            mismatches.append(line)
pct = lambda a, b: 100.0 * a / b if b else 0.0
print(f"workbooks read: {nums['read']}, with column formulas: {nums['with']}, of those lifted completely: {nums['fully']}")
print(f"formula cells: {nums['formulas']}; in column runs: {nums['run_cells']} ({pct(nums['run_cells'], nums['formulas']):.1f} %) in {nums['runs']} runs")
print(f"lifted and verified: {nums['lifted']} cells ({pct(nums['lifted'], nums['run_cells']):.1f} % of run cells, {pct(nums['lifted'], nums['formulas']):.1f} % of all formula cells) in {nums['lifted_runs']} runs")
print(f"translated but values differ: {nums['mismatch']} cells in {nums['mismatch_runs']} runs")
print("not translated, by reason (cells):")
for r, n in reasons.most_common():
    print(f"  {n:>9}  {r}")
for m in mismatches:
    print(m)
