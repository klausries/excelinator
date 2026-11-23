"""A check of a release as it is installed: xlcells and its engine, and the MCP server.

    python tools/smoke_release.py [--mcp COMMAND ...]

`python` is the Python of the release (the folder's xlcells-python.cmd on Windows, or
an environment with the wheel installed); `--mcp` the command that starts its server
(the rest of the line), default `python -m xlcells.mcp`. Needs nothing but the release:
the workbook it works on is made here. Exits with 1 and says what failed.
"""
import json
import os
import subprocess
import sys
import tempfile

import xlcells.openpyxl as xo
from xlcells._needs_engine import engine


def check(what, ok, said=""):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok else ": " + str(said)), flush=True)
    if not ok:
        sys.exit(1)


def workbook(path):
    wb = xo.Workbook()
    ws = wb.active
    ws.title = "Sales"
    ws.append(["item", "qty", "price", "amount"])
    for k in range(4):
        ws.append(["item{0}".format(k), k + 1, 2, "=ROUND(B{0}*C{0}*(1+Model!$B$1),2)".format(k + 2)])
    m = wb.create_sheet("Model")
    m["A1"], m["B1"] = "Tax rate", 0.19
    m["A2"], m["B2"] = "Gross", "=SUM(Sales!D2:D5)"
    wb.save(path)


def mcp(command, messages):
    p = subprocess.run(command, input="".join(json.dumps(m) + "\n" for m in messages), capture_output=True, text=True, encoding="utf-8", timeout=300)
    replies = [json.loads(line) for line in p.stdout.splitlines() if line.strip()]
    if len(replies) != len(messages):
        check("the server answers every message", False, p.stderr[-2000:] or p.stdout[-2000:])
    return replies


def app(command, folder):
    """The app of the server (--app): its page and the program's call, a browser's way:
    one connection kept open while another asks."""
    import http.client

    p = subprocess.Popen(command + ["--app", folder, "--port", "0"], stdout=subprocess.PIPE, text=True, encoding="utf-8")
    try:
        line = p.stdout.readline()
        check("the app's server starts", line.startswith("listening on http://"), line)
        host, port = line.strip()[len("listening on http://"):].rsplit(":", 1)
        kept = http.client.HTTPConnection(host, int(port), timeout=20)
        kept.request("GET", "/app/local/")
        page = kept.getresponse().read().decode()
        check("the app's page", "main.client.dart.js" in page, page[:200])
        other = http.client.HTTPConnection(host, int(port), timeout=20)
        other.request("GET", "/app/local/app.css")
        css = other.getresponse()
        check("a second connection while the first is kept open", css.status == 200 and len(css.read()) > 0)
        other.request("POST", "/api/local/workspace", body="{}", headers={"Content-Type": "application/json"})
        answer = json.loads(other.getresponse().read())
        check("the app's interface", any(f["name"] == "model.xlsx" for f in answer["files"]), answer)
    finally:
        p.kill()
        p.wait()


def main():
    args = sys.argv[1:]
    command = args[args.index("--mcp") + 1 :] if "--mcp" in args else [sys.executable, "-m", "xlcells.mcp"]
    e = engine("the smoke test")
    print("engine:", e.__name__, e.engine_info(), "Python", sys.version.split()[0], sys.platform, flush=True)
    d = tempfile.mkdtemp(prefix="xlcells-smoke-")
    path = os.path.join(d, "model.xlsx")
    workbook(path)

    wb = xo.load_workbook(path)
    report = wb.calculate()
    check("the formulas are calculated", report["computed"] == 5, report)
    wb.save(os.path.join(d, "calculated.xlsx"))
    value = xo.load_workbook(os.path.join(d, "calculated.xlsx"), data_only=True)["Sales"]["D3"].value
    check("a value is right", abs(value - 4.76) < 1e-9, value)
    check("the regions are found", sorted(wb.regions) == ["Model", "Sales"], sorted(wb.regions))
    frame = wb.regions["Sales"].to_polars()
    check("a region is a dataframe", frame.shape == (4, 4), frame.shape)
    store = wb.to_store(os.path.join(d, "model.store"))
    store.run()
    again = store.recalculate()
    check("the store calculates without the workbook", len(again["done"]) == 2, again)

    replies = mcp(
        command,
        [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "open_workbook", "arguments": {"path": path}}},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "set_value", "arguments": {"store": path + ".store", "region": "Model", "name": "Tax rate", "value": 0.2}}},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "export_workbook", "arguments": {"store": path + ".store", "path": os.path.join(d, "out.xlsx")}}},
        ],
    )
    check("the server starts", replies[0]["result"]["serverInfo"]["name"] == "xl-mcp", replies[0])
    for r in replies[1:]:
        check("tool call {0}".format(r["id"]), not r["result"].get("isError"), r)
    out = xo.load_workbook(os.path.join(d, "out.xlsx"), data_only=True)
    check("the workbook comes back with the change", abs(out["Sales"]["D3"].value - 4.8) < 1e-9, out["Sales"]["D3"].value)
    app(command, d)
    print("smoke test passed")


if __name__ == "__main__":
    main()
