# Funciones de BookOS Desktop

Lo que usa quien se sienta delante, con la versión en que llegó y lo que
cambió después. Es el registro de cara al usuario; el del código son las
marcas de versión junto al código; desde 0.2.0 se usan
`// añadido en la 0.2.0` y `// modificado en la 0.2.0 -- …`.

- **0.1.0** — del primer commit (16-ago-2026) a «pre 1.0.1» (12-oct-2026).
- **0.1.1** — desde el 17-oct-2026.
- **0.2.0** — movimiento continuo y medidas de respuesta.

## 0.2.0

- Fondo de pantalla: Ajustes › Fondo de pantalla elige cómo encaja la imagen, como en Windows: Rellenar (de serie), Ajustar, A lo ancho, Estirar y Normal (tamaño real, centrada). Vale para imágenes y vídeos, y el cristal del panel y del dock, y las miniaturas de escritorios, siguen el mismo encuadre. Se guarda en `panel.conf` (`fondo_encaje`) y viaja por `ApplyConfig`. Antes la imagen siempre se estiraba y se deformaba si no tenía la forma de la pantalla.
- Diagnóstico: coste de pintura del shell (`shell_ms`), entrada hasta presentación (`entrada_ms`) y primera presentación de cada emergente (`abrir_ms`). Incluye la pintura del shell en el presupuesto de fotograma. Las latencias solo se consumen tras un encolado aceptado y su presentación.
- Escritorios: cuatro dedos desplazan la tira continuamente. Al soltar decide la posición proyectada por el impulso reciente; cancelar devuelve el escritorio original. Los extremos ofrecen resistencia. Los atajos usan el mismo muelle, incluida la inversión durante el recorrido.
- Exposición: tres dedos mueven las ventanas hacia sus miniaturas y permiten devolverlas antes de soltar. Abrir, cerrar y redirigir conservan posición y velocidad.
- Launchpad: el pellizco de cinco dedos controla alfa y escala mientras la mano sigue apoyada. Las emergentes comparten un muelle para abrir y cerrar sin reiniciar el movimiento.
- Física compartida: los tokens se expresan con amortiguación y respuesta. El movimiento ordinario usa amortiguación crítica (100 %); solo el impulso capaz de alcanzar el destino permite un 80 %. La proyección integra una deceleración exponencial de constante 200 ms: es un parámetro de movimiento, no una espera de reconocimiento.
- Agarre: apoyar los dedos detiene el recorrido visible; cancelar sin elegir eje lo libera hacia su destino anterior. Los deslizamientos se reconocen tras 10 puntos. El exceso en los límites no acumula distancia invisible al invertir.
- Pantalla táctil: el apoyo resalta los controles del shell y la acción se confirma al levantar el dedo. Salir de una holgura de 10 puntos cancela el toque; solo una zona que admite arrastre puede convertirlo en desplazamiento. Los clientes conservan `wl_touch`. Cancelar el dispositivo o bloquear evita confirmar el botón pendiente.
- Bloqueo: reloj, acceso, música y energía tienen texturas propias. Su entrada no rasteriza por fotograma; escribir solo repinta el acceso. El ejemplo `bloqueo` exporta un PNG de cada capa.
- Dock: el realce se funde y los iconos rebotan mediante composición de texturas. El hover no vuelve a rasterizar la barra.
- Menú BookOS (el del logo, Meta+Z): sin iconos, más compacto y con esquinas de 10 px, como el menú de macOS. El ancho se ajusta a la entrada más larga y «Cerrar sesión» lleva el nombre de la cuenta. Los atajos se dibujan con los símbolos de macOS: Meta+L es ⌘L y Ctrl+Alt+Supr es ⌃⌥⌦. Los menús del dock y del escritorio comparten el radio y el alto de fila.
- Efectos reducidos: los muelles saltan al destino y se elimina el bote del dock.
- Mostrar escritorio: cuatro dedos hacia abajo apartan las ventanas siguiendo la mano, y hacia arriba las devuelven si ya estaban apartadas. Se puede deshacer antes de soltar, y al soltar decide la posición proyectada, como en la exposición. La exposición pide el doble de recorrido (480 en vez de 240).
- Exposición: tocar una ventana la sube al frente al instante; la vuelta se pinta con el apilado real.
- Panel: el resaltado del ratón y el de la tarjeta abierta se funden al entrar y al salir, y pasan de un widget a otro cruzándose. El cambio de icono usa un muelle crítico sin rebote, el punto de escritorios se redirige sin reiniciarse al cambiar varias veces seguidas, y «Desconectado» abre y cierra su hueco en vez de empujar a los vecinos de golpe.
- Panel: los widgets se eligen, ordenan y ocultan desde `ApplyConfig` (`centro`, `derecha`) y el cambio se ve al instante, sin reiniciar. Un nombre desconocido o repetido se rechaza con un error en vez de ignorarse, y `GetConfig` devuelve `widgets_disponibles` (solo lectura). Widgets nuevos: `monitor` (CPU, RAM y temperatura del procesador), `trafico` (bajada y subida de red) y `consumo` (vatios de la batería); se actualizan cada 2 s y solo cuestan algo si están en el panel.
- Vista de escritorios (Meta+W): cuatro dedos hacia arriba la abren siguiendo a la mano y, abierta, hacia abajo la cierran. Al soltar decide la posición proyectada por el impulso; cancelar la devuelve. Las ventanas y la franja leen el mismo muelle, así que van a la vez. Antes subir disparaba la apertura a un tercio del recorrido y el resto del movimiento se ignoraba.
- Hoja de atajos: las teclas se dibujan como teclas. Las letras y las flechas van cuadradas y todas con la misma altura; `←/→` y «Flechas» se reparten en una tecla por flecha; Mayús es `⇧`. Los gestos llevan su flecha como tecla tenue.
- Captura de pantalla: la barra de la capa (Impr, Meta+Mayús+S) tiene un tercer grupo. «Sin cursor / Con cursor» deja la flecha fuera de la foto, que es lo de serie; «Sin espera / 3 s / 5 s / 10 s» espera antes de capturar, con la capa ya cerrada, para abrir un menú o quitar la mano. Las dos se recuerdan en `panel.conf` (`captura_cursor`, `captura_retardo`) y viajan por `ApplyConfig`. Las capturas del portal y de `grim` siguen llevando el cursor.
- Todas las ventanas (tres dedos arriba, Meta+Tab): las miniaturas ya no están topadas a 300×250. Crecen hasta el 45 % del ancho y el 50 % del alto de la pantalla, y el reparto elige la rejilla que da la miniatura mayor (2×2 con cuatro ventanas, 4×2 con ocho), sin renunciar a la fila única si pierde menos del 20 %. Deja libres 110 px arriba y abajo para el panel y el dock.
- Emojis: al desplazar la rejilla, la fila que sale se corta contra el borde y la que entra asoma cortada, a tamaño real. Antes la de arriba se montaba sobre la cabecera y desaparecía de golpe, y la de abajo salía encogida.
- Buscador: el resaltado de la elegida se desliza entre filas, la tarjeta crece y encoge con un muelle al cambiar los resultados, el realce del ratón se funde y aparece en su sitio sin caer. Ejemplo `buscador_png` → `/tmp/buscador/` con capturas a mitad y el coste de repintar.
- Tarjetas: interruptores, OSD, teclado en pantalla, desplazamiento de emojis y menús del greeter usan muelle crítico en vez de curva con duración; pulsar a mitad conserva la velocidad. Bluetooth, Wi-Fi y la Estación de Control dicen «No conectado» donde antes ponían «Emparejado», nada o «Sin conexión».
- Cambiar de escritorio con cuatro dedos: el aviso «Escritorio N» sale al soltar y solo si se llega; antes salía al empezar a mover. El fondo se desliza a la misma velocidad que las ventanas en pantallas con escala distinta de 1: corría a la mitad y la costura se quedaba a media pantalla.
- Estación de Control: las baldosas, las tarjetas de Wi-Fi y Bluetooth, los botones de la cabecera, del volumen, del brillo y de la música actúan al soltar encima, no al apretar. Al apretar la baldosa se hunde (encoge un 8 % y se oscurece) con un muelle; salirse antes de soltar la devuelve y no hace nada.
- Tarjetas de Wi-Fi y Bluetooth: apagados, enseñan «Wi-Fi apagado» o «Bluetooth apagado» en el centro en vez de la lista de dispositivos «No conectado».
- Teclado en pantalla: interruptor propio en Ajustes › Pantallas (`teclado_pantalla` en panel.conf); apagado, el táctil sigue funcionando y el teclado no sale.
- Guiones anidados: `deslizar inicio 4`, `deslizar avance dx dy`, `deslizar fin|cancelar` y `pellizcar inicio 4`, `pellizcar avance escala`, `pellizcar fin|cancelar`. Admiten los milisegundos hasta la siguiente orden al final; permiten capturar con los dedos apoyados y atraviesan `input::handle`. El recorrido reproducible está en `crates/bookos-comp/tests/movimiento_020.guion`.
- Verificación táctil: `dedo widget volumen`, `dedo apoyar x y`, `dedo mover x y`, `dedo soltar|cancelar` separan las fases. `comprobar emergente nombre|ninguna` hace fallar el guion si una tarjeta se confirma antes de tiempo o tras cancelar.
- Color de carpeta: arrastrar rápido por el selector de color libre ya no se atasca. Cada repintado cuesta un tercio y se junta en uno por fotograma, también en los demás arrastres dentro de una tarjeta, como el del volumen.
- Cuentagotas: después de elegir un color, la rueda y los clics vuelven a llegar a la ventana de debajo sin tener que mover antes el ratón; la suelta del clic que elige ya no le llega.
- Cuentagotas: una lupa sigue al puntero con 15 × 15 píxeles ampliados, el central marcado y el hex debajo; el clic copia ese píxel. Lleva borde oscuro para verse sobre blanco.
- Barra superior: tamaño pequeño, mediano o grande en Ajustes › Escritorio (`panel_tamano` en panel.conf). Todo el panel crece a la vez y el alto se ajusta ±10 % al alto de la pantalla.
- Launchpad: su icono del dock va por el tema de iconos (claro, oscuro, tintados) como el resto de apps; el cohete guardado en panel.conf se cambia solo.
- Acerca de: dibujo del Galaxy Book5 Pro o del Book5 Pro 360 según el modelo; los demás equipos, uno de frente.
- Buscador: «brillo 30» o «volumen 50» (también `brightness`, `volume`, con o sin `%`) dan una fila que lo pone con Intro y enseña el aviso de siempre.
- Zenith en la isla: la onda de «Te escucho…» sigue la voz de verdad (antes no recibía el nivel y quedaba plana); «Pensando…» lleva una onda propia que recorre las barras; un error se sacude dos veces y media al aterrizar la tarjeta. Con las animaciones de la isla apagadas, la onda se queda quieta y no hay sacudida.
- Zenith, pulsar para hablar: mantener Meta+Alt+A (o la tecla remapeada al asistente) más de 400 ms escucha sin cortar en los silencios y envía al soltar; la isla dice «Suelta para enviar». Un toque sigue escuchando y cortando solo. Un clic fuera mientras escucha lo minimiza a la burbuja sin dejar de escuchar; Esc y la ✕ cancelan.
- Zenith: sale en la isla aunque las actividades estén apagadas en Ajustes. El brillo de borde se funde al aparecer y al irse (antes se encendía y apagaba de golpe) y, mientras escucha, se aviva con la voz; la píldora crece hasta un 4 % al hablarle y la onda sigue la voz sin escalones.
- Zenith: nueva animación del borde al activarlo. Un filo blanco recorre el contorno, cede a la luz con los colores del fondo, que corre despacio mientras escucha y se ensancha con la voz, y más deprisa mientras piensa; se abre más arriba, junto a la isla, y se recoge al apagarse.
- Zenith: la píldora de la isla es más estrecha (de 440 a 300) y de una línea, con un canto de color que gira, un orbe que late con la voz y una onda de colores; mientras dictas se ve el final de la frase.
- Zenith: la isla sigue a la vista aunque el panel esté oculto; las demás actividades se van con él.
- Gestos: en la exposición de tres dedos, pulsar mientras se abre elige la ventana que se ve debajo del cursor. Con cuatro dedos, subir la mano con algo de deriva lateral ya no cambia de escritorio.
- Vista de escritorios: la ✕ de las ventanas es un botón redondo con fondo propio, que se ve sobre cualquier ventana y se pone rojo al señalarlo.
- Meta+Tab y exposición: debajo de las miniaturas sale la aplicación de la ventana elegida, con icono y nombre grandes, y el título de la ventana debajo.
- Pantalla de acceso: la contraseña se ve con los mismos puntos grandes del bloqueo y el texto del campo es algo mayor. Hora y fecha van igual en las dos pantallas.
- Bloqueo (Meta+L): en vez de los tres puntos, batería, red y botón de apagado arriba a la derecha, como en la pantalla de acceso.
- Zenith: al llegar la respuesta o un error, la píldora se despliega en tarjeta en su sitio, sin desaparecer y volver a crecer; minimizar y volver hacen la entrada completa en el sitio nuevo, sin teletransportarse.
- Selector de emojis: 21 nuevos (🏳️‍⚧️ bandera trans, ⚧️, 🥀 rosa marchita, 🫶, 🥹, 🫠, 🫡, 🩷, 🩵, 🩶, 💅, 🫂, 🌷, 💐…). La bandera arcoíris pasa a Banderas y se encuentra también por «gay» y «lgtb».
- Selector de emojis: pasar el ratón ya no va a tirones; cada repintado baja de 12,4 a 4,0 ms (debug) al no pintar los fondos de celda y de categoría que no se ven.
- Panel: pasar de una tarjeta a otra (clic en otro widget, Meta+Z → Meta+X) abre la nueva al instante, como una barra de menús. Antes esperaba a que la anterior terminara de salir: ~500 ms por cambio.
- Hoja de atajos: mantener Meta sola un segundo enseña todos los atajos de teclado y gestos; al soltarla se va sin abrir el launchpad. También desde el buscador («Atajos de teclado»).
- Buscador: se cierra tras cualquier acción (luz nocturna, mantener despierto, nuevo escritorio seguían con él abierto y parecía que no hacían nada); «Mantener despierto» desde el buscador ahora sí lo cambia; los lanzadores cuyo `TryExec` no existe ya no salen (el `bookos-captura` roto tapaba la acción de captura).
- Guiones anidados: `apretar tecla` y `levantar tecla` para mantener una tecla pulsada.
- Launchpad: los puntos de página se pulsan y llevan a su página; con el dock escondido ya no se le reserva su franja y los puntos bajan al fondo.
- Avisos más compactos: el texto va bajo el título, a la derecha del icono, y no debajo del icono; uno de una línea pasa de 85 a 75 de alto. El de conectar o desconectar el cargador sale en el idioma de la sesión («Charger connected» en inglés).
- Cristal del panel y del dock: el fondo se sube ya reducido a un octavo (−49 MB de memoria de GPU medidos en anidado), con el mismo aspecto.

No se añade una pausa para entrar en multitarea: el gesto actual controla la
exposición directamente y no comparte un recorrido de «ir a casa» que haya
que distinguir por aceleración. El desenfoque de movimiento y el estiramiento
quedan fuera hasta medir los saltos entre fotogramas en la sesión DRM;
no se introducen como efecto global.

Validación de estos ajustes: 656 pruebas superadas y una ignorada, compilación
del workspace y Clippy sin avisos nuevos. El guion anidado comprueba apoyo,
cancelación y liberación, además de las inversiones de gesto. Se detectó una
limitación intermitente al tomar capturas consecutivas: algún PNG omite las
ventanas, aunque las trazas conservan las cuatro mapeadas, con alfa 1 y sin
cambios de geometría. No se ha aislado la causa; las capturas no sustituyen
la validación pendiente en DRM y con dispositivos físicos.

Medidas locales (perfil dev con dependencias optimizadas, 2880×1800): antes,
6,02–6,38 ms por tecla del bloqueo y 9,06–10,16 ms con música; con la capa de
acceso, 2,37–2,86 ms en ambos casos. El hover del dock costaba 1,38 ms por
pintado; las pruebas de la nueva ruta verifican cero invocaciones de pintura
para el movimiento del bloqueo y el hover.

En 30 arranques de cada reproductor con `--help`, `pw-play` dio mediana
0,243 ms y máximo 1,135 ms; `canberra-gtk-play`, 0,226 y 0,352 ms. Se conserva
la reproducción actual y se añade `arranque_ms` a las trazas de depuración
para medir también los eventos de la sesión real.

Quedan condicionados a medición en sesión DRM: precalentar tarjetas si
`abrir_ms` supera un fotograma, sustituir el audio si el arranque supera ese
plazo y confirmar el plano del cursor tras despertar. El registro anterior
contiene fallos de prueba del plano, pero no demuestra que todos los
movimientos caigan a composición. El controlador es `xe`; los estados de PSR
y panel replay no se han podido leer: sus parámetros y debugfs no son
accesibles con los permisos de esta sesión. No se han
cambiado VRR, PSR ni ajustes de potencia. El anidado comprueba los flujos y la
composición visual, no la suavidad a 120 Hz ni DPMS.

Para repetir la batería de forma aislada, usar `XDG_CONFIG_HOME` temporal y
`--test-threads=1`: las pruebas existentes comparten la paleta global y el
buscador lee las sugerencias de `usos.conf` del usuario.

Cada función nueva o mejorada se apunta aquí en la versión en la que entra.

---

## Atajos de teclado

| Atajo | Qué hace | Versión |
|---|---|---|
| Meta | Abrir o cerrar el launchpad | 0.1.0 |
| Meta+Espacio | Buscador de aplicaciones, ajustes y acciones | 0.1.0 |
| Meta+Return · Ctrl+Alt+T | Abrir un terminal | 0.1.0 |
| Alt+Tab | Elegir aplicación por icono | 0.1.0 |
| Meta+Tab | Elegir ventana por miniatura | 0.1.0 |
| Meta+F | Maximizar o restaurar | 0.1.0 |
| Meta+flechas | Encajar en media pantalla; otra flecha, en un cuarto | 0.1.0 |
| Meta+Q | Cerrar la ventana | 0.1.0 |
| Meta+H | Minimizar | 0.1.0 |
| Meta+W | Vista de escritorios | 0.1.0 |
| Meta+Ctrl+D | Mostrar el escritorio | 0.1.0 |
| Meta+1…5 | Ir a un escritorio | 0.1.0 |
| Meta+Ctrl+← / → | Escritorio anterior / siguiente (con Mayús, llevándose la ventana) | 0.1.0 |
| Meta+Esc · botón de encendido | Diálogo de apagado | 0.1.0 |
| Meta+L | Bloquear | 0.1.0 |
| Meta+Mayús+S | Captura de pantalla | 0.1.0 |
| Meta+Z · Meta+X · Meta+E | Menú BookOS · Centro de control · Explorador | 0.1.0 |
| Meta+Alt+B · Meta+Alt+D | Panel o dock: esquivar ventanas o siempre visibles | 0.1.0 |
| Meta+Alt+T | Ventana siempre encima | 0.1.0 |
| Meta+Alt+F | Panel de diagnóstico (fotogramas) | 0.1.0 |
| Alt+F3 | Menú de la ventana | 0.1.0 |
| Meta+arrastrar | Mover la ventana (botón derecho: redimensionar) | 0.1.0 |
| Ctrl+Alt+F1…F12 | Cambiar de terminal virtual | 0.1.0 |
| Meta+. | Selector de emojis | **0.1.1** |
| Meta+Alt+C | Cuentagotas: copiar el color de la pantalla | **0.1.1** |
| Meta+Alt+S · Meta+Alt+Mayús+S | Salvapantallas, en su estilo o en el otro | **0.1.1** |
| Meta+Alt+O | Apagar solo las pantallas, sin suspender | **0.1.1** |

Las teclas que Linux no usa (Copilot y similares) se pueden remapear en
`teclas.conf` desde 0.1.0. En 0.1.1 pueden apagar la pantalla o poner el
salvapantallas.

## Gestos del touchpad

| Gesto | Qué hace | Versión |
|---|---|---|
| Pellizco de 5 dedos hacia dentro / fuera | Abrir / cerrar el launchpad (con 4 hasta la 0.2.0; en la 0.2.0 se lee del touchpad, porque libinput no emite gestos de 5) | 0.1.0 |
| 4 dedos a un lado | Cambiar de escritorio | 0.1.0 |
| 4 dedos abajo | Apartar las ventanas y ver el escritorio | 0.1.0 |
| 4 dedos arriba | Devolver las ventanas o, si no hay, vista de escritorios | 0.1.0 |
| 3 dedos arriba / abajo | Ver todas las ventanas / cerrar esa vista | 0.1.0 |
| Pellizco de 2 dedos | Zoom de la aplicación (pasa a la ventana) | 0.1.0 |

**0.1.1:** la vista de todas las ventanas sale desde donde está cada una, en
vez de aparecer de golpe. Al apartar las ventanas, se van animadas por el borde
y vuelven por donde se fueron. Ahora se recuerda por escritorio, y cambiar de
escritorio ya no las devuelve. En el modo kiosko no hay gestos.

**0.2.0:** escritorios, exposición y launchpad siguen al dedo hasta soltarlo;
los dos dedos de zoom siguen reservados a la aplicación.

## Pantalla táctil y teclado en pantalla — 0.1.1

- La pantalla táctil funciona: toques y arrastres llegan a las ventanas, y
  donde no hay ventana hacen de clic.
- Teclado en pantalla, que sube desde abajo cuando una ventana pide escribir.
  Toma el color del acento.
- Se puede desactivar la pantalla táctil en Ajustes; sin ella, el teclado en
  pantalla se retira.

## Sonidos del sistema

| Sonido | Cuándo | Versión |
|---|---|---|
| `desktop-login` | Al entrar en la sesión, con el escritorio ya pintado | 0.1.0 |
| `screen-lock` · `screen-unlock` | Bloquear · desbloquear | 0.1.0 |
| `screen-capture` | Hacer una captura | 0.1.0 |
| `audio-volume-change` | Al soltar la tecla de volumen | 0.1.0 |
| `power-plug` · `power-unplug` | Conectar · desconectar el cargador | 0.1.0 |

Tema de sonido propio (BookOS, con respaldo en freedesktop) y opción de
silenciar los sonidos de la interfaz desde 0.1.0.

**0.1.1:** al suspender desde el menú ya no suena el candado. Se oía dos veces,
antes de dormir y al despertar.

## Ventanas y multitarea

| Función | Versión |
|---|---|
| Encajar arrastrando al borde: lados = mitades, esquinas = cuartos | 0.1.0 |
| Barra de título del sistema, con doble clic para maximizar | 0.1.0 |
| Minimizar con efecto «lámpara» hacia el dock; cerrar con la apertura al revés | 0.1.0 |
| Siempre encima | 0.1.0 |
| Escritorios virtuales, también por monitor | 0.1.0 |
| Mover ventanas entre escritorios y monitores | 0.1.0 |
| **Grupos lado a lado**: dos ventanas encajadas forman un grupo con separador arrastrable | **0.1.1** |
| **Grupos en pestañas**, con barra de pestañas propia | **0.1.1** |
| Unir, separar y cambiar de lado a lado a pestañas desde el menú de la ventana | **0.1.1** |
| Los diálogos van con su ventana: al minimizar, mover, cambiar de escritorio o de monitor | **0.1.1** |
| Alt+Tab muestra un grupo como una sola celda | **0.1.1** |

**0.1.1, mejoras:**
- Al maximizar una ventana de un grupo, la otra se queda en su mitad, y al
  restaurarla vuelve el grupo.
- Lado a lado, cada ventana lleva su propia barra: Firefox ya no sale con dos.
- «Siempre encima» se recuerda por aplicación y lo anuncia un aviso.
- Los diálogos nuevos salen sobre su ventana, no en una esquina, y sin el efecto
  de abrir desde el dock.
- Los menús de las aplicaciones X11 (VirtualBox) salen donde deben.
- En pantallas con escala, las aplicaciones X11 reciben el puntero en toda la
  pantalla y su cursor tiene el tamaño correcto.

## Dock

| Función | Versión |
|---|---|
| Aplicaciones ancladas y abiertas, con punto bajo las abiertas | 0.1.0 |
| Menú de cada icono (abrir otra ventana, anclar, cerrar…) | 0.1.0 |
| Esquivar ventanas o quedarse siempre visible | 0.1.0 |
| **Arrastrar iconos para reordenarlos** (el orden se guarda) o sacarlos del dock | **0.1.1** |
| El icono bota al abrir la aplicación | **0.1.1** |

**0.1.1, mejoras:** «Cerrar» cierra también las ventanas minimizadas y las
apartadas. Con la aplicación cerrada, el menú dice «Abrir». Los iconos que
cambian solos (calendario y similares) solo se recargan cuando cambian.

## Panel superior

| Widget | Versión |
|---|---|
| Reloj y calendario | 0.1.0 |
| Batería, con perfiles de energía | 0.1.0 |
| Wi-Fi y Bluetooth, con sus tarjetas | 0.1.0 |
| Volumen y brillo, con sus tarjetas | 0.1.0 |
| Notificaciones (campana con contador y «No molestar») | 0.1.0 |
| Escritorios | 0.1.0 |
| Centro de control | 0.1.0 |
| **«Desconectado» unos segundos junto al icono al perder la Wi-Fi o soltar un dispositivo Bluetooth** | **0.1.1** |

**0.1.1, centro de control:** baldosas más limpias (solo el icono), iconos
macizos, se puede pulsar la barra de la canción para saltar dentro de ella, y en
oscuro lo seleccionado se distingue mejor. La tarjeta Wi-Fi sabe qué redes están
guardadas y permite elegir si se conectan solas.

## Launchpad y buscador

| Función | Versión |
|---|---|
| Launchpad con páginas, carpetas y búsqueda | 0.1.0 |
| Buscador (Meta+Espacio) de aplicaciones y acciones del sistema | 0.1.0 |
| **Sugerencias con el campo vacío: lo que más abres** | **0.1.1** |
| **Calculadora en el buscador** | **0.1.1** |
| **Una aplicación ya abierta lleva a su ventana en vez de abrir otra** | **0.1.1** |

**0.1.1, mejoras:** en los dos, lo que más usas sale antes a igual coincidencia.
Las acciones del sistema compiten con las aplicaciones, así que «so» ya
encuentra Sonido. El buscador es más grande y ya no oscurece el fondo.

**0.2.0:** el cambio de página del launchpad y abrir o cerrar una carpeta van
con muelle en vez de con duración fija: sin rebote, y repetirlos deprisa no
da tirones.

## Escritorio

| Función | Versión |
|---|---|
| Iconos de archivos y carpetas, con selección por banda | 0.1.0 |
| Fondos de imagen y de vídeo | 0.1.0 |
| Tema claro, oscuro o automático según la hora | 0.1.0 |
| Acentos de color | 0.1.0 |
| **Menú del botón derecho propio**: abrir, papelera, nueva carpeta, cambiar fondo, pantalla, mostrar u ocultar iconos | **0.1.1** |
| **Color de carpeta** (el mismo que pone el explorador; desde la 0.2.0, con su mismo diálogo: vista previa, color libre y hexadecimal) | **0.1.1** |
| **Acento automático, sacado del fondo** | **0.1.1** |

## Notificaciones

| Función | Versión |
|---|---|
| Avisos flotantes en pila, con botones de acción | 0.1.0 |
| Responder sin abrir la aplicación | 0.1.0 |
| «No molestar» (las críticas pasan) | 0.1.0 |

**0.1.1:**
- Tarjeta más compacta: de 116 a 85 px de alto.
- Otra de la misma aplicación sustituye a la que hay delante en vez de apilarse.
- Sobre un vídeo o un juego a pantalla completa van directas a la campana; las
  críticas, no.

## Bloqueo y sesión

| Función | Versión |
|---|---|
| Pantalla de bloqueo con reloj, música y botón de apagado | 0.1.0 |
| Desbloqueo con huella | 0.1.0 |
| Diálogo de apagado: dormir, bloquear, salir, reiniciar, apagar | 0.1.0 |
| Bloqueo, atenuación y suspensión por inactividad | 0.1.0 |
| **Desbloqueo con PIN** | **0.1.1** |
| **Salvapantallas** (estela o rebote) y apagado de pantalla por inactividad, cada uno con su plazo | **0.1.1** |
| **Pantalla de acceso propia (greeter)**, con el aspecto de la sesión | **0.1.1** |

**0.1.1, mejoras:**
- Tras suspender, la huella sigue funcionando.
- Al despertar, la pantalla de bloqueo sale entera; antes faltaban cosas hasta
  pulsar una tecla.
- La pantalla ya no se enciende un instante al dormir.
- La tarjeta de música del bloqueo va en blanco sobre cristal, como el resto.

## Capturas, pantallas y compartir

| Función | Versión |
|---|---|
| Captura de pantalla o de una zona, a archivo o al portapapeles | 0.1.0 |
| Compartir pantalla en el navegador (Meet y similares), con permiso | 0.1.0 |
| Varios monitores y selector de proyección (Fn+F4) | 0.1.0 |
| Selector de archivos del sistema (explorador de BookOS) | 0.1.0 |
| **Cuentagotas** (Meta+Alt+C): copia `#RRGGBB` y avisa con una muestra; lupa desde la 0.2.0 | **0.1.1** |

## Brillo, luz y energía

| Función | Versión |
|---|---|
| Teclas de volumen, brillo y touchpad, con aviso en pantalla | 0.1.0 |
| Brillo automático (pantalla y luz del teclado) | 0.1.0 |
| Luz nocturna | 0.1.0 |
| Aviso al conectar o desconectar el cargador | 0.1.0 |

**0.1.1:** el brillo automático hace transiciones suaves que se redirigen si
cambia la luz. La luz del teclado se recupera si se recarga su driver.

## Isla de actividades y música — 0.1.0

La isla muestra la canción en curso con carátula, la batería de los Galaxy Buds
(`bookos-budsd`) y otras actividades del sistema.

**0.1.1:** gasta menos memoria; ya no copia las carátulas cada segundo.

## Rendimiento — 0.1.1

- Menos memoria en reposo: un asignador ajustado (de 70,4 a 52,3 MB anónimos tras
  abrir y cerrar launchpad, centro y buscador) y los iconos enormes se reducen
  antes de guardarse.
- El panel, el dock y las tarjetas reutilizan sus búferes en vez de crearlos
  en cada repintado.

## Modo kiosko — 0.1.1

`bookos-comp --kiosko programa`: solo ese programa, a pantalla completa, sin
escritorio, atajos ni gestos. Lo usa el asistente; el compositor termina con el
código de salida del programa.
