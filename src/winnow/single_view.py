"""Single-image view: zoom/pan (QGraphicsView), live brightness, and native
OS drag-out (left-drag drops a file:// URL into any app, like the GNOME viewer).
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
from PySide6.QtCore import QEvent, QMimeData, QPoint, Qt, QUrl, Signal
from PySide6.QtGui import QDrag, QImage, QPixmap
from PySide6.QtWidgets import (
    QGraphicsPixmapItem,
    QGraphicsScene,
    QGraphicsView,
)

MIN_SCALE = 0.02
MAX_SCALE = 40.0


def adjust_brightness(src: QImage, factor: float, gamma: float = 1.0) -> QImage:
    """Return a brightness/gamma-adjusted copy. factor 1.0 / gamma 1.0 = identity."""
    if abs(factor - 1.0) < 1e-3 and abs(gamma - 1.0) < 1e-3:
        return src
    img = src.convertToFormat(QImage.Format_RGBA8888)
    w, h = img.width(), img.height()
    buf = img.constBits()
    arr = np.frombuffer(buf, dtype=np.uint8).reshape(h, w, 4).astype(np.float32)
    rgb = arr[..., :3]
    if abs(gamma - 1.0) >= 1e-3:
        rgb = 255.0 * np.power(np.clip(rgb / 255.0, 0, 1), 1.0 / max(gamma, 1e-3))
    rgb = rgb * factor
    arr[..., :3] = np.clip(rgb, 0, 255)
    out = arr.astype(np.uint8)
    result = QImage(out.data, w, h, QImage.Format_RGBA8888).copy()
    return result


class SingleView(QGraphicsView):
    # emitted when the user zooms (so the status bar can show zoom %)
    zoomChanged = Signal(float)
    doubleClicked = Signal()
    contextRequested = Signal(QPoint)   # global position of a right-click

    def __init__(self, parent=None):
        super().__init__(parent)
        self._scene = QGraphicsScene(self)
        self.setScene(self._scene)
        self._item = QGraphicsPixmapItem()
        self._item.setTransformationMode(Qt.SmoothTransformation)
        self._scene.addItem(self._item)

        self.setRenderHints(self.renderHints())
        self.setDragMode(QGraphicsView.NoDrag)
        self.setTransformationAnchor(QGraphicsView.AnchorUnderMouse)
        self.setResizeAnchor(QGraphicsView.AnchorViewCenter)
        self.setAlignment(Qt.AlignCenter)
        self.setStyleSheet("background: #202020;")

        self._orig_image: QImage | None = None
        self._abs_path: Path | None = None
        self._brightness = 1.0
        self._gamma = 1.0
        self._fit = True          # auto-fit until the user manually zooms
        self._press_pos: QPoint | None = None
        self._pan_last: QPoint | None = None
        self._left_mode: str | None = None   # 'pan' | 'dragout', decided on drag
        self._left_ctrl: bool = False        # Ctrl held at left-press -> drag-out

    # ---- content ----------------------------------------------------
    def set_image(self, image: QImage | None, abs_path: Path | None, keep_view: bool = False):
        self._orig_image = image
        self._abs_path = abs_path
        if image is None or image.isNull():
            self._item.setPixmap(QPixmap())
            return
        self._render()
        if not keep_view or self._fit:
            self.fit()

    def clear(self):
        self._orig_image = None
        self._abs_path = None
        self._item.setPixmap(QPixmap())

    def _render(self):
        if self._orig_image is None:
            return
        adjusted = adjust_brightness(self._orig_image, self._brightness, self._gamma)
        pm = QPixmap.fromImage(adjusted)
        self._item.setPixmap(pm)
        self._scene.setSceneRect(pm.rect())

    # ---- brightness -------------------------------------------------
    def set_brightness(self, factor: float):
        self._brightness = max(0.05, min(factor, 5.0))
        self._render()

    def bump_brightness(self, delta: float):
        self.set_brightness(self._brightness + delta)
        return self._brightness

    def set_gamma(self, gamma: float):
        self._gamma = max(0.1, min(gamma, 5.0))
        self._render()

    def reset_adjustments(self):
        self._brightness = 1.0
        self._gamma = 1.0
        self._render()

    @property
    def brightness(self) -> float:
        return self._brightness

    # ---- zoom -------------------------------------------------------
    def current_scale(self) -> float:
        return self.transform().m11()

    def fit(self):
        if self._item.pixmap().isNull():
            return
        self.resetTransform()
        self.fitInView(self._item, Qt.KeepAspectRatio)
        self._fit = True
        self.zoomChanged.emit(self.current_scale())
        self._update_cursor()

    def actual_size(self):
        self.resetTransform()
        self._fit = False
        self.zoomChanged.emit(self.current_scale())
        self._update_cursor()

    def zoom_by(self, factor: float):
        scale = self.current_scale()
        new = max(MIN_SCALE, min(scale * factor, MAX_SCALE))
        factor = new / scale
        if abs(factor - 1.0) < 1e-4:
            return
        self.scale(factor, factor)
        self._fit = False
        self.zoomChanged.emit(self.current_scale())
        self._update_cursor()

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self._fit:
            self.fit()
        self._update_cursor()

    # Zoom sensitivity. Trackpads emit many small high-resolution deltas per
    # gesture, so we scale the zoom *proportionally* to the delta instead of a
    # fixed step (a fixed step made a light two-finger pinch explode).
    _PIXEL_ZOOM_RATE = 1.0015    # per pixel of high-res (trackpad) delta
    _NOTCH_ZOOM_RATE = 1.20      # per full 120-unit mouse-wheel notch

    def wheelEvent(self, event):
        # Scroll/pinch zooms, centered under the cursor (AnchorUnderMouse).
        pd = event.pixelDelta()
        ad = event.angleDelta()
        if not pd.isNull() and pd.y() != 0:
            factor = self._PIXEL_ZOOM_RATE ** pd.y()          # trackpad: gentle
        elif ad.y() != 0:
            factor = self._NOTCH_ZOOM_RATE ** (ad.y() / 120.0)  # mouse wheel
        else:
            event.ignore()
            return
        # cap any single event so one burst can't jump wildly
        factor = max(0.5, min(factor, 2.0))
        self.zoom_by(factor)
        event.accept()

    def viewportEvent(self, event):
        # Native trackpad pinch (where the platform delivers it as a gesture).
        if event.type() == QEvent.NativeGesture and event.gestureType() == Qt.ZoomNativeGesture:
            self.zoom_by(1.0 + event.value())
            return True
        return super().viewportEvent(event)

    # ---- pan / drag-out ---------------------------------------------
    # Context-aware left-drag:
    #   * zoomed in (pannable)  -> pan the image
    #   * fit / not zoomed      -> OS drag-out (copy file)
    #   * Ctrl + left-drag      -> always drag-out (escape hatch)
    # Middle-drag always pans. Cursor shows an open hand when panning is possible.
    def _can_pan(self) -> bool:
        h, v = self.horizontalScrollBar(), self.verticalScrollBar()
        return h.maximum() > h.minimum() or v.maximum() > v.minimum()

    def _update_cursor(self):
        if self._pan_last is not None:
            self.viewport().setCursor(Qt.ClosedHandCursor)
        elif self._can_pan():
            self.viewport().setCursor(Qt.OpenHandCursor)
        else:
            self.viewport().unsetCursor()

    def _pan_to(self, pos: QPoint):
        delta = pos - self._pan_last
        self._pan_last = pos
        h, v = self.horizontalScrollBar(), self.verticalScrollBar()
        h.setValue(h.value() - delta.x())
        v.setValue(v.value() - delta.y())

    def mousePressEvent(self, event):
        btn = event.button()
        if btn == Qt.MiddleButton:
            self._pan_last = event.position().toPoint()
            self.viewport().setCursor(Qt.ClosedHandCursor)
            event.accept()
            return
        if btn == Qt.LeftButton:
            self._press_pos = event.position().toPoint()
            self._left_mode = None
            self._left_ctrl = bool(event.modifiers() & Qt.ControlModifier)
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event):
        # active middle-button pan
        if self._pan_last is not None and event.buttons() & Qt.MiddleButton:
            self._pan_to(event.position().toPoint())
            event.accept()
            return
        # left-button drag: decide pan vs drag-out on first real movement
        if self._press_pos is not None and event.buttons() & Qt.LeftButton:
            moved = (event.position().toPoint() - self._press_pos).manhattanLength() > 8
            if self._left_mode is None and moved:
                if not self._left_ctrl and self._can_pan():
                    self._left_mode = "pan"
                    self._pan_last = event.position().toPoint()
                    self.viewport().setCursor(Qt.ClosedHandCursor)
                else:
                    self._left_mode = "dragout"
            if self._left_mode == "pan":
                self._pan_to(event.position().toPoint())
                event.accept()
                return
            if self._left_mode == "dragout" and self._abs_path is not None:
                self._press_pos = None
                self._left_mode = None
                self._start_drag_out()
                return
        super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MiddleButton:
            self._pan_last = None
            self._update_cursor()
            event.accept()
            return
        if event.button() == Qt.LeftButton:
            was_pan = self._left_mode == "pan"
            self._press_pos = None
            self._left_mode = None
            if was_pan:
                self._pan_last = None
                self._update_cursor()
                event.accept()
                return
        super().mouseReleaseEvent(event)

    def mouseDoubleClickEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.doubleClicked.emit()
            event.accept()
            return
        super().mouseDoubleClickEvent(event)

    def contextMenuEvent(self, event):
        # Scroll-area context-menu events land here; re-emit for MainWindow.
        self.contextRequested.emit(event.globalPos())
        event.accept()

    def _start_drag_out(self):
        if self._abs_path is None:
            return
        drag = QDrag(self)
        mime = QMimeData()
        mime.setUrls([QUrl.fromLocalFile(str(self._abs_path))])
        drag.setMimeData(mime)
        pm = self._item.pixmap()
        if not pm.isNull():
            thumb = pm.scaled(160, 160, Qt.KeepAspectRatio, Qt.SmoothTransformation)
            drag.setPixmap(thumb)
            drag.setHotSpot(thumb.rect().center())
        drag.exec(Qt.CopyAction)
