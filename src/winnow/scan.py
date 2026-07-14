"""Folder scanning for image files."""

from __future__ import annotations

import os
from pathlib import Path

# Extensions Qt/Pillow can typically display. Kept lowercase.
IMAGE_EXTENSIONS = {
    ".jpg", ".jpeg", ".jpe", ".png", ".bmp", ".gif", ".tif", ".tiff",
    ".webp", ".ppm", ".pgm", ".pbm", ".pnm", ".xbm", ".xpm", ".ico",
}


def is_image(path: Path) -> bool:
    return path.suffix.lower() in IMAGE_EXTENSIONS


def scan_folder(
    root: Path,
    recursive: bool = True,
    exclude_dirs: set[str] | None = None,
) -> list[Path]:
    """Return absolute paths of image files under ``root``.

    ``exclude_dirs`` is a set of directory *names* to skip entirely (e.g. the
    bucket folders like ``_rejected``). Hidden dirs (starting with ``.``) are
    always skipped.
    """
    exclude = set(exclude_dirs or ())
    root = root.resolve()
    results: list[Path] = []

    if not recursive:
        try:
            entries = sorted(root.iterdir())
        except OSError:
            return results
        for entry in entries:
            if entry.is_file() and is_image(entry):
                results.append(entry)
        return results

    for dirpath, dirnames, filenames in os.walk(root):
        # Prune excluded and hidden directories in-place so os.walk skips them.
        dirnames[:] = [
            d for d in dirnames
            if d not in exclude and not d.startswith(".")
        ]
        dirnames.sort()
        base = Path(dirpath)
        for name in sorted(filenames):
            p = base / name
            if is_image(p):
                results.append(p)
    return results
