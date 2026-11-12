@echo off
rem Windows wrapper for ksim: finds Python and runs the ksim script next to this file.
where py >nul 2>nul
if %errorlevel%==0 goto usepy
python "%~dp0ksim" %*
exit /b %errorlevel%
:usepy
py -3 "%~dp0ksim" %*
exit /b %errorlevel%
