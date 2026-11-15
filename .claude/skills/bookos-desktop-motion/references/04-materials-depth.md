# 4 · Native Materials & Visual Depth

Cristal, translucidez, sombras, elevación y profundidad, sin salir de la
identidad del HIG (§6 sombras, §7 transparencia y blur, §4 radios).

## Cómo se hace hoy

| Material | Dónde | Coste conocido |
|---|---|---|
| Cristal esmerilado (panel, dock, emergentes) | `desenfoque.rs`, GPU | lee el framebuffer del compositor; el shell no ve lo de debajo |
| Fondo de tarjeta | `tema::card()` | opaco o compuesto |
| Relleno translúcido en tiny-skia | shell | **17× más caro** que opaco (nota `tiny-skia-translucido-caro`) |
| `Shadow` de `container` en iced_tiny_skia | shell | SDF por píxel de toda la caja; **75 %** del coste en el diálogo de color (45 → 11 ms sin ella) |
| Sombra rasterizada en tiras | `emergente/color_carpeta.rs::sombra_rasterizada` | idéntica a escala 2, coste fijo |
| Bloqueo | `bloqueo.rs` | blanco sobre `FONDO_CAMPO` fijo, no sigue el tema |

## Reglas

1. **El cristal lo hace el compositor**, no el shell. El shell pinta contenido
   sobre transparente y el compositor pone el desenfoque detrás.
2. **Colores compuestos, no alfa**, en el shell: mezclar el tinte con `card()`
   y pintar opaco cuando el fondo debajo es conocido. Alfa real solo donde
   tiene que verse el escritorio, y entonces lo resuelve el compositor.
3. **Sombras**: nunca `Shadow` de iced en superficies grandes o animadas. Usar
   la sombra rasterizada (tiras cacheadas) o una sombra del compositor.
   Niveles del HIG §6; no inventar otros.
4. **Radios concéntricos**: radio interior = radio exterior − margen. Menú
   (18, margen 5) → realce 13 (`R_ITEM_POPOVER`). Muestras: radio ≤ alto de la barra.
5. **Profundidad por capas**, no por decoración: ventana < panel/dock <
   emergente < modal < bloqueo. Cada nivel sube en sombra y en desenfoque,
   nunca solo en uno.
6. **Contraste sobre cristal**: texto principal ≥ 4,5:1 contra el peor caso
   del fondo desenfocado (blanco y negro puros detrás).
7. **Tema claro y oscuro** con el mismo material: el cristal cambia de tinte,
   no de estructura. El bloqueo es la excepción documentada.
8. **Alto contraste** (`aplicar_alto_contraste`): el cristal pasa a opaco.

## Criterios medibles

- Coste de pintado de una emergente nueva medido con un ejemplo en
  `crates/bookos-shell/examples/` (a ráfagas, ver 05) y ≤ 4 ms en debug a
  escala 2. Si supera el tope, quitar alfa y sombras de iced antes de nada.
- 0 usos nuevos de `Shadow` de iced en superficies > 300×300 lógicos.
- Contraste ≥ 4,5:1 texto, ≥ 3:1 iconos y bordes funcionales, en claro, oscuro
  y alto contraste.
- Radios: cada realce interior cumple `r_int = r_ext − margen` ± 1 px.

## Revisión

1. Ejemplo PNG (`menu_logo`, `bloqueo`, `escritorio`, `tarjetas_png`…) en
   claro y oscuro, sobre fondo claro y oscuro.
2. Ampliar las esquinas al 400 %: el realce no asoma ni deja hueco.
3. `grep -n "shadow\|Shadow" <archivo>` en lo tocado.
4. Medir antes y después con el ejemplo correspondiente.
