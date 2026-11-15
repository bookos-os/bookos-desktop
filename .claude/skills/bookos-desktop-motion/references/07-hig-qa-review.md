# 7 · HIG Compliance & Visual QA

Cómo comprobar que un cambio respeta el HIG de BookOS y se ve bien, y cómo
ampliar el HIG sin cambiar su identidad.

## Relación con el HIG

- Fuente: `~/Descargas/BookOS/BookOS-HIG/BookOS-HIG.md`, `bookos-tokens.json`,
  láminas `01 · Color` … `14 · Full App Window`.
- El HIG está escrito para web (CSS, `cubic-bezier`, `transform`). En el
  escritorio, sus valores viven en `tema.rs`. **El código manda en la
  traducción; el HIG manda en la identidad** (paleta, tipografía, radios,
  niveles de sombra, duraciones).
- Ampliaciones que propone esta skill (no cambian valores visibles):

| HIG (web) | Traducción nativa |
|---|---|
| «animar solo `transform` y `opacity`» | mover texturas ya rasterizadas; no re-rasterizar en tiny-skia por fotograma |
| `cubic-bezier(.34,1.4,.64,1)` spring pop | `M_REBOTE` solo con momento; si no, `M_SUAVE` |
| «nada dura más de 300 ms» | respuesta del muelle ≤ 0,315 s; excepciones `D_TEMA`, `D_DESBLOQUEO` con motivo |
| blur 20 px | `desenfoque.rs` en el compositor |
| `will-change` solo durante la animación | `animando()` apaga el repintado al asentarse |
| — | gestos, foco, presupuesto de 8,3 ms, reposo sin despertares (módulos 3, 5, 6) |

Si una regla del HIG no se puede cumplir en nativo, **se documenta la
traducción aquí y se pregunta** antes de cambiar un valor visible.

## Crítica de diseño (orden)

1. **Jerarquía**: un solo elemento principal por vista. ¿Qué se ve primero?
2. **Alineación y ritmo**: rejilla, espaciados de la escala, márgenes iguales
   arriba/abajo/lados en menús.
3. **Coherencia**: mismo radio, fila y realce que las superficies hermanas
   (menú del logo = dock = escritorio).
4. **Estados**: vacío, cargando, error, texto largo (nombres de usuario,
   idiomas), hover, pulsado, foco de teclado, deshabilitado.
5. **Temas**: claro, oscuro, alto contraste; fondo claro y oscuro detrás del
   cristal.
6. **Escala**: 1,0, 1,75, 2,0; resoluciones 1280×800 y 1440×900 lógicas.
7. **Movimiento**: abre/cierra desde el origen; interrumpible; efectos reducidos.

## Herramientas de captura

| Qué | Cómo |
|---|---|
| Menú del logo | `cargo run -p bookos-shell --example menu_logo` → `/tmp/menu_logo/*.png` |
| Bloqueo | `--example bloqueo` → `/tmp/bloqueo/*.png` |
| Escritorio | `--example escritorio` → `/tmp/escritorio/*.png` |
| Isla | `--example actividad` → `/tmp/actividad/*.png` |
| Anidado real | `dbus-run-session -- ./target/debug/bookos-comp` con `BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_GUION=…` |

Aislamiento obligatorio en anidado: `dbus-run-session`, `XDG_PICTURES_DIR`,
`XDG_CONFIG_HOME` (copia reciente de `panel.conf`), `XDG_DATA_HOME`,
`BOOKOS_ESCRITORIO`, `PIPEWIRE_RUNTIME_DIR=/nonexistent`. Anotar el brillo
antes y restaurarlo. `cargo build -p bookos-comp` antes del guion.

## Criterios medibles de QA

- Captura **antes y después**, mismas condiciones, mirada por una persona o
  modelo; diferencias explicadas.
- Claro + oscuro + escala 2 como mínimo para todo cambio visible.
- Texto largo: el nombre más largo esperado no se sale ni se recorta si cabe
  (medir con `widget::ancho_de`, no con `recortar_texto`).
- Tests de `crates/bookos-shell/tests/panel.rs` en verde (llevan medidas
  cableadas: si cambias `FILA` o márgenes, actualiza el test y dilo).
- `FUNCIONES.md` y marcas `// 0.x.y` / `// modified 0.x.y -- …` actualizadas.

## Procedimiento de revisión de un cambio

1. Leer el diff: ¿tokens o valores mágicos? ¿alguna llamada bloqueante nueva?
2. Módulo 1: cada animación orienta algo; tokens correctos.
3. Módulo 3: interrumpible y continuo.
4. Módulo 4: sin alfa ni `Shadow` caros nuevos; radios concéntricos.
5. Módulo 5: medir antes/después a ráfagas e intercalado.
6. Módulo 6: reposo sin despertares nuevos.
7. Capturas en temas, escala y efectos reducidos.
8. Lo que dependa de KMS, vblank, PSR o gestos reales: «sin probar en sesión
   real» en el resumen si no se pudo.
