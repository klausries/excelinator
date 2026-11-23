"""Makes the demo workbook of the documentation (docs/demo/sales-plan.xlsx) with the
openpyxl interface of xlcells, its formulas calculated by xlcells' engine.

    uv run python tools/make_demo.py
"""
from pathlib import Path

import xlcells.openpyxl as xo
from xlcells.openpyxl.styles import Font, PatternFill

OUT = Path(__file__).resolve().parent.parent / "docs" / "demo" / "sales-plan.xlsx"

wb = xo.Workbook()
ws = wb.active
ws.title = "Sales"
head = ["Month", "Region", "Product", "Units", "Price", "Revenue", "Discount", "Net"]
ws.append(head)
rows = [
    ("Jan", "North", "Basic", 120, 19.9), ("Jan", "South", "Pro", 45, 49.0), ("Feb", "North", "Pro", 52, 49.0),
    ("Feb", "South", "Basic", 140, 19.9), ("Mar", "North", "Basic", 133, 19.9), ("Mar", "West", "Pro", 61, 49.0),
    ("Apr", "West", "Basic", 98, 19.9), ("Apr", "South", "Pro", 58, 49.0), ("May", "North", "Pro", 70, 49.0),
    ("May", "West", "Basic", 151, 19.9), ("Jun", "South", "Basic", 160, 19.9), ("Jun", "North", "Pro", 66, 49.0),
]
for k, (month, region, product, units, price) in enumerate(rows, start=2):
    ws.append([month, region, product, units, price, f"=D{k}*E{k}", f"=IF(D{k}>=100,F{k}*Model!$B$2,0)", f"=ROUND((F{k}-G{k})*(1+Model!$B$1),2)"])
for cell in ws[1]:
    cell.font = Font(bold=True, color="FFFFFF")
    cell.fill = PatternFill("solid", fgColor="107C41")
ws.column_dimensions["C"].width = 12
m = wb.create_sheet("Model")
m.append(["Tax rate", 0.19])
m.append(["Volume discount", 0.05])
m.append(["Total net", "=SUM(Sales!H2:H13)"])
m.append(["Average units", "=ROUND(AVERAGE(Sales!D2:D13),1)"])
m.append(["Best month", '=INDEX(Sales!A2:A13,MATCH(MAX(Sales!H2:H13),Sales!H2:H13,0))'])
for cell in m["A"]:
    cell.font = Font(bold=True)
m.column_dimensions["A"].width = 18
print(wb.calculate())
OUT.parent.mkdir(parents=True, exist_ok=True)
wb.save(OUT)
print("written:", OUT)
