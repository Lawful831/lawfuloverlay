# -*- mode: python ; coding: utf-8 -*-
"""
PyInstaller spec for the LawfulOverlay CLIENT.
Produces a single, self-contained .exe — no separate folder needed.

Build with:
    pyinstaller LawfulOverlay.spec
or via the helper script:
    scripts\build.bat
"""

from PyInstaller.utils.hooks import collect_all, collect_submodules

datas = []
binaries = []
hiddenimports = []

# ── websockets ────────────────────────────────────────────────────────────────
# websockets uses a lot of internal submodules that PyInstaller can miss.
for pkg in ("websockets",):
    tmp = collect_all(pkg)
    datas    += tmp[0]
    binaries += tmp[1]
    hiddenimports += tmp[2]

# numpy + Pillow (PIL) ─ collect_all handles DLLs & data files
for pkg in ("numpy", "PIL"):
    tmp = collect_all(pkg)
    datas    += tmp[0]
    binaries += tmp[1]
    hiddenimports += tmp[2]

# Tkinter is part of the stdlib; make sure its data files travel with the exe
hiddenimports += collect_submodules("tkinter")

# ─────────────────────────────────────────────────────────────────────────────
a = Analysis(
    ["app.py"],
    pathex=[],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    # Strip discord.py & aiohttp — client no longer needs them
    excludes=["discord", "aiohttp", "nacl"],
    noarchive=False,
    optimize=1,          # basic bytecode optimisation (strips docstrings)
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="LawfulOverlay",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    # windowed=True → no console window; set to False while debugging
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=None,           # replace with "assets/icon.ico" when you have one
)
