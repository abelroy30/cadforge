@echo off
REM cadforge - build and start the studio, then open it in your browser.
setlocal

where cargo >nul 2>nul
if errorlevel 1 (
  echo.
  echo   Rust is not installed. Get it from https://rustup.rs and run this again.
  echo   Everything else is already in this folder - no other setup needed.
  echo.
  pause
  exit /b 1
)

echo Starting cadforge on http://localhost:8787 ...
start "" /b cmd /c "timeout /t 4 >nul & start "" http://localhost:8787"
cargo run --release -- %*
endlocal
