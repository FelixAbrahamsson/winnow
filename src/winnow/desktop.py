"""Register a .desktop launcher + icons so the file manager offers
'Open With -> Winnow' for folders and images.

Works from any install method because the icons ship inside the package
(``winnow.resources``); the launcher points at whichever ``winnow`` is running.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from importlib.resources import files
from pathlib import Path

ICON_SIZES = (48, 64, 128, 256)
_MIME = ";".join(
    [
        "inode/directory",
        "image/jpeg",
        "image/png",
        "image/bmp",
        "image/gif",
        "image/tiff",
        "image/webp",
    ]
)


def _winnow_exe() -> str:
    """Absolute path to the currently running winnow entry point."""
    if getattr(sys, "frozen", False):  # PyInstaller binary
        return os.path.abspath(sys.executable)
    found = shutil.which("winnow")
    if found:
        return os.path.abspath(found)
    return os.path.abspath(sys.argv[0] or "winnow")


def install_desktop() -> Path:
    """Write the launcher + icons into ~/.local and refresh caches. Returns the
    path to the installed .desktop file."""
    apps = Path.home() / ".local/share/applications"
    icons = Path.home() / ".local/share/icons/hicolor"
    apps.mkdir(parents=True, exist_ok=True)

    res = files("winnow.resources")
    for sz in ICON_SIZES:
        data = (res / f"winnow-{sz}.png").read_bytes()
        dest = icons / f"{sz}x{sz}" / "apps"
        dest.mkdir(parents=True, exist_ok=True)
        (dest / "winnow.png").write_bytes(data)

    exe = _winnow_exe()
    desktop = apps / "winnow.desktop"
    desktop.write_text(
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Version=1.0\n"
        "Name=Winnow\n"
        "GenericName=Image Culling Tool\n"
        "Comment=Fast keyboard-driven image culling / selection\n"
        f"Exec={exe} %f\n"
        "Icon=winnow\n"
        "Terminal=false\n"
        "Categories=Graphics;Viewer;2DGraphics;\n"
        f"MimeType={_MIME};\n"
    )
    desktop.chmod(0o755)

    for cmd in (
        ["update-desktop-database", str(apps)],
        ["gtk-update-icon-cache", "-f", str(icons)],
    ):
        try:
            subprocess.run(cmd, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except FileNotFoundError:
            pass
    return desktop
