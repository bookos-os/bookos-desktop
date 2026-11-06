#!/usr/bin/env bash
# Deja BookOS en la lista de sesiones del gestor de login.
#
# Hace falta root: /usr/share/wayland-sessions es del sistema. Se instala el
# guion en /usr/local/bin y la entrada .desktop, que es lo que lee SDDM (y
# GDM, y greetd) para saber qué sesiones ofrecer.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "Hace falta root: sudo $0" >&2
    exit 1
fi

aqui="$(cd "$(dirname "$0")" && pwd)"
# La recuperación gráfica necesita un compositor independiente y Qt/Python.
if ! command -v kwin_wayland >/dev/null 2>&1 || ! python3 -c 'import PySide6.QtWidgets' >/dev/null 2>&1; then
    echo "La recuperación gráfica requiere kwin_wayland, python3 y PySide6 (QtWidgets)." >&2
    exit 1
fi
if [ -e /etc/pam.d/system-auth ]; then
    cuenta_pam=system-auth
elif [ -e /etc/pam.d/common-account ]; then
    cuenta_pam=common-account
else
    echo "No se reconoce la política PAM de cuentas; no se instalarán los servicios de bloqueo." >&2
    cuenta_pam=""
fi
# Solo release. Antes se instalaba el más reciente de release y debug, y con
# un `cargo build` de desarrollo de por medio acababa instalado el de
# depuración: el 2026-10-15 bookos-comp (577 MB, sin optimizar) gastaba un 63 %
# de CPU de forma continua y calentaba el equipo hasta encender los
# ventiladores en el modo silencioso. No se comparan fechas con el de
# depuración: cargo no reenlaza en release lo que no ha cambiado, y quedaría
# «más viejo» estando al día.
binario_release() {
    local release="$aqui/../target/release/$1"
    if [ ! -x "$release" ]; then
        echo "Compila primero: cargo build --release -p $1" >&2
        exit 1
    fi
    echo "$release"
}
comp_bin="$(binario_release bookos-comp)"
# Build before installing so D-Bus never points to a missing executable.
system_bin="$(binario_release bookos-system)"
budsd_bin="$(binario_release bookos-budsd)"
# Las unidades vienen con la ruta del RPM (/usr/libexec); en desarrollo el
# binario va a /usr/local/libexec, así que aquí se reescribe al vuelo. Antes
# era al revés y el paquete instalado quedaba apuntando a /usr/local.
install -Dm755 "$system_bin" /usr/local/libexec/bookos-system
install -Dm755 "$budsd_bin" /usr/local/libexec/bookos-budsd
install -Dm755 "$comp_bin" /usr/local/bin/bookos-comp
# Por `mktemp` y no por una ruta fija de /tmp: esto corre como root, y un
# `> /tmp/nombre-que-se-sabe` lo puede haber dejado preparado cualquiera como
# enlace a un fichero del sistema, que es entonces lo que se sobrescribe. Los
# ficheros PAM de más abajo ya lo hacían así.
temporal_unidad="$(mktemp)"
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/org.bookos.System1.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/share/dbus-1/services/org.bookos.System1.service
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/bookos-system.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/lib/systemd/user/bookos-system.service
sed 's|/usr/libexec/bookos-budsd|/usr/local/libexec/bookos-budsd|' \
    "$aqui/org.bookos.Buds.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/share/dbus-1/services/org.bookos.Buds.service
sed 's|/usr/libexec/bookos-budsd|/usr/local/libexec/bookos-budsd|' \
    "$aqui/bookos-budsd.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/lib/systemd/user/bookos-budsd.service
rm -f -- "$temporal_unidad"
# El target que declara la sesión gráfica. Sin él no arranca el portal: ver el
# comentario de bookos-session.target.
install -Dm644 "$aqui/bookos-session.target" /usr/local/lib/systemd/user/bookos-session.target

install -Dm755 "$aqui/bookos-session" /usr/local/bin/bookos-session
install -Dm644 "$aqui/bookos-recovery.py" /usr/local/libexec/bookos-recovery.py
install -Dm755 "$aqui/bookos-restaurar" /usr/local/libexec/bookos-restaurar
install -Dm644 "$aqui/bookos.desktop" /usr/share/wayland-sessions/bookos.desktop
# Lanzadores del launchpad: solo la entrada, no el binario —cada app de
# BookOS se instala por su cuenta y esto es lo que la hace aparecer en el
# launchpad y resolverle el icono. Si la app en sí no está instalada, el
# lanzador queda ahí sin más: el mismo trato que le da freedesktop a
# cualquier .desktop huérfano.
for lanzador in "$aqui"/../data/applications/*.desktop; do
    install -Dm644 "$lanzador" "/usr/local/share/applications/$(basename "$lanzador")"
done
if [ -n "$cuenta_pam" ] && [ ! -e /etc/pam.d/bookos ]; then
    temporal_pam="$(mktemp)"
    sed "s/account include system-auth/account include $cuenta_pam/" "$aqui/bookos.pam" > "$temporal_pam"
    install -Dm644 "$temporal_pam" /etc/pam.d/bookos
    rm -f -- "$temporal_pam"
fi
if [ -n "$cuenta_pam" ] && [ ! -e /etc/pam.d/bookos-fingerprint ]; then
    temporal_pam="$(mktemp)"
    sed "s/account include system-auth/account include $cuenta_pam/" "$aqui/bookos-fingerprint.pam" > "$temporal_pam"
    install -Dm644 "$temporal_pam" /etc/pam.d/bookos-fingerprint
    rm -f -- "$temporal_pam"
fi

# El backend de portales. El compositor implementa ScreenCast y Screenshot
# dentro de su propio proceso, así que aquí solo se declara quién los atiende:
# el .portal dice qué interfaces hay detrás del nombre de bus, y el
# bookos-portals.conf dice que en esta sesión mandan esas y no las de GTK.
#
# En minúsculas: xdg-desktop-portal pasa `XDG_CURRENT_DESKTOP` a minúsculas
# antes de buscar `<escritorio>-portals.conf` (portals.conf(5)). Con el nombre
# `BookOS-portals.conf` no lo encontraba y el selector de archivos y el resto
# de portales caían en xdg-desktop-portal-gtk. Se borra el viejo para que no
# quede un fichero muerto que parezca el bueno.
install -Dm644 "$aqui/bookos.portal" /usr/share/xdg-desktop-portal/portals/bookos.portal
install -Dm644 "$aqui/bookos-portals.conf" /usr/share/xdg-desktop-portal/bookos-portals.conf
rm -f /usr/share/xdg-desktop-portal/BookOS-portals.conf

# La tapa de la cámara del Galaxy Book: el compositor la lee directamente para
# el aviso de «Cámara desactivada». Va en /etc y no en /usr/local, que udev no
# lee; se recarga y se aplica ya para no tener que reiniciar.
install -Dm644 "$aqui/70-bookos-camara.rules" /etc/udev/rules.d/70-bookos-camara.rules
udevadm control --reload-rules || true
udevadm trigger --subsystem-match=input --action=change || true

# El número de serie de «Acerca de este PC», legible para `wheel`.
install -Dm644 "$aqui/71-bookos-serie.rules" /etc/udev/rules.d/71-bookos-serie.rules
udevadm control --reload-rules || true
udevadm trigger --subsystem-match=dmi --action=change || true

echo "Instalado. En la pantalla de login ya sale 'BookOS' en la lista de sesiones."
echo "El registro de cada arranque queda en \$XDG_RUNTIME_DIR/bookos-session.log"
