@echo off
:: ============================================================
::  LawfulOverlay — Build Script
::  Builds the overlay client into dist\LawfulOverlay.exe
::  (and the server into dist\lawful-overlay-server.exe)
:: ============================================================
setlocal

set "ROOT=%~dp0.."
pushd "%ROOT%"

echo.
echo  LawfulOverlay Build Script
echo  ==========================
echo.

where cargo >nul 2>&1
if errorlevel 1 (
    echo [ERROR] cargo not found in PATH. Install Rust from https://rustup.rs
    pause
    exit /b 1
)

echo [INFO] Building release binaries ...
cargo build --release --locked -p lawful-overlay -p lawful-overlay-server
if errorlevel 1 (
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

if not exist dist mkdir dist
copy /y target\release\lawful-overlay.exe dist\LawfulOverlay.exe >nul
copy /y target\release\lawful-overlay-server.exe dist\lawful-overlay-server.exe >nul

echo.
echo [OK] dist\LawfulOverlay.exe
echo [OK] dist\lawful-overlay-server.exe
echo.
popd
pause
endlocal
