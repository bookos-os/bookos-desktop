---
name: bookos-desktop-motion
description: >
  Movimiento, materiales, ritmo de fotogramas y consumo del escritorio BookOS
  (compositor Smithay/GLES + shell iced/tiny-skia). Úsala al diseñar, implementar
  o revisar cualquier animación, transición de ventana o escritorio, gesto,
  material translúcido, sombra, shader o cambio que afecte a la fluidez o a la
  batería en bookos-desktop. Amplía el HIG de BookOS; no lo sustituye.
---

# BookOS Desktop Motion

Objetivo: un escritorio que se sienta premium y fluido **sin gastar más
batería**, coherente con el HIG existente.

## Pila real (comprobada en el repo, 2026-11-11)

| Capa | Tecnología | Dónde |
|---|---|---|
| Compositor | Smithay 0.7.0 **parcheado** (`vendor/smithay`), `renderer_gl` (GLES2), `backend_egl`, `desktop`, `xwayland` | `crates/bookos-comp` |
| Salida | KMS atómico + libinput + libseat (sesión real); winit (anidado) | `backend/udev.rs`, `backend/winit.rs` |
| Bucle | calloop; repinta por `needs_redraw` y vblank | `backend/mod.rs` |
| Shell (panel, dock, emergentes, bloqueo) | iced 0.14 con **tiny-skia en CPU**, sube texturas al GLES | `crates/bookos-shell` |
| Efectos de GPU | shaders GLES propios: `desenfoque`, `genio`, `banda`, `brillo`, `captura`, `salvapantallas` | `crates/bookos-comp/src` |
| Tokens de movimiento | `D_*` (duraciones), `C_*` (curvas), `ParametrosMuelle` (`M_RAPIDO`, `M_SUAVE`, `M_REBOTE`), `Muelle`, `Transicion`, `Realce`, `efectos_reducidos()` | `crates/bookos-shell/src/tema.rs` |
| HIG | `../BookOS-HIG/BookOS-HIG.md` (§8 movimiento), `bookos-tokens.json` | fuera del repo |

**No hay wgpu ni GPUI** en bookos-desktop. Antes de aplicar esta skill a otro
repo (explorer, greeter, Settings), vuelve a comprobar su `Cargo.toml`. Si la
pila ha cambiado, actualiza esta tabla antes de seguir.

## Cuándo usarla

- Añadir o cambiar una animación, transición, gesto o efecto visual.
- Tocar `tema.rs` (tokens), `backend/*` (ritmo), shaders, `desenfoque`, sombras.
- Revisar un cambio visible antes de darlo por bueno.
- Investigar tirones, fotogramas perdidos o consumo en reposo.

## Flujo

1. **Situar**: qué módulo aplica (tabla de abajo) y leer su referencia.
2. **Leer antes de escribir**: buscar en el repo cómo se resuelve ya algo
   parecido (`grep` por `Muelle`, `Transicion`, `animando`, `needs_redraw`) y
   copiar ese patrón.
3. **Medir antes**: anotar `fps`, `peor_ms`, `perdidos`, `entrada_ms` y el
   coste de pintado ([05](references/05-gpu-frame-pacing.md)).
4. **Implementar** con tokens existentes; un valor nuevo necesita motivo
   escrito en el comentario.
5. **Medir después** con el mismo método e intercalando binarios.
6. **Revisar** con [07](references/07-hig-qa-review.md): capturas en claro y
   oscuro, efectos reducidos, batería.

## Módulos

| # | Módulo | Referencia |
|---|---|---|
| 1 | Desktop Motion Language | [01-motion-language.md](references/01-motion-language.md) |
| 2 | Window & Workspace Transitions | [02-window-workspace.md](references/02-window-workspace.md) |
| 3 | Gesture Physics & Interruptibility | [03-gesture-physics.md](references/03-gesture-physics.md) |
| 4 | Native Materials & Visual Depth | [04-materials-depth.md](references/04-materials-depth.md) |
| 5 | GPU Rendering & Frame Pacing | [05-gpu-frame-pacing.md](references/05-gpu-frame-pacing.md) |
| 6 | Power-Aware Animation | [06-power-aware.md](references/06-power-aware.md) |
| 7 | HIG Compliance & Visual QA | [07-hig-qa-review.md](references/07-hig-qa-review.md) |

## Reglas duras

1. **Latencia cero**: la respuesta a una entrada sale en el **mismo** fotograma
   que la procesa. Nada espera a un temporizador para empezar.
2. **Interrumpible**: toda animación se puede redirigir a mitad desde su
   posición y velocidad actuales. Nunca se reinicia desde cero.
3. **Solo transformación y opacidad** se animan en caliente. Tamaño, layout y
   texto no se re-rasterizan por fotograma.
4. **Reposo = cero trabajo**: sin animación activa no hay repintado, ni
   temporizador, ni sondeo. `animando()` decide; al llegar a destino se apaga.
5. **Presupuesto**: 8,3 ms por fotograma a 120 Hz en el panel interno, contando
   el pintado CPU del shell, que **no** sale en `render_ms`.
6. **Tokens o motivo**: duraciones, curvas, muelles, radios y sombras salen de
   `tema.rs` / HIG. Lo nuevo lleva el porqué y lo que se midió.
7. **Efectos reducidos** respetados siempre: los muelles saltan al destino y
   desaparecen el rebote y el parallax, pero la orientación se mantiene
   (fundido corto).
8. **No sustituir el HIG**: se amplía. Si una regla del HIG choca con el
   escritorio nativo, se propone la traducción en [07](references/07-hig-qa-review.md)
   y se pregunta antes de cambiar valores visibles.
9. Sin CSS, DOM, WebView ni dependencias nuevas sin avisar.
10. Probar en anidado con bus aislado (`dbus-run-session`) y **verificar en
    sesión real** lo que dependa de KMS, vblank o PSR.

## Fuentes de los principios

- `motion-design` (LottieFiles, instalada en `~/.Codex/skills`): propósito,
  timing, coreografía y personalidad, adaptados aquí a muelles y al compositor.
- WWDC18 *Designing Fluid Interfaces* (nota de memoria
  `principios-interfaces-fluidas`): latencia cero, interrumpible, proyección
  de momento, muelles con amortiguación/respuesta.
- `critique`, `rust-best-practices` y `rust-ui-performance` **no están
  instaladas** en esta máquina. De ellas solo se aplican principios generales
  (crítica por jerarquía, ritmo y coherencia; Rust idiomático sin `clone`
  gratuito ni asignaciones por fotograma; medir antes de optimizar). Si se
  instalan, contrastar y citar en vez de esta paráfrasis.
