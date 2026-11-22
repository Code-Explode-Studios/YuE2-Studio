@echo off
rem Quick local rebuild; test suite and ZIP are skipped.
call "%~dp0scripts\launch-local-build.cmd" Fast
exit /b %ERRORLEVEL%
