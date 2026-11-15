#!/usr/bin/env bash
# Compila e instala la versión del árbol de trabajo: lo de siempre tras un
# cambio, en una orden. Se ejecuta como usuario normal, **no** con sudo: pide la
# contraseña solo para instalar.
#
#     ./actualizar.sh
#
# Después hay que cerrar sesión y volver a entrar: el compositor que está
# corriendo sigue siendo el viejo hasta que se relanza.
set -eu

if [ "$(id -u)" -eq 0 ]; then
    # Con sudo, cargo compilaría como root y dejaría target/ con ficheros de
    # root: el siguiente `cargo test` de la usuaria fallaría sin poder
    # escribirlos.
    echo "Ejecútalo sin sudo: ./actualizar.sh (la contraseña se pide al instalar)." >&2
    exit 1
fi

aqui="$(cd "$(dirname "$0")" && pwd)"
cd "$aqui"

# El greeter solo se toca si ya está puesto: instalarlo de cero crea una cuenta
# de sistema y cambia greetd, y eso no es «actualizar».
paquetes=(-p bookos-comp -p bookos-system -p bookos-budsd)
greeter=0
if [ -x /usr/local/libexec/bookos-greeter ]; then
    greeter=1
    paquetes+=(-p bookos-greeter -p bookos-pam-huella)
fi

echo "==> Compilando en release: ${paquetes[*]}"
cargo build --release "${paquetes[@]}"

echo "==> Instalando la sesión (pide la contraseña)"
sudo "$aqui/session/instalar.sh"
if [ "$greeter" -eq 1 ]; then
    echo "==> Instalando la pantalla de acceso"
    sudo bash "$aqui/session/greeter/instalar.sh"
fi

# Lo que se instaló tiene que ser lo recién compilado. Ya pasó que la sesión
# llevaba un bookos-comp anterior a los cambios y se perdió el tiempo
# depurando código que no estaba corriendo.
comprobar() {
    if ! cmp -s "target/release/$1" "$2"; then
        echo "ERROR: $2 no coincide con target/release/$1" >&2
        exit 1
    fi
}
comprobar bookos-comp /usr/local/bin/bookos-comp
comprobar bookos-system /usr/local/libexec/bookos-system
comprobar bookos-budsd /usr/local/libexec/bookos-budsd
if [ "$greeter" -eq 1 ]; then
    comprobar bookos-greeter /usr/local/libexec/bookos-greeter
fi

echo
echo "Listo. Cierra sesión y vuelve a entrar para usar la versión nueva."
