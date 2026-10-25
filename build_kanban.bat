@echo off
setlocal
cd /d "%~dp0"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-kanban.ps1" %*
set "BUILD_EXIT=%ERRORLEVEL%"
echo.
if not "%BUILD_EXIT%"=="0" echo [ERROR] Packaging failed. See the error and log path above.
if /I not "%~1"=="-NoPause" pause
exit /b %BUILD_EXIT%
