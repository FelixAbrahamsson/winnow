"""Main window: coordinates the single/grid views, info panel, status bar,
keyboard shortcuts, and full-image loading with a small prefetch cache."""

from __future__ import annotations

from collections import OrderedDict
from pathlib import Path

from PySide6.QtCore import QEvent, QMimeData, Qt, QUrl
from PySide6.QtGui import QGuiApplication, QImage, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QApplication,
    QCheckBox,
    QComboBox,
    QDialog,
    QDockWidget,
    QFileDialog,
    QFormLayout,
    QFrame,
    QHBoxLayout,
    QLabel,
    QMainWindow,
    QMenu,
    QPushButton,
    QScrollArea,
    QStackedWidget,
    QTextBrowser,
    QVBoxLayout,
    QWidget,
)

from . import details as det
from .buckets import Bucket
from .grid_view import GridView
from .loader import Loader
from .model import Session
from .single_view import SingleView

FULL_CACHE_CAP = 7


class InfoPanel(QWidget):
    def __init__(self, session: Session, parent=None):
        super().__init__(parent)
        self.session = session
        outer = QVBoxLayout(self)
        outer.setContentsMargins(8, 8, 8, 8)

        # --- sort controls ---
        sort_row = QHBoxLayout()
        sort_row.addWidget(QLabel("Sort:"))
        self.sort_combo = QComboBox()
        for key, label in session.sortable_keys():
            self.sort_combo.addItem(label, key)
        self.desc_check = QCheckBox("desc")
        sort_row.addWidget(self.sort_combo, 1)
        sort_row.addWidget(self.desc_check)
        outer.addLayout(sort_row)

        line = QFrame()
        line.setFrameShape(QFrame.HLine)
        outer.addWidget(line)

        # --- scrollable detail + metadata forms ---
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        body = QWidget()
        self.body_layout = QVBoxLayout(body)
        self.body_layout.setAlignment(Qt.AlignTop)

        self.details_form = QFormLayout()
        self.body_layout.addWidget(self._section("Image"))
        self.body_layout.addLayout(self.details_form)

        self.meta_header = self._section("Metadata")
        self.meta_form = QFormLayout()
        self.body_layout.addWidget(self.meta_header)
        self.body_layout.addLayout(self.meta_form)
        self.meta_header.setVisible(bool(session.metadata))

        scroll.setWidget(body)
        outer.addWidget(scroll, 1)

    def _section(self, text: str) -> QLabel:
        lbl = QLabel(text)
        lbl.setStyleSheet("font-weight:600; margin-top:6px; color:#8ab4f8;")
        return lbl

    @staticmethod
    def _clear(form: QFormLayout):
        while form.rowCount():
            form.removeRow(0)

    def update_for(self, item):
        self._clear(self.details_form)
        self._clear(self.meta_form)
        if item is None:
            return
        for label, value in det.details_for(item):
            v = QLabel(value)
            v.setWordWrap(True)
            v.setTextInteractionFlags(Qt.TextSelectableByMouse)
            self.details_form.addRow(QLabel(label + ":"), v)
        meta = self.session.metadata.get(item.rel_path)
        self.meta_header.setVisible(bool(self.session.metadata))
        for k, val in meta.items():
            v = QLabel(str(val))
            v.setWordWrap(True)
            v.setTextInteractionFlags(Qt.TextSelectableByMouse)
            self.meta_form.addRow(QLabel(k + ":"), v)


class MainWindow(QMainWindow):
    def __init__(self, session: Session):
        super().__init__()
        self.session = session
        self.loader = Loader(thumb_size=256)
        self._gen = 0
        self._full_cache: "OrderedDict[str, QImage]" = OrderedDict()
        self._syncing = False

        self.setWindowTitle(f"winnow — {session.root}")
        self.resize(1200, 800)

        # central stacked: single (0) / grid (1)
        self.single = SingleView()
        self.grid = GridView(session, self.loader)
        self.stack = QStackedWidget()
        self.stack.addWidget(self.single)
        self.stack.addWidget(self.grid)
        self.setCentralWidget(self.stack)

        # info dock
        self.info = InfoPanel(session)
        dock = QDockWidget("Details", self)
        dock.setWidget(self.info)
        dock.setFeatures(QDockWidget.DockWidgetMovable | QDockWidget.DockWidgetClosable)
        self.addDockWidget(Qt.RightDockWidgetArea, dock)
        self.info_dock = dock

        # status bar labels
        self.lbl_counter = QLabel("0/0")
        self.lbl_name = QLabel("")
        self.lbl_zoom = QLabel("")
        self.lbl_bright = QLabel("")
        self.lbl_mode = QLabel("single")
        sb = self.statusBar()
        sb.addWidget(self.lbl_counter)
        sb.addWidget(self.lbl_name, 1)
        sb.addPermanentWidget(self.lbl_bright)
        sb.addPermanentWidget(self.lbl_zoom)
        sb.addPermanentWidget(self.lbl_mode)

        # right-click context menu on both views (they emit a global position)
        self.single.contextRequested.connect(self._show_context_menu)
        self.grid.contextRequested.connect(self._show_context_menu)

        # mouse side buttons (Back/Forward) -> prev/next, everywhere
        QApplication.instance().installEventFilter(self)

        self._wire_signals()
        self._register_shortcuts()
        self._show_current()

    # ---- signal wiring ---------------------------------------------
    def _wire_signals(self):
        self.session.currentChanged.connect(self._on_current_changed)
        self.session.listChanged.connect(self._on_list_changed)
        self.session.statusMessage.connect(lambda m: self.statusBar().showMessage(m, 4000))
        self.loader.fullReady.connect(self._on_full_ready)
        self.single.zoomChanged.connect(self._on_zoom_changed)
        self.single.doubleClicked.connect(self._toggle_fullscreen)
        self.grid.activatedItem.connect(self._open_in_single)
        self.grid.selectionModel().currentChanged.connect(self._on_grid_current)
        self.info.sort_combo.currentIndexChanged.connect(self._apply_sort)
        self.info.desc_check.toggled.connect(self._apply_sort)

    def eventFilter(self, obj, event):
        if event.type() == QEvent.MouseButtonPress:
            btn = event.button()
            if btn == Qt.BackButton:
                self.session.prev()
                return True
            if btn == Qt.ForwardButton:
                self.session.next()
                return True
        return super().eventFilter(obj, event)

    # ---- shortcuts --------------------------------------------------
    def _sc(self, seq: str, slot):
        s = QShortcut(QKeySequence(seq), self)
        s.setContext(Qt.ApplicationShortcut)
        s.activated.connect(slot)
        return s

    def _register_shortcuts(self):
        self._sc("Right", self.session.next)
        self._sc("Space", self.session.next)
        self._sc("Left", self.session.prev)
        self._sc("PgDown", lambda: self.session.jump(10))
        self._sc("PgUp", lambda: self.session.jump(-10))
        self._sc("Home", lambda: self.session.set_index(0))
        self._sc("End", lambda: self.session.set_index(self.session.count() - 1))

        self._sc("+", lambda: self.single.zoom_by(1.25))
        self._sc("=", lambda: self.single.zoom_by(1.25))
        self._sc("-", lambda: self.single.zoom_by(1 / 1.25))
        self._sc("F", self.single.fit)
        self._sc("A", self.single.actual_size)

        self._sc("Ctrl+Z", self.session.undo)
        self._sc("Ctrl+Shift+Z", self.session.redo)
        self._sc("Ctrl+Y", self.session.redo)

        self._sc("Ctrl+C", self._copy_name)
        self._sc("Ctrl+Shift+C", self._copy_path)
        self._sc("Ctrl+Shift+X", self._copy_file)

        self._sc("G", self._toggle_grid)
        self._sc("I", self._toggle_info)
        self._sc("F11", self._toggle_fullscreen)
        self._sc("Ctrl+O", self._open_folder)
        self._sc("?", self._show_help)
        self._sc("F1", self._show_help)

        self._sc("]", lambda: self._bump_brightness(0.1))
        self._sc("[", lambda: self._bump_brightness(-0.1))
        self._sc("\\", self._reset_brightness)
        self._sc("}", lambda: self._bump_gamma(0.1))
        self._sc("{", lambda: self._bump_gamma(-0.1))

        # bucket hotkeys
        used = set()
        for bucket in self.session.buckets:
            self._sc(bucket.key, self._make_bucket_slot(bucket))
            used.add(bucket.key.lower())
        # convenient reject aliases
        reject = self.session.buckets[0]
        for alias in ("Backspace", "X"):
            if alias.lower() not in used:
                self._sc(alias, self._make_bucket_slot(reject))

    def _make_bucket_slot(self, bucket: Bucket):
        def slot():
            self._move_to_bucket(bucket)
        return slot

    # ---- actions ----------------------------------------------------
    def _move_to_bucket(self, bucket: Bucket):
        if self.stack.currentWidget() is self.grid:
            rows = self.grid.selected_rows()
            if rows:
                items = [self.session.items[r] for r in rows]
                self.session.move_items_to(items, bucket)
                return
        self.session.move_current_to(bucket)

    def _copy_name(self):
        item = self.session.current()
        if item:
            QGuiApplication.clipboard().setText(item.name)
            self.statusBar().showMessage(f"Copied filename: {item.name}", 3000)

    def _copy_path(self):
        item = self.session.current()
        if item:
            QGuiApplication.clipboard().setText(str(item.abs_path))
            self.statusBar().showMessage(f"Copied path: {item.abs_path}", 3000)

    def _copy_file(self):
        """Put the image file on the clipboard so it can be pasted into a file
        manager (no dragging needed)."""
        item = self.session.current()
        if not item:
            return
        url = QUrl.fromLocalFile(str(item.abs_path))
        mime = QMimeData()
        mime.setUrls([url])
        mime.setText(str(item.abs_path))
        # GNOME Files / Nautilus paste needs this specific format.
        mime.setData("x-special/gnome-copied-files", b"copy\n" + url.toEncoded())
        QGuiApplication.clipboard().setMimeData(mime)
        self.statusBar().showMessage(f"Copied file (paste into a file manager): {item.name}", 4000)

    def _bump_brightness(self, delta: float):
        val = self.single.bump_brightness(delta)
        self.lbl_bright.setText(f"☀ {val*100:.0f}%")

    def _reset_brightness(self):
        self.single.reset_adjustments()
        self.lbl_bright.setText("☀ 100%")

    def _bump_gamma(self, delta: float):
        self.single.set_gamma(self.single._gamma + delta)

    def _toggle_grid(self):
        if self.stack.currentWidget() is self.single:
            self.stack.setCurrentWidget(self.grid)
            self.lbl_mode.setText("grid")
            self.grid.select_row(self.session.index)
            self.grid.setFocus()
        else:
            self._open_in_single(self.session.index)

    def _open_in_single(self, row: int):
        if row is not None and row >= 0:
            self.session.set_index(row)
        self.stack.setCurrentWidget(self.single)
        self.lbl_mode.setText("single")
        self.single.setFocus()

    def _toggle_info(self):
        self.info_dock.setVisible(not self.info_dock.isVisible())

    # ---- context menu ----------------------------------------------
    def _show_context_menu(self, global_pos):
        menu = QMenu(self)
        menu.addAction("Next  →", self.session.next)
        menu.addAction("Previous  ←", self.session.prev)
        menu.addSeparator()
        for b in self.session.buckets:
            label = f"Reject  ({b.key})" if b.is_reject else f"Move to “{b.name}”  ({b.key})"
            menu.addAction(label, self._make_bucket_slot(b))
        menu.addAction("Undo move  (Ctrl+Z)", self.session.undo)
        menu.addSeparator()
        menu.addAction("Fit to window  (F)", self.single.fit)
        menu.addAction("Actual size 100%  (A)", self.single.actual_size)
        menu.addAction("Toggle fullscreen  (F11)", self._toggle_fullscreen)
        menu.addAction("Toggle grid / single  (G)", self._toggle_grid)
        menu.addSeparator()
        menu.addAction("Copy filename  (Ctrl+C)", self._copy_name)
        menu.addAction("Copy full path  (Ctrl+Shift+C)", self._copy_path)
        menu.addAction("Copy image file — paste into file manager  (Ctrl+Shift+X)", self._copy_file)
        menu.addSeparator()
        menu.addAction("Keyboard shortcuts…  (?)", self._show_help)
        menu.addAction("Open folder…  (Ctrl+O)", self._open_folder)
        menu.exec(global_pos)

    # ---- help -------------------------------------------------------
    def _help_sections(self):
        bucket_rows = []
        for b in self.session.buckets:
            if b.is_reject:
                bucket_rows.append((f"{b.key} / Backspace / X", "Reject → move to _rejected/"))
            else:
                bucket_rows.append((b.key, f"Move to “{b.name}” ({b.folder}/)"))
        return [
            ("Navigation", [
                ("→ / Space", "Next image"),
                ("←", "Previous image"),
                ("Mouse Back / Forward", "Previous / next image"),
                ("Page Down / Page Up", "Jump ±10"),
                ("Home / End", "First / last image"),
            ]),
            ("Sort into buckets", bucket_rows + [
                ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo the last move"),
            ]),
            ("Zoom & pan", [
                ("Scroll wheel / pinch", "Zoom in / out (toward cursor)"),
                ("+ / -", "Zoom in / out"),
                ("F", "Fit to window"),
                ("A", "Actual size (100%)"),
                ("Left-drag (when zoomed)", "Pan the image"),
                ("Middle-drag", "Pan the image"),
                ("Double-click", "Toggle fullscreen"),
            ]),
            ("Image adjust", [
                ("] / [", "Brightness up / down"),
                ("\\", "Reset brightness & gamma"),
                ("} / {", "Gamma up / down"),
            ]),
            ("Files & views", [
                ("Left-drag (when fit)", "Drag file out to another app (copy)"),
                ("Ctrl + left-drag", "Drag file out (works even when zoomed)"),
                ("Ctrl+Shift+X", "Copy image file — paste into a file manager"),
                ("Ctrl+C / Ctrl+Shift+C", "Copy filename / full path"),
                ("G", "Toggle grid ↔ single view"),
                ("I", "Toggle info panel"),
                ("F11", "Fullscreen"),
                ("Ctrl+O", "Open another folder"),
                ("? / F1", "This shortcuts list"),
            ]),
        ]

    def _build_help_html(self) -> str:
        parts = [
            "<style>"
            "h3{color:#8ab4f8;margin:14px 0 4px;} "
            "td{padding:2px 14px 2px 0;vertical-align:top;} "
            "kbd{background:#333;border:1px solid #555;border-radius:4px;"
            "padding:1px 6px;font-family:monospace;color:#eee;}"
            "</style>"
        ]
        for title, rows in self._help_sections():
            parts.append(f"<h3>{title}</h3><table>")
            for keys, desc in rows:
                parts.append(f"<tr><td><kbd>{keys}</kbd></td><td>{desc}</td></tr>")
            parts.append("</table>")
        return "".join(parts)

    def _show_help(self):
        dlg = QDialog(self)
        dlg.setWindowTitle("winnow — shortcuts")
        dlg.resize(560, 680)
        lay = QVBoxLayout(dlg)
        browser = QTextBrowser()
        browser.setHtml(self._build_help_html())
        lay.addWidget(browser)
        btn = QPushButton("Close")
        btn.clicked.connect(dlg.accept)
        lay.addWidget(btn, alignment=Qt.AlignRight)
        dlg.exec()

    def _toggle_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
        else:
            self.showFullScreen()

    def _open_folder(self):
        folder = QFileDialog.getExistingDirectory(self, "Open image folder", str(self.session.root))
        if folder:
            self._reopen(Path(folder))

    def _reopen(self, folder: Path):
        new = Session(folder, recursive=self.session.recursive)
        self.session = new
        # simplest robust path: rebuild the window
        from .app import launch_window
        launch_window(new, replace=self)

    # ---- sort -------------------------------------------------------
    def _apply_sort(self):
        key = self.info.sort_combo.currentData()
        if key:
            self.session.apply_sort(key, self.info.desc_check.isChecked())

    # ---- current image display -------------------------------------
    def _on_current_changed(self, index: int):
        self._show_current()
        if not self._syncing and self.stack.currentWidget() is self.grid:
            self._syncing = True
            self.grid.select_row(index)
            self._syncing = False

    def _on_grid_current(self, current, previous):
        if self._syncing or not current.isValid():
            return
        self._syncing = True
        self.session.set_index(current.row())
        self._syncing = False

    def _on_list_changed(self):
        self.grid.refresh()

    def _show_current(self):
        item = self.session.current()
        n = self.session.count()
        if item is None:
            self.single.clear()
            self.lbl_counter.setText(f"0/{n}")
            self.lbl_name.setText("— empty —")
            self.info.update_for(None)
            return
        self.lbl_counter.setText(f"{self.session.index + 1}/{n}")
        self.lbl_name.setText(item.rel_path)
        self.info.update_for(item)

        self._gen += 1
        gen = self._gen
        cached = self._full_cache.get(item.rel_path)
        if cached is not None:
            self._full_cache.move_to_end(item.rel_path)
            self.single.set_image(cached, item.abs_path)
        else:
            self.loader.request_full(item.rel_path, item.abs_path, gen)
        self._prefetch()

    def _prefetch(self):
        for off in (1, -1, 2):
            j = self.session.index + off
            if 0 <= j < self.session.count():
                it = self.session.items[j]
                if it.rel_path not in self._full_cache:
                    self.loader.request_full(it.rel_path, it.abs_path, self._gen)

    def _on_full_ready(self, rel: str, image: QImage, generation: int):
        # cache regardless (prefetch), trim LRU
        self._full_cache[rel] = image
        self._full_cache.move_to_end(rel)
        while len(self._full_cache) > FULL_CACHE_CAP:
            self._full_cache.popitem(last=False)
        cur = self.session.current()
        if cur is not None and cur.rel_path == rel:
            self.single.set_image(image, cur.abs_path)

    def _on_zoom_changed(self, scale: float):
        self.lbl_zoom.setText(f"{scale * 100:.0f}%")
