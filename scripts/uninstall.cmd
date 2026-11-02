@echo off
rem CSR uninstaller for Windows (beta): double-click this file. It asks for Administrator rights and runs
rem uninstall.ps1 from this folder. Data (configs, runner keys, logs) is kept; to delete it as well, run
rem   uninstall.cmd -RemoveData
rem from a command prompt (you must type DELETE to confirm).
set "CSR_ARG="
if /i "%~1"=="-RemoveData" set "CSR_ARG=-RemoveData"
fltmc >nul 2>&1
if errorlevel 1 (
  set "CSR_SELF=%~f0"
  powershell -NoProfile -Command "if ($env:CSR_ARG) { Start-Process -FilePath $env:CSR_SELF -ArgumentList $env:CSR_ARG -Verb RunAs } else { Start-Process -FilePath $env:CSR_SELF -Verb RunAs }"
  exit /b
)
set "CSR_PS=%~dp0uninstall.ps1"
rem leave this folder first: the uninstaller deletes C:\Program Files\csr, where this file may live
cd /d "%TEMP%"
powershell -NoProfile -ExecutionPolicy Bypass -File "%CSR_PS%" %CSR_ARG% & echo. & pause & exit /b
