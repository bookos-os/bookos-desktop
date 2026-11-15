# 6 · Power-Aware Animation

Animar sin gastar batería. Contexto de hardware y medidas en
`~/Descargas/eficiencia.md`.

## Principio: race to sleep, también en la interfaz

En Lunar Lake con `intel_pstate` (HWP) la CPU ya sube, termina y duerme. Lo
que gasta es **despertarla a menudo**: un temporizador de 16 ms que no pinta
nada cuesta más que una animación corta que acaba y se calla.

## Reglas

1. **Una animación termina**: cuando el muelle llega a destino (umbral de
   posición y velocidad), `animando()` devuelve `false` y no se pide otro
   fotograma. Prohibido «por si acaso».
2. **Sin temporizadores en reposo**: rampas (`brillo_auto`, `atenuacion`),
   relojes y medidores se cancelan al acabar o al cerrarse lo que los enseña.
   El reloj del panel despierta cada minuto al cambio de minuto, no cada
   segundo.
3. **Repintar solo lo visible**: nada se anima en una salida apagada (DPMS),
   bajo el bloqueo o en un escritorio que no se ve.
4. **Cadencia según contenido**, no fija: lo que cambia una vez por segundo
   (cifras, batería) repinta una vez por segundo aunque el panel vaya a 120 Hz.
5. **Animaciones infinitas** (spinners, salvapantallas, fondos de vídeo): solo
   mientras se ven, con tope de fps propio si la animación no necesita 120
   (vídeo de fondo al fps del vídeo).
6. **Efectos reducidos y batería**: el modo ahorro puede activar efectos
   reducidos o bajar a 60 Hz; la orientación (fundidos cortos) se mantiene.
7. **Nada de sondeo** donde hay señal: udev, inotify, D-Bus `PropertiesChanged`
   antes que leer cada N segundos.

## Criterios medibles

| Medida | Cómo | Objetivo |
|---|---|---|
| Despertares del compositor en reposo | `voluntary_ctxt_switches` del hilo principal, dos lecturas a 10 s, escritorio quieto | unidades por segundo |
| Despertares del shell en reposo | igual, proceso `bookos-shell` | ≤ 2/s |
| `fps` en reposo | log de métricas | ~0 |
| Temporizadores vivos tras cerrar una emergente | traza o `loop_handle` | 0 |
| Potencia en reposo | `current_now × voltage_now` en batería, 5 min, 3 tandas | no sube respecto a la línea base |

Referencia medida el 2026-11-11 **con carga** (VM activa): compositor 283
despertares/s, shell 57/s. Falta la medida en reposo limpio: es lo primero que
hay que tomar antes de optimizar.

## Revisión

1. Antes/después de un cambio: despertares en reposo de compositor y shell.
2. Cerrar la emergente o terminar el gesto y comprobar que `fps` cae a ~0 en
   el siguiente bloque de 5 s.
3. `grep -n "Timer::\|from_millis\|insert_source" <archivo>`: cada temporizador
   tiene su cancelación.
4. Con la pantalla apagada o bloqueada: cero repintados de lo que hay detrás.
