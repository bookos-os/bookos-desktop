# 2 · Window & Workspace Transitions

Transiciones del compositor: abrir, cerrar, minimizar, maximizar, encajar,
escritorios, exposición, launchpad, conmutador, bloqueo.

## Dónde vive cada una

| Transición | Archivo | Técnica actual |
|---|---|---|
| Abrir / cerrar ventana | `ventanas.rs`, `cierre.rs` | escala + opacidad sobre la superficie; al cerrar se capturan las texturas porque el cliente destruye la superficie en el mismo lote |
| Minimizar (lámpara mágica) | `genio.rs` | textura + shader GLES de deformación |
| Maximizar / encajar | `ventanas.rs` | `D_VENTANA` + `C_VENTANA` |
| Escritorios | `escritorios.rs` | un solo `Space`; la tira sigue a cuatro dedos |
| Exposición, launchpad | `multitarea.rs`, `emergente/launchpad.rs` | gesto continuo + muelle |
| Tema | `fundido.rs` | foto del tema viejo encima, desvaneciéndose |
| Bloqueo | `shell.rs` (`desbloquear`), `bloqueo.rs` | dos tiempos, `D_DESBLOQUEO` |

## Reglas

1. **Animar la superficie viva** mientras exista: escalar/trasladar un
   elemento del render cuesta casi cero. Solo se hace textura si la
   deformación no es afín (genio) o si el cliente ya no está (cierre).
2. **Nunca redimensionar el cliente por fotograma.** Durante maximizar o
   encajar se escala la textura actual hacia el rectángulo final y se manda
   **un** `configure` con el tamaño definitivo. Si el buffer nuevo llega a
   mitad, se cambia sin salto (se mantiene el rectángulo pintado).
3. **Continuidad espacial**: minimizar va al icono del dock y restaurar sale
   de él; cerrar es abrir al revés; un escritorio sale por el lado contrario
   al que entra el siguiente.
4. **El foco va con la intención**: la ventana que se trae al frente recibe
   el teclado en el mismo fotograma en que empieza a subir, no al terminar.
5. **Ventanas X11**: `configure` solo a no override-redirect; los menús X11 se
   dejan donde el cliente los pone (ver nota de memoria `menus-x11-override-redirect`).
6. **Apilado**: lo que se anima por encima no puede cambiar el orden lógico de
   entrada; durante la animación los clics van a la geometría **final**.
7. **Salida antes que entrada** cuando se encadenan (desbloqueo: se va el
   bloqueo y luego entran las ventanas); el solape no puede dejar menos de
   200 ms a la segunda pieza.

## Criterios medibles

- Ninguna transición de ventana hace más de **1** `configure` por cambio de
  estado (contar con `WAYLAND_DEBUG=client`).
- `perdidos=0` en «ritmo de dibujo» durante maximizar/restaurar con dos
  ventanas abiertas.
- Clic a mitad de transición: llega a la ventana destino (guion con `clic`
  a 100 ms).
- Una transición iniciada y revertida antes de terminar vuelve sin salto: la
  posición del fotograma siguiente difiere < 2 px lógicos de la anterior.

## Revisión

1. Guion anidado que dispare la transición y la revierta a mitad (`tecla` + `espera 80` + `tecla`).
2. `WAYLAND_DEBUG=client` en el cliente: contar `configure`.
3. Traza provisional del avance por fotograma en `backend/mod.rs` para ver la
   curva real (el guion se come las animaciones cortas); quitarla después.
4. Verificar en sesión real si toca vblank, KMS o planos.
