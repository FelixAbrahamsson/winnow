# -*- mode: python ; coding: utf-8 -*-
#
# PyInstaller spec for winnow (one-dir build).
# Build from the repository root:
#
#     pyinstaller packaging/winnow.spec --noconfirm
#
# Produces dist/winnow/ containing the `winnow` executable and its Qt runtime.

import os

ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))

a = Analysis(
    [os.path.join(ROOT, "src", "winnow", "__main__.py")],
    pathex=[os.path.join(ROOT, "src")],
    binaries=[],
    datas=[(os.path.join(ROOT, "src", "winnow", "resources"), "winnow/resources")],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=["tkinter"],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="winnow",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    disable_windowed_traceback=False,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="winnow",
)
