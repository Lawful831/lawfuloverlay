@echo off
:: ============================================================
::  LawfulOverlay — Build Script
::  Packages app.py into a single standalone .exe
:: ============================================================
setlocal EnableDelayedExpansion

set "ROOT=%~dp0.."
set "VENV=%ROOT%\venv"
set "PYTHON=python"

echo.
echo  LawfulOverlay Build Script
echo  ==========================
echo.

:: ── 1. Locate Python ──────────────────────────────────────────────────────
where python >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Python not found in PATH. Please install Python 3.10+.
    pause
    exit /b 1
)
for /f "tokens=*" %%v in ('python --version 2^>^&1') do set "PY_VER=%%v"
echo [INFO] Using %PY_VER%

:: ── 2. Create / reuse venv ────────────────────────────────────────────────
if not exist "%VENV%\Scripts\activate.bat" (
    echo [INFO] Creating virtual environment at %VENV% ...
    python -m venv "%VENV%"
)
call "%VENV%\Scripts\activate.bat"

:: ── 3. Install / upgrade dependencies ────────────────────────────────────
echo [INFO] Installing client dependencies ...
pip install --quiet --upgrade pip
pip install --quiet -r "%ROOT%\requirements.txt"

echo [INFO] Installing PyInstaller ...
pip install --quiet --upgrade pyinstaller

:: ── 4. Clean previous build artefacts ────────────────────────────────────
echo [INFO] Cleaning previous build...
if exist "%ROOT%\dist\LawfulOverlay.exe" (
    del /f /q "%ROOT%\dist\LawfulOverlay.exe"
)
if exist "%ROOT%\build\LawfulOverlay" (
    rmdir /s /q "%ROOT%\build\LawfulOverlay"
)

:: ── 5. Run PyInstaller ────────────────────────────────────────────────────
echo [INFO] Running PyInstaller ...
cd /d "%ROOT%"
pyinstaller LawfulOverlay.spec --noconfirm

if errorlevel 1 (
    echo.
    echo [ERROR] Build failed. See output above for details.
    pause
    exit /b 1
)

:: ── 6. Report ─────────────────────────────────────────────────────────────
echo.
echo  ╔══════════════════════════════════════════════════════╗
echo  ║  Build complete!                                     ║
echo  ║  Output: dist\LawfulOverlay.exe                      ║
echo  ╚══════════════════════════════════════════════════════╝
echo.

:: Show file size
for %%f in ("%ROOT%\dist\LawfulOverlay.exe") do (
    set /a "SIZE_MB=%%~zf / 1048576"
    echo  Size: !SIZE_MB! MB
)
echo.

pause
endlocal
