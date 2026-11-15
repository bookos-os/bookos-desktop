# 3 · Gesture Physics & Interruptibility

Gestos del touchpad (3/4/5 dedos), pantalla táctil, arrastres y su física.

## Piezas existentes

- `tema::Muelle` / `Transicion`: integran posición y velocidad; aceptan
  destino nuevo sin perder velocidad.
- `gestos.rs`, `cinco_dedos.rs` (libinput llega a 4 dedos; el quinto se lee
  con `EVIOCGMTSLOTS`), `escritorios.rs`, `multitarea.rs`.
- FUNCIONES.md ya describe: agarre que detiene, proyección por impulso,
  histéresis de 10 puntos, rebote solo con impulso.

## Reglas

1. **Seguimiento 1:1** respetando el punto de agarre: lo que está bajo el dedo
   se queda bajo el dedo.
2. **Proyectar el destino** desde posición + velocidad al soltar, no desde la
   posición actual. Fórmula de referencia (desaceleración de scroll de Apple,
   recordada, no verificada): `proyección = v/1000 · d/(1−d)` con `d = 0,998`
   por ms.
3. **Velocidad de entrada al muelle** = velocidad del gesto al soltar. Sin
   ese traspaso, el movimiento se frena en seco y luego arranca.
4. **Agarre a mitad**: apoyar los dedos durante una animación la detiene
   donde está (posición actual, velocidad 0) y el gesto continúa desde ahí.
5. **Goma en los bordes**: pasado el límite, desplazamiento
   `x' = (1 − 1/(x·c/d + 1))·d` con `c ≈ 0,55`, `d` = dimensión del
   contenedor. Nada de topes duros.
6. **Histéresis** de 10 pt antes de decidir eje o gesto; la decisión no
   retrasa el seguimiento (el contenido ya se mueve).
7. **Velocidad estimada** con los últimos 50–100 ms de muestras, no con las
   dos últimas (el ruido de libinput da picos).
8. **Rebote** (`M_REBOTE`) solo si la velocidad al soltar supera el umbral del
   gesto; si no, `M_SUAVE`.
9. **Táctil**: realce al bajar, acción al levantar, cancelar saliendo de una
   holgura de 10 pt.

## Criterios medibles

- Latencia gesto→píxel (`entrada_ms`) ≤ 1 vblank + tiempo de escena; objetivo
  < 12 ms a 120 Hz en sesión real.
- Error de seguimiento: |posición pintada − dedo| < 1 px lógico durante el
  arrastre (traza provisional).
- Continuidad al soltar: |v_muelle(t₀) − v_gesto| < 5 %.
- Interrupción: agarrar a mitad no produce salto > 2 px.
- Ningún gesto deja `needs_redraw` encendido tras asentarse (comprobar
  `saltados` crecer y `fps` caer a ~0 en reposo).

## Revisión

1. Guion con `deslizar inicio/avance/fin` y velocidades distintas; capturas a
   mitad y al final.
2. Gráfica de posición por fotograma desde traza provisional: sin escalones
   ni inversiones no pedidas.
3. Probar con efectos reducidos: el gesto sigue 1:1 y al soltar salta al
   destino.
4. Gestos de 3/4 dedos: validar en sesión real (libinput real, no guion).
