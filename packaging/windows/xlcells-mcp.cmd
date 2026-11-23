@echo off
rem The MCP server of excelinator (stdio; --app FOLDER for the app in the browser).
"%~dp0python\python.exe" -X utf8 -m xlcells.mcp %*
