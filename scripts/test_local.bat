@echo off
:: ============================================================
::  LawfulOverlay — Local Test Script
::
::  Starts the mock WebSocket server and the overlay client
::  so you can verify everything works without Docker or Discord.
::  Your saved overlay settings are left untouched: the client
::  runs against a throwaway settings file pointing at the mock.
:: ============================================================
setlocal

set "ROOT=%~dp0.."
pushd "%ROOT%"

echo.
echo  LawfulOverlay -- Local Test
echo  ============================
echo.

where cargo >nul 2>&1
if errorlevel 1 (
    echo [ERROR] cargo not found in PATH. Install Rust from https://rustup.rs
    pause
    exit /b 1
)

echo [INFO] Building ...
cargo build -p lawful-overlay -p lawful-overlay-server
if errorlevel 1 (
    echo [ERROR] Build failed.
    pause
    exit /b 1
)

:: ── 1. Mock server in its own window ──────────────────────────────────────
echo.
echo  [1/2] Launching mock WebSocket server in a new window...
echo        (Fake messages every 4 seconds)
echo.
start "LawfulOverlay - Mock Server" cmd /k "%ROOT%\target\debug\mock-server.exe --interval 4"

timeout /t 2 /nobreak >nul

:: ── 2. Overlay client, with throwaway settings ────────────────────────────
set "LAWFUL_OVERLAY_CONFIG=%TEMP%\lawful_overlay_test_settings.json"
> "%LAWFUL_OVERLAY_CONFIG%" echo {"ws_url": "ws://127.0.0.1:8765"}

echo  [2/2] Launching overlay client...
echo        The overlay should appear and start receiving test messages.
echo.
"%ROOT%\target\debug\lawful-overlay.exe"

echo.
echo  Client exited. Close the "LawfulOverlay - Mock Server" window to stop.
echo.
popd
pause
endlocal
