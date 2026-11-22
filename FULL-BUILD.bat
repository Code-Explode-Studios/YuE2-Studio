@echo off
rem Clean dependency install, tests, optimised portable EXE and ZIP.
call "%~dp0scripts\launch-local-build.cmd" Full
exit /b %ERRORLEVEL%
