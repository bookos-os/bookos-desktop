# Acceso de BookOS

`bookos-greeter` es una aplicación iced 0.14 con renderizado por CPU. Cage
muestra únicamente esta aplicación; greetd gestiona PAM y arranca
`/usr/local/bin/bookos-session` con la identidad del usuario autenticado.
Al recibir la confirmación de arranque, el greeter termina y Cage libera
el asiento para el compositor de BookOS.

Comparte tokens, iconos y campo con el bloqueo de Meta+L (`bookos-shell`), con
la disposición de la ventana de acceso de macOS: fecha y hora arriba, teñidas
con la paleta, y abajo las cuentas —avatar y nombre sueltos sobre el fondo—,
que crecen al acercar el puntero.
Elegir una lleva al bloque de avatar, nombre y campo; pulsar el nombre o Esc
vuelve a la lista. Arriba a la derecha van los controles interactivos de
idioma y distribución de teclado (`Alt+Shift`), batería y modo de energía,
red Wi-Fi (activar/desactivar, escanear, conectar con contraseña o desconectar)
y el menú de apagar, reiniciar, suspender e hibernar. El campo de contraseña
incluye botón para mostrar u ocultar los caracteres y aviso de `Bloq Mayús`.

Incluye entrada manual de usuario («Otro usuario»), preguntas PAM secretas y
visibles, mensajes informativos, cancelación y reintento. No presupone que la primera
pregunta sea una contraseña: permite, por ejemplo, responder a un segundo
factor o a una solicitud de cambio de contraseña si PAM los pide.
La vista previa nunca conecta con greetd.

## Compilar y previsualizar

```sh
cargo build --release -p bookos-greeter
cargo run -p bookos-greeter -- --preview
cargo test -p bookos-greeter
cargo clippy -p bookos-greeter --all-targets -- -D warnings
```

La vista previa abre una ventana en la sesión Wayland actual. Fuera de ella,
el binario exige `GREETD_SOCK`, que proporciona greetd; no autentica por su
cuenta ni necesita ejecutarse como root.

## Instalar

En Fedora, instala las dependencias de ejecución:

```sh
sudo dnf install greetd cage
```

BookOS debe estar instalado previamente con `session/instalar.sh`, incluidos
el compositor y el programa de sesión. El instalador del greeter comprueba
también `/etc/pam.d/greetd` o `/usr/lib/pam.d/greetd`: se utiliza la política de login de la distribución,
sin sustituirla por la política de desbloqueo de BookOS.

El cage de serie no configura libinput y mueve el puntero en píxeles de una
salida a escala 1: el touchpad va unas 2,5 veces más lento que en la sesión y
el toque no hace clic. `cage/bookos-entrada.patch` le hace leer del entorno la
velocidad, el toque, el scroll natural y la escala del puntero, que
`bookos-greeter-start` saca de lo que publica la primera cuenta. Se compila en
un contenedor de Fedora (hace falta `podman`) y queda en `target/bookos-cage`:

```sh
bash session/greeter/cage/compilar.sh
sudo bash session/greeter/instalar.sh
```

Si falta, el arranque cae al `cage` del sistema.

Esto instala los binarios, crea la cuenta de sistema `bookos-greeter` y configura
`greetd.service` mediante un drop-in. Conserva los archivos de configuración
existentes y no habilita ni reinicia servicios. Para instalar una compilación
debug, pasa `BOOKOS_GREETER_BIN` con su ruta absoluta mediante `sudo env`.
`DESTDIR` permite preparar los archivos en un directorio de empaquetado sin
crear usuarios ni ejecutar systemd.

El teclado y el idioma de bienvenida son los del sistema (la distribución X11
de `localectl` y el `LANG` de `/etc/locale.conf`, que es lo que deja el
asistente de primer arranque), y `es` si no hay ninguno. Puedes fijarlos en
`/etc/default/bookos-greeter` (por ejemplo `XKB_DEFAULT_LAYOUT=us`) o
directamente desde el desplegable de idioma y teclado en la esquina superior
derecha, que guarda la preferencia en `/var/lib/bookos-greeter/greeter.env`,
traduce al vuelo los caracteres imprimibles y pasa `XKB_DEFAULT_LAYOUT` y
`LANG` a la sesión de BookOS al entrar.

## Apariencia de cada usuario

El greeter no puede leer `~/.config`, así que el compositor copia lo necesario
a `/var/lib/bookos/apariencia/<login>/` al iniciar la sesión real y cada vez que
se aplican ajustes: las claves de tema, acento, efectos, escala, teclado y reloj del
bloqueo de `panel.conf`, la `palette.css` de BookOS Settings (de ahí salen el
acento y el tono del reloj), el fondo de cada tema y el avatar. La carpeta raíz
admite escritura de cualquiera (como `/tmp`), por lo que el greeter solo acepta
la subcarpeta cuyo dueño es el uid de esa cuenta. Sin nada publicado se usan el
tema oscuro, el fondo de serie y el icono de AccountsService. Un fondo de vídeo
no se publica.

Cage no escala la salida. El greeter aplica la escala guardada en BookOS o, si
no hay, la deduce del alto del panel (1,75 en 2880×1800). La aplica multiplicando
sus medidas, no con el `scale_factor` de iced, y parte el fondo en losas de 256:
así un fotograma de la lente repinta solo la zona de la lista (medido a
2880×1800: de 50 ms a 14).

Para ver la vista previa con una carpeta publicada de prueba:
`BOOKOS_APARIENCIA_PUBLICA=/ruta cargo run -p bookos-greeter -- --preview`.

## Activar y recuperar

Haz el cambio desde una TTY y conserva el nombre de tu gestor anterior:

```sh
readlink -f /etc/systemd/system/display-manager.service
```

Sea cual sea el gestor actual (SDDM, `plasmalogin`…), una sola orden lo
sustituye:

```sh
sudo systemctl enable --force greetd.service
sudo reboot
```

`--force` reescribe el enlace `display-manager.service`. No lo hagas en dos
pasos (`disable` del anterior y `enable` de greetd): si el segundo se queda sin
ejecutar, el equipo arranca sin gestor y solo ofrece el `login` de texto.
No uses `--now` desde una sesión gráfica: detenería esa sesión. Cage permite cambiar
a otra TTY con Ctrl+Alt+F2 para consultar errores:

```sh
sudo journalctl -b -u greetd.service
```

Para volver al gestor anterior desde esa TTY (aquí `plasmalogin`):

```sh
sudo systemctl enable --force plasmalogin.service
sudo reboot
```

Para devolver greetd a la configuración de su paquete, elimina únicamente
`/etc/systemd/system/greetd.service.d/bookos.conf` y ejecuta
`sudo systemctl daemon-reload`.

## Alcance y validación

No incluye selector de sesiones ni teclado virtual en pantalla. La sesión está
fijada a BookOS y Cage usa su comportamiento predeterminado de monitores. La
lista sale de `/etc/passwd` (uid 1000–59999 con shell de entrada); las cuentas
de red entran por «Otro usuario». La autenticación sigue siendo la de greetd,
no la del desbloqueo: el login y el desbloqueo tienen identidades y ciclos de
vida distintos.

Las pruebas con IPC simulado verifican el protocolo y la transición de estados;
no sustituyen una prueba en una TTY con el PAM y el hardware de la instalación.
Antes de distribuirlo, comprueba allí contraseña correcta e incorrecta,
cancelación, salida de BookOS y regreso al login, cambio a otra TTY y reinicio,
y que apagar y suspender funcionan desde la cuenta `bookos-greeter`.

Referencias: [protocolo de greetd](https://github.com/kennylevinsen/greetd/tree/master/greetd_ipc),
[Cage](https://github.com/cage-kiosk/cage).
