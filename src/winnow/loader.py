"""Background image loading with an on-disk thumbnail cache.

QImage is safe to build off the main thread; we do the decode + resize in a
QThreadPool worker and hand the finished QImage back via a signal. A monotonic
"generation" lets the UI ignore results for requests it no longer cares about
(e.g. the user paged past them).
"""

from __future__ import annotations

import hashlib
from pathlib import Path

from PySide6.QtCore import QObject, QRunnable, QThreadPool, Signal
from PySide6.QtGui import QImage

try:
    from PIL import Image, ImageOps
    _HAVE_PIL = True
except ImportError:  # pragma: no cover
    _HAVE_PIL = False


def _cache_dir() -> Path:
    d = Path.home() / ".cache" / "winnow" / "thumbs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _cache_key(path: Path, mtime: float, size: int, thumb: int) -> Path:
    h = hashlib.sha1(f"{path}:{mtime}:{size}:{thumb}".encode()).hexdigest()
    return _cache_dir() / f"{h}.jpg"


def _pil_to_qimage(img) -> QImage:
    if img.mode not in ("RGB", "RGBA"):
        img = img.convert("RGBA" if "A" in img.mode else "RGB")
    data = img.tobytes()
    if img.mode == "RGBA":
        qi = QImage(data, img.width, img.height, 4 * img.width, QImage.Format_RGBA8888)
    else:
        qi = QImage(data, img.width, img.height, 3 * img.width, QImage.Format_RGB888)
    return qi.copy()  # detach from the temporary buffer


def load_thumbnail(path: Path, mtime: float, size: int, thumb: int = 256) -> QImage | None:
    """Return a cached or freshly generated thumbnail QImage."""
    cache_file = _cache_key(path, mtime, size, thumb)
    if cache_file.exists():
        qi = QImage(str(cache_file))
        if not qi.isNull():
            return qi
    if not _HAVE_PIL:
        qi = QImage(str(path))
        if qi.isNull():
            return None
        return qi.scaled(thumb, thumb, aspectMode=1, mode=1)  # KeepAspectRatio, Smooth
    try:
        with Image.open(path) as im:
            im = ImageOps.exif_transpose(im)
            im.thumbnail((thumb, thumb), Image.LANCZOS)
            rgb = im.convert("RGB")
            rgb.save(cache_file, "JPEG", quality=85)
            return _pil_to_qimage(rgb)
    except Exception:
        return None


def load_full(path: Path) -> QImage | None:
    """Load a full-resolution image (EXIF-oriented) as a QImage."""
    if _HAVE_PIL:
        try:
            with Image.open(path) as im:
                im = ImageOps.exif_transpose(im)
                return _pil_to_qimage(im)
        except Exception:
            pass
    qi = QImage(str(path))
    return None if qi.isNull() else qi


class _Signals(QObject):
    thumbReady = Signal(str, QImage, int)   # rel_path, image, generation
    fullReady = Signal(str, QImage, int)    # rel_path, image, generation


class _ThumbTask(QRunnable):
    def __init__(self, signals, rel, path, mtime, size, thumb, gen):
        super().__init__()
        self.signals, self.rel, self.path = signals, rel, path
        self.mtime, self.size, self.thumb, self.gen = mtime, size, thumb, gen

    def run(self):
        img = load_thumbnail(self.path, self.mtime, self.size, self.thumb)
        if img is not None:
            self.signals.thumbReady.emit(self.rel, img, self.gen)


class _FullTask(QRunnable):
    def __init__(self, signals, rel, path, gen):
        super().__init__()
        self.signals, self.rel, self.path, self.gen = signals, rel, path, gen

    def run(self):
        img = load_full(self.path)
        if img is not None:
            self.signals.fullReady.emit(self.rel, img, self.gen)


class Loader(QObject):
    """Facade over a QThreadPool for thumbnail and full-image loads."""

    thumbReady = Signal(str, QImage, int)
    fullReady = Signal(str, QImage, int)

    def __init__(self, thumb_size: int = 256, parent=None):
        super().__init__(parent)
        self.thumb_size = thumb_size
        self._pool = QThreadPool.globalInstance()
        self._sig = _Signals()
        self._sig.thumbReady.connect(self.thumbReady)
        self._sig.fullReady.connect(self.fullReady)
        self._pending_thumbs: set[str] = set()

    def request_thumb(self, rel: str, path: Path, mtime: float, size: int, generation: int = 0):
        task = _ThumbTask(self._sig, rel, path, mtime, size, self.thumb_size, generation)
        self._pool.start(task)

    def request_full(self, rel: str, path: Path, generation: int = 0):
        self._pool.start(_FullTask(self._sig, rel, path, generation))
