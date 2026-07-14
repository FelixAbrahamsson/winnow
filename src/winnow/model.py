"""Session state: the working image list, current position, sorting, and the
reversible move/undo engine shared by 'reject' and every category bucket."""

from __future__ import annotations

import shutil
from dataclasses import dataclass, field
from pathlib import Path

from PySide6.QtCore import QObject, Signal

from .buckets import Bucket, bucket_folder_names, load_buckets
from .metadata import Metadata
from .scan import scan_folder


class ImageItem:
    __slots__ = ("abs_path", "rel_path", "_stat")

    def __init__(self, abs_path: Path, root: Path):
        self.abs_path = abs_path
        self.rel_path = abs_path.relative_to(root).as_posix()
        self._stat = None

    @property
    def name(self) -> str:
        return self.abs_path.name

    def stat(self):
        if self._stat is None:
            try:
                self._stat = self.abs_path.stat()
            except OSError:
                self._stat = None
        return self._stat

    def size_bytes(self) -> int:
        st = self.stat()
        return st.st_size if st else 0

    def mtime(self) -> float:
        st = self.stat()
        return st.st_mtime if st else 0.0


@dataclass
class MoveOp:
    item: ImageItem
    from_abs: Path
    to_abs: Path
    list_index: int
    bucket_name: str


# Built-in sort keys -> label
BUILTIN_SORTS = {
    "name": "Name",
    "path": "Path",
    "mtime": "Date modified",
    "size": "File size",
}


def _unique_dest(dest: Path) -> Path:
    """Avoid clobbering an existing file at the destination."""
    if not dest.exists():
        return dest
    stem, suffix, parent = dest.stem, dest.suffix, dest.parent
    i = 1
    while True:
        cand = parent / f"{stem}__{i}{suffix}"
        if not cand.exists():
            return cand
        i += 1


class Session(QObject):
    currentChanged = Signal(int)      # new index (may equal old)
    listChanged = Signal()            # list length/order changed
    statusMessage = Signal(str)       # transient status text

    def __init__(
        self,
        root: Path,
        recursive: bool = True,
        buckets_config: Path | None = None,
        metadata_path: Path | None = None,
    ):
        super().__init__()
        self.root = root.resolve()
        self.recursive = recursive
        self.buckets: list[Bucket] = load_buckets(self.root, buckets_config)
        self.metadata: Metadata = (
            Metadata.load(metadata_path, self.root) if metadata_path else Metadata.empty()
        )

        exclude = bucket_folder_names(self.buckets, self.root)
        paths = scan_folder(self.root, recursive=self.recursive, exclude_dirs=exclude)
        self.items: list[ImageItem] = [ImageItem(p, self.root) for p in paths]
        self.index: int = 0
        self.undo_stack: list[MoveOp] = []
        self.redo_stack: list[MoveOp] = []
        self.sort_key: str = "name"
        self.sort_reverse: bool = False

    # ---- navigation -------------------------------------------------
    def count(self) -> int:
        return len(self.items)

    def current(self) -> ImageItem | None:
        if 0 <= self.index < len(self.items):
            return self.items[self.index]
        return None

    def set_index(self, i: int):
        if not self.items:
            return
        i = max(0, min(i, len(self.items) - 1))
        if i != self.index:
            self.index = i
        self.currentChanged.emit(self.index)

    def next(self):
        self.set_index(self.index + 1)

    def prev(self):
        self.set_index(self.index - 1)

    def jump(self, delta: int):
        self.set_index(self.index + delta)

    # ---- sorting ----------------------------------------------------
    def sortable_keys(self) -> list[tuple[str, str]]:
        keys = list(BUILTIN_SORTS.items())
        for col in self.metadata.columns:
            keys.append((f"meta:{col}", f"[meta] {col}"))
        return keys

    def _sort_value(self, item: ImageItem, key: str):
        if key == "name":
            return (0, 0.0, item.name.lower())
        if key == "path":
            return (0, 0.0, item.rel_path.lower())
        if key == "mtime":
            return (0, item.mtime(), "")
        if key == "size":
            return (0, float(item.size_bytes()), "")
        if key.startswith("meta:"):
            return self.metadata.sort_value(item.rel_path, key[5:])
        return (0, 0.0, item.name.lower())

    def apply_sort(self, key: str, reverse: bool):
        self.sort_key = key
        self.sort_reverse = reverse
        self.items.sort(key=lambda it: self._sort_value(it, key), reverse=reverse)
        self.index = 0  # jump to the first image of the new ordering
        self.listChanged.emit()
        self.currentChanged.emit(self.index)

    # ---- move / undo engine ----------------------------------------
    def bucket_by_name(self, name: str) -> Bucket | None:
        for b in self.buckets:
            if b.name == name:
                return b
        return None

    def _do_move(self, item: ImageItem, bucket: Bucket) -> MoveOp | None:
        try:
            idx = self.items.index(item)
        except ValueError:
            return None
        dest = bucket.target_dir(self.root) / item.rel_path  # preserve subfolders
        dest = _unique_dest(dest)
        try:
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(item.abs_path), str(dest))
        except OSError as e:
            self.statusMessage.emit(f"Move failed: {e}")
            return None
        op = MoveOp(item, item.abs_path, dest, idx, bucket.name)
        del self.items[idx]
        return op

    def _clamp_index(self):
        if self.index >= len(self.items):
            self.index = max(0, len(self.items) - 1)

    def move_current_to(self, bucket: Bucket):
        item = self.current()
        if item is None:
            return
        op = self._do_move(item, bucket)
        if op is None:
            return
        self.undo_stack.append(op)
        self.redo_stack.clear()
        self._clamp_index()
        self.listChanged.emit()
        self.currentChanged.emit(self.index)
        verb = "Rejected" if bucket.is_reject else f"→ {bucket.name}"
        self.statusMessage.emit(f"{verb}: {item.name}")

    def move_items_to(self, items: list[ImageItem], bucket: Bucket):
        """Bulk move (grid multi-select). Each file is an independent undo step."""
        moved = 0
        for item in list(items):
            op = self._do_move(item, bucket)
            if op is not None:
                self.undo_stack.append(op)
                moved += 1
        if moved:
            self.redo_stack.clear()
            self._clamp_index()
            self.listChanged.emit()
            self.currentChanged.emit(self.index)
            verb = "Rejected" if bucket.is_reject else f"→ {bucket.name}"
            self.statusMessage.emit(f"{verb} {moved} image(s)")

    def undo(self):
        if not self.undo_stack:
            self.statusMessage.emit("Nothing to undo")
            return
        op = self.undo_stack.pop()
        restore = _unique_dest(op.from_abs)
        try:
            restore.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(op.to_abs), str(restore))
        except OSError as e:
            self.statusMessage.emit(f"Undo failed: {e}")
            self.undo_stack.append(op)
            return
        op.item.abs_path = restore
        op.item._stat = None
        idx = min(op.list_index, len(self.items))
        self.items.insert(idx, op.item)
        self.index = idx
        self.redo_stack.append(op)
        self.listChanged.emit()
        self.currentChanged.emit(self.index)
        self.statusMessage.emit(f"Undo: restored {op.item.name}")

    def redo(self):
        if not self.redo_stack:
            self.statusMessage.emit("Nothing to redo")
            return
        op = self.redo_stack.pop()
        dest = _unique_dest(op.to_abs)
        try:
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(op.item.abs_path), str(dest))
        except OSError as e:
            self.statusMessage.emit(f"Redo failed: {e}")
            self.redo_stack.append(op)
            return
        # Update where the item lives; re-remove it from the working list.
        try:
            idx = self.items.index(op.item)
        except ValueError:
            idx = self.index
        op.item.abs_path = dest
        op.item._stat = None
        if 0 <= idx < len(self.items):
            del self.items[idx]
            self.index = min(idx, max(0, len(self.items) - 1))
        self.undo_stack.append(MoveOp(op.item, op.from_abs, dest, idx, op.bucket_name))
        self.listChanged.emit()
        self.currentChanged.emit(self.index)
        self.statusMessage.emit(f"Redo: {op.bucket_name} {op.item.name}")
