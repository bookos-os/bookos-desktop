# 1 · Desktop Motion Language

El lenguaje de movimiento de BookOS: qué se mueve, por qué y con qué carácter.
Amplía el §8 del HIG (pensado para web) con su equivalente nativo.

## Carácter

- **Breve y físico.** La HIG ya lo fija: 120–280 ms, nada más de 300 ms salvo
  progreso. En el escritorio, «breve» se expresa con muelles de **respuesta**
  corta, no con duraciones.
- **Orientar, no decorar.** Cada movimiento responde a una de estas preguntas:
  ¿de dónde viene?, ¿adónde va?, ¿qué ha cambiado?, ¿qué ha pasado con mi
  acción? Si no responde ninguna, sobra.
- **Calma por defecto.** Amortiguación 1,0 (sin rebote). El rebote es la
  excepción y solo cuando el usuario aporta momento.

## Tokens (tal como están en `tema.rs`)

| Token | Valor | Uso | Equivalente HIG |
|---|---|---|---|
| `M_RAPIDO` | amort. 1,0, resp. 0,21 s | realces, selección, cambios de estado | 100–220 ms ease-out |
| `M_SUAVE` | amort. 1,0, resp. 0,315 s | tarjetas, ventanas, páginas | 250–280 ms |
| `M_REBOTE` | amort. 0,8, resp. 0,28 s | solo con momento de gesto | spring pop |
| `D_HOVER` | 120 ms + `C_SUAVE` | hover, pulsación | 100–130 ms |
| `D_POPOVER` | 180 ms + `C_MUELLE_POPOVER` | popovers | 180–220 ms |
| `D_TARJETA` | 220 ms + `C_ENTRADA` | tarjetas | 180–220 ms |
| `D_MODAL` | 250 ms + `C_MUELLE` | modal, conmutador | 250 ms |
| `D_VENTANA` | 260 ms + `C_VENTANA` | maximizar, encajar | — (nuevo, nativo) |
| `D_PAGINA` | 280 ms + `C_ENTRADA` | página completa | 280 ms |
| `D_TEMA` | 450 ms + `C_TEMA` | claro↔oscuro | excepción justificada en código |
| `D_DESBLOQUEO` | 400 ms | salida del bloqueo, dos tiempos | excepción justificada |

**Respuesta ≠ duración**: la respuesta es el periodo natural del muelle. Con
amortiguación 1,0 llega al 99 % en ~1× la respuesta, pero nunca bloquea: se
puede redirigir en cualquier momento.

### Cuándo duración y cuándo muelle

- **Muelle** (`Muelle`, `Transicion`): todo lo que el usuario puede
  interrumpir o redirigir (abrir/cerrar emergentes, realces, ventanas,
  escritorios, gestos).
- **Duración + curva**: transiciones de una pieza que no se interrumpen por
  diseño (fundido de tema, desbloqueo) o barridos de progreso.
- Nunca mezclar los dos en la misma propiedad.

## Coreografía

- **Una pieza principal por transición**; el resto la acompaña o espera.
- Escalonado (stagger) entre 15 y 30 ms por elemento y **máximo 5 escalones**.
  Más allá, todo el grupo entra a la vez.
- Lo que entra lo hace desde el origen que lo provocó (icono del dock, widget
  del panel, cursor). Lo que sale vuelve por el mismo camino.
- Entrada más larga que la salida (≈ 1 : 0,7). Salir rápido devuelve el
  control antes.

## Desplazamientos y escalas canónicas

| Movimiento | Valor | Fuente |
|---|---|---|
| Popover | `scale .96→1` + `translateY −8→0` | HIG §9.11 |
| Modal | `scale .92→1` + `translateY 12→0` | HIG §8.3 |
| Menú de energía del bloqueo | baja 18 px desde el botón | `bloqueo.rs` |
| Página | solo opacidad `.55→1` | HIG §8.3 |

Valores nuevos solo con motivo escrito y captura comparativa.

## Criterios medibles

- 100 % de las propiedades animadas usan un token de la tabla o un comentario
  con motivo y medida.
- Ninguna animación interrumpible usa duración fija.
- Ninguna transición supera 300 ms salvo las excepciones de la tabla.
- Rebote solo en código que recibe una velocidad de gesto.

## Revisión

1. `grep -n "Duration::from_millis" <archivo>`: cada número suelto, ¿es token o
   tiene motivo?
2. Para cada animación, decir en una frase qué orienta. Si no hay frase, se quita.
3. Comprobar el sentido espacial: abrir y cerrar desde el mismo origen.
