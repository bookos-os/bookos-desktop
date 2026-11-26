#!/usr/bin/env bash
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "Hace falta root: sudo bash session/plymouth/instalar.sh" >&2
    exit 1
fi

aqui="$(cd "$(dirname "$0")" && pwd)"
origen="$aqui/bookos"
destino="/usr/share/plymouth/themes/bookos"

if [ ! -f "$origen/watermark.png" ] || [ ! -f "$origen/throbber-0001.png" ]; then
    python3 "$aqui/generar.py"
fi

install -d "$destino"
install -m 644 "$origen/bookos.plymouth" "$destino/bookos.plymouth"
install -m 644 "$origen"/*.png "$destino/"

# Conserva los glifos de desbloqueo de disco (LUKS) del tema spinner si existen.
for recurso in lock.png entry.png bullet.png capslock.png keyboard.png keymap-render.png; do
    if [ -f "/usr/share/plymouth/themes/spinner/$recurso" ] && [ ! -e "$destino/$recurso" ]; then
        install -m 644 "/usr/share/plymouth/themes/spinner/$recurso" "$destino/$recurso"
    fi
done

if command -v plymouth-set-default-theme >/dev/null 2>&1; then
    plymouth-set-default-theme -R bookos
    echo "Tema Plymouth 'bookos' activado e initramfs regenerado."
else
    echo "Aviso: no se encontró plymouth-set-default-theme; ficheros copiados en $destino."
fi
