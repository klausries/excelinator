"""The check of a port (see `xl_engine::port`), run as a person runs it: the project's
check.py with this Python, in its folder. Used while a port is written, to find the
columns and cells whose first form does not give the workbook's values."""
import json
import subprocess
import sys
from pathlib import Path


def differing(folder):
    """(region, name, how) of every column and cell whose values are not the workbook's.
    A program that does not run at all counts every item ported as Polars as wrong."""
    folder = Path(folder)
    status = folder / "status.json"
    if status.exists():
        status.unlink()
    done = subprocess.run([sys.executable, "check.py"], cwd=folder, capture_output=True, text=True, encoding="utf-8", timeout=1800)
    if not status.exists():
        items = json.loads((folder / "logic.json").read_text(encoding="utf-8"))["items"]
        if not any(i["how"] == "polars" for i in items):
            raise RuntimeError("the check of the port does not run: " + (done.stderr or done.stdout)[-1500:])
        return [[i["region"], i["name"], i["how"]] for i in items if i["how"] in ("polars", "plan")]
    items = json.loads(status.read_text(encoding="utf-8"))["items"]
    return [[i["region"], i["name"], i["how"]] for i in items if i["state"] == "differs"]
