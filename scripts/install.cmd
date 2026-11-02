@echo off
rem CSR installer for Windows (beta): double-click this file in the unpacked zip.
rem It asks for Administrator rights and runs install.ps1 from this folder.
rem For options (-Rpc, -CkbDir, -NoRanking, -NoBrowser) run install.ps1 in an Administrator PowerShell instead.
fltmc >nul 2>&1
if errorlevel 1 (
  set "CSR_SELF=%~f0"
  powershell -NoProfile -Command "Start-Process -FilePath $env:CSR_SELF -Verb RunAs"
  exit /b
)
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
echo.
pause
