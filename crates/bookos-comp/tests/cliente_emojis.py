"""Editor real para probar el selector con Qt sobre Wayland y XWayland."""

import os
from pathlib import Path

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QApplication, QPlainTextEdit

app = QApplication([])
app.setApplicationName("Prueba de emojis")
editor = QPlainTextEdit()
editor.setWindowTitle("Prueba de inserción de emojis")
editor.resize(700, 520)
editor.setPlainText("inicio: ")
cursor = editor.textCursor()
cursor.movePosition(cursor.MoveOperation.End)
editor.setTextCursor(cursor)
resultado = Path(os.environ["BOOKOS_EMOJIS_RESULTADO"])

def guardar():
    resultado.write_text(editor.toPlainText())

editor.textChanged.connect(guardar)
guardar()
editor.show()
editor.setFocus()
QTimer.singleShot(45000, app.quit)
app.exec()
