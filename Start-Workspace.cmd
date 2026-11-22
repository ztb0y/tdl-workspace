@echo off
setlocal
node.exe "%~dp0scripts\start-workspace.mjs"
set "exitCode=%ERRORLEVEL%"
if not "%exitCode%"=="0" pause
exit /b %exitCode%
