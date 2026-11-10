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

:: Fish Audio voice: the API key is baked into the exe at build time.
:: It comes from the LAWFUL_FISH_API_KEY variable or, failing that, from
:: fish.env next to this repository's Cargo.toml (gitignored). Never commit it.
if not defined LAWFUL_FISH_API_KEY if exist fish.env (
    for /f "usebackq eol=# tokens=1,* delims==" %%A in ("fish.env") do (
        if /i "%%A"=="LAWFUL_FISH_API_KEY" set "LAWFUL_FISH_API_KEY=%%B"
    )
)
if defined LAWFUL_FISH_API_KEY (
    echo [INFO] Fish Audio: key included. PRIVATE build, do not publish this exe.
) else (
    echo [INFO] Fish Audio: no key, the exe will use the Edge and Windows voices.
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
