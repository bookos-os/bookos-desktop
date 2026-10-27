"""Regenera el atlas integrado con Pango/Cairo y Noto Color Emoji del sistema.

Es una herramienta de mantenimiento; el shell usa el PNG y no requiere Pango.
El orden es el del catálogo, por lo que debe regenerarse al cambiar ese archivo.
"""

from pathlib import Path
import math
import re

import cairo
import gi

gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import Pango, PangoCairo

raiz = Path(__file__).resolve().parents[1]
catalogo = raiz / "src/emergente/emojis/catalogo.rs"
simbolos = re.findall(r'simbolo: "([^"]+)"', catalogo.read_text())
columnas, celda = 18, 64
superficie = cairo.ImageSurface(
    cairo.FORMAT_ARGB32, columnas * celda, math.ceil(len(simbolos) / columnas) * celda
)
contexto = cairo.Context(superficie)
fuente = Pango.FontDescription("Noto Color Emoji")
fuente.set_absolute_size(48 * Pango.SCALE)
for indice, simbolo in enumerate(simbolos):
    disposicion = PangoCairo.create_layout(contexto)
    disposicion.set_font_description(fuente)
    disposicion.set_text(simbolo, -1)
    tinta, _ = disposicion.get_pixel_extents()
    contexto.move_to(
        (indice % columnas) * celda + (celda - tinta.width) / 2 - tinta.x,
        (indice // columnas) * celda + (celda - tinta.height) / 2 - tinta.y,
    )
    PangoCairo.show_layout(contexto, disposicion)
destino = raiz / "assets/emojis/noto.png"
superficie.write_to_png(str(destino))
print(f"{len(simbolos)} emojis escritos en {destino}")
