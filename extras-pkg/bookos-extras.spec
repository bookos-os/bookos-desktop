Name:           bookos-extras
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        Navegación táctil estilo Android y cortina de notificaciones ampliada para BookOS
License:        LicenseRef-BookOS
URL:            https://github.com/bookos/bookos-desktop

%description
Paquete opcional de extras para BookOS Desktop. Añade navegación táctil por
gestos desde los bordes (estilo Android) y una cortina ampliada de
notificaciones en Wayland (wlr-layer-shell). No se instala por defecto con
el escritorio base.

%install
install -Dm755 "%{binary_path}" "%{buildroot}%{_bindir}/bookos-extras"
install -Dm644 "%{service_path}" "%{buildroot}%{_userunitdir}/bookos-extras.service"
install -d "%{buildroot}%{_userunitdir}/bookos-session.target.wants"
ln -s ../bookos-extras.service "%{buildroot}%{_userunitdir}/bookos-session.target.wants/bookos-extras.service"
install -Dm644 "%{readme_path}" "%{buildroot}%{_docdir}/bookos-extras/README.md"

%files
%{_bindir}/bookos-extras
%{_userunitdir}/bookos-extras.service
%{_userunitdir}/bookos-session.target.wants/bookos-extras.service
%doc %{_docdir}/bookos-extras/README.md

%changelog
* Mon Nov 23 2026 BookOS <bookos@localhost> - 0.2.0-1
- Primera versión del paquete opcional bookos-extras (navegación táctil y cortina ampliada de notificaciones)
