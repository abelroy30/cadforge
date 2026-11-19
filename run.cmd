@echo off
REM cadforge - start the studio and open it in your browser.
REM
REM Windows 11: just double-click this file.
REM   * first run  - builds the server (a few minutes, needs Rust: see below)
REM   * later runs - starts immediately and opens the browser for you
REM
REM To stop the studio, close this window or press Ctrl+C.
setlocal
cd /d "%~dp0"
title cadforge

if exist "target\release\cadforge.exe" goto :run

where cargo >nul 2>nul
if errorlevel 1 goto :norust

echo.
echo   First run: building cadforge. This takes a few minutes and only
echo   happens once; everything after this starts in a second.
echo.
cargo build --release
if errorlevel 1 goto :buildfailed
echo.

:run
echo   Starting cadforge. A browser window will open by itself.
echo   Keep this window open while you work. Press Ctrl+C to stop.
echo.
target\release\cadforge.exe %*
echo.
echo   cadforge has stopped.
pause
goto :eof

:norust
echo.
echo   Rust is not installed, and it is needed once to build the studio.
echo.
echo     1. Open https://rustup.rs in your browser
echo     2. Download and run rustup-init.exe, and accept the defaults
echo     3. Close this window, then double-click run.cmd again
echo.
echo   Nothing else has to be installed to run the studio itself: the web
echo   interface, the CAD readers and the sample documents are all inside.
echo.
pause
exit /b 1

:buildfailed
echo.
echo   The build failed. Scroll up for the first error message.
echo   The usual cause is the Visual Studio C++ build tools, which the Rust
echo   installer offers to add - re-run rustup-init.exe and accept them.
echo.
pause
exit /b 1
