"""QApplication setup, dark theme, and window lifecycle."""

from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QPalette
from PySide6.QtWidgets import QApplication

from .model import Session

# Keep strong references so windows aren't garbage-collected.
_windows: list = []


def _dark_palette() -> QPalette:
    p = QPalette()
    p.setColor(QPalette.Window, QColor(37, 37, 37))
    p.setColor(QPalette.WindowText, QColor(220, 220, 220))
    p.setColor(QPalette.Base, QColor(30, 30, 30))
    p.setColor(QPalette.AlternateBase, QColor(45, 45, 45))
    p.setColor(QPalette.Text, QColor(220, 220, 220))
    p.setColor(QPalette.Button, QColor(50, 50, 50))
    p.setColor(QPalette.ButtonText, QColor(220, 220, 220))
    p.setColor(QPalette.Highlight, QColor(90, 140, 220))
    p.setColor(QPalette.HighlightedText, QColor(255, 255, 255))
    p.setColor(QPalette.ToolTipBase, QColor(45, 45, 45))
    p.setColor(QPalette.ToolTipText, QColor(230, 230, 230))
    return p


def create_app(argv=None) -> QApplication:
    app = QApplication.instance()
    if app is None:
        app = QApplication(argv or [])
    app.setApplicationName("winnow")
    app.setStyle("Fusion")
    app.setPalette(_dark_palette())
    return app


def launch_window(session: Session, replace=None):
    from .main_window import MainWindow

    win = MainWindow(session)
    _windows.append(win)
    win.show()
    if replace is not None:
        try:
            _windows.remove(replace)
        except ValueError:
            pass
        replace.close()
        replace.deleteLater()
    return win
