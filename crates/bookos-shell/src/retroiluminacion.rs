//! Escrituras de brillo serializadas fuera del hilo de renderizado.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::config::BrilloAutomatico;

/// Lo que eligió el usuario, visible para la tarjeta sin pasarle la `Config`.
/// Lo escribe el compositor al arrancar y al cambiarlo; es el mismo reparto
/// que `tema::aplicar_modo`. Bit 0 pantalla, bit 1 teclado.
static AUTOMATICO: AtomicU8 = AtomicU8::new(0);

// 0.1.0
pub fn automatico() -> BrilloAutomatico {
    let bits = AUTOMATICO.load(Ordering::Relaxed);
    BrilloAutomatico {
        pantalla: bits & 1 != 0,
        teclado: bits & 2 != 0,
    }
}

// 0.1.0
pub fn aplicar_automatico(elegido: BrilloAutomatico) {
    let bits = u8::from(elegido.pantalla) | u8::from(elegido.teclado) << 1;
    AUTOMATICO.store(bits, Ordering::Relaxed);
}

/// El brillo de pantalla que pide la luz ahora mismo, sin el ajuste a mano. 0
/// es «no se sabe»: sin automático o sin lectura todavía. El compositor lo
/// escribe cada vez que evalúa; la tarjeta lo lee para enseñar cuánto se ha
/// movido el usuario respecto a él.
static OBJETIVO_SENSOR: AtomicU8 = AtomicU8::new(0);

// 0.1.0
pub fn objetivo_sensor() -> Option<u8> {
    Some(OBJETIVO_SENSOR.load(Ordering::Relaxed)).filter(|&n| n > 0)
}

// 0.1.0
pub fn poner_objetivo_sensor(nivel: Option<u8>) {
    OBJETIVO_SENSOR.store(nivel.unwrap_or(0), Ordering::Relaxed);
}

/// ¿Hay sensor de luz ambiente? Se mira en sysfs y no en iio-sensor-proxy para
/// que la tarjeta no espere al bus: un `readdir` de `/sys/bus/iio/devices`.
// 0.1.0
pub fn hay_sensor_luz() -> bool {
    std::fs::read_dir("/sys/bus/iio/devices").is_ok_and(|dir| {
        // `raw` en los sensores HID, como el del ISH; `input` en otros
        // controladores que ya dan lux.
        dir.filter_map(Result::ok).any(|e| {
            ["in_illuminance_raw", "in_illuminance_input"]
                .iter()
                .any(|f| e.path().join(f).exists())
        })
    })
}

type Cola = (Mutex<BTreeMap<(String, String), u32>>, Condvar);

/// Conserva el último valor solicitado por dispositivo. Un arrastre no crea
/// procesos en paralelo ni permite que una escritura antigua termine después.
// 0.1.0
pub fn solicitar(subsistema: &str, dispositivo: &str, nivel: u32) {
    if !matches!(subsistema, "backlight" | "leds") || dispositivo.contains('/') {
        return;
    }
    static COLA: OnceLock<Arc<Cola>> = OnceLock::new();
    let cola = COLA.get_or_init(|| {
        let cola: Arc<Cola> = Arc::new((Mutex::new(BTreeMap::new()), Condvar::new()));
        let worker = cola.clone();
        std::thread::Builder::new()
            .name("bookos-brillo".into())
            .spawn(move || {
                loop {
                    let lote = {
                        let mut pendientes = worker.0.lock().unwrap();
                        while pendientes.is_empty() {
                            pendientes = worker.1.wait(pendientes).unwrap();
                        }
                        std::mem::take(&mut *pendientes)
                    };
                    for ((subsistema, dispositivo), nivel) in lote {
                        let result = std::process::Command::new("busctl")
                            .args([
                                "--timeout=2s",
                                "call",
                                "org.freedesktop.login1",
                                "/org/freedesktop/login1/session/auto",
                                "org.freedesktop.login1.Session",
                                "SetBrightness",
                                "ssu",
                                &subsistema,
                                &dispositivo,
                                &nivel.to_string(),
                            ])
                            .output();
                        match result {
                            Ok(out) if out.status.success() => {}
                            Ok(out) => eprintln!(
                                "No se pudo cambiar el brillo de {dispositivo}: {}",
                                String::from_utf8_lossy(&out.stderr).trim()
                            ),
                            Err(e) => {
                                eprintln!("No se pudo cambiar el brillo de {dispositivo}: {e}")
                            }
                        }
                    }
                }
            })
            .expect("hilo de retroiluminación");
        cola
    });
    cola.0
        .lock()
        .unwrap()
        .insert((subsistema.into(), dispositivo.into()), nivel);
    cola.1.notify_one();
}
