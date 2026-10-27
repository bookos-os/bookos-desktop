"""Clientes reales para BOOKOS_SELFTEST_MULTITAREA, con Wayland o X11 de Qt."""

from PySide6.QtCore import QTimer, Qt
from PySide6.QtWidgets import QApplication, QDialog, QLabel, QPushButton, QTextEdit, QVBoxLayout, QWidget

app = QApplication([])
app.setApplicationName("Prueba de multitarea")
ventanas = []
for titulo, contenido in [
    ("Editor de prueba", "Un documento y su diálogo deben volver juntos al cambiar de aplicación."),
    ("Referencia de prueba", "Arrastra el separador para ajustar los dos paneles."),
    ("Otra aplicación de prueba", "Esta ventana conserva su propia entrada en Alt+Tab."),
]:
    ventana = QWidget()
    ventana.setWindowTitle(titulo)
    ventana.setMinimumSize(240, 180)
    ventana.resize(500, 360)
    disposicion = QVBoxLayout(ventana)
    disposicion.addWidget(QLabel(titulo))
    texto = QTextEdit(contenido)
    disposicion.addWidget(texto)
    ventana.show()
    ventanas.append(ventana)

dialogo = QDialog(ventanas[0])
dialogo.setWindowTitle("Párrafo de prueba")
dialogo.setWindowModality(Qt.WindowModality.WindowModal)
disposicion = QVBoxLayout(dialogo)
disposicion.addWidget(QLabel("Diálogo hijo del editor"))
boton = QPushButton("Aceptar")
boton.clicked.connect(dialogo.accept)
disposicion.addWidget(boton)
dialogo.resize(280, 140)
QTimer.singleShot(600, dialogo.show)
QTimer.singleShot(60000, app.quit)
app.exec()
