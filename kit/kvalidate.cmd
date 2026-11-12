@echo off
rem Windows wrapper for kvalidate: finds Python and runs the kvalidate script next to this file.
where py >nul 2>nul
if %errorlevel%==0 goto usepy
python "%~dp0kvalidate" %*
exit /b %errorlevel%
:usepy
py -3 "%~dp0kvalidate" %*
exit /b %errorlevel%
