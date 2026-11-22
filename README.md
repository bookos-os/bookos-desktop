# BookOS Desktop

BookOS Desktop is the Wayland desktop environment for BookOS. The compositor uses
Smithay and GLES. It draws the panel, dock, Launchpad, and lock screen in the same
process with iced and `tiny-skia`. A separate service handles networking, audio,
and Bluetooth so system operations don't delay desktop rendering.

You can run BookOS inside an existing Linux desktop or install it as the **BookUI**
session. The feature log tracks changes by version. [Feature log](FUNCIONES.md)

## Get the code

```bash
git clone https://github.com/bookos-os/bookos-desktop.git
cd bookos-desktop
```

## Requirements

- Linux with an active graphical session for nested mode, or access to DRM/KMS and
  a seat for a native session.
- Rust and Cargo. Dependencies in the current `Cargo.lock` require Rust 1.88 or
  newer. Check your versions with `rustc --version` and `cargo --version`.
  [Install Rust](https://rust-lang.org/tools/install/)
- A C/C++ compiler, `pkg-config`, and system libraries. The default build enables
  both `winit` and `udev`, so it requires the DRM, GBM, libinput, and libseat
  headers even when you plan to run in nested mode.

On Fedora, install the build dependencies from the distribution's repositories.

```bash
sudo dnf install cargo gcc-c++ pkgconf-pkg-config clang-devel \
  libinput-devel libdrm-devel libseat-devel \
  mesa-libgbm-devel libglvnd-devel wayland-devel libxkbcommon-devel pipewire-devel
```

X11 applications require Xwayland (`xorg-x11-server-Xwayland` on Fedora). Portals
require `xdg-desktop-portal`. The network, Bluetooth, and audio popovers need
`bookos-system`, NetworkManager, BlueZ, and PipeWire with `pactl` and `wpctl`.
Automatic brightness, fingerprint unlock, and power profiles depend on compatible
hardware and services such as `iio-sensor-proxy`, `fprintd` with `fprintd-pam`, and
`power-profiles-daemon`. BookOS apps such as Settings, Files, and Player are
installed separately.

## Try the desktop

With Konsole installed, run this from an existing graphical session.

```bash
cargo run -p bookos-comp -- 'konsole --separate'
```

BookOS opens in a window with a terminal inside it. `--separate` keeps Konsole from
reusing a window in the host session. Omit the client to open just the desktop,
or pass `-f` to the compositor for a fullscreen preview. If your host desktop
already runs a notification server, prefix `cargo run` with `dbus-run-session --`
to test notifications on a separate bus.

### Install as a desktop session

The installer checks for KWin Wayland, Python 3, and PySide6 (`QtWidgets`) for the
graphical recovery screen. Build as a regular user and use `sudo` for installation.

```bash
cargo build --release -p bookos-comp -p bookos-system -p bookos-budsd
sudo ./session/instalar.sh
```

Log out and select **BookUI** in your display manager. The installer registers the
session, user units, lock screen PAM policies, and BookOS portals. To update a
local installation, run `./actualizar.sh` as a regular user, then log out and
back in.

The optional BookOS login screen runs on greetd and Cage. It offers a preview mode,
account selection, PAM authentication, and keyboard, Wi-Fi, and power controls.
[Login screen setup](session/greeter/README.md)

If the compositor exits with an error, the session supervisor saves
`compositor-last-crash.log` under `${XDG_STATE_HOME:-$HOME/.local/state}/bookos/`.
Recovery can retry the session, return to the display manager, start Plasma or
GNOME if installed, or restore a Snapper snapshot on a compatible system. If the
session was locked, recovery only offers a return to the display manager. A text
prompt on the TTY can take over if graphical recovery fails. The latest startup
log is at `$XDG_RUNTIME_DIR/bookos-session.log`.
[Session supervisor](session/bookos-session) · [Recovery](session/bookos-recovery.py)

## Implemented features

| Area | What you can do |
| --- | --- |
| Windows and multitasking | Run Wayland and X11 apps, move and resize windows, snap them to halves or quarters, and group them into split panes or tabs. Minimize and restore windows from the dock with the Magic Lamp animation. Switch windows by icon or thumbnail. Use the desktop overview to create and rename workspaces and move windows between them. |
| Panel, dock, and Launchpad | Check system status, open control popovers, pin apps, manage multiple windows from the dock, and organize the Launchpad into folders. Start apps automatically with XDG autostart entries. |
| Files and search | Create, rename, sort, copy, compress, or trash files on the desktop, create shortcuts, and preview images, PDFs, and text. Search for apps, settings, and actions, find files with `plocate`, and send queries to Zenith when installed. |
| Notifications and privacy | View grouped notifications, progress bars, and images (`image-data`). Mute individual apps, turn on Do Not Disturb, and reply directly when the sender supports `inline-reply`. Panel indicators show when the camera, microphone, or screen sharing is in use. |
| Lock screen and power | Lock immediately, after inactivity, or on resume. Unlock with your password through PAM, a PIN set in Settings, or an enrolled fingerprint if enabled. Use the screensaver, turn displays off, suspend, or switch power profiles. The desktop asks for confirmation before powering off or logging out. |
| Displays and screenshots | Configure display layout, primary output, mode, rotation, scaling, and variable refresh rate (VRR). Save a screenshot to disk or the clipboard with region selection, a delay, an optional cursor, and an interactive thumbnail. The screen sharing portal streams entire displays through PipeWire. |
| Activities and audio | Control playback from Control Center or the lock screen. The activity island shows live tasks from integrated apps, including playback, timers, recording, and Zenith. Use `bookos-budsd` to check battery levels and access supported Galaxy Buds controls. |
| Network and devices | Join open or secured Wi-Fi networks, enter passwords from the Wi-Fi popover, manage Bluetooth, and select audio inputs and outputs. Confirm Bluetooth pairing in Settings. The system service mounts removable drives through UDisks2 when permitted by system policy. |
| Accessibility and input | Navigate the panel and dock by keyboard, use Orca with AT-SPI2 on desktop surfaces, adjust contrast, motion, and cursor size, and enable Sticky Keys or Slow Keys. The desktop also supports touchscreens, an on-screen keyboard, and an emoji picker. |

### Common shortcuts

Hold **Meta** for one second to see the shortcut and gesture sheet. On a touchpad,
swipe up with three fingers to show windows, swipe sideways with four to switch
workspaces, or pinch with five to open the Launchpad on supported hardware.
[Keyboard shortcuts](FUNCIONES.md#atajos-de-teclado) ·
[Touchpad gestures](FUNCIONES.md#gestos-del-touchpad)

| Shortcut | Action |
| --- | --- |
| Meta, Meta+Space, Meta+. | Open the Launchpad, search, or the emoji picker. |
| Meta+W, Alt+Tab, Meta+Tab | Open the desktop overview, the icon switcher, or the thumbnail switcher. |
| Meta+arrow keys, Alt+F3 | Snap the window or open its menu. |
| Meta+Z, Meta+X, Meta+P | Open the BookOS menu, Control Center, or display projection controls. |
| Print Screen or Meta+Shift+S | Open the screenshot tool. |
| Meta+L, Meta+Esc | Lock the screen or open the power menu. |
| Meta+Alt+P, Meta+Alt+L | Focus the panel or toggle Orca. |

In a DRM session, `Ctrl+Alt+F1` through `Ctrl+Alt+F12` switch virtual terminals.
In an emergency, press `Meta+Alt+Backspace` twice within two seconds to return to
the TTY.

## Configuration and integration

Desktop settings live in `~/.config/bookos/panel.conf`, or under
`$XDG_CONFIG_HOME/bookos/` when that variable is set. BookOS Settings uses
`org.bookos.Desktop` to read and apply settings. Appearance, wallpaper, panel and
dock size, workspace count and names, input preferences, and per-app notification
muting can all change while the session is running.

```ini
centro = reloj
derecha = escritorios, red, brillo, bateria
tema = automatico
tema_claro_desde = 07:00
tema_oscuro_desde = 20:00
acento = azul
efectos = completos
escritorios = 2
bloqueo_inactividad = 900
```

| Component | Responsibility |
| --- | --- |
| [`bookos-comp`](crates/bookos-comp) | Wayland compositor, Xwayland, input, displays, and portals. `org.bookos.Desktop` provides `GetConfig`, `ApplyConfig`, `GetOutputs`, `ApplyOutputConfig`, and the activity API. It checks the program associated with the D-Bus sender before accepting an external activity. |
| [`bookos-shell`](crates/bookos-shell) | Desktop surfaces drawn inside the compositor. Widgets isolate failures and redraw when their state changes. |
| [`bookos-system`](crates/bookos-system) | `org.bookos.System1` exposes network, Bluetooth, audio, and storage state and operations. It connects to NetworkManager and BlueZ through D-Bus. It still runs `pactl` and `wpctl` for audio. |
| [`bookos-budsd`](crates/bookos-budsd) | `org.bookos.Buds` provides battery levels, noise control, and other controls depending on the model. |
| [`bookos-greeter`](crates/bookos-greeter) and [`bookos-pam-huella`](crates/bookos-pam-huella) | Optional login screen and PAM module for password and fingerprint authentication with greetd. |

The BookOS XDG backend provides `Settings`, `FileChooser`, `Screenshot`, and
`ScreenCast`. Apps that query `Settings` receive the theme, accent color, contrast,
and motion preference. `FileChooser` opens the BookOS file manager as a file picker.
[XDG portal backend](session/bookos.portal)

The compositor draws the panel in its first frame using the data already
available and redraws when something changes. The clock updates on the minute.
Use a `--release` build to measure rendering cost or frame pacing. The development
profile also optimizes rendering dependencies. [Cargo profiles](Cargo.toml)

## Build and verify

```bash
cargo build --workspace
test_config=$(mktemp -d)
XDG_CONFIG_HOME="$test_config" cargo test --workspace -- --test-threads=1
python3 -B -m unittest discover -s session -p 'test_*.py'
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

Rust tests share some appearance settings. Run them serially with a temporary
configuration directory to keep tests from interfering with each other or your
session preferences. Some tests start a private D-Bus session and need permission
to open a local socket. Add `--offline` if Cargo already has the dependencies
cached. On Fedora, the formatting and lint checks use the `rustfmt` and `clippy`
packages.

The following tests export PNGs of the panel, window menu, and power confirmation
dialog for visual inspection.

```bash
BOOKOS_PANEL_PNG=/tmp/bookos-panel.png cargo test -p bookos-shell --test panel -- --test-threads=1
BOOKOS_MENU_VENTANA_PNG=/tmp/bookos-menu.png cargo test -p bookos-shell --test menu_ventana
BOOKOS_DUMP_CONFIRMACION=/tmp/bookos-confirmation.png cargo test -p bookos-shell --test desktop_mejoras
```

To test window groups in a nested session, use PySide6 and a temporary directory
for settings and screenshots. The test opens Qt clients and checks snapping, split
resizing, tabs, dialogs, and window switching.

```bash
multitasking_test=$(mktemp -d)
mkdir -p "$multitasking_test/config" "$multitasking_test/screenshots"
XDG_CONFIG_HOME="$multitasking_test/config" XDG_PICTURES_DIR="$multitasking_test/screenshots" \
  BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_MULTITAREA=wayland \
  dbus-run-session -- cargo run -p bookos-comp
```

Set `BOOKOS_SELFTEST_MULTITAREA=x11` to repeat the test with Xwayland. Debug builds
also accept `BOOKOS_SELFTEST_GUION` with the path to an input script.
[Compositor self-tests](crates/bookos-comp/src/selftest.rs) ·
[Motion test script](crates/bookos-comp/tests/movimiento_020.guion)

For hardware diagnostics, `bookos-comp --drm-info` lists GPUs and displays,
`--panel-info` shows the state read by the panel, and `--cursor-info` lists the
cursor files in use.

## Roadmap

| Priority | Work | Current limitation |
| --- | --- | --- |
| High | Review how window groups behave when minimizing a tab or returning focus to a dialog. | The nested self-test fails at group minimization on Wayland and dialog focus on Xwayland. |
| High | Validate native DRM/KMS at 60, 120, and 144 Hz, including VRR, display power management, hotplug, and suspend and resume. Measure power consumption during active use and check the hardware cursor plane after wake. | Nested mode can test interactions and composition. Refresh rates, power states, and display transitions require physical hardware. |
| High | Finish testing login, locking, and recovery on a physical TTY, and add a keyboard layout selector to the lock screen. | The greeter tests use a simulated greetd. Graphical recovery depends on KWin and PySide6. |
| High | Integrate a session polkit agent and start a session keyring. | The greeter has its own rules. The remaining work is covered in the [credential integration assessment](VIABILIDAD-CREDENCIALES.md). |
| High | Share a single window and export frames through DMA-BUF. | `ScreenCast` shares whole displays, includes the cursor, and copies frames through the CPU. |
| Medium | Label every popover control for Orca, extend text scaling to the shell, and add an indicator for Sticky Keys. | AT-SPI2 covers the panel, dock, activity island, and notifications. The Wi-Fi popover exposes its controls, while several others mainly announce their titles. Text in the panel, dock, and notifications stays at a fixed size. |
| Medium | Publish files copied or cut in the BookOS file manager to the system clipboard and add clipboard history with handling for sensitive data. | The desktop can paste files from apps that publish the clipboard selection, such as Dolphin and Nautilus. The BookOS file manager keeps its selection within the app. |
| Medium | Make panel and dock placement configurable across multiple monitors. Expose more preferences in Settings, including gestures and shortcuts. | Displays can be configured independently. The panel and dock follow the primary display. |
| Medium | Complete the audio and Bluetooth popovers and use native interfaces for audio and MPRIS. | Pairing confirmation happens in Settings. The service exposes audio profiles and uses `pactl` and `wpctl`. |
| Medium | Add window rules and remember window geometry across sessions. | The compositor remembers “Always on top” per app and preserves a group's layout while it is open. |
| Medium | Review the About This PC and Appearance interaction tests and the lock screen entry animation test. | The tests fail when revealing the serial number, selecting the automatic accent color, and checking animation progress on the lock screen. |
| Medium | Expand visual coverage across themes and fractional scales and measure redraw costs for the Launchpad and desktop overview. | Tests can export PNGs and the compositor records frame timing. More combinations and native DRM measurements are still needed. |

RPM packaging and installation images are maintained in a separate repository.
[BookOS-ISO](https://github.com/bookos-os/BookOS-ISO)

## Report a bug

Include steps to reproduce the issue, the backend (`winit` or DRM/KMS), and the
relevant log. For vulnerabilities, use the private reporting channel described in
the security policy. [Issues](https://github.com/bookos-os/bookos-desktop/issues) ·
[Security policy](SECURITY.md)

## License

Proprietary software (`LicenseRef-BookOS`). Smithay, Cage, Heroicons, and the emoji
atlas retain their own licenses. [BookOS license](LICENSE)
