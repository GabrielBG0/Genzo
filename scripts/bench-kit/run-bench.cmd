@echo off
rem Double-click to run the Genzo benchmark (Windows). Runs run-bench.ps1
rem without changing the system's PowerShell execution policy.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-bench.ps1" %*
