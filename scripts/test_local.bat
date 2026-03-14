@echo off
:: ============================================================
::  LawfulOverlay — Local Test Script
::
::  Starts the mock WebSocket server and the overlay client
::  so you can verify everything works without Docker or Discord.
:: ============================================================
setlocal EnableDelayedExpansion

:: Resolve the project root relative to this script's location
set "SCRIPTS=%~dp0"
:: Strip trailing backslash, then go one level up
set "ROOT=%SCRIPTS:~0,-1%\.."

set "VENV=%ROOT%\venv"
set "PYTHON_EXE=%VENV%\Scripts\python.exe"

echo.
echo  LawfulOverlay -- Local Test
echo  ============================
echo.

:: ── 1. Ensure venv exists ──────────────────────────────────────────────────
if not exist "%VENV%\Scripts\activate.bat" (
    echo [INFO] Virtual environment not found -- creating one ...
    python -m venv "%VENV%"
    if errorlevel 1 (
        echo [ERROR] Failed to create venv. Is Python 3.10+ installed?
        pause
        exit /b 1
    )
    echo [INFO] Installing dependencies ...
    "%PYTHON_EXE%" -m pip install --quiet --upgrade pip
    "%PYTHON_EXE%" -m pip install --quiet -r "%ROOT%\requirements.txt"
) else (
    echo [INFO] Using existing venv at %VENV%
)

:: ── 2. Ensure websockets is available (fast check) ────────────────────────
"%PYTHON_EXE%" -c "import websockets" >nul 2>&1
if errorlevel 1 (
    echo [INFO] Installing websockets ...
    "%PYTHON_EXE%" -m pip install --quiet websockets
)

:: ── 3. Launch mock server in a new labelled window ────────────────────────
echo.
echo  [1/2] Launching mock WebSocket server in a new window...
echo        (Fake messages every 4 seconds)
echo.

:: Use the helper bat to avoid inner-quote issues entirely
start "LawfulOverlay - Mock Server" cmd /k "%SCRIPTS%_start_mock.bat"

:: Give the server 2 seconds to bind the port
timeout /t 2 /nobreak >nul

:: ── 4. Launch the overlay client in this window ───────────────────────────
echo  [2/2] Launching overlay client...
echo        The overlay should appear and start receiving test messages.
echo.

"%PYTHON_EXE%" "%ROOT%\app.py"

echo.
echo  Client exited. Close the "LawfulOverlay - Mock Server" window to stop.
echo.
pause
endlocal
