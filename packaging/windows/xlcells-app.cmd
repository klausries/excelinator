@echo off
rem The app in the browser on the Excel files of a folder: xlcells-app.cmd FOLDER [PORT]
if "%~1"=="" (
  echo usage: xlcells-app.cmd FOLDER [PORT]
  exit /b 2
)
set "PORT=%~2"
if "%PORT%"=="" set "PORT=8787"
echo The app: http://127.0.0.1:%PORT%/   (stop with Ctrl+C)
start "" "http://127.0.0.1:%PORT%/"
"%~dp0python\python.exe" -X utf8 -m xlcells.mcp --app "%~1" --port %PORT%
