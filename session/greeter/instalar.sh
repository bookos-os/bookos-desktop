#!/usr/bin/env bash
set -eu

aqui="$(cd "$(dirname "$0")" && pwd)"
destino="${DESTDIR:-}"
binario="${BOOKOS_GREETER_BIN:-$aqui/../../target/release/bookos-greeter}"

if [ ! -x "$binario" ]; then
    echo "Compila primero: cargo build --release -p bookos-greeter" >&2
    exit 1
fi
greetd_bin=/usr/bin/greetd
if [ -z "$destino" ]; then
    if [ "$(id -u)" -ne 0 ]; then
        echo "Hace falta root: sudo bash session/greeter/instalar.sh" >&2
        exit 1
    fi
    for programa in greetd cage dbus-run-session; do
        if ! command -v "$programa" >/dev/null 2>&1; then
            echo "Falta $programa; instala greetd, cage y dbus antes de continuar." >&2
            exit 1
        fi
    done
    if { [ ! -f /etc/pam.d/greetd ] && [ ! -f /usr/lib/pam.d/greetd ]; } || [ ! -x /usr/local/bin/bookos-session ]; then
        echo "Hace falta la política PAM de greetd y BookOS instalado con session/instalar.sh." >&2
        exit 1
    fi
    greetd_bin="$(command -v greetd)"
    if ! id bookos-greeter >/dev/null 2>&1; then
        useradd --system --create-home --home-dir /var/lib/bookos-greeter \
            --shell /usr/sbin/nologin bookos-greeter
    fi
fi

install -Dm755 "$binario" "$destino/usr/local/libexec/bookos-greeter"
# Sin el cage parcheado se arranca con el de serie: funciona, pero con el
# touchpad lento y sin toque para hacer clic.
cage_bin="${BOOKOS_CAGE_BIN:-$aqui/../../target/bookos-cage}"
if [ -x "$cage_bin" ]; then
    install -Dm755 "$cage_bin" "$destino/usr/local/libexec/bookos-cage"
else
    echo "Aviso: no está $cage_bin; compílalo con session/greeter/cage/compilar.sh." >&2
fi
# Cada sesión deja aquí su apariencia; con el bit pegajoso, como /tmp, nadie
# puede borrar ni sustituir la carpeta de otro.
install -d -m 1777 "$destino/var/lib/bookos/apariencia"
install -Dm755 "$aqui/bookos-greeter-start" "$destino/usr/local/libexec/bookos-greeter-start"
# La reinstalación actualiza el ejecutable, pero conserva los ajustes locales.
if [ ! -e "$destino/etc/greetd/bookos.toml" ]; then
    install -Dm644 "$aqui/bookos.toml" "$destino/etc/greetd/bookos.toml"
fi
# /etc/default/bookos-greeter ya no se crea: con XKB_DEFAULT_LAYOUT=es dentro
# tapaba la distribución del sistema, la que deja el asistente de primer
# arranque. Sin él, bookos-greeter-start la toma de localed.
install -Dm644 "$aqui/50-bookos-greeter.rules" "$destino/etc/polkit-1/rules.d/50-bookos-greeter.rules"
install -d "$destino/etc/systemd/system/greetd.service.d"
if [ ! -e "$destino/etc/systemd/system/greetd.service.d/bookos.conf" ]; then
    printf '[Service]\nExecStart=\nExecStart=%s --config /etc/greetd/bookos.toml\n' \
        "$greetd_bin" > "$destino/etc/systemd/system/greetd.service.d/bookos.conf"
    chmod 644 "$destino/etc/systemd/system/greetd.service.d/bookos.conf"
fi
if [ -z "$destino" ]; then
    systemctl daemon-reload
fi
echo "Greeter instalado. No se ha activado greetd ni detenido el gestor actual."
echo "Consulta session/greeter/README.md para activarlo y volver al gestor anterior."
