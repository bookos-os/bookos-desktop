#!/usr/bin/env python3
"""Genera los PNG del tema Plymouth de BookOS a partir del SVG oficial."""

from pathlib import Path
import math

from PySide6.QtCore import Qt, QRectF
from PySide6.QtGui import QColor, QImage, QPainter, QPen
from PySide6.QtSvg import QSvgRenderer

AQUI = Path(__file__).resolve().parent
RAIZ = AQUI.parent.parent
SALIDA = AQUI / "bookos"
SVG = RAIZ / "crates/bookos-shell/assets/bookos.svg"

LOGO_LADO = 128
SPINNER_LADO = 36
FOTOGRAMAS = 36
ASPAS = 12


def generar_logo() -> None:
    renderer = QSvgRenderer(str(SVG))
    if not renderer.isValid():
        raise RuntimeError(f"No se pudo cargar {SVG}")
    img = QImage(LOGO_LADO, LOGO_LADO, QImage.Format_RGBA8888)
    img.fill(Qt.transparent)
    painter = QPainter(img)
    painter.setRenderHint(QPainter.Antialiasing, True)
    painter.setRenderHint(QPainter.SmoothPixmapTransform, True)
    renderer.render(painter, QRectF(0, 0, LOGO_LADO, LOGO_LADO))
    painter.end()
    img.save(str(SALIDA / "watermark.png"))


def generar_spinner() -> None:
    centro = SPINNER_LADO / 2.0
    radio_int = 7.2
    radio_ext = 13.5
    grosor = 2.4

    for idx in range(FOTOGRAMAS):
        img = QImage(SPINNER_LADO, SPINNER_LADO, QImage.Format_RGBA8888)
        img.fill(Qt.transparent)
        painter = QPainter(img)
        painter.setRenderHint(QPainter.Antialiasing, True)

        fase = (idx / FOTOGRAMAS) * ASPAS
        for aspa in range(ASPAS):
            angulo = (aspa / ASPAS) * math.tau - math.pi / 2.0
            distancia = (aspa - fase) % ASPAS
            # Cola suave de 7 aspas, con acento sutil en la cabeza.
            peso = max(0.0, 1.0 - (ASPAS - 1 - distancia) / 8.5)
            alfa = int(round((0.18 + 0.78 * (peso ** 1.6)) * 255))
            r = int(round(164 + (244 - 164) * peso))
            g = int(round(187 + (247 - 187) * peso))
            b = 255
            pen = QPen(QColor(r, g, b, alfa), grosor, Qt.SolidLine, Qt.RoundCap)
            painter.setPen(pen)
            cos_a = math.cos(angulo)
            sin_a = math.sin(angulo)
            painter.drawLine(
                round(centro + cos_a * radio_int, 2),
                round(centro + sin_a * radio_int, 2),
                round(centro + cos_a * radio_ext, 2),
                round(centro + sin_a * radio_ext, 2),
            )
        painter.end()
        sufijo = f"{idx + 1:04d}.png"
        img.save(str(SALIDA / f"throbber-{sufijo}"))
        img.save(str(SALIDA / f"animation-{sufijo}"))


def main() -> None:
    SALIDA.mkdir(parents=True, exist_ok=True)
    generar_logo()
    generar_spinner()


if __name__ == "__main__":
    main()
