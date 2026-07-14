"""Contact-sheet grid view for fast bulk culling. Thumbnails load lazily as
cells become visible, so it scales to thousands of images."""

from __future__ import annotations

from PySide6.QtCore import (
    QAbstractListModel,
    QMimeData,
    QModelIndex,
    QPoint,
    QSize,
    Qt,
    QUrl,
    Signal,
)
from PySide6.QtGui import QColor, QDrag, QIcon, QImage, QPixmap
from PySide6.QtWidgets import QListView

THUMB = 200


class GridModel(QAbstractListModel):
    def __init__(self, session, loader, parent=None):
        super().__init__(parent)
        self.session = session
        self.loader = loader
        self._cache: dict[str, QPixmap] = {}
        self._pending: set[str] = set()
        self._placeholder = self._make_placeholder()
        loader.thumbReady.connect(self._on_thumb)

    def _make_placeholder(self) -> QIcon:
        pm = QPixmap(THUMB, THUMB)
        pm.fill(QColor(45, 45, 45))
        return QIcon(pm)

    def rowCount(self, parent=QModelIndex()):
        return 0 if parent.isValid() else self.session.count()

    def data(self, index, role=Qt.DisplayRole):
        if not index.isValid():
            return None
        row = index.row()
        if row >= self.session.count():
            return None
        item = self.session.items[row]
        if role == Qt.DisplayRole:
            return item.name
        if role == Qt.ToolTipRole:
            meta = self.session.metadata.get(item.rel_path)
            extra = "\n".join(f"{k}: {v}" for k, v in meta.items())
            return item.rel_path + ("\n" + extra if extra else "")
        if role == Qt.DecorationRole:
            pm = self._cache.get(item.rel_path)
            if pm is not None:
                return QIcon(pm)
            self._request(item)
            return self._placeholder
        return None

    def _request(self, item):
        if item.rel_path in self._pending:
            return
        self._pending.add(item.rel_path)
        self.loader.request_thumb(
            item.rel_path, item.abs_path, item.mtime(), item.size_bytes()
        )

    def _on_thumb(self, rel: str, image: QImage, generation: int):
        self._pending.discard(rel)
        self._cache[rel] = QPixmap.fromImage(image)
        # Find the row (list may have been re-sorted) and refresh it.
        for row, item in enumerate(self.session.items):
            if item.rel_path == rel:
                idx = self.index(row)
                self.dataChanged.emit(idx, idx, [Qt.DecorationRole])
                break

    def refresh(self):
        self.beginResetModel()
        self.endResetModel()


class GridView(QListView):
    activatedItem = Signal(int)      # row double-clicked / entered
    contextRequested = Signal(QPoint)  # global position of a right-click

    def __init__(self, session, loader, parent=None):
        super().__init__(parent)
        self.session = session
        self.setModel(GridModel(session, loader, self))
        self.setViewMode(QListView.IconMode)
        self.setResizeMode(QListView.Adjust)
        self.setMovement(QListView.Static)
        self.setUniformItemSizes(True)
        self.setIconSize(QSize(THUMB, THUMB))
        self.setGridSize(QSize(THUMB + 24, THUMB + 40))
        self.setSpacing(6)
        self.setSelectionMode(QListView.ExtendedSelection)
        self.setWordWrap(True)
        self.setDragEnabled(True)
        self.setStyleSheet("QListView{background:#252525;color:#ddd;} ")
        self.doubleClicked.connect(lambda idx: self.activatedItem.emit(idx.row()))

    def model(self) -> GridModel:
        return super().model()

    def selected_rows(self) -> list[int]:
        return sorted(i.row() for i in self.selectionModel().selectedIndexes())

    def refresh(self):
        self.model().refresh()

    def select_row(self, row: int):
        if 0 <= row < self.session.count():
            idx = self.model().index(row)
            self.setCurrentIndex(idx)
            self.scrollTo(idx)

    def contextMenuEvent(self, event):
        self.contextRequested.emit(event.globalPos())
        event.accept()

    def startDrag(self, supported_actions):
        rows = self.selected_rows()
        if not rows:
            return
        urls = [QUrl.fromLocalFile(str(self.session.items[r].abs_path)) for r in rows]
        mime = QMimeData()
        mime.setUrls(urls)
        drag = QDrag(self)
        drag.setMimeData(mime)
        first = self.model().data(self.model().index(rows[0]), Qt.DecorationRole)
        if isinstance(first, QIcon):
            drag.setPixmap(first.pixmap(THUMB, THUMB))
        drag.exec(Qt.CopyAction)
