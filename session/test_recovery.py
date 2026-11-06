import importlib.util
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("recovery", Path(__file__).with_name("bookos-recovery.py"))
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class RecoveryTests(unittest.TestCase):
    def run_supervisor(self, failure, reply="0"):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            binary = root / "compositor"
            binary.write_text("#!/bin/bash\n" + failure + "\n")
            binary.chmod(0o700)
            helper = root / "recovery.py"
            helper.write_text("import sys\nfrom pathlib import Path\n"
                              f"Path({str(root / 'called')!r}).write_text(' '.join(sys.argv))\n"
                              f"sys.exit({reply})\n")
            script = Path(__file__).with_name("bookos-session").read_text()
            body = "sesion_tmp=" + script.split("\nsesion_tmp=", 1)[1]
            header = 'set -u\numask 077\nbinario="$TEST_ROOT/compositor"\nrecuperacion="$TEST_ROOT/recovery.py"\nregistro="$TEST_ROOT/session.log"\nlanzar_autostart() { :; }\n'
            env = os.environ | {"TEST_ROOT": str(root), "XDG_RUNTIME_DIR": str(root), "XDG_STATE_HOME": str(root / "state")}
            result = subprocess.run(["bash", "-c", header + body], env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            called = root / "called"
            log = root / "state/bookos/compositor-last-crash.log"
            return called.read_text() if called.exists() else None, log.read_text() if log.exists() else None

    def test_normal_logout_does_not_open_recovery(self):
        self.assertEqual(self.run_supervisor("exit 0"), (None, None))

    def test_crash_records_exit_code_and_opens_recovery(self):
        called, log = self.run_supervisor("echo fallo-simulado; exit 42")
        self.assertIn("--status 42", called)
        self.assertNotIn("--locked", called)
        self.assertIn("fallo-simulado", log)

    def test_crash_while_locked_is_reported_as_locked(self):
        called, _ = self.run_supervisor('rm -f "$BOOKOS_SESSION_LOCK_FILE"; exit 42')
        self.assertIn("--locked", called)

    def test_lock_marker_failure_requires_login_even_with_stale_marker(self):
        called, _ = self.run_supervisor("exit 77")
        self.assertIn("--locked", called)

    def test_explicit_retry_restarts_compositor(self):
        called, _ = self.run_supervisor('if [ -f "$TEST_ROOT/attempt" ]; then exit 0; fi\ntouch "$TEST_ROOT/attempt"; exit 42', "10")
        self.assertIn("--status 42", called)

    def test_only_installed_sessions(self):
        with patch.object(recovery.shutil, "which", side_effect=lambda name: "/usr/bin/startplasma-wayland" if name == "startplasma-wayland" else None):
            self.assertEqual(recovery.sessions(), [("Plasma", "KDE", ["/usr/bin/startplasma-wayland"])])

    def test_retry_is_explicit_and_never_allowed_when_locked(self):
        with patch.object(recovery, "prompt", return_value="retry"), patch.object(recovery.subprocess, "call") as call:
            self.assertEqual(recovery.recover(SimpleNamespace(locked=False)), 10)
            self.assertEqual(recovery.recover(SimpleNamespace(locked=True)), 0)
            call.assert_not_called()

    def test_locked_session_cannot_launch_another_desktop(self):
        with patch.object(recovery, "prompt", return_value="Plasma"), patch.object(recovery.subprocess, "call") as call:
            self.assertEqual(recovery.recover(SimpleNamespace(locked=True)), 0)
            call.assert_not_called()

    def test_fallback_gets_clean_environment(self):
        with patch.dict(os.environ, {"WAYLAND_DISPLAY": "broken", "DISPLAY": ":99", "BOOKOS_SESSION_LOCK_FILE": "old", "QT_QPA_PLATFORM": "wayland", "XDG_CURRENT_DESKTOP": "BookOS"}), \
             patch.object(recovery, "prompt", return_value="Plasma"), \
             patch.object(recovery, "sessions", return_value=[("Plasma", "KDE", ["/usr/bin/startplasma-wayland"])]), \
             patch.object(recovery.subprocess, "call", return_value=0) as call:
            self.assertEqual(recovery.recover(SimpleNamespace(locked=False)), 0)
            env = call.call_args.kwargs["env"]
            self.assertEqual(env["XDG_CURRENT_DESKTOP"], "KDE")
            for key in ("WAYLAND_DISPLAY", "DISPLAY", "BOOKOS_SESSION_LOCK_FILE", "QT_QPA_PLATFORM"):
                self.assertNotIn(key, env)

    def test_light_palette_not_overridden_by_dark_media_fallback(self):
        with tempfile.TemporaryDirectory() as root:
            config = Path(root) / "bookos"
            config.mkdir()
            config.joinpath("panel.conf").write_text("tema = claro\n")
            config.joinpath("palette.css").write_text(":root{--bg:#ffffff;--blue:#123456;} :root.dark-mode{--bg:#111111;} @media(prefers-color-scheme:dark){:root:not(.light-mode){--bg:#222222;}}")
            with patch.dict(os.environ, {"XDG_CONFIG_HOME": root}):
                self.assertEqual(recovery.palette()["bg"], "#ffffff")
                self.assertEqual(recovery.palette()["blue"], "#123456")

    def test_points_newest_first_without_current_system(self):
        text = 'number,date,description\n0,,current\n3,2026-10-20 18:12:00,Antes\n7,2026-11-02 12:00:00,"Con, coma"\n'
        self.assertEqual([p["number"] for p in recovery.parse_points(text)], ["7", "3"])
        self.assertEqual(recovery.parse_points(text)[0]["description"], "Con, coma")

    def test_choosing_reboot_reboots(self):
        with patch.object(recovery, "prompt", return_value="reboot"), patch.object(recovery.subprocess, "call") as call:
            self.assertEqual(recovery.recover(SimpleNamespace(locked=False)), 0)
            call.assert_called_once_with(["systemctl", "reboot"])


class RestoreScriptTests(unittest.TestCase):
    """The real script against stubbed system commands; the "disk" is a directory."""

    def run_script(self, *args, kernel=None, options="rw,subvol=/root", mv_fails=False):
        root = Path(self.tmp.name)
        disk, bin_dir = root / "disk", root / "bin"
        kernel = kernel or next(p.name.removeprefix("vmlinuz-") for p in Path("/boot").glob("vmlinuz-[0-9]*"))
        (disk / "root/.snapshots/5/snapshot/usr/lib/modules" / kernel).mkdir(parents=True)
        (disk / "root/.snapshots/5/snapshot/.snapshots").mkdir()
        (disk / "root/.snapshots/5/snapshot/marca").write_text("punto")
        (disk / "root/marca").write_text("actual")
        bin_dir.mkdir()
        stubs = {
            "findmnt": f'case "$*" in *OPTIONS*) echo "{options}";; *) echo "/dev/x[/root]";; esac',
            "mount": f'for a; do t="$a"; done; rmdir "$t"; ln -s "{disk}" "$t"',
            "umount": 'rm "$1"; mkdir "$1"',
            "snapper": f'echo "$*" >> "{root}/snapper.log"',
            "grubby": f'echo "$*" > "{root}/grubby.log"',
            "sync": "true",
            "btrfs": 'case "$2" in snapshot) cp -a "$3" "$4";; delete) rm -rf "$3";; esac',
        }
        if mv_fails:
            # Only the second rename fails, the point where a revert is needed.
            stubs["mv"] = f'case "$1" in *root.nuevo) exit 1;; esac; /usr/bin/mv "$@"'
        for name, body in stubs.items():
            path = bin_dir / name
            path.write_text("#!/bin/sh\n" + body + "\n")
            path.chmod(0o700)
        env = os.environ | {"PATH": f"{bin_dir}:/usr/bin:/bin"}
        script = Path(__file__).with_name("bookos-restaurar")
        return disk, subprocess.run(["sh", str(script), *args], env=env, capture_output=True, text=True, timeout=10)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(lambda: self.tmp.cleanup())

    def test_restore_swaps_root_and_keeps_points(self):
        disk, result = self.run_script("restaurar", "5")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((disk / "root/marca").read_text(), "punto")
        self.assertTrue((disk / "root/.snapshots/5/snapshot").is_dir())
        previous = list(disk.glob("root.antes-*"))
        self.assertEqual(len(previous), 1)
        self.assertEqual((previous[0] / "marca").read_text(), "actual")
        log = Path(self.tmp.name) / "snapper.log"
        self.assertIn("Antes de restaurar el punto 5", log.read_text())
        self.assertIn("/boot/vmlinuz-", (Path(self.tmp.name) / "grubby.log").read_text())

    def test_no_compatible_kernel_touches_nothing(self):
        disk, result = self.run_script("restaurar", "5", kernel="0.0.0-no-existe")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("kernel", result.stderr)
        self.assertEqual((disk / "root/marca").read_text(), "actual")
        self.assertFalse((Path(self.tmp.name) / "snapper.log").exists())

    def test_failed_swap_puts_root_back(self):
        disk, result = self.run_script("restaurar", "5", mv_fails=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((disk / "root/marca").read_text(), "actual")
        self.assertFalse(list(disk.glob("root.antes-*")))

    def test_other_layout_is_refused(self):
        disk, result = self.run_script("restaurar", "5", options="rw,subvol=/@")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((disk / "root/marca").read_text(), "actual")

    def test_bad_number_is_refused(self):
        for bad in ("", "5;reboot", "../5"):
            _, result = self.run_script("restaurar", bad)
            self.assertIn("no válido", result.stderr)
            self.tmp.cleanup()
            self.tmp = tempfile.TemporaryDirectory()


if __name__ == "__main__":
    unittest.main()
