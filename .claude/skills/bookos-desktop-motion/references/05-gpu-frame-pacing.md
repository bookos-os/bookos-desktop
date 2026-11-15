# 5 · GPU Rendering & Frame Pacing

Cómo llega un fotograma a la pantalla en BookOS y cómo medirlo.

## Camino de un fotograma

1. Entrada (libinput/winit) → `input.rs` / `keybinds.rs` → estado +
   `needs_redraw`.
2. `avanzar_animaciones` integra muelles. **Corre en cada vuelta del bucle con
   `needs_redraw`, también con un fotograma en vuelo**: no es «una por
   fotograma». Por eso `animar_emergente` lleva su propio tope de 6 ms.
3. El shell pinta en CPU (tiny-skia) las superficies que cambiaron y las sube
   como textura.
4. Escena del compositor (GLES2): ventanas, texturas del shell, desenfoque,
   shaders; damage tracking de Smithay.
5. KMS atómico en el vblank (`udev.rs`); direct scanout cuando una superficie
   cubre la salida.

Panel interno: **2880×1800 a 120 Hz** → **8,3 ms** por fotograma.

## Reglas

1. **Pintado CPU fuera del camino caliente**: rasterizar en tiny-skia solo
   cuando cambia el contenido. Las animaciones mueven texturas ya pintadas
   (transformación y opacidad en GLES).
2. **Damage exacto**: solo se repinta lo dañado. Un elemento que no se mueve
   no daña. Las métricas `saltados` (damage vacío) deben crecer en reposo.
3. **Nada bloqueante en el hilo del compositor**: ni `Command::output()`, ni
   D-Bus síncrono, ni disco. Medidos: `busctl` 8–29 ms y `gsettings` 8–9 ms.
   Va a otro hilo con un canal, o se cachea.
4. **Sin asignar por fotograma**: reutilizar `Vec`, sin `clone()` de
   estructuras grandes, sin `format!` en el camino caliente.
5. **Shaders**: compilar una vez (`compile_custom_*` al iniciar o en la
   primera vez) y guardar el programa; nunca por fotograma.
6. **Direct scanout y planos**: no romperlos con decoraciones encima de una
   ventana a pantalla completa sin motivo.
7. **Un fotograma en vuelo**: no encolar otro hasta el vblank; el damage que
   llega entre medias se aplaza (`aplazados`), no se pierde.

## Métricas disponibles

`/run/user/1000/bookos-session.log` (sesión real) y el log anidado escriben
cada 5 s por salida:

`ritmo de dibujo salida=… fps media_ms escena_ms render_ms shell_ms entrada_ms peor_ms saltados aplazados perdidos`

- `render_ms` **no incluye** el pintado CPU del shell: un render bajo no
  demuestra que el fotograma llegara a tiempo.
- `BOOKOS_PERFIL=1` en anidado registra «escena lenta» > 1 ms.
- Primera presentación de cada emergente: `abrir_ms`.

## Criterios medibles

| Métrica | Objetivo |
|---|---|
| `perdidos` durante una animación | 0 |
| `peor_ms` en animación | < 8,3 (120 Hz) |
| `escena_ms + render_ms + pintado shell` por fotograma | < 6 ms (margen para vblank) |
| `entrada_ms` | < 12 ms en sesión real |
| `abrir_ms` de una emergente | ≤ 1 fotograma desde la entrada (≤ 16 ms) |
| `fps` en reposo | ~0 (solo cursor), `saltados` creciendo |
| Llamadas a procesos o D-Bus síncrono en el hilo principal | 0 nuevas |

## Medir bien

- **A ráfagas**: un ejemplo en bucle cerrado sale ~3× optimista (6,5 ms frente
  a ~18 ms reales) porque la CPU no sube de frecuencia con trabajo a ráfagas.
  Poner `sleep(16 ms)` entre repintados.
- **Intercalar binarios** antes/después (A B A B…): la misma variante varía de
  6,5 a 10 ms entre tandas.
- Debug con dependencias O3; no hay permiso para `--release` sin preguntar.
- El anidado tiene techo de ~28 fps en algunos casos: el número absoluto de fps
  solo vale en sesión real; en anidado, comparar.
