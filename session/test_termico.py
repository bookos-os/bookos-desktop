#!/usr/bin/env python3
"""Pruebas unitarias para la máquina de estados térmica de `bookos-termico.py`."""

import importlib.util
from pathlib import Path
import tempfile
import unittest


def cargar_modulo():
    ruta = Path(__file__).with_name("bookos-termico.py")
    spec = importlib.util.spec_from_file_location("bookos_termico", ruta)
    assert spec is not None and spec.loader is not None
    modulo = importlib.util.module_from_spec(spec)
    import sys

    sys.modules[spec.name] = modulo
    spec.loader.exec_module(modulo)
    return modulo


bt = cargar_modulo()


def lectura(
    temp_c: float = 44.0,
    cpu_pct: float = 5.0,
    perfil_ppd: str = "balanced",
    ac: bool = True,
    choices: tuple[str, ...] = ("low-power", "quiet", "balanced", "performance"),
):
    return bt.Lectura(
        temp_c=temp_c,
        cpu_pct=cpu_pct,
        perfil_ppd=perfil_ppd,
        ac=ac,
        choices=choices,
    )


class TestControlTermico(unittest.TestCase):
    def test_entra_en_quiet_con_ac_y_low_power_en_bateria(self):
        ctrl = bt.ControlTermico()
        # No entra antes de cumplir ENTRAR_S (15 s)
        for t in (0.0, 3.0, 6.0, 9.0, 12.0):
            self.assertIsNone(ctrl.evaluar(lectura(ac=True), t))
            self.assertFalse(ctrl.sosteniendo)

        # A los 15 s entra en quiet + balance_power con AC
        accion = ctrl.evaluar(lectura(ac=True), 15.0)
        self.assertEqual(accion, bt.ActivarSilencio("quiet", "balance_power"))
        self.assertTrue(ctrl.sosteniendo)

        # Desenchufar mientras sostiene cambia a low-power + power sin soltar hold
        ajuste = ctrl.evaluar(lectura(ac=False, perfil_ppd="power-saver"), 18.0)
        self.assertEqual(ajuste, bt.AjustarSilencio("low-power", "power"))
        self.assertTrue(ctrl.sosteniendo)

        # Volver a enchufar restaura quiet + balance_power
        ajuste_ac = ctrl.evaluar(lectura(ac=True, perfil_ppd="power-saver"), 21.0)
        self.assertEqual(ajuste_ac, bt.AjustarSilencio("quiet", "balance_power"))

    def test_en_bateria_o_sin_quiet_elige_low_power(self):
        ctrl = bt.ControlTermico()
        self.assertIsNone(ctrl.evaluar(lectura(ac=True, choices=("low-power", "balanced")), 0.0))
        accion = ctrl.evaluar(lectura(ac=True, choices=("low-power", "balanced")), 15.0)
        self.assertEqual(accion, bt.ActivarSilencio("low-power", "power"))

    def test_ignora_picos_breves_debajo_de_salida(self):
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        self.assertTrue(ctrl.sosteniendo)

        # Ráfaga corta de CPU (< 6 s) y temperatura moderada (60 °C < 65 °C)
        self.assertIsNone(ctrl.evaluar(lectura(temp_c=60.0, cpu_pct=50.0, perfil_ppd="power-saver"), 18.0))
        # Vuelve a bajar antes de SALIR_CARGA_S -> sigue en silencio
        self.assertIsNone(ctrl.evaluar(lectura(temp_c=58.0, cpu_pct=12.0, perfil_ppd="power-saver"), 21.0))
        self.assertTrue(ctrl.sosteniendo)

    def test_sale_por_temperatura_sostenida_pico_y_carga(self):
        # 1. Temperatura sostenida >= 65 °C durante 3 s
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        self.assertIsNone(ctrl.evaluar(lectura(temp_c=65.0, perfil_ppd="power-saver"), 20.0))
        accion = ctrl.evaluar(lectura(temp_c=66.0, perfil_ppd="power-saver"), 23.0)
        self.assertIsInstance(accion, bt.SoltarSilencio)
        self.assertFalse(ctrl.sosteniendo)

        # 2. Pico >= 72 °C sale en el acto
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        accion = ctrl.evaluar(lectura(temp_c=72.0, perfil_ppd="power-saver"), 18.0)
        self.assertIsInstance(accion, bt.SoltarSilencio)
        self.assertFalse(ctrl.sosteniendo)

        # 3. Carga CPU >= 45 % sostenida 6 s
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        self.assertIsNone(ctrl.evaluar(lectura(cpu_pct=55.0, perfil_ppd="power-saver"), 20.0))
        self.assertIsNone(ctrl.evaluar(lectura(cpu_pct=60.0, perfil_ppd="power-saver"), 23.0))
        accion = ctrl.evaluar(lectura(cpu_pct=48.0, perfil_ppd="power-saver"), 26.0)
        self.assertIsInstance(accion, bt.SoltarSilencio)
        self.assertFalse(ctrl.sosteniendo)

    def test_histeresis_no_reentra_en_zona_templada(self):
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        ctrl.evaluar(lectura(temp_c=73.0, perfil_ppd="power-saver"), 18.0)
        self.assertFalse(ctrl.sosteniendo)

        # Con 60 °C (por encima de 56 °C) no reentra nunca
        for t in (21.0, 30.0, 50.0, 80.0):
            self.assertIsNone(ctrl.evaluar(lectura(temp_c=60.0, cpu_pct=5.0), t))

        # Cuando baja <= 56 °C y <= 22 % CPU necesita de nuevo 15 s
        self.assertIsNone(ctrl.evaluar(lectura(temp_c=55.0, cpu_pct=10.0), 83.0))
        self.assertIsNone(ctrl.evaluar(lectura(temp_c=55.0, cpu_pct=10.0), 95.0))
        accion = ctrl.evaluar(lectura(temp_c=55.0, cpu_pct=10.0), 98.0)
        self.assertIsInstance(accion, bt.ActivarSilencio)

    def test_no_interfiere_con_performance_ni_power_saver_manual(self):
        ctrl = bt.ControlTermico()
        for perfil in ("performance", "power-saver"):
            self.assertIsNone(ctrl.evaluar(lectura(perfil_ppd=perfil), 0.0))
            self.assertIsNone(ctrl.evaluar(lectura(perfil_ppd=perfil), 20.0))
            self.assertFalse(ctrl.sosteniendo)

    def test_cooldown_tras_liberacion_manual_o_reanudacion(self):
        ctrl = bt.ControlTermico()
        ctrl.evaluar(lectura(), 0.0)
        ctrl.evaluar(lectura(), 15.0)
        self.assertTrue(ctrl.al_liberar_usuario(20.0))
        self.assertFalse(ctrl.sosteniendo)

        # Durante COOLDOWN_MANUAL_S (45 s -> hasta t=65) no vuelve a empezar
        self.assertIsNone(ctrl.evaluar(lectura(), 30.0))
        self.assertIsNone(ctrl.evaluar(lectura(), 64.0))
        self.assertIsNone(ctrl.evaluar(lectura(), 66.0))
        accion = ctrl.evaluar(lectura(), 81.0)
        self.assertIsInstance(accion, bt.ActivarSilencio)

        # En suspensión suelta el hold y tras despertar respeta COOLDOWN_REANUDAR_S (10 s)
        self.assertTrue(ctrl.al_sueno(True, 90.0))
        self.assertIsNone(ctrl.evaluar(lectura(), 95.0))
        self.assertFalse(ctrl.al_sueno(False, 100.0))
        self.assertIsNone(ctrl.evaluar(lectura(), 109.0))
        self.assertIsNone(ctrl.evaluar(lectura(), 110.0))
        self.assertIsInstance(ctrl.evaluar(lectura(), 125.0), bt.ActivarSilencio)


class TestSysfsHelpers(unittest.TestCase):
    def test_parsear_proc_stat(self):
        muestra = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n"
        self.assertEqual(bt.parsear_proc_stat(muestra), (150, 1000))

    def test_buscar_zona_termica_prefiere_tcpu_e_ignora_hwmon(self):
        with tempfile.TemporaryDirectory() as tmp:
            raiz = Path(tmp)
            (raiz / "thermal_zone0").mkdir()
            (raiz / "thermal_zone0" / "type").write_text("acpitz\n")
            (raiz / "thermal_zone0" / "temp").write_text("41000\n")

            (raiz / "thermal_zone5").mkdir()
            (raiz / "thermal_zone5" / "type").write_text("TCPU\n")
            (raiz / "thermal_zone5" / "temp").write_text("43050\n")

            elegida = bt.SysfsHardware.buscar_zona_termica(raiz)
            self.assertEqual(elegida, raiz / "thermal_zone5" / "temp")


if __name__ == "__main__":
    unittest.main()
