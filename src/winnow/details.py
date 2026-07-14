"""Intrinsic image details (resolution, size, format) for the info panel."""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

try:
    from PIL import Image
    _HAVE_PIL = True
except ImportError:  # pragma: no cover
    _HAVE_PIL = False

_dim_cache: dict[str, tuple[int, int, str]] = {}


def _human_size(n: int) -> str:
    f = float(n)
    for unit in ("B", "KB", "MB", "GB"):
        if f < 1024 or unit == "GB":
            return f"{f:.0f} {unit}" if unit == "B" else f"{f:.1f} {unit}"
        f /= 1024
    return f"{n} B"


def dimensions(path: Path) -> tuple[int, int, str]:
    """(width, height, format) read from the header only; cached per path."""
    key = str(path)
    if key in _dim_cache:
        return _dim_cache[key]
    result = (0, 0, path.suffix.lstrip(".").upper())
    if _HAVE_PIL:
        try:
            with Image.open(path) as im:
                result = (im.width, im.height, im.format or result[2])
        except Exception:
            pass
    _dim_cache[key] = result
    return result


def details_for(item) -> list[tuple[str, str]]:
    """Ordered (label, value) rows describing the image file."""
    rows: list[tuple[str, str]] = []
    w, h, fmt = dimensions(item.abs_path)
    if w and h:
        mp = (w * h) / 1_000_000
        rows.append(("Resolution", f"{w} x {h}  ({mp:.1f} MP)"))
    rows.append(("Format", fmt))
    rows.append(("File size", _human_size(item.size_bytes())))
    st = item.stat()
    if st:
        rows.append(("Modified", datetime.fromtimestamp(st.st_mtime).strftime("%Y-%m-%d %H:%M")))
    rows.append(("Path", item.rel_path))
    return rows
