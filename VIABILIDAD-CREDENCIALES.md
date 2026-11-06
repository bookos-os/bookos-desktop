# BookOS — Credenciales, contraseñas y biometría: viabilidad y complejidad

Solo análisis. No se ha tocado código. Fecha: 2026-11-02.

## 1. Veredicto

| Pieza | Viabilidad | Complejidad | Esfuerzo orientativo |
|---|---|---|---|
| Servicio Secret Service propio con oo7 (`BookOS Secrets`) | Media: oo7 aún es pre-1.0 | Media | 2–3 semanas de integración, no de desarrollo |
| Agente polkit con huella | Alta | Media | 2–3 semanas |
| Llavero desbloqueado con el login (PAM) | Alta | Media | 1 semana + pruebas |
| Página «Contraseñas» en Ajustes (sobre Secret Service) | Alta | Baja–media | 1–2 semanas |
| App `BookOS Passwords` en GPUI | Alta | Alta (es una app entera) | 4–6 semanas |
| Huella → autorizar operaciones del sistema (polkit, sudo) | Alta | Baja–media | incluido en el agente |
| Huella → desbloquear el llavero | **No viable tal cual** | — | ver §5 |
| Passkeys / WebAuthn con huella en Firefox/Chromium | Baja a medio plazo | Muy alta | no recomendable ahora |

Las estimaciones son mías, no medidas. Lo que sí está comprobado en esta máquina y en el repo va marcado como tal.

## 2. Estado real de la máquina y el repo (comprobado)

- `fprintd 1.94.5` y `fprintd-pam` instalados; el repo ya tiene `crates/bookos-pam-huella` (409 líneas), servicios PAM separados (`/etc/pam.d/bookos`, `bookos-fingerprint`, `bookos-greetd`) y el bloqueo con huella validado en lector real. **Es la base más valiosa y ya está hecha.**
- `libfido2 1.16` instalado (solo biblioteca; no hay nada de passkeys).
- KWallet está presente (`kf6-kwallet`, `pam-kwallet`, `kwalletmanager5`) y existe una cartera real en `~/.local/share/kwalletd/kdewallet.kwl`. En D-Bus solo aparecen **activables** `kwalletd6` y `org.kde.secretservicecompat`; **nadie posee `org.freedesktop.secrets` ahora mismo**, ningún `kwalletd` corre y no hay agente polkit de sesión (solo `polkitd`). No hay oo7, gnome-keyring ni seahorse instalados.
- El README de BookOS ya lista como pendiente P0: «**A polkit agent and a keyring**» (sin agente, lo que pide administrador falla sin diálogo; sin llavero, navegadores y editores no guardan contraseñas). Este trabajo cierra un hueco ya reconocido.
- `/etc/pam.d/bookos-greetd` ya tiene `-auth optional pam_gnome_keyring.so` y `-session optional pam_gnome_keyring.so auto_start`: el `-` hace que fallen en silencio al no estar instalado. Es el sitio exacto donde enganchar el desbloqueo del llavero.
- Ajustes (`BookOS-Settings`) es **Tauri 2 + JS vanilla**, no GPUI. Backend `src-tauri/src/main.rs` (5 945 líneas), `zbus 4` ya como dependencia, páginas en `src/modules/pages.js` (la de seguridad, con `bloqueo_huella`, ya existe alrededor de la línea 2697). El README de Ajustes está desfasado (habla de Arch y 0.4.0; el sistema real es Fedora).

## 3. oo7 / GNOME Keyring / KWallet

**oo7** (verificado en [su repositorio](https://github.com/linux-credentials/oo7) y [Fedora](https://packages.fedoraproject.org/pkgs/oo7/oo7-daemon)): no es solo una biblioteca. Incluye cliente, `oo7-daemon` (servidor Secret Service), PAM, portal `impl.portal.Secret`, un reemplazo de `secret-tool`, helpers para git y cargo. Objetivo declarado: sustituir a gnome-keyring. Pero:

- crates.io marca 0.6.0 (feb 2026) como release inestable; Fedora empaqueta `0.7.0~alpha` solo en Rawhide/45. **En Fedora 44 habría que compilarlo nosotros.**
- Desbloqueo: `--login` o una credencial systemd (`oo7.keyring-encryption-password`) que exige systemd ≥ 258.
- Convivir con gnome-keyring exige matarlo antes; no hay arbitraje, quien llega primero se queda el nombre de bus.
- No encontré un anuncio fechado de «listo para producción». Trátese como alfa avanzada.

| | oo7-daemon | GNOME Keyring | KWallet (ksecretd) |
|---|---|---|---|
| Lenguaje | Rust | C | C++/Qt |
| Secret Service | Sí | Sí (referencia) | Sí, vía `ksecretd` desde KF 6.14, con errores abiertos ([bug 504656](https://bugs.kde.org/show_bug.cgi?id=504656), [504312](https://bugs.kde.org/show_bug.cgi?id=504312)) |
| Madurez | Alfa/beta | Muy alta | Alta en KDE, reciente la capa Secret Service |
| Dependencias | Pocas | GLib | Qt/KF6 (aquí ya instaladas) |
| Fedora 44 | Compilar | Paquete | Ya instalado |
| Memoria | Pequeña (Rust) | Pequeña | Mayor (Qt) |
| Riesgo de conflicto | Alto con los otros dos | Alto con KWallet | Ya hay cartera del usuario |

**Recomendación:** no escribir un servicio nuevo. Dos caminos reales:

- **A (pragmático, recomendado para empezar):** usar el Secret Service que ya hay (KWallet/`ksecretd`, ya instalado, con la cartera del usuario) y construir encima. Cero criptografía nueva y cero mantenimiento de un daemon. Desventaja: arrastra Qt/KF6 residente y tiene bugs abiertos.
- **B (a medio plazo):** oo7-daemon compilado a RPM propio, con el cliente/PAM de oo7. Mejor encaje con «Rust, ligero, sin Qt», pero se asume el riesgo de una alfa guardando las contraseñas del usuario. Solo cuando haya versión 0.7 estable.

Elegir uno y **desactivar los otros por activación D-Bus** (un solo `org.freedesktop.secrets`). Migrar la cartera de KWallet al formato de oo7 requiere herramienta propia; no la he comprobado.

## 4. Arquitectura propuesta (modular)

```
Ajustes (Tauri)  /  BookOS Passwords (GPUI, opcional)
        │  D-Bus org.freedesktop.Secret.* (zbus, ya en el repo)
        ▼
 Secret Service  ──(A: ksecretd | B: oo7-daemon)──  archivo cifrado en ~/.local/share
        ▲
 PAM sesión (desbloqueo con contraseña de login)

 Autorización:  polkit ◄── bookos-polkit-agent ──► fprintd (huella) / contraseña
 Passkeys:      aparte, no mezclar (§6)
```

Reglas: PAM, Secret Service, polkit y WebAuthn son cuatro sistemas distintos. Cada uno con su proceso o sin proceso propio:

1. **Almacén**: el Secret Service elegido. Ningún secreto en texto plano ni en el JSON de Ajustes.
2. **Agente polkit**: preferiblemente **dentro de `bookos-comp`/`bookos-shell`**, que ya hospeda el bloqueo, PAM en hilo aparte y diálogos propios (`autenticar.rs`, 579 líneas, ya implementa conversación PAM). Un daemon más contradice «evitar procesos residentes». Hay que implementar `org.freedesktop.PolicyKit1.AuthenticationAgent` y llamar al helper `polkit-agent-helper-1` (presente en `/usr/lib/polkit-1`). El diálogo reutiliza el estilo del bloqueo y el servicio `bookos-fingerprint`.
3. **Interfaz**: primero una página en Ajustes sobre el Secret Service; la app GPUI independiente solo si Ajustes se queda corto.
4. **Aislamiento (Flatpak)**: el portal `impl.portal.Secret` reparte un secreto por app. Hay que añadirlo a `session/bookos-portals.conf`; oo7 trae su portal, KWallet tiene `impl.portal.desktop.kwallet` (visible ya en el bus).

## 5. Cómo usar la huella — y su límite de seguridad

- **Autorizar operaciones (polkit, sudo, instalación de paquetes):** viable. `pam_fprintd` + agente polkit. Es lo que ya hace el bloqueo.
- **Desbloquear el llavero con la huella: no.** La huella solo dice «coincide»; no produce una clave. El llavero se cifra con una clave derivada de la contraseña, así que **tras un arranque en frío siempre hay que introducir la contraseña** (igual que macOS tras reiniciar). El desbloqueo en el login con `pam_gnome_keyring`/`pam-kwallet`/`oo7 --login` funciona si se entra con contraseña. Si se entra con huella (ruta `done` de `pam_bookos_huella`), PAM **no recibe la contraseña** y el llavero queda bloqueado; hay que decidirlo en el diseño: o la primera entrada tras arrancar exige contraseña, o se pide la contraseña al abrir el llavero.
- **Rellenar una contraseña con la huella** (Touch ID como en Safari) es un control de UI: pedir huella a polkit/fprintd antes de revelar un secreto *ya desbloqueado*. Es una puerta de comodidad, no cifrado. Hay que decirlo así al usuario.
- Alternativa criptográficamente correcta: TPM2 (`systemd-creds`, el propio mecanismo que usa oo7) para sellar la clave a la máquina. No investigado a fondo; la Galaxy Book5 Pro debería tener TPM, no lo he comprobado.

Riesgos de huella aquí: el lector Egis necesitó un parche propio (ver `libfprint-fmv-etu906axx-e-fix4Book5Pro` y la memoria del proyecto), `fprintd` llegó a borrar la huella y a fallar tras suspender. Cualquier flujo nuevo hereda esa fragilidad: la contraseña debe seguir siempre disponible.

## 6. Passkeys / WebAuthn

Es lo más difícil y lo menos maduro en Linux:

- Firefox y Chromium no usan un «Touch ID del sistema» para passkeys. Usan sus propios flujos (llaves USB FIDO2, Bluetooth/híbrido, y gestores como KeePassXC/Bitwarden vía extensión). Existe el `xdg-desktop-portal` de credenciales en desarrollo, **no verificado** su estado en Fedora 44 ni en estos navegadores; habría que comprobarlo antes de planificar.
- Un autenticador de plataforma con huella exigiría implementar CTAP2/UHID virtual o un proveedor propio, con claves en TPM: criptografía y superficie de ataque grandes. **No lo recomiendo** como parte de este proyecto.
- Lo realista: soportar llaves FIDO2 físicas (ya funciona con `libfido2`) y dejar los passkeys a un gestor con extensión de navegador. `BookOS Passwords` no necesita ser un autenticador WebAuthn.

## 7. ¿Gestor propio o interfaz sobre lo existente?

**Interfaz sobre lo existente.** El estándar Secret Service ya cubre almacenamiento, cifrado y compatibilidad con aplicaciones de terceros (navegadores, NetworkManager, editores). Un formato propio rompería compatibilidad y obligaría a mantener criptografía. Lo único «propio» que merece la pena: la UI, el agente polkit integrado en el shell y las políticas de huella.

Limitación de un front-end sobre Secret Service: solo ve atributos `label`/`attributes` y el secreto. No hay campos de usuario/URL/notas/TOTP estructurados de forma estándar; para un gestor «tipo Contraseñas de Apple» habría que convenir atributos propios (`xdg:schema`, `url`, `username`), que otras apps no entenderán. Aceptable para v1.

## 8. Integración en BookOS-Settings: dificultad

**Baja–media para la parte de lectura/gestión; media si se quiere diálogo y huella.**

Por qué es razonable:
- Ya hay página de seguridad con `bloqueo_huella`, detección de lector y un patrón claro (`tauriInvoke` → comando Rust en `main.rs`).
- `zbus` ya está en `src-tauri/Cargo.toml` (versión 4): sirve para hablar con `org.freedesktop.Secret.Service` sin dependencias nuevas. Aun así hay que verificar que la API de bajo nivel de `zbus 4` maneje bien los `ay`/variantes del Secret Service; la memoria del proyecto avisa de que `gdbus` los corrompe, y conviene probar con Gio o zbus, no con `gdbus`.
- Nada de crypto en JS: el cifrado de sesión de Secret Service (`dh-ietf1024-sha256-aes128-cbc-pkcs7`) debería usarse con el modo `plain` solo sobre bus de sesión, o con un crate de oo7 (`oo7` cliente), lo cual **sería una dependencia nueva a consultar contigo**.

Qué es incómodo:
- `main.rs` tiene 5 945 líneas y `pages.js` casi 2 700 solo hasta la seguridad: añadir otra página sube la deuda. Conviene un módulo Rust nuevo (`secretos.rs`) y no engordar `main.rs`.
- **Ajustes oculto consume ~360 MB** (WebView); poner contraseñas en una WebView amplía la superficie (XSS en la UI, volcados de memoria, captura de pantalla). Revelar secretos en HTML es peor que hacerlo en una ventana nativa. Esto empuja a que `BookOS Passwords` sea GPUI/nativa y Ajustes solo muestre estado (llavero activo, bloqueado, política de huella) y un botón «Abrir Contraseñas».
- El estado de «quién es el proveedor del Secret Service» debería poder verse y repararse desde Ajustes (conflictos KWallet/oo7).

Reparto recomendado: Ajustes = configuración (activar llavero, huella para autorizar, bloqueo automático del llavero); app aparte = ver/buscar/editar secretos.

## 9. Riesgos

1. **Alfa de oo7** custodiando contraseñas reales: pérdida de datos. Mitigar con copia cifrada y empezar por KWallet.
2. **Dos proveedores Secret Service** a la vez: las apps leen el que gane el nombre de bus, y parecen «perder» contraseñas. Debe haber un único propietario y una prueba que lo verifique.
3. **Huella ≠ clave**: expectativa errónea del usuario (§5).
4. **Agente polkit en el compositor**: un fallo del diálogo bloquea la elevación de privilegios y, si corre dentro de `bookos-comp`, puede afectar a toda la sesión. Mitigar con proceso hijo o con el `catch_unwind` que ya usa el shell.
5. **Wayland**: un cliente cualquiera puede capturar pantalla; los diálogos de contraseña deberían ser superficies del compositor, no de la app.
6. **Probar en anidado no vale**: el dispositivo biométrico, el bus de sesión y los nombres D-Bus se comportan distinto en sesión real (ver notas de verificación del proyecto). Cada criterio de aceptación de abajo debe repetirse en sesión real.
7. **Datos del usuario reales**: la cartera `kdewallet` existente no debe tocarse en pruebas; usar `XDG_DATA_HOME` aislado y bus aislado.
8. Pasar a oo7 desde KWallet: sin herramienta de migración verificada.

## 10. Plan por fases

| Fase | Contenido | Criterio de aceptación |
|---|---|---|
| 0 | Decidir A (KWallet) o B (oo7); fijar un único propietario de `org.freedesktop.secrets` | `busctl --user status org.freedesktop.secrets` devuelve un solo proceso, estable tras reiniciar sesión |
| 1 | Llavero arrancado con la sesión y desbloqueado por PAM en `bookos-greetd` | Tras login con contraseña, `secret-tool store/lookup` funciona sin pedir nada; Chromium/Firefox guardan y recuperan una contraseña |
| 2 | Agente polkit en el shell con diálogo BookOS | `pkexec true` y un montaje que requiere admin muestran el diálogo; contraseña correcta/incorrecta/cancelar se comportan bien; sin agente colgado al cerrar sesión |
| 3 | Huella en el agente (servicio PAM `bookos-fingerprint`) | Huella autoriza `pkexec`; contraseña sigue disponible; la huella apagada tras suspender no se toma por «sin lector» |
| 4 | Página en Ajustes: estado del llavero, política, botón de abrir | Cambios persisten, se refleja en `bookos_apply_config`, sin secretos en el JSON de config |
| 5 | `BookOS Passwords` (GPUI): listar, buscar, crear, editar, borrar, bloqueo | Un secreto creado con `secret-tool` aparece; uno creado en la UI lo lee `secret-tool`; bloqueado no revela nada |
| 6 | Revelar con huella (puerta de UI, §5) | Sin huella válida no se muestra el secreto; con contraseña como alternativa |
| 7 (opcional) | Llaves FIDO2 físicas; evaluar portal de credenciales | Registro y login con una llave real en un sitio de prueba |

Cada fase: pruebas unitarias donde haya lógica propia, prueba manual en sesión real y, para la UI, captura revisada en tema claro y oscuro.

## 11. Lo que no he podido comprobar

- Estado actual del portal de credenciales/passkeys en Firefox y Chromium de Fedora 44.
- Si la Galaxy Book5 tiene TPM2 utilizable y systemd ≥ 258 (necesario para la credencial de oo7).
- Compilación de `oo7-daemon` en esta máquina.
- Compatibilidad de la cartera existente con una migración.
- Rendimiento y memoria de cualquiera de los tres proveedores (no medido).
- Si `zbus 4` de Ajustes basta para el protocolo de sesión de Secret Service.

## Fuentes

- [oo7 (repositorio)](https://github.com/linux-credentials/oo7)
- [oo7-daemon en Fedora](https://packages.fedoraproject.org/pkgs/oo7/oo7-daemon)
- [oo7-daemon: un año después (All Systems Go 2025)](https://app.media.ccc.de/v/all-systems-go-2025-369-oo7-daemon-one-year-later-progress-challenges-and-what-s-next)
- [KWallet → Secret Service, MR 97](https://invent.kde.org/frameworks/kwallet/-/merge_requests/97)
- [Bug 504656 (ksecretd se cuelga)](https://bugs.kde.org/show_bug.cgi?id=504656)
- [Bug 504312 (plasma-nm y backend Secret Service)](https://bugs.kde.org/show_bug.cgi?id=504312)
- [Arch Wiki: KDE Wallet](https://wiki.archlinux.org/title/KDE_Wallet)
