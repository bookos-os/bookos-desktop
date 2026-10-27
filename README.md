<div align="center">

# BookOS Desktop

**A Wayland compositor with the desktop drawn inside it.**

[![License](https://img.shields.io/badge/license-proprietary-lightgrey?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024%20edition-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Smithay](https://img.shields.io/badge/Smithay-0.7-orange?style=flat-square)](https://github.com/Smithay/smithay)
[![iced](https://img.shields.io/badge/iced-0.14%20·%20tiny--skia-6c7fd6?style=flat-square)](https://iced.rs)
[![Wayland](https://img.shields.io/badge/Wayland-native-ffbc00?style=flat-square)](https://wayland.freedesktop.org)

</div>

---

## What this is

A complete desktop in **one process**: the compositor speaks Wayland to clients
and, in that same frame loop, paints its own panel, dock, popover cards,
launchpad and lock screen.

There is no separate `plasmashell`, no IPC between the desktop and the
compositor, no second drawing context. The panel is painted in the **same first
frame** as the compositor: there is no loading screen when you log in.

| Crate | What it is |
|---|---|
| **`bookos-comp`** | The compositor: Smithay 0.7, GLES2, XWayland, `winit` (nested) and `udev` (real KMS) backends, and the XDG portals |
| **`bookos-shell`** | The desktop: panel, dock, popovers, launchpad and lock screen, with iced 0.14 rasterised on the CPU (`iced_tiny_skia`) |
| **`bookos-system`** | The out-of-process service for NetworkManager, BlueZ and audio, so a slow `nmcli` can never stall a frame |

> [!IMPORTANT]
> The price of drawing the shell inside the compositor is that a `panic` there
> would take the session down with it. That is why `bookos-shell` **must not
> panic outwards**: every call is wrapped in `catch_unwind`, and every widget in
> its own. If one widget fails, only that component goes out of service; the
> desktop and the windows carry on.

---

## Contents

- [What it does](#what-it-does)
- [Where it stands](#where-it-stands)
- [The three house rules](#the-three-house-rules)
- [Build and test](#build-and-test)
- [Install as a session](#install-as-a-session)
- [Shortcuts](#shortcuts)
- [Configuration](#configuration)
- [Talking to the rest of the system](#talking-to-the-rest-of-the-system)
- [Architecture](#architecture)
- [Testing](#testing)
- [Roadmap](#roadmap)
- [License](#license)

---

## What it does

**Windows** — xdg-shell and XWayland, click-to-focus, move and resize with
<kbd>Meta</kbd>+drag, maximise, snapping to halves and quarters by dragging to an
edge or a corner, and animations on open, resize and close. The close animation
works however the window goes away — the title bar button, <kbd>Meta</kbd>+<kbd>Q</kbd>
or the application quitting — because it keeps the textures the client left
behind at the moment it destroys its window.

Menus and submenus (`xdg_popup`) are positioned inside the screen and chain like
they do on KDE: moving along a Qt menu bar — VirtualBox, Dolphin — goes from one
menu to the next without closing. Keyboard focus reaches a menu on the first key
press and not when it opens, which is what KWin does and what Qt expects.

The title bar buttons sit quietly: no background at rest, a dimmed glyph —
dimmer still on an inactive window — and they only light up under the pointer,
the close one in red.

**Virtual desktops** — two out of the box (`escritorios = N` in the config, one
to five), with shortcuts, gestures and a panel indicator. The switch **slides**,
macOS style: both sets of windows move at once and you can see where you are
going. Windows on desktops that are off-screen leave the `Space` and come back to
their exact position; the client never notices.

<kbd>Meta</kbd>+<kbd>W</kbd> opens the overview: a strip of live thumbnails
along the top and, below it, **every window of the current desktop in a grid**,
each with its title bar. On opening, the windows travel from where they are to
their cell, and on closing they travel back. From there you can create and
delete desktops without closing their windows, and a double click on the name
renames it. Dragging a window — from the grid or from a thumbnail — onto
another thumbnail moves it to that desktop; a plain click on it goes to its
desktop and brings it to the front. Clicking the active dot in the panel opens
that same view.

The **dock only shows the windows of the desktop you are on**: an application
open on another desktop does not light up its dot here.

**Touchpad gestures** — four fingers sideways switch desktop; four down push the
windows aside and up brings them back — or, if none are pushed aside, open the
desktop strip —; three up expose every window on the desktop and down closes that
view; and a four- or five-finger pinch opens and closes the launchpad. The
thresholds were measured with `libinput debug-events` on real hardware, not
estimated.

**Switchers** — both carry **one cell per window**, ordered by recent use: two
open terminals are two cells and not one, which is what left
<kbd>Alt</kbd>+<kbd>Tab</kbd> doing nothing when it grouped by application.
<kbd>Alt</kbd>+<kbd>Tab</kbd> shows icons; <kbd>Meta</kbd>+<kbd>Tab</kbd>, a live
preview of each window. Both cycle while you hold the modifier, commit when you
release it, and accept the mouse. With three fingers up, the same view stays put
until you pick one or press <kbd>Esc</kbd>.

**Minimise** — <kbd>Meta</kbd>+<kbd>H</kbd> sends the window to the dock with the
*magic lamp* effect: it freezes into a texture and a dedicated shader deforms it
— wide at the top, narrowing down the neck to its icon — because that is not a
scale and cannot be done with the client's live surface. If the driver will not
compile the shader, it just shrinks without deforming. The dock icon works as a
toggle: an open window minimises, and a minimised one runs the same deformation
backwards to the exact place it came from. The window stays alive while it is put
away: it only leaves the `Space`, the same as one on another desktop.

**Panel** — clock, battery, network, Bluetooth, volume, brightness,
notifications and control centre. Each widget is a module with its own refresh,
its own alarm and the udev subsystems it cares about.

**Light and dark** — switching theme crossfades the whole screen instead of
flipping in one frame: the compositor photographs the scene with the old theme
and fades that photo out over the new one.

**Notifications** — the toasts **stack**, up to three at a time under the panel
with the newest on top. Each one leaves on its own timer, a fourth pushes the
oldest out with its exit animation, and the rest slide to close the gap. An
update to the same notification (`replaces_id`, a progress bar) stays in its
place. The body gets two lines, and the icon comes from `image-path` too, which
is where `notify-send -i` puts it. Opening any panel card sends the toasts away
— they fell right on top of the notifications card and its Do Not Disturb
switch — and while a card is open new ones go straight to the list; critical
ones still show.

**Popover cards** — clicking a widget opens its own: power with PPD profiles,
Wi-Fi networks, Bluetooth devices, sound, brightness, calendar, notifications,
and a control station with quick toggles and MPRIS media control.

**Dock** — configurable launchers, context menu, open-window indicator,
minimise/restore toggling and frosted glass behind it.

**Search** — <kbd>Meta</kbd>+<kbd>Space</kbd> opens a KRunner-style card in the
middle of the screen. It finds applications and settings, takes commands and
shows system state without having to open a full application first.

**Emojis** — <kbd>Meta</kbd>+<kbd>.</kbd> abre un popover HIG con el acento actual,
354 emojis Noto en color, búsqueda en castellano e inglés y recientes de la sesión.
Las flechas recorren la rejilla, Tab cambia de categoría, Intro inserta y Esc cierra.
Usa text-input-v3 cuando el campo lo admite; en los demás clientes pega desde el
portapapeles, que conserva el emoji elegido. Sin una ventana enfocada, lo copia.
`cargo run -p bookos-shell --example emojis --offline` genera las vistas de prueba
en `/tmp/bookos-emojis`. El atlas y sus licencias están en `assets/emojis` del shell;
el script `scripts/generar_emojis.py` permite regenerarlo con Pango/Cairo y Noto.

**Launchpad** — every installed application, paginated, with search and an
animated transition between pages. **Folders**: created by dragging one icon onto
another, opened by clicking, renamed by clicking their name from inside, tinted
with any of the ten palette colours — the row of dots that appears inside — and
dissolved on their own once a single application is left. **Right-click** brings
out the ✕ to take off the grid whatever you do not want to see; that **hides, it
does not uninstall** — the desktop does not delete programs from the system — and
is undone by removing its name from `~/.config/bookos/launchpad.conf`, which is
where all of this lives: one line per folder, `name[:colour] = exec1, exec2, …`.
There are no folders while searching: the search runs over every application,
inside a folder or not.

**The dock stays visible with the launchpad open**, macOS style, and that is
where pinning happens: drag an icon from the grid onto the dock to pin it, and
drag it up out of the dock to let it go. It is the same as the "Pin to dock" item
in the right-click menu, but without having to open the application first. With
`launchpad_dock = no` in `panel.conf` the dock disappears from the launchpad and
the gesture with it; the menu stays.

**Lock screen** — laid out like the macOS lock: large clock and date up top, and
below them the sign-in block — profile picture cropped to a circle, name,
password field and the line that says what is going on. The picture comes from
`avatar` in the config, from `~/.face` or from AccountsService, and failing all
that, from the initials. It authenticates through the system PAM stack on a
separate worker, so a slow check cannot freeze the compositor.

<kbd>Meta</kbd>+<kbd>L</kbd> locks straight away. With the **fingerprint** on,
the reader listens the whole time the screen is locked: a finger that does not
match shakes the sign-in block and says so in red while the reader keeps
waiting, and a match says «Huella reconocida» before letting you in. **Unlocking
fades** the lock out and lifts it slightly over the desktop instead of cutting to
it; the windows are already there underneath, so the animation never delays the
security boundary.

**OSD** — the capsule that appears when you touch volume, brightness, keyboard
backlight or the touchpad.

**Dynamic activities** — an island centred under the panel for live tasks from
the system applications. The player offers artwork, progress, transport, volume
and queue; the clock shows timers; and the recorder lets you pause or stop a
recording. It only accepts the closed set of identifiers from Player, Clock and
Voice Recorder, and each activity has a compact and an expanded state, light and
dark themes and a reduced-motion option.
An app's activity is hidden while that app has the focus, since you are already looking at it, and comes back when you leave it.

**Displays and BookOS Settings** — the compositor exposes over D-Bus the census
of outputs, modes, refresh rate, fractional scale, rotation and VRR. BookOS
Settings uses that contract; it no longer has a KDE path (no Kvantum, no Plasma
wallpaper packages). Appearance, wallpaper, lock screen, activities
and effects also reload live; migrating the rest of the preferences is still in
progress.

---

## Where it stands

Not everything here is equally mature. This table separates what is usable today
from what still needs integration work for a production session.

| Area | State | What is there today |
|---|---|---|
| Wayland and X11 windows | ✅ Working | xdg-shell, XWayland, focus, move, resize, maximise, fullscreen and snapping to halves/quarters |
| Window animations | ✅ Working | Open, close, resize, a reversible Magic Lamp towards the dock and a crossfade between light and dark |
| Desktops and Exposé | ✅ Working | 1–5 desktops, names, overview with the current desktop's windows in an animated grid, live thumbnails, drag between desktops, per-desktop dock and gestures |
| Panel, dock and launchpad | ✅ Working | Modular widgets, folders, search, pinning and context menus |
| Notifications | 🟡 Partial | D-Bus server, stacked toasts (up to three), history, Do Not Disturb, `ActionInvoked` actions, progress, visual grouping per application and keyboard navigation; images sent as pixels (`image-data`), persistent reply and per-application preferences are missing |
| Lock screen | 🟡 Partial | PAM password and fingerprint authentication (validated with a real Egis reader: a non-matching finger shakes in red, a match fades the lock out), Meta+L locks straight away, automatic lock and lock on resume; advanced policies remain |
| Displays | ✅ Working | Several DRM/KMS outputs, 2D layout, independent scale/mode/Hz/VRR, primary output, EDID profiles and hotplug; panel and dock follow the primary |
| BookOS Settings | 🟡 Partial | Displays, appearance, wallpaper, lock screen, fingerprint preference, live dock icon size and activity reload; further panel/gesture controls remain |
| Dynamic activities | 🟡 Partial | Player, Timer and Voice Recorder; hardening the D-Bus identity and the final app integration are missing |
| Screenshots | ✅ Working | Region selector on Print, to file or to the clipboard, and `zwlr_screencopy_v1` v3 for `grim` and friends (`wl_shm` only) |
| Screen sharing | 🟡 Partial | `impl.portal.ScreenCast` and `Screenshot` inside the compositor, a PipeWire node driven by frames, backpressure, `Request` cancellation, `Session.Closed` and a permission card of its own. Tested nested with `gst-launch-1.0 pipewiresrc`: correct image at 2240×1400. Whole displays only, the cursor is always included, and frames go through the CPU: the DMA-BUF path is missing |
| Foreign applications | 🟡 Partial | The portal serves `impl.portal.Settings`, so GTK, Qt and Tauri follow BookOS's theme, accent, contrast and reduced motion live, and `impl.portal.FileChooser`, so their open and save dialogs are BookOS's own file explorer. Menus and submenus (`xdg_popup`) work, including Qt menu bars. `xdg-activation`, server-side decoration, text-input/input-method, idle-notify, layer-shell and foreign-toplevel are integrated. There is no polkit agent yet |
| Accessibility | 🟡 Partial | Reduced motion, high contrast, focus and keyboard navigation of notifications; global text scaling and AT-SPI are missing |

> [!NOTE]
> The `winit` backend is a nested preview for development. The real test of DRM,
> VRR, 120 Hz, suspend and hotplug happens in a proper session from a TTY.

---

## The three house rules

<table>
<tr><td width="33%" valign="top">

### 🔋 No wake-ups to spare

The compositor only repaints when something changed, and the timers line up with
what is on screen: the clock wakes on the **minute change**, not every second.
Before adding a timer, look for the kernel event that already tells you — udev
netlink, for instance. This is battery life on a laptop, not a
micro-optimisation.

</td><td width="33%" valign="top">

### ⚡ Painted on the first frame

The panel's data comes from **sysfs and libc**: no D-Bus, no daemons, no waiting
on anyone. A state that needs a service (volume, media) is not made up by reading
files blindly: it waits until there is someone to talk to, and until then it is
not drawn. What genuinely is D-Bus — notifications — the compositor handles on a
separate thread, and the panel paints all the same without having talked to the
bus.

</td><td width="33%" valign="top">

### 📐 Measure before claiming

"This is faster" only gets written after running it. When something is
counter-intuitive, the comment says **what was measured and with what number** —
and if it could not be verified, it says that too.

</td></tr>
</table>

---

## Build and test

```bash
cargo build                                  # debug
cargo test --workspace                       # 433 tests, green as of 2026-09-23

# Nested inside an existing graphical session, with a test client
cargo run -p bookos-comp -- -f konsole
```

<details>
<summary><b>System dependencies</b> (only for the real KMS backend)</summary>

<br>

The nested backend (`winit`) needs nothing special: it runs inside an existing
graphical session. The `udev` backend, which is the one a real session uses,
needs the headers for:

```
libinput  libgbm  libdrm  libseat  libdisplay-info
```

On Fedora: `libinput-devel mesa-libgbm-devel libdrm-devel libseat-devel libdisplay-info-devel`

</details>

> [!NOTE]
> **Dependencies are optimised in debug builds too** (`[profile.dev.package."*"]
> opt-level = 3`). Without that, `cargo run` is unusable: measured, the dock goes
> from 1 ms per repaint to 59, and moving the mouse gives 17 fps. The time is
> spent inside `tiny-skia` and `resvg`, not in this repository's code. Our own
> code stays unoptimised, so compiling is still fast and panics point at the
> right line — but to **measure**, use `--release`.

### Inspecting the detected hardware

```bash
cargo run -p bookos-comp -- --drm-info      # GPU and displays
cargo run -p bookos-comp -- --panel-info    # the states the panel reads
cargo run -p bookos-comp -- --cursor-info   # which file each cursor resolves to
```

None of them opens the Wayland socket or creates a seat: querying the hardware
must not have side effects.

---

## Install as a session

```bash
sudo ./session/instalar.sh
```

It leaves in place:

- `bookos-session` in `/usr/local/bin` and the `.desktop` entry in
  `/usr/share/wayland-sessions`, which is what SDDM, GDM and greetd read to know
  which sessions to offer. **BookOS** then shows up in the login list.
- `bookos-system` in `/usr/local/libexec`, with its D-Bus activation file and the
  `bookos-system.service` user unit. **Without this the sound, Wi-Fi and
  Bluetooth cards say the service is unavailable** — that is not a bug in the
  shell, it is the service simply not being installed.
- The portal files, `bookos.portal` and `BookOS-portals.conf`, in
  `/usr/share/xdg-desktop-portal`, which is what makes screen sharing and the
  theme handoff to GTK/Qt/Tauri applications work.

The log of each start-up lands in `$XDG_RUNTIME_DIR/bookos-session.log`.

### Pantalla de acceso de BookOS

El crate `bookos-greeter` ofrece un login básico en iced sobre greetd y Cage.
Incluye usuario, preguntas PAM y entrada a la sesión de BookOS. Consulta
[instalación, vista previa y recuperación](session/greeter/README.md).

---

## Shortcuts

Window organisation: **Alt+F3**, or right-click a server-side title bar, opens
the window menu. **Meta+Alt+T** toggles Always on top. The menu also sends the
window to another desktop on its current monitor or to another connected
monitor (its active desktop). Sending to another desktop does not follow the
window. Moving between monitors preserves maximised/fullscreen state and
translates the saved floating geometry using logical coordinates.

| Shortcut | What it does |
|---|---|
| <kbd>Meta</kbd>+<kbd>Return</kbd>, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd> | Open a terminal (`BOOKOS_TERMINAL`, `bookos-shell` by default) |
| <kbd>Meta</kbd> alone | Open or close the launchpad |
| <kbd>Meta</kbd>+<kbd>Space</kbd> | Central search: applications, settings, commands and states |
| <kbd>Meta</kbd>+<kbd>.</kbd> | Selector de emojis: búsqueda, categorías y recientes de la sesión; Intro inserta y Esc cierra |
| <kbd>Meta</kbd>+<kbd>W</kbd> | The overview: desktop thumbnails and the current desktop's windows in a grid |
| <kbd>Alt</kbd>+<kbd>Tab</kbd> · <kbd>Meta</kbd>+<kbd>Tab</kbd> | Window switcher: icons or live thumbnails (with <kbd>Shift</kbd>, backwards) |
| <kbd>Meta</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to that desktop |
| <kbd>Meta</kbd>+<kbd>Ctrl</kbd>+<kbd>←</kbd>/<kbd>→</kbd> | Previous or next desktop |
| <kbd>Meta</kbd>+<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>←</kbd>/<kbd>→</kbd> | Send the focused window to the previous or next desktop |
| <kbd>Meta</kbd>+<kbd>Ctrl</kbd>+<kbd>D</kbd> | Push the windows aside to see the desktop, or bring them back |
| <kbd>Meta</kbd>+<kbd>F</kbd> | Maximise the focused window, or restore it |
| <kbd>Meta</kbd>+<kbd>H</kbd> | Minimise to the dock; its icon toggles minimise/restore with Magic Lamp |
| <kbd>Meta</kbd>+<kbd>←→↑↓</kbd> | Snap to half the screen; another arrow, to a quarter |
| <kbd>Meta</kbd>+<kbd>Q</kbd> | Close the focused window |
| <kbd>Meta</kbd>+<kbd>E</kbd> | Open the BookOS file explorer |
| <kbd>Meta</kbd>+<kbd>Z</kbd> | The BookOS menu (About, Settings…), same as clicking the logo |
| <kbd>Meta</kbd>+<kbd>L</kbd> | Lock straight away (password or fingerprint unlocks it) |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>O</kbd> | Apagar solo las pantallas; el equipo continúa trabajando. Una entrada las enciende tras el margen inicial de dos segundos. Requiere la sesión DRM. |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> | Salvapantallas: «BookOS» en texto sobre negro. Una tecla o un movimiento lo retira. |
| <kbd>Meta</kbd>+<kbd>Esc</kbd> · power button | The power dialog: sleep, lock, log out, restart, shut down |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>A</kbd> | Start or finish a spoken command for the BookOS assistant |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd> / <kbd>D</kbd> | The panel / the dock: dodge windows or always visible |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>F</kbd> | Diagnostics overlay: fps, frame cost and dropped frames per monitor |
| <kbd>Meta</kbd>+drag | Move the window (with the right button, resize) |
| drag to an edge | Snap: the sides give halves, the corners quarters |

The compositor takes a logind `handle-power-key` inhibitor while it runs, so the
**power button** reaches the desktop and opens the dialog instead of powering
the machine off; nothing to configure in `/etc`, and logind gets the key back
when the session ends. <kbd>Meta</kbd>+<kbd>Esc</kbd> opens it either way.

And only in a real session on a TTY:
<kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd> to switch virtual
terminal, and <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Backspace</kbd> **twice within two seconds** to
end the compositor. One press only shows a warning: on a laptop Ctrl and Alt sit
side by side, and deleting a word with Ctrl+Backspace used to brush Alt and close
the whole session.

---

## Configuration

One file, `~/.config/bookos/panel.conf`. The keys are in Spanish because the
project's language is Spanish:

```ini
# The panel widgets, per zone.
centro = reloj
derecha = escritorios, red, brillo, bateria
escala = 1.75

# claro, oscuro or automatico. With «automatico» the theme follows the clock,
# and these two say when each one starts.
tema = automatico
tema_claro_desde = 07:00
tema_oscuro_desde = 20:00

# The accent colour, from the closed table: azul, indigo, morado, rosa, rojo,
# naranja, amarillo, verde, turquesa or grafito. It is also picked from the
# BookOS menu → «Apariencia…», which applies it live and writes it here.
acento = azul
alto_contraste = no
avatar = /path/to/avatar.png

# Visual effects: «completos» or «reducidos». Reduced drops the blur behind the
# panel and the dock, and the window animations —open, minimise, move, snap—;
# the shell's own (popovers, desktop switch) stay. Below 20 % battery with no
# charger it turns itself on, without touching this key.
efectos = completos

# Automatic brightness, per light, from the ambient light sensor (through
# iio-sensor-proxy). Also toggled with the «A» buttons in the brightness card.
# Adjusting by hand while it is on is remembered: the screen keeps the offset,
# the keyboard keeps its level until the light crosses from dark to bright.
brillo_automatico_pantalla = no
brillo_automatico_teclado = no

# Lock screen. The positions are fractions of the logical height: 0.36 is 36 %,
# so the composition survives HiDPI and other resolutions.
bloqueo_animaciones = si
bloqueo_fecha = si
bloqueo_medios = si
bloqueo_reloj_y = 0.08
bloqueo_acceso_y = 0.36
bloqueo_medios_y = 0.68
bloqueo_reloj_tamano = 144
bloqueo_avatar_tamano = 132
bloqueo_inactividad = 900
# Salvapantallas a los cinco minutos; cero desactiva la entrada automática.
# Respeta los inhibidores y no sustituye el bloqueo de seguridad.
salvapantallas_inactividad = 300
# Zero turns automatic suspend off. When on, it counts from the moment the
# session is locked and respects Wayland and logind inhibitors.
suspension_inactividad = 0

# Dynamic activities from Player, Clock and Voice Recorder.
actividades = si
actividades_animaciones = si
temporizador_siempre_visible = no

escritorios = 2
nombres_escritorios = Escritorio 1, Escritorio 2
dock = konsole:Terminal:utilities-terminal, firefox:Navegador:firefox
launchpad_dock = si

# One wallpaper per theme: switching from light to dark takes the wallpaper
# with it. `fondo` on its own still works and applies to both.
fondo_claro = /usr/share/wallpapers/BookOS/Light/blue.png
fondo_oscuro = /usr/share/wallpapers/BookOS/Dark/blue_dark.png

# Se aceptan PNG, JPEG, WebP, SVG y WebP animado. Los fondos animados se
# congelan mientras está activado «Reducir efectos».

# Cursor and input. The speeds use libinput's scale: [-1, 1].
cursor = 24
teclado = es
velocidad_touchpad = 0.3
velocidad_raton = 0.0
toque_para_clic = si
scroll_natural = si
```

If the file does not exist, the defaults are used and the panel still paints on
the first frame **without touching the disk**. If it exists but has a typo, that
line is ignored and a warning is logged: losing the whole panel over a comma is
worse than losing one widget.

<details>
<summary><b>Why it is not TOML</b></summary>

<br>

Reading two lists of names does not justify pulling in `toml`, which drags the
whole of `serde` into a crate that today has no serialisation dependency at all.
The format above parses in twenty lines and cannot fail in interesting ways.

</details>

---

## Talking to the rest of the system

**`org.bookos.Desktop`** — the contract with BookOS Settings, served by the
compositor. `GetConfig` and `ApplyConfig` exchange a versioned JSON document and
validate the whole update before persisting it, so Settings never has to parse
`panel.conf` itself. `GetOutputs` / `ApplyOutputConfig` plus the
`OutputsChanged` signal cover displays; `ReloadConfig` reloads one section live;
`GetCapabilities` says what this compositor can do; `PublishActivity` /
`CloseActivity` drive the dynamic activities island.

`GetConfig` includes `tema_efectivo`: the colour that is painted **right now**,
already resolved. `tema` is the preference, and with `automatico` a client cannot
know what colour to paint itself without redoing the time-of-day calculation here
— which is exactly what Settings used to do, with its own `date +%H:%M`. It is
read-only: `ApplyConfig` ignores it rather than rejecting it, so a client can
hand back the object it read without the whole call failing.

**XDG portals**, served from inside the compositor:
`org.freedesktop.impl.portal.ScreenCast`, `Screenshot`, `FileChooser`, `Request`,
`Session` and `Settings`.

`FileChooser` answers `OpenFile`, `SaveFile` and `SaveFiles` by launching the
BookOS file explorer (`../explorer`) in picker mode on this compositor's display:
filters, multiple selection, the current folder and the suggested name all
arrive, and closing the request closes the picker. `session/bookos-portals.conf`
routes `FileChooser=bookos`.

The `Settings` portal is what makes a GTK, Qt or Tauri application follow
BookOS's dark mode: it answers the `org.freedesktop.appearance` namespace with
`color-scheme` (1 dark, 2 light), `accent-color`, `contrast` and
`reduced-motion`, and emits `SettingChanged` when any of them changes.

**`org.bookos.System1`** (`bookos-system`) — network, Bluetooth and audio,
outside the compositor's process. The panel reads a cache and queues operations;
NetworkManager, BlueZ and the audio processes are dealt with elsewhere. Settings
shares the state through `GetState` and the `StateChanged` signal, and sends
structured operations to `Perform`. **The protocol does not accept shell
commands.**

```sh
cargo build --release -p bookos-system
sudo ./session/instalar.sh
```

It also **mounts removable drives on its own**, the job `kded` did under
Plasma: USB sticks, external disks and SD cards are mounted through udisks2 in
`/run/media/$USER/<label>` when the session starts and whenever one is plugged
in. Only what udisks does not mark as a system disk and that has a physical
drive behind it (no loop devices); internal partitions and encrypted volumes are
left alone, and a drive unmounted by hand is not mounted again. It asks polkit
without interaction, so it never pops up a password dialog.

It needs NetworkManager, BlueZ, `rfkill`, `pactl` and `wpctl` (PipeWire with
pipewire-pulse). For development, `cargo run -p bookos-system` on the session bus
works without installing it.

Version 1 of the state carries `network`, `bluetooth`, `audio` and a per-domain
`errors`. A domain with no data is `null`; an error does not turn into a
switched-off device. Queries never force a scan. Changes and provider restarts
trigger fresh reads. Wi-Fi passwords are requested through a separate method and
never appear in the state. Pairing goes through `PairingRequested` and
`AnswerPairing`, confirmed in Settings.

---

## Architecture

```
crates/
├── bookos-comp/            # the compositor
│   ├── backend/
│   │   ├── winit.rs        # nested: develop without leaving your session
│   │   ├── udev.rs         # real KMS: the actual session
│   │   └── mod.rs          # frame composition and layer order
│   ├── shell.rs            # the shell surfaces inside the Space
│   ├── ventanas.rs         # snapping, resize animation, focus
│   ├── decoracion.rs       # title bar and window buttons
│   ├── escritorios.rs      # virtual desktops, the overview grid and "show desktop"
│   ├── genio.rs            # the magic lamp: capture to texture and warp shader
│   ├── cierre.rs           # the close animation, from the textures a window leaves
│   ├── fundido.rs          # the light/dark crossfade
│   ├── handlers.rs         # xdg-shell: toplevels, menus and popup grabs
│   ├── gestos.rs           # touchpad gestures, without depending on Smithay
│   ├── input.rs            # libinput: pointer, keyboard, touchpad
│   ├── keybinds.rs         # shortcuts and actions
│   ├── desenfoque.rs       # the frosted glass, in GL
│   ├── autenticar.rs       # PAM for the lock screen
│   ├── ajustes.rs          # the org.bookos.Desktop contract
│   ├── apariencia.rs       # theme, accent and wallpaper applied live
│   ├── pantallas.rs        # output model, validation and persistence
│   ├── portal.rs           # ScreenCast, Screenshot, FileChooser and Settings portals
│   ├── pw.rs · emision.rs  # PipeWire node and the frame stream
│   ├── captura.rs          # screenshots and zwlr_screencopy
│   ├── notificaciones.rs   # org.freedesktop.Notifications server
│   ├── multimedia.rs       # MPRIS
│   ├── metricas.rs         # fps, frame cost, dropped frames
│   ├── xwayland.rs         # X11 clients
│   └── selftest.rs         # input checks and the hand-written script
├── bookos-shell/           # the desktop
│   ├── widget.rs           # the Widget trait and its isolation
│   ├── widgets/            # clock, battery, network, bluetooth, volume, brightness…
│   ├── emergente/          # each widget's card, the launchpad and the menus
│   ├── view.rs             # the panel layout
│   ├── dock.rs · apps.rs   # the dock and the .desktop census
│   ├── tema.rs             # the design-system tokens
│   ├── icono.rs            # embedded icons + the system theme
│   ├── actividad.rs        # Player, Timer and Voice Recorder island
│   ├── conmutador.rs       # Alt+Tab, Meta+Tab and Exposé
│   ├── notificaciones.rs   # notification model and history
│   ├── toast.rs            # the stacked pop-in notifications
│   ├── bloqueo.rs          # lock screen
│   ├── diagnostico.rs      # the metrics overlay
│   └── osd.rs              # the volume and brightness capsule
└── bookos-system/          # network, bluetooth and audio, out of process
    ├── network.rs · bluetooth.rs · audio.rs
    └── pairing.rs          # the org.bluez.Agent1 agent
```

**Logical versus physical pixels.** Smithay's `Space` reasons in logical units
and the buffers are physical. Confusing them already broke the panel at
fractional scale: at 1.75, a 335-logical card gives a 586 px buffer and the
compositor asks for 586.25 — a quarter of a pixel — and bilinear filtering
smears the whole text. Every surface goes through `a_pixel_entero`.

**A single GPU context.** The shell rasterises on the CPU with `tiny-skia` and
the compositor uploads the result as a texture. A panel repaints a tiny strip
very rarely, so it is not worth opening a second GPU backend with `wgpu` just for
that.

---

## Testing

```bash
cargo test --workspace                            # 433 tests

# Actually look at what gets painted, instead of assuming
BOOKOS_PANEL_PNG=/tmp/panel.png cargo test --test panel
BOOKOS_ENERGIA_PNG=/tmp/energia.png cargo test --test panel

# Checks inside a real session
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_CONMUTADOR=1 cargo run -p bookos-comp -- -f konsole
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_WIDGETS=1 cargo run -p bookos-comp
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_ESCRITORIOS=1 cargo run -p bookos-comp -- -f konsole

# The per-widget isolation: kill one and the panel carries on
BOOKOS_SHELL_PANIC_TEST=reloj cargo run -p bookos-comp -- -f

# A hand-written walk through real applications (menus, overview, lock…)
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_GUION=/tmp/guion.txt \
  ./target/debug/bookos-comp 'konsole --separate'
```

The script takes one order per line: `espera ms`, `mover x y`, `clic x y [der]`,
`pulsar x y` / `soltar` for drags, `tecla meta+w` (combinations with `+`),
`escribir palabra` to type into a search box,
`tema claro|oscuro`, `captura` and `fin`. `mover`, `clic` and `tecla` accept a
trailing wait in milliseconds — the default 250 ms swallows short animations
whole. Coordinates are logical; screenshots go to `$XDG_PICTURES_DIR/Capturas`.
Launch `konsole` with `--separate` and give it about eight seconds, or it reuses
the host's konsole and the window never shows up nested.

For the notifications, run it under `dbus-run-session`: the host desktop already
owns `org.freedesktop.Notifications` on the normal bus.

Nearly every popover has its own `BOOKOS_*_PNG` variable to dump what it draws.
**If you touch something visible, look at it** — a test that only checks nothing
panicked has no idea whether the card came out blank.

The `bookos-system` test uses a private D-Bus bus and needs permission to create
a local socket; it does not touch the hardware or the user's session.

---

## Roadmap

The priority is turning the existing pieces into a coherent, configurable and
secure platform. The work is organised in these stages:

### P0 · Foundations of a complete session

- **A single configuration API.** `org.bookos.Desktop` already exposes
  `GetConfig` and `ApplyConfig`, validating the whole update before persisting
  it. Appearance, wallpaper, lock screen, activities and effects apply live; what
  is left is rebuilding panel, dock, desktops and input devices without a
  restart, and extending the contract to gestures, shortcuts and notification
  preferences.
- **Wayland protocols.** Already in: `relative-pointer`, `pointer-constraints`,
  `idle-inhibit`, `fractional-scale`, `viewporter`, `presentation-time`,
  `cursor-shape`, `xdg-activation`, `text-input-v3`, input method,
  `ext-idle-notify`, layer shell and `ext-foreign-toplevel-list`.
- **Portals.** `ScreenCast`, `Screenshot` and `Settings` are in, inside the
  compositor itself, with cancellation, session close and per-stream
  backpressure, and `FileChooser` opens the BookOS file explorer. Left to do:
  picking a single window, and getting rid of the CPU round trip — today every shared frame is composed
  separately and read back from the GPU with `glReadPixels`; the right path is
  exporting a DMA-BUF and handing it to PipeWire untouched.
- **Lock-screen security.** Already there: PAM on a worker, automatic lock and
  lock on resume, KMS DPMS, respect for `idle-inhibit`, a Caps Lock warning,
  progressive backoff after failures, and configurable automatic suspend that
  also respects inhibitors, and fingerprint unlocking validated on a real
  reader. Left: switching keyboard layout from the lock screen.
- **A polkit agent and a keyring.** Without an agent, anything that asks for
  administrator rights fails without a dialog; without a keyring started with
  the session, browsers and editors cannot keep their passwords and tokens.
- **Advanced multi-display shell.** The primary output carries the panel, the
  dock and the interactive surfaces; what is left is allowing them to be
  duplicated, or the panel and dock split across monitors, from Settings.

### P1 · System integration

- Talk directly to NetworkManager, BlueZ, WirePlumber/PipeWire, MPRIS and logind
  instead of repeatedly shelling out to `nmcli`, `bluetoothctl`, `wpctl`,
  `busctl` and `systemctl`.
- Verify the D-Bus owner of dynamic activities; a list of allowed `app_id`s does
  not on its own prove which process is publishing.
- Finish persistent reply, per-application preferences and `image-data` images
  (album art, chat avatars) in notifications; stacking, grouping, `ActionInvoked`
  actions and progress already work.
- Turn the search box into a provider system: applications, files, settings,
  calculator, conversions, commands, history and actions.
- Complete Wi-Fi with a password, Bluetooth pairing and audio profiles without
  leaving the shell's own interface.

### P2 · Experience and accessibility

- Global text scaling and full keyboard navigation; after that, AT-SPI
  integration for a screen reader.
- Per-window rules and remembered geometry. Always on top, moving to another
  desktop (menu, shortcut or dragging in the overview) and the close animation
  are already in.
- Clipboard history with special handling for sensitive content.
- Careful support for touchscreen, stylus and on-screen keyboard.

### P3 · Visual system and performance

- Keep the HIG tokens as the single source of truth and generate from them both
  the shell's Rust values and Settings' CSS variables.
- Visual tests in light and dark at scales 1, 1.25, 1.5, 1.75 and 2.
- Keep layout and static widgets on the CPU; leave the GPU composition, blur,
  large shadows, transforms, Magic Lamp and continuous motion.
- Drop the re-rasterisation of the launchpad's SVGs while searching. iced's cache
  is purged per draw, so filtering throws away the icons that leave the grid and
  clearing the query means reparsing them: measured in release, 37 ms per draw
  against the 12 of repainting it unchanged. The panel and the dock no longer
  suffer from this — each has its own renderer — but the launchpad steps on
  itself.
- Extend the diagnostics overlay with damage, texture uploads and GPU memory;
  validate at 60, 120 and 144 Hz.
- Move the overview's hover highlight to the GPU. The strip is repainted in
  full on the CPU for every frame of the hover fade: measured nested at scale
  1.75, 31 ms per frame, down to 14 ms by clearing the buffer with the strip's
  colour instead of blending a translucent fill over it. The rest is iced
  clearing a full-size clip mask per layer.
- A power-saving profile: cap drawing at 60 fps on battery with VRR on (the
  laptop's panel goes down to 48 Hz), reduced effects and a paused animated
  wallpaper, switched on from power-profiles-daemon or low battery. At rest the
  compositor already draws nothing — measured nested, 0 fps with only the clock
  ticking — so the saving is in active use, and it has to be measured with
  `power_now` in a real session.

### What "ready for daily use" means

A feature is not finished just because it shows up: it has to work on the nested
and the DRM backend where applicable, respect fractional scale, light and dark
themes, reduced motion, the keyboard and recoverable errors, and have at least
one logic or visual test. The complete session must be able to lock, suspend,
share the screen and recover its configuration without depending on Plasma.

### Session recovery

`bookos-session` supervises the compositor. A non-zero exit saves
`$XDG_STATE_HOME/bookos/compositor-last-crash.log` (default
`~/.local/state/bookos/`) and launches `session/bookos-recovery.py` outside the
failed compositor. Normal logout does not show an error.

The recovery screen uses the HIG dialog pattern and reads Settings'
`~/.config/bookos/palette.css`. It offers an explicit retry, an installed
Plasma/GNOME Wayland session, or return to the login manager. It never retries
automatically in a loop and cannot restore applications disconnected by the
crash. A crash while locked permits only return to login, preserving the
authentication boundary.

The graphical recovery requires **Python 3, PySide6 QtWidgets and KWin
Wayland**. KWin temporarily owns a private recovery display; it exits before
the selected desktop starts. Without usable recovery graphics the supervisor
tries a text prompt on the controlling terminal, then returns to login if no
terminal is available. This covers process exits, not a frozen kernel/GPU or
a compositor that remains alive but stops responding.

`session/instalar.sh` installs the compositor, supervisor and recovery helper.
The ISO's `rpm/bookos-desktop.spec` also installs `session/bookos-recovery.py`
at `/usr/libexec/bookos-recovery.py` and declares the runtime dependencies above.

Verification:

```bash
python3 -B -m unittest discover -s session -p 'test_*.py'
BOOKOS_MENU_VENTANA_PNG=/tmp/menu.png cargo test -p bookos-shell --test menu_ventana
# In a nested development session, with isolated configuration and D-Bus:
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_ORGANIZAR=1 cargo run -p bookos-comp -- 'konsole --separate'
```

The organisation self-test uses two real clients and a simulated second output
with fractional scale and negative coordinates; it is not a physical DRM
hotplug or crash-recovery test.

---

## Desktop improvements: confirmation, fingerprint and live controls

Power actions that lose work use a HIG confirmation with Cancel selected
initially: suspend, log out, restart and shut down, from the BookOS menu, the
energy chooser, Ctrl+Alt+Delete and the lock-screen power menu. Tab/left/right
select a button; Enter activates it; Escape cancels. **Locking is not
confirmed**: nothing is lost and the password undoes it, so Meta+L locks at once,
as it does everywhere else. Automatic locking and suspend handling do not wait
for a dialog. Meta+Alt+Backspace, pressed twice, remains the explicit emergency
exit.

Settings → Desktop exposes `dock_tamano` (32–80 logical pixels, default 50).
Saving resizes, repaints and repositions the dock without restarting the session;
hit testing and the reserved window area follow the new size. The number of
desktops still requires a session restart when changed from this Settings page.

Password unlocking uses the dedicated `/etc/pam.d/bookos` service. Keeping it
separate from the distribution's general `system-auth`/`common-auth` stack
prevents an enabled `pam_fprintd` module from delaying or replacing an explicit
password attempt. The development installer creates it without overwriting a
local administrator's existing policy.

Settings → Lock screen exposes `bloqueo_huella` (off by default). It requires an
enrolled fingerprint, fprintd and the dedicated `/etc/pam.d/bookos-fingerprint`
service. The RPM installs it and requires `fprintd-pam`; the development installer
preserves an existing service and selects `system-auth` or `common-account` for
account validation. It does not modify the shared password authentication stack.
The biometric policy uses `pam_fprintd.so max-tries=3 timeout=15`, following the
[upstream module manual](https://manpages.debian.org/trixie/libpam-fprintd/pam_fprintd.8.en.html).

Fingerprint checking runs separately from password checking, so the password
field stays usable. Each finger that does not match reaches the screen as it
happens — `pam_fprintd` reports it as a `PAM_ERROR_MSG` while it keeps waiting —
and shakes the sign-in block. When the 15-second `timeout` runs out with nobody
touching the reader, `pam_fprintd` answers exactly what it answers with no
reader at all; the two are told apart by time (measured: 0.26 s with nothing
enrolled, 15.4 s for a timeout), and a timeout starts a new wait instead of
switching the fingerprint off. F9 retries after three failed fingers (with a
3-second local delay).
Only one sensor worker can run at a time, and results are tied to the current
lock generation. Unlocking discards outstanding results; a previous sensor
worker can retain the device until its bounded PAM timeout. PAM authentication
**and account validation** must both pass. Identity comes from the session UID,
not `$USER`. This is an alternative authentication method, **not two-factor
authentication**. Missing modules, sensors or fingerprints leave the screen locked.

Panel widgets fade a neutral update highlight over 220 ms without changing
layout; reduced effects disable it. Open cards are repainted when system events
change their data, including the card-only refresh path. Animation frames do
not poll hardware and the final frame clears the pending animation.

Verification (never executes a real shutdown/reboot):

```bash
cargo test --workspace --offline
BOOKOS_DUMP_CONFIRMACION=/tmp/confirmacion.png cargo test -p bookos-shell --test desktop_mejoras
# Nested development session, using isolated configuration and D-Bus:
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_MEJORAS=1 cargo run -p bookos-comp
```

The tests cover confirmation/cancellation, fractional-scale rendering, live
dock resizing, open-card repainting, config validation and fail-closed auth
results. Fingerprint unlocking, the non-matching finger and the timeout were
validated with a real reader (Egis Match-on-Chip) in a nested session; actual
suspend/resume still needs interactive validation after installation. No system PAM files or installed
compositor binaries are changed by the tests.

## License

Proprietary: all rights reserved (`LicenseRef-BookOS`). The terms are in
[`LICENSE`](LICENSE).

```
Copyright (C) 2026 BookOS. All rights reserved.
```

The design system's icons come from [Heroicons](https://heroicons.com) (MIT).
Smithay (`vendor/smithay`, MIT), Cage (`bookos-cage`, MIT) and the emoji atlas
(`assets/emojis`) keep their own licences, and so do all dependencies.

---

<div align="center">

**[BookOS](https://github.com/Evelynx08)** · Built to run on a real laptop

</div>

## Grupos de multitarea

Al encajar una ventana en la mitad izquierda y otra en la derecha del mismo
monitor se forma un grupo. El separador central ajusta ambos anchos y respeta
los mínimos declarados por las aplicaciones cuando caben en la pantalla.
Meta + botón derecho permite ajustar el reparto desde cualquiera de los paneles.

El menú de ventana (`Alt+F3` o clic derecho en la barra de BookOS) permite unir
la ventana con la última utilizada en ese monitor, cambiar entre dos paneles y
pestañas, y separar el grupo. Los grupos de más de dos ventanas usan pestañas.
Arrastrar una barra mueve el grupo y minimizarlo o trasladarlo a otro escritorio
o monitor conserva sus ventanas juntas. Maximizar o solicitar pantalla completa
en una ventana separa el grupo.

Alt+Tab y Meta+Tab muestran una entrada por grupo y reúnen cada diálogo con su
ventana principal. Volver desde el dock o el conmutador recupera el diálogo;
cambiar de pestaña oculta también sus diálogos. Se usan las relaciones de padre
que declaran Wayland y X11; dos documentos independientes del mismo proceso
siguen siendo unidades distintas hasta que se unen explícitamente.

Prueba con clientes Qt reales (requiere el PySide6 ya utilizado por la pantalla
de recuperación):

```bash
cargo test --workspace --offline
cargo build -p bookos-comp --offline
# Usar directorios temporales existentes para configuración y capturas.
XDG_CONFIG_HOME=/tmp/bookos-prueba/config XDG_PICTURES_DIR=/tmp/bookos-prueba/imagenes \
  BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_MULTITAREA=wayland \
  dbus-run-session -- ./target/debug/bookos-comp
# Repetir con BOOKOS_SELFTEST_MULTITAREA=x11 para comprobar Xwayland.
```

El autotest comprueba encaje, redimensionado con ratón, pestañas, cierre de la
pestaña activa, diálogos, conmutación, minimizar/restaurar y cambio de escritorio.
Guarda capturas del reparto y de las pestañas para inspección visual. La prueba
se ejecuta en el backend anidado; no comprueba una sesión DRM física.
