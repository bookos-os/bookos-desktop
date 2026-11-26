#!/usr/bin/env bash
# Compila `bookos-extras` y genera paquetes instalables .rpm y .deb en extras-pkg/dist/.
set -euo pipefail
umask 022

aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/.." && pwd)"
cd "$raiz"

version="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
arch_rpm="$(uname -m)"
case "$arch_rpm" in
    x86_64)  arch_deb="amd64" ;;
    aarch64) arch_deb="arm64" ;;
    *)       arch_deb="$arch_rpm" ;;
esac

echo "==> Compilando bookos-extras v${version} (release)"
cargo build --release -p bookos-extras --offline

binario="$raiz/target/release/bookos-extras"
servicio="$aqui/bookos-extras.service"
readme="$aqui/README.md"
dist="$aqui/dist"
mkdir -p "$dist"

# --- Paquete RPM ------------------------------------------------------------
if command -v rpmbuild >/dev/null 2>&1; then
    echo "==> Generando paquete RPM (${arch_rpm})"
    rpm_top="$(mktemp -d)"
    trap 'rm -rf "$rpm_top"' EXIT
    mkdir -p "$rpm_top"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,TMP}
    rpmbuild -bb "$aqui/bookos-extras.spec" \
        --define "_topdir $rpm_top" \
        --define "_tmppath $rpm_top/TMP" \
        --define "pkg_version $version" \
        --define "binary_path $binario" \
        --define "service_path $servicio" \
        --define "readme_path $readme" \
        --define "_build_id_links none" \
        --nocheck
    find "$rpm_top/RPMS" -name '*.rpm' -exec cp -v {} "$dist/" \;
else
    echo "Aviso: rpmbuild no disponible, se omite .rpm"
fi

# --- Paquete DEB ------------------------------------------------------------
if command -v dpkg-deb >/dev/null 2>&1; then
    echo "==> Generando paquete DEB (${arch_deb})"
    deb_root="$(mktemp -d)"
    chmod 755 "$deb_root"
    mkdir -p "$deb_root/DEBIAN"
    chmod 755 "$deb_root/DEBIAN"
    mkdir -p "$deb_root/usr/bin"
    mkdir -p "$deb_root/usr/lib/systemd/user/bookos-session.target.wants"
    mkdir -p "$deb_root/usr/share/doc/bookos-extras"

    install -m755 "$binario" "$deb_root/usr/bin/bookos-extras"
    install -m644 "$servicio" "$deb_root/usr/lib/systemd/user/bookos-extras.service"
    ln -s ../bookos-extras.service \
        "$deb_root/usr/lib/systemd/user/bookos-session.target.wants/bookos-extras.service"
    install -m644 "$readme" "$deb_root/usr/share/doc/bookos-extras/README.md"

    cat > "$deb_root/DEBIAN/control" <<EOF
Package: bookos-extras
Version: ${version}-1
Section: utils
Priority: optional
Architecture: ${arch_deb}
Maintainer: BookOS <bookos@localhost>
Description: Navegación táctil estilo Android y cortina de notificaciones ampliada para BookOS
 Paquete opcional de extras para BookOS Desktop. No se instala por defecto
 con el escritorio base.
EOF

    deb_out="$dist/bookos-extras_${version}-1_${arch_deb}.deb"
    dpkg-deb --build --root-owner-group "$deb_root" "$deb_out"
    rm -rf "$deb_root"
    echo "Generado: $deb_out"
else
    echo "Aviso: dpkg-deb no disponible, se omite .deb"
fi

echo
echo "Paquetes listos en: $dist"
ls -lh "$dist"
