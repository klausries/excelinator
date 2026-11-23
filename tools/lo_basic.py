"""LibreOffice driven by a Basic macro, without LibreOffice's Python.

LibreOffice's own Python (the only one its UNO bridge loads into) is a separate program
inside the application bundle, which macOS may refuse to start (quarantine of a
downloaded application). The office itself runs; so the work that needs UNO is done by
a Basic macro inside it, and this module (plain Python, any version) prepares it, runs
it and keeps it going:

- a LibreOffice profile of its own, whose `Standard` library holds the macros below
  (macros of the profile are trusted, whatever the macro security setting);
- `run(sub, items, status)`: writes the items to a job file, starts
  `soffice --headless "macro:///Standard.Module1.<sub>(job)"`, and watches the status
  files the macro writes after each item. An item that takes too long (or makes
  LibreOffice write huge temporary files) is given up: LibreOffice is stopped, the item
  marked failed, and the rest goes on in a new LibreOffice.

The macros write text (UTF-8) only; the Python side turns it into what the tools need.
"""
import os
import shutil
import subprocess
import tempfile
import time

SOFFICE_CANDIDATES = ["/Applications/LibreOffice.app/Contents/MacOS/soffice", "soffice", "libreoffice"]

BASIC = r'''
Option Explicit

Const T = 9

Function LoadArgs()
  Dim a(3) As New com.sun.star.beans.PropertyValue
  a(0).Name = "Hidden" : a(0).Value = True
  a(1).Name = "ReadOnly" : a(1).Value = True
  a(2).Name = "UpdateDocMode" : a(2).Value = 0
  a(3).Name = "MacroExecutionMode" : a(3).Value = 0
  LoadArgs = a()
End Function

Function StoreArgs(filter As String)
  Dim a(1) As New com.sun.star.beans.PropertyValue
  a(0).Name = "FilterName" : a(0).Value = filter
  a(1).Name = "Overwrite" : a(1).Value = True
  StoreArgs = a()
End Function

Sub WriteFile(path As String, text As String)
  Dim sfa As Object, tos As Object, url As String
  sfa = createUnoService("com.sun.star.ucb.SimpleFileAccess")
  url = ConvertToURL(path)
  If sfa.exists(url) Then sfa.kill(url)
  tos = createUnoService("com.sun.star.io.TextOutputStream")
  tos.setOutputStream(sfa.openFileWrite(url))
  tos.setEncoding("UTF-8")
  tos.writeString(text)
  tos.closeOutput()
End Sub

Function ReadLines(path As String)
  Dim sfa As Object, tis As Object, lines() As String, n As Long
  sfa = createUnoService("com.sun.star.ucb.SimpleFileAccess")
  tis = createUnoService("com.sun.star.io.TextInputStream")
  tis.setInputStream(sfa.openFileRead(ConvertToURL(path)))
  tis.setEncoding("UTF-8")
  n = -1
  Do While Not tis.isEOF()
    n = n + 1
    ReDim Preserve lines(n)
    lines(n) = tis.readLine()
  Loop
  tis.closeInput()
  ReadLines = lines()
End Function

Function Esc(s) As String
  Dim x As String
  x = Replace(CStr(s), "\", "\\")
  x = Replace(x, Chr(9), "\t")
  x = Replace(x, Chr(10), "\n")
  Esc = Replace(x, Chr(13), "\r")
End Function

Function B(v) As String
  If v Then B = "1" Else B = "0"
End Function

Function Num(v) As String
  Num = Trim(Str(v))
End Function

Function Escapement(cell) As String
  ' Some cells have no value for it (rich text): 0 then.
  On Error GoTo None
  Escapement = cell.CharEscapement
  Exit Function
None:
  Escapement = "0"
End Function

Function Line2(b) As String
  If b.OuterLineWidth = 0 And b.InnerLineWidth = 0 And b.LineWidth = 0 Then
    Line2 = ""
  Else
    Line2 = b.LineStyle & "," & b.LineWidth & "," & b.Color
  End If
End Function

Function FormatOf(cell, formats) As String
  On Error GoTo None
  FormatOf = Esc(formats.getByKey(cell.NumberFormat).FormatString)
  Exit Function
None:
  FormatOf = "\N"
End Function

Function CellLine(sh, r As Long, c As Long, formats) As String
  Dim cell As Object, s As String, back As String, prot As Object
  cell = sh.getCellByPosition(c, r)
  If cell.IsCellBackgroundTransparent Then back = "" Else back = cell.CellBackColor
  prot = cell.CellProtection
  s = "C" & Chr(T) & r & Chr(T) & c
  s = s & Chr(T) & Esc(cell.CharFontName) & Chr(T) & Num(cell.CharHeight) & Chr(T) & B(cell.CharWeight >= 150) & Chr(T) & B(cell.CharPosture <> 0)
  s = s & Chr(T) & cell.CharUnderline & Chr(T) & cell.CharStrikeout & Chr(T) & cell.CharColor & Chr(T) & Escapement(cell) & Chr(T) & back
  s = s & Chr(T) & Line2(cell.LeftBorder2) & Chr(T) & Line2(cell.RightBorder2) & Chr(T) & Line2(cell.TopBorder2) & Chr(T) & Line2(cell.BottomBorder2)
  s = s & Chr(T) & Line2(cell.DiagonalTLBR2) & Chr(T) & Line2(cell.DiagonalBLTR2)
  s = s & Chr(T) & CLng(cell.HoriJustify) & Chr(T) & cell.VertJustify & Chr(T) & B(cell.IsTextWrapped) & Chr(T) & cell.RotateAngle & Chr(T) & cell.ParaIndent
  s = s & Chr(T) & B(cell.ShrinkToFit) & Chr(T) & B(prot.IsLocked) & Chr(T) & B(prot.IsFormulaHidden) & Chr(T) & FormatOf(cell, formats)
  CellLine = s & Chr(10)
End Function

' Load, recalculate everything, save as xlsx. Job lines: source, target, status file.
Sub Oracle(job As String)
  Dim items, parts, i As Long
  items = ReadLines(job)
  For i = LBound(items) To UBound(items)
    parts = Split(items(i), Chr(T))
    If UBound(parts) >= 2 Then OracleOne(parts(0), parts(1), parts(2))
  Next
  StarDesktop.terminate()
End Sub

Sub OracleOne(src As String, dst As String, status As String)
  Dim doc As Object
  On Error GoTo Failed
  doc = StarDesktop.loadComponentFromURL(ConvertToURL(src), "_blank", 0, LoadArgs())
  If IsNull(doc) Then
    WriteFile(status, "not-a-spreadsheet")
    Exit Sub
  End If
  If Not HasUnoInterfaces(doc, "com.sun.star.sheet.XCalculatable") Then
    doc.close(True)
    WriteFile(status, "not-a-spreadsheet")
    Exit Sub
  End If
  doc.calculateAll()
  doc.storeToURL(ConvertToURL(dst), StoreArgs("Calc MS Excel 2007 XML"))
  doc.close(True)
  WriteFile(status, "ok")
  Exit Sub
Failed:
  WriteFile(status, "error: " & Err & " " & Error$ & " (line " & Erl & ")")
  On Error Resume Next
  If Not IsNull(doc) Then doc.close(True)
End Sub

' The formatting of sampled cells, and the document as HTML. Job lines: source, output
' prefix (writes <prefix>.cells.tsv and <prefix>.html), status file.
Sub Look(job As String)
  Dim items, parts, i As Long
  items = ReadLines(job)
  For i = LBound(items) To UBound(items)
    parts = Split(items(i), Chr(T))
    If UBound(parts) >= 2 Then LookOne(parts(0), parts(1), parts(2))
  Next
  StarDesktop.terminate()
End Sub

Function CfCells(sh, lastRow As Long, lastCol As Long, formats) As String
  Dim out As String, cfs, rngs, k As Long, i As Long, j As Long, r As Long, c As Long
  On Error GoTo Done
  cfs = sh.ConditionalFormats.ConditionalFormats
  For i = LBound(cfs) To UBound(cfs)
    rngs = cfs(i).Range.RangeAddresses
    For j = LBound(rngs) To UBound(rngs)
      k = 0
      For r = rngs(j).StartRow To IIf(rngs(j).EndRow < lastRow, rngs(j).EndRow, lastRow)
        For c = rngs(j).StartColumn To IIf(rngs(j).EndColumn < lastCol, rngs(j).EndColumn, lastCol)
          If k < 60 Then out = out & CellLine(sh, r, c, formats)
          k = k + 1
        Next
      Next
    Next
  Next
Done:
  CfCells = out
End Function

Sub LookOne(src As String, prefix As String, status As String)
  Dim doc As Object, sh As Object, cur As Object, formats As Object, out As String
  Dim i As Long, r As Long, c As Long, lastRow As Long, lastCol As Long, small As Boolean, hidden As Boolean
  On Error GoTo Failed
  doc = StarDesktop.loadComponentFromURL(ConvertToURL(src), "_blank", 0, LoadArgs())
  If IsNull(doc) Then
    WriteFile(status, "not-a-spreadsheet")
    Exit Sub
  End If
  If Not HasUnoInterfaces(doc, "com.sun.star.sheet.XSpreadsheetDocument") Then
    doc.close(True)
    WriteFile(status, "not-a-spreadsheet")
    Exit Sub
  End If
  formats = doc.NumberFormats
  small = True
  For i = 0 To doc.Sheets.Count - 1
    sh = doc.Sheets.getByIndex(i)
    cur = sh.createCursor()
    cur.gotoEndOfUsedArea(False)
    lastRow = cur.RangeAddress.EndRow
    lastCol = cur.RangeAddress.EndColumn
    cur = sh.createCursor()
    cur.gotoStartOfUsedArea(False)
    If (CDbl(lastRow) + 1) * (CDbl(lastCol) + 1) > 2000000 Then small = False
    hidden = False
    For r = 0 To lastRow
      If Not sh.Rows.getByIndex(r).IsVisible Then
        hidden = True
        Exit For
      End If
    Next
    If Not hidden Then
      For c = 0 To lastCol
        If Not sh.Columns.getByIndex(c).IsVisible Then
          hidden = True
          Exit For
        End If
      Next
    End If
    out = out & "S" & Chr(T) & Esc(sh.Name) & Chr(T) & B(sh.IsVisible) & Chr(T) & lastRow & Chr(T) & lastCol & Chr(T) & cur.RangeAddress.StartRow & Chr(T) & cur.RangeAddress.StartColumn & Chr(T) & B(hidden) & Chr(10)
    For r = 0 To IIf(lastRow < 39, lastRow, 39)
      For c = 0 To IIf(lastCol < 14, lastCol, 14)
        out = out & CellLine(sh, r, c, formats)
      Next
    Next
    out = out & CfCells(sh, lastRow, lastCol, formats)
  Next
  WriteFile(prefix & ".cells.tsv", out)
  ' A huge used area makes a huge export: left out.
  If small Then doc.storeToURL(ConvertToURL(prefix & ".html"), StoreArgs("HTML (StarCalc)"))
  doc.close(True)
  WriteFile(status, "ok")
  Exit Sub
Failed:
  WriteFile(status, "error: " & Err & " " & Error$ & " (line " & Erl & ")")
  On Error Resume Next
  If Not IsNull(doc) Then doc.close(True)
End Sub
'''


def soffice():
    for c in SOFFICE_CANDIDATES:
        if os.path.isabs(c) and os.path.exists(c):
            return c
        if not os.path.isabs(c) and shutil.which(c):
            return shutil.which(c)
    raise RuntimeError("LibreOffice (soffice) not found")


def _xml(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def profile():
    """A LibreOffice profile of these tools (English), with the macros installed."""
    path = os.path.join(tempfile.gettempdir(), "xlcells_lo_profile")
    module = os.path.join(path, "user", "basic", "Standard", "Module1.xba")
    if not os.path.exists(module):
        subprocess.run([soffice(), "--headless", "--terminate_after_init", "-env:UserInstallation=file://" + path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
    if not os.path.exists(module):
        raise RuntimeError("LibreOffice did not make its profile in " + path)
    # English, whatever the system's language: number formats are then named as Excel
    # names them ("General", "BOOLEAN").
    settings = os.path.join(path, "user", "registrymodifications.xcu")
    if os.path.exists(settings):
        with open(settings, encoding="utf-8") as f:
            xcu = f.read()
        items = "".join('<item oor:path="/org.openoffice.Setup/L10N"><prop oor:name="%s" oor:op="fuse"><value>en-US</value></prop></item>\n' % k
                        for k in ("ooLocale", "ooSetupSystemLocale") if 'oor:name="%s"' % k not in xcu)
        if items:
            with open(settings, "w", encoding="utf-8") as f:
                f.write(xcu.replace("</oor:items>", items + "</oor:items>"))
    with open(module, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE script:module PUBLIC "-//OpenOffice.org//DTD OfficeDocument 1.0//EN" "module.dtd">\n'
                '<script:module xmlns:script="http://openoffice.org/2000/script" script:name="Module1" script:language="StarBasic" script:moduleType="normal">'
                + _xml(BASIC) + "</script:module>\n")
    return path


def _dir_size(path):
    total = 0
    for d, _s, names in os.walk(path):
        for n in names:
            try:
                total += os.path.getsize(os.path.join(d, n))
            except OSError:
                pass
    return total


def run(sub, items, timeout=120, max_temp=1 << 30, log=print):
    """Run the macro `sub` over items: tuples of fields whose last field is the status
    file the macro writes when it is done with the item. Returns {status file: status}.
    An item without a status after `timeout` seconds is given up."""
    prof = profile()
    work = tempfile.mkdtemp(prefix="xlcells_lo_")
    temp = os.path.join(work, "tmp")
    os.makedirs(temp)
    env = dict(os.environ, TMPDIR=temp)
    pending = [tuple(map(str, it)) for it in items]
    for it in pending:
        if os.path.exists(it[-1]):
            os.remove(it[-1])
    result = {}
    idle_rounds = 0
    while pending:
        job = os.path.join(work, "job.tsv")
        with open(job, "w", encoding="utf-8") as f:
            f.write("".join("\t".join(it) + "\n" for it in pending))
        proc = subprocess.Popen([soffice(), "--headless", "--invisible", "--norestore", "--nologo", "--nodefault", "-env:UserInstallation=file://" + prof,
                                 'macro:///Standard.Module1.%s("%s")' % (sub, job)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
        done, last = 0, time.time()
        stuck = False
        while proc.poll() is None:
            time.sleep(0.5)
            n = sum(1 for it in pending if os.path.exists(it[-1]))
            if n > done:
                done, last = n, time.time()
            if time.time() - last > timeout or _dir_size(temp) > max_temp:
                stuck = True
                proc.kill()
                proc.wait()
                break
        for it in pending:
            if os.path.exists(it[-1]):
                with open(it[-1], encoding="utf-8", errors="replace") as f:
                    result[it[-1]] = f.read().strip()
        rest = [it for it in pending if it[-1] not in result]
        if rest and len(rest) == len(pending) and not stuck:
            idle_rounds += 1
            if idle_rounds >= 3:
                raise RuntimeError("the LibreOffice macro %s does not run (soffice exits without doing anything)" % sub)
        else:
            idle_rounds = 0
        if rest and (stuck or len(rest) < len(pending) or idle_rounds):
            # The item it was at is given up; LibreOffice starts again with the rest.
            first = rest[0]
            why = "error: timeout or crash (given up after %d s)" % timeout if stuck else "error: LibreOffice ended while at this file"
            with open(first[-1], "w", encoding="utf-8") as f:
                f.write(why)
            result[first[-1]] = why
            log("given up: " + first[0])
            rest = rest[1:]
        pending = rest
        shutil.rmtree(temp, ignore_errors=True)
        os.makedirs(temp)
    shutil.rmtree(work, ignore_errors=True)
    return result
