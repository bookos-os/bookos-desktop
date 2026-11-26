#!/usr/bin/env python3
"""Demonio térmico y de silencio adaptativo para BookOS (Galaxy Book5 Pro / Lunar Lake).

Cuando el usuario está en modo `balanced`, observa temperatura de CPU y carga
media. En reposo o baja carga prolongada sostiene `power-saver` mediante
`HoldProfile` de `power-profiles-daemon` (y en corriente alterna selecciona el
perfil hardware `quiet` de `samsung-galaxybook` junto con `balance_power` si está
disponible), de modo que el ventilador permanezca apagado o silencioso sin
reducir las prestaciones ni la refrigeración bajo carga real.
"""

from __future__ import annotations

from dataclasses import dataclass
import glob
import os
from pathlib import Path
import signal
import sys
import tempfile

try:
    import gi

    gi.require_version("Gio", "2.0")
    gi.require_version("GLib", "2.0")
    from gi.repository import Gio, GLib
except Exception:  # pragma: no cover - permite importar la lógica pura sin PyGObject
    Gio = None  # type: ignore[assignment]
    GLib = None  # type: ignore[assignment]


POLL_S = 3
TEMP_ENTRAR_C = 56.0
CPU_ENTRAR_PCT = 22.0
ENTRAR_S = 15.0

TEMP_SALIR_C = 65.0
SALIR_TEMP_S = 3.0
TEMP_PICO_C = 72.0

CPU_SALIR_PCT = 45.0
SALIR_CARGA_S = 6.0

COOLDOWN_MANUAL_S = 45.0
COOLDOWN_REANUDAR_S = 10.0

ESTADO_PATH = Path("/run/bookos-termico/estado")
RUTAS_PLATFORM_PROFILE = (
    Path("/sys/firmware/acpi/platform_profile"),
    Path("/sys/class/platform-profile/platform-profile-0/profile"),
)
RUTAS_PLATFORM_CHOICES = (
    Path("/sys/firmware/acpi/platform_profile_choices"),
    Path("/sys/class/platform-profile/platform-profile-0/choices"),
)

BUS_PPD = "net.hadess.PowerProfiles"
OBJ_PPD = "/net/hadess/PowerProfiles"
IFACE_PPD = "net.hadess.PowerProfiles"
IFACE_PROPS = "org.freedesktop.DBus.Properties"

BUS_TERMICO = "org.bookos.Termico"
OBJ_TERMICO = "/org/bookos/Termico"
IFACE_TERMICO = "org.bookos.Termico"

INTROSPECCION_XML = """
<node>
  <interface name="org.bookos.Termico">
    <method name="Liberar"/>
    <method name="Estado">
      <arg name="estado" type="s" direction="out"/>
    </method>
  </interface>
</node>
"""


@dataclass(frozen=True)
class Lectura:
    temp_c: float
    cpu_pct: float
    perfil_ppd: str
    ac: bool
    choices: tuple[str, ...]


@dataclass(frozen=True)
class ActivarSilencio:
    perfil_hw: str
    epp: str


@dataclass(frozen=True)
class AjustarSilencio:
    perfil_hw: str
    epp: str


@dataclass(frozen=True)
class SoltarSilencio:
    motivo: str


Accion = ActivarSilencio | AjustarSilencio | SoltarSilencio


class ControlTermico:
    """Máquina de estados pura con histéresis para silencio adaptativo."""

    def __init__(self) -> None:
        self.sosteniendo: bool = False
        self.perfil_hw_actual: str | None = None
        self.epp_actual: str | None = None
        self.desde_baja_carga: float | None = None
        self.desde_calor: float | None = None
        self.desde_carga: float | None = None
        self.cooldown_hasta: float = 0.0
        self.en_sueno: bool = False

    @staticmethod
    def _objetivo_hw(ac: bool, choices: tuple[str, ...]) -> tuple[str, str]:
        if ac and "quiet" in choices:
            return ("quiet", "balance_power")
        return ("low-power", "power")

    def _reset_salidas(self) -> None:
        self.desde_calor = None
        self.desde_carga = None

    def _soltar(self, motivo: str) -> SoltarSilencio:
        self.sosteniendo = False
        self.perfil_hw_actual = None
        self.epp_actual = None
        self.desde_baja_carga = None
        self._reset_salidas()
        return SoltarSilencio(motivo)

    def evaluar(self, lectura: Lectura, ahora: float) -> Accion | None:
        if self.en_sueno:
            self.desde_baja_carga = None
            self._reset_salidas()
            return None

        if not self.sosteniendo:
            if lectura.perfil_ppd != "balanced" or ahora < self.cooldown_hasta:
                self.desde_baja_carga = None
                return None

            if lectura.temp_c <= TEMP_ENTRAR_C and lectura.cpu_pct <= CPU_ENTRAR_PCT:
                if self.desde_baja_carga is None:
                    self.desde_baja_carga = ahora
                if ahora - self.desde_baja_carga >= ENTRAR_S:
                    perfil_hw, epp = self._objetivo_hw(lectura.ac, lectura.choices)
                    self.sosteniendo = True
                    self.perfil_hw_actual = perfil_hw
                    self.epp_actual = epp
                    self.desde_baja_carga = None
                    self._reset_salidas()
                    return ActivarSilencio(perfil_hw, epp)
            else:
                self.desde_baja_carga = None
            return None

        # Si el usuario o un tercero forzó performance sin esperar a ProfileReleased.
        if lectura.perfil_ppd not in ("balanced", "power-saver"):
            self.al_soltar_externo(ahora)
            return SoltarSilencio(f"perfil externo {lectura.perfil_ppd}")

        if lectura.temp_c >= TEMP_PICO_C:
            return self._soltar(f"pico térmico {lectura.temp_c:.1f} °C")

        if lectura.temp_c >= TEMP_SALIR_C:
            if self.desde_calor is None:
                self.desde_calor = ahora
            if ahora - self.desde_calor >= SALIR_TEMP_S:
                return self._soltar(f"temperatura sostenida {lectura.temp_c:.1f} °C")
        else:
            self.desde_calor = None

        if lectura.cpu_pct >= CPU_SALIR_PCT:
            if self.desde_carga is None:
                self.desde_carga = ahora
            if ahora - self.desde_carga >= SALIR_CARGA_S:
                return self._soltar(f"carga sostenida {lectura.cpu_pct:.1f} %")
        else:
            self.desde_carga = None

        perfil_hw, epp = self._objetivo_hw(lectura.ac, lectura.choices)
        if perfil_hw != self.perfil_hw_actual or epp != self.epp_actual:
            self.perfil_hw_actual = perfil_hw
            self.epp_actual = epp
            return AjustarSilencio(perfil_hw, epp)

        return None

    def al_soltar_externo(self, ahora: float) -> None:
        self.sosteniendo = False
        self.perfil_hw_actual = None
        self.epp_actual = None
        self.desde_baja_carga = None
        self._reset_salidas()
        self.cooldown_hasta = max(self.cooldown_hasta, ahora + COOLDOWN_MANUAL_S)

    def al_liberar_usuario(self, ahora: float) -> bool:
        estaba = self.sosteniendo
        self.al_soltar_externo(ahora)
        return estaba

    def al_sueno(self, durmiendo: bool, ahora: float) -> bool:
        if durmiendo:
            self.en_sueno = True
            estaba = self.sosteniendo
            self.sosteniendo = False
            self.perfil_hw_actual = None
            self.epp_actual = None
            self.desde_baja_carga = None
            self._reset_salidas()
            return estaba
        self.en_sueno = False
        self.desde_baja_carga = None
        self._reset_salidas()
        self.cooldown_hasta = max(self.cooldown_hasta, ahora + COOLDOWN_REANUDAR_S)
        return False


def resolver_primera_existente(rutas: tuple[Path, ...]) -> Path | None:
    for ruta in rutas:
        if ruta.exists():
            return ruta
    return None


def parsear_proc_stat(texto: str) -> tuple[int, int] | None:
    """Extrae `(ocupado, total)` de la primera línea `cpu ` de `/proc/stat`."""
    for linea in texto.splitlines():
        if linea.startswith("cpu "):
            campos = linea.split()[1:]
            if len(campos) < 4:
                return None
            valores = [int(c) for c in campos]
            inactivo = valores[3] + (valores[4] if len(valores) > 4 else 0)
            total = sum(valores)
            return (total - inactivo, total)
    return None


def escribir_atomico(ruta: Path, contenido: str, modo: int = 0o644) -> None:
    ruta.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_str = tempfile.mkstemp(prefix=".estado-", dir=str(ruta.parent))
    tmp = Path(tmp_str)
    try:
        os.chmod(fd, modo)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(contenido)
        os.replace(tmp, ruta)
    except Exception:
        tmp.unlink(missing_ok=True)
        raise


class SysfsHardware:
    """Acceso de bajo overhead a sensores térmicos, CPU, AC y `platform_profile`."""

    def __init__(self, estado_path: Path = ESTADO_PATH) -> None:
        self.estado_path = estado_path
        self._zona_temp = self.buscar_zona_termica()
        self._muestra_cpu: tuple[int, int] | None = self._leer_muestra_cpu()

    @staticmethod
    def buscar_zona_termica(raiz: Path = Path("/sys/class/thermal")) -> Path | None:
        """Busca zona térmica CPU (`TCPU` > `x86_pkg_temp` > `acpitz`).

        Nunca lee `/sys/class/hwmon/hwmon1/fan1_input` (`acpi_fan`), que en
        esta BIOS dispara errores ACPI `FAN0._FST` en cada lectura.
        """
        preferencia = {"TCPU": 0, "x86_pkg_temp": 1, "acpitz": 2}
        mejor: tuple[int, Path] | None = None
        for entry in sorted(raiz.glob("thermal_zone*")):
            tipo_path = entry / "type"
            temp_path = entry / "temp"
            if not tipo_path.is_file() or not temp_path.is_file():
                continue
            try:
                tipo = tipo_path.read_text(encoding="utf-8").strip()
            except OSError:
                continue
            prio = preferencia.get(tipo)
            if prio is not None and (mejor is None or prio < mejor[0]):
                mejor = (prio, temp_path)
        return mejor[1] if mejor is not None else None

    def leer_temp_c(self) -> float | None:
        if self._zona_temp is None:
            self._zona_temp = self.buscar_zona_termica()
        if self._zona_temp is None:
            return None
        try:
            raw = self._zona_temp.read_text(encoding="utf-8").strip()
            return int(raw) / 1000.0
        except (OSError, ValueError):
            return None

    @staticmethod
    def _leer_muestra_cpu() -> tuple[int, int] | None:
        try:
            return parsear_proc_stat(Path("/proc/stat").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None

    def leer_cpu_pct(self) -> float:
        actual = self._leer_muestra_cpu()
        if actual is None:
            return 0.0
        anterior = self._muestra_cpu
        self._muestra_cpu = actual
        if anterior is None:
            return 0.0
        delta_total = actual[1] - anterior[1]
        if delta_total <= 0:
            return 0.0
        delta_ocupado = max(0, actual[0] - anterior[0])
        return min(100.0, (delta_ocupado / delta_total) * 100.0)

    @staticmethod
    def leer_ac() -> bool:
        adp1 = Path("/sys/class/power_supply/ADP1/online")
        if adp1.is_file():
            try:
                return adp1.read_text(encoding="utf-8").strip() == "1"
            except OSError:
                pass
        for ps in sorted(Path("/sys/class/power_supply").glob("*")):
            tipo = ps / "type"
            online = ps / "online"
            if not tipo.is_file() or not online.is_file():
                continue
            try:
                if tipo.read_text(encoding="utf-8").strip() == "Mains":
                    return online.read_text(encoding="utf-8").strip() == "1"
            except OSError:
                continue
        return False

    @staticmethod
    def resolver_platform_profile() -> Path | None:
        return resolver_primera_existente(RUTAS_PLATFORM_PROFILE)

    @staticmethod
    def resolver_platform_choices() -> Path | None:
        return resolver_primera_existente(RUTAS_PLATFORM_CHOICES)

    def leer_choices(self) -> tuple[str, ...]:
        ruta = self.resolver_platform_choices()
        if ruta is None:
            return ()
        try:
            return tuple(ruta.read_text(encoding="utf-8").split())
        except OSError:
            return ()

    def escribir_platform_profile(self, perfil: str) -> None:
        ruta = self.resolver_platform_profile()
        if ruta is None:
            return
        try:
            ruta.write_text(f"{perfil}\n", encoding="utf-8")
        except OSError:
            pass

    @staticmethod
    def escribir_epp(valor: str) -> None:
        for avail_path_str in sorted(
            glob.glob("/sys/devices/system/cpu/cpu*/cpufreq/energy_performance_available_preferences")
        ):
            avail_path = Path(avail_path_str)
            epp_path = avail_path.with_name("energy_performance_preference")
            try:
                disponibles = avail_path.read_text(encoding="utf-8").split()
                if valor in disponibles:
                    epp_path.write_text(f"{valor}\n", encoding="utf-8")
            except OSError:
                continue

    def escribir_estado(self, estado: str) -> None:
        try:
            escribir_atomico(self.estado_path, f"{estado}\n", modo=0o644)
        except OSError:
            pass


class ServicioTermico:
    """Conecta `ControlTermico` con systemd-logind y power-profiles-daemon."""

    def __init__(self) -> None:
        if Gio is None or GLib is None:
            raise RuntimeError("PyGObject (Gio/GLib) es obligatorio para ejecutar el servicio")
        self.bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        self.hw = SysfsHardware()
        self.control = ControlTermico()
        self.cookie: int | None = None
        self.loop = GLib.MainLoop()
        self._subscripciones: list[int] = []
        self._reg_id: int | None = None
        self._own_id: int | None = None

    def iniciar(self) -> None:
        self.hw.escribir_estado("normal")

        node_info = Gio.DBusNodeInfo.new_for_xml(INTROSPECCION_XML)
        self._reg_id = self.bus.register_object(
            OBJ_TERMICO,
            node_info.interfaces[0],
            self._on_method_call,
            None,
            None,
        )
        self._own_id = Gio.bus_own_name_on_connection(
            self.bus,
            BUS_TERMICO,
            Gio.BusNameOwnerFlags.REPLACE,
            None,
            None,
        )

        self._subscripciones.append(
            self.bus.signal_subscribe(
                BUS_PPD,
                IFACE_PPD,
                "ProfileReleased",
                OBJ_PPD,
                None,
                Gio.DBusSignalFlags.NONE,
                self._on_profile_released,
                None,
            )
        )
        self._subscripciones.append(
            self.bus.signal_subscribe(
                "org.freedesktop.login1",
                "org.freedesktop.login1.Manager",
                "PrepareForSleep",
                "/org/freedesktop/login1",
                None,
                Gio.DBusSignalFlags.NONE,
                self._on_prepare_for_sleep,
                None,
            )
        )

        GLib.timeout_add_seconds(POLL_S, self._on_tick)

    def _on_method_call(
        self,
        _connection: Gio.DBusConnection,
        _sender: str,
        _object_path: str,
        _interface_name: str,
        method_name: str,
        _parameters: GLib.Variant,
        invocation: Gio.DBusMethodInvocation,
        _user_data: object,
    ) -> None:
        if method_name == "Liberar":
            ahora = GLib.get_monotonic_time() / 1_000_000.0
            if self.control.al_liberar_usuario(ahora):
                self._liberar_hold()
            else:
                self.hw.escribir_estado("normal")
            invocation.return_value(None)
        elif method_name == "Estado":
            estado = "balanced-quiet" if self.control.sosteniendo else "normal"
            invocation.return_value(GLib.Variant("(s)", (estado,)))
        else:
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                f"Método desconocido: {method_name}",
            )

    def _leer_perfil_ppd(self) -> str | None:
        try:
            res = self.bus.call_sync(
                BUS_PPD,
                OBJ_PPD,
                IFACE_PROPS,
                "Get",
                GLib.Variant("(ss)", (IFACE_PPD, "ActiveProfile")),
                GLib.VariantType("(v)"),
                Gio.DBusCallFlags.NONE,
                1500,
                None,
            )
            return str(res.unpack()[0])
        except GLib.Error:
            return None

    def _tomar_hold(self) -> bool:
        if self.cookie is not None:
            return True
        try:
            res = self.bus.call_sync(
                BUS_PPD,
                OBJ_PPD,
                IFACE_PPD,
                "HoldProfile",
                GLib.Variant(
                    "(sss)",
                    (
                        "power-saver",
                        "Silencio adaptativo en baja carga",
                        BUS_TERMICO,
                    ),
                ),
                GLib.VariantType("(u)"),
                Gio.DBusCallFlags.NONE,
                2000,
                None,
            )
            self.cookie = int(res.unpack()[0])
            return True
        except GLib.Error:
            self.cookie = None
            return False

    def _liberar_hold(self) -> None:
        cookie = self.cookie
        self.cookie = None
        if cookie is not None:
            try:
                self.bus.call_sync(
                    BUS_PPD,
                    OBJ_PPD,
                    IFACE_PPD,
                    "ReleaseProfile",
                    GLib.Variant("(u)", (cookie,)),
                    None,
                    Gio.DBusCallFlags.NONE,
                    2000,
                    None,
                )
            except GLib.Error:
                pass
        self.hw.escribir_estado("normal")

    def _aplicar_perfil_silencioso(self, perfil_hw: str, epp: str) -> None:
        if perfil_hw == "quiet":
            self.hw.escribir_platform_profile("quiet")
        else:
            self.hw.escribir_platform_profile("low-power")
        self.hw.escribir_epp(epp)

    def _on_profile_released(
        self,
        _connection: Gio.DBusConnection,
        _sender: str,
        _object_path: str,
        _interface_name: str,
        _signal_name: str,
        parameters: GLib.Variant,
        _user_data: object,
    ) -> None:
        cookie_soltada = int(parameters.unpack()[0])
        if self.cookie is not None and cookie_soltada == self.cookie:
            self.cookie = None
            ahora = GLib.get_monotonic_time() / 1_000_000.0
            self.control.al_soltar_externo(ahora)
            self.hw.escribir_estado("normal")

    def _on_prepare_for_sleep(
        self,
        _connection: Gio.DBusConnection,
        _sender: str,
        _object_path: str,
        _interface_name: str,
        _signal_name: str,
        parameters: GLib.Variant,
        _user_data: object,
    ) -> None:
        durmiendo = bool(parameters.unpack()[0])
        ahora = GLib.get_monotonic_time() / 1_000_000.0
        if self.control.al_sueno(durmiendo, ahora):
            self._liberar_hold()

    def _on_tick(self) -> bool:
        temp_c = self.hw.leer_temp_c()
        if temp_c is None:
            return True
        perfil_ppd = self._leer_perfil_ppd()
        if perfil_ppd is None:
            return True

        lectura = Lectura(
            temp_c=temp_c,
            cpu_pct=self.hw.leer_cpu_pct(),
            perfil_ppd=perfil_ppd,
            ac=self.hw.leer_ac(),
            choices=self.hw.leer_choices(),
        )
        ahora = GLib.get_monotonic_time() / 1_000_000.0
        accion = self.control.evaluar(lectura, ahora)

        if isinstance(accion, ActivarSilencio):
            # Escribimos el estado *antes* de HoldProfile para que el panel no
            # muestre el amarillo de ahorro de energía en la transición.
            self.hw.escribir_estado("balanced-quiet")
            if self._tomar_hold():
                self._aplicar_perfil_silencioso(accion.perfil_hw, accion.epp)
            else:
                self.control.al_soltar_externo(ahora)
                self.hw.escribir_estado("normal")
        elif isinstance(accion, AjustarSilencio):
            self._aplicar_perfil_silencioso(accion.perfil_hw, accion.epp)
        elif isinstance(accion, SoltarSilencio):
            self._liberar_hold()

        return True

    def detener(self) -> None:
        self._liberar_hold()
        for sub in self._subscripciones:
            self.bus.signal_unsubscribe(sub)
        self._subscripciones.clear()
        if self._own_id is not None:
            Gio.bus_unown_name(self._own_id)
            self._own_id = None
        if self._reg_id is not None:
            self.bus.unregister_object(self._reg_id)
            self._reg_id = None
        if self.loop.is_running():
            self.loop.quit()


def main() -> int:
    servicio = ServicioTermico()

    def _salir(_sig: int, _frame: object) -> None:
        servicio.detener()

    signal.signal(signal.SIGTERM, _salir)
    signal.signal(signal.SIGINT, _salir)

    servicio.iniciar()
    try:
        servicio.loop.run()
    finally:
        servicio.detener()
    return 0


if __name__ == "__main__":
    sys.exit(main())
