@echo off
REM Keyboard TestKit - Windows Run Script
REM This script builds and runs the keyboard testing utility

echo ========================================
echo    Keyboard TestKit
echo ========================================
echo.

REM Check if cargo is available
where cargo >nul 2>nul
if %ERRORLEVEL% neq 0 (
    echo ERROR: Cargo not found. Please install Rust from https://rustup.rs
    echo.
    pause
    exit /b 1
)

REM Build in release mode for better performance
echo Building Keyboard TestKit...
cargo build --release
if %ERRORLEVEL% neq 0 (
    echo.
    echo ERROR: Build failed. Please check the error messages above.
    pause
    exit /b 1
)

echo.
echo Build successful! Starting Keyboard TestKit...
echo.
echo Controls:
echo   Tab / Shift+Tab  - Switch between views
echo   1-9              - Jump to specific view
echo   R                - Reset current test
echo   Shift+R          - Reset all tests
echo   P                - Pause/Resume
echo   Q                - Quit
echo.

REM Run the application
cargo run --release

pause
