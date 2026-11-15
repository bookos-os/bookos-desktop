//! `org.freedesktop.ScreenSaver`: las apps piden que no se apague la pantalla.
//!
//! Hasta ahora solo contaba el protocolo de Wayland `zwp_idle_inhibit_v1`, y
//! muchas apps no lo usan: Firefox (Aether Browser) prueba **primero** este
//! nombre del bus, y Chromium, Electron y WebKitGTK solo hablan por aquí. Sin
//! nadie que lo atendiera, la pantalla se atenuaba a mitad de un vídeo.
//!
//! Cada `Inhibit` devuelve una cookie y se apunta con el nombre único de quien
//! la pidió: si la app se cierra o se cuelga sin llamar a `UnInhibit`, la
//! inhibición se suelta al irse su conexión del bus (ver
//! `Aviso::RemitenteCaido`), y no deja la pantalla encendida para siempre.

use smithay::reexports::calloop::channel::Sender;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ajustes::Aviso;

// añadido en la 0.2.2
pub const NOMBRE: &str = "org.freedesktop.ScreenSaver";

/// Las dos rutas que usan las apps: KDE y Chromium van a la larga, Firefox y
/// GNOME a la corta. Las dos comparten el contador de cookies.
const RUTAS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

struct ScreenSaver {
    canal: Sender<Aviso>,
    siguiente: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    // añadido en la 0.2.2
    fn inhibit(
        &self,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        application_name: String,
        reason_for_inhibit: String,
    ) -> u32 {
        // Empieza en 1: algunas apps toman la cookie 0 como «falló».
        let cookie = self.siguiente.fetch_add(1, Ordering::Relaxed);
        let remitente = cabecera.sender().map(|u| u.to_string()).unwrap_or_default();
        tracing::info!(
            app = application_name,
            motivo = reason_for_inhibit,
            cookie,
            "una app pide que no se apague la pantalla"
        );
        let _ = self.canal.send(Aviso::Inhibir(cookie, remitente));
        cookie
    }

    // añadido en la 0.2.2
    fn un_inhibit(&self, cookie: u32) {
        let _ = self.canal.send(Aviso::Desinhibir(cookie));
    }

    /// El salvapantallas de BookOS no se pregunta por aquí; se contesta para
    /// que las apps que lo consultan antes de inhibir no den error.
    // añadido en la 0.2.2
    fn get_active(&self) -> bool {
        false
    }
}

/// Atiende `org.freedesktop.ScreenSaver` en la conexión de ajustes. Si otro
/// escritorio ya tiene el nombre (desarrollando anidado sobre Plasma), se
/// avisa y se sigue: `vigilar_nombres` lo volverá a pedir.
// añadido en la 0.2.2
pub fn arrancar(conexion: &zbus::blocking::Connection, canal: Sender<Aviso>) {
    let siguiente = Arc::new(AtomicU32::new(1));
    for ruta in RUTAS {
        let servido = conexion.object_server().at(
            ruta,
            ScreenSaver {
                canal: canal.clone(),
                siguiente: siguiente.clone(),
            },
        );
        if let Err(err) = servido {
            tracing::warn!("sin {NOMBRE} en {ruta}: {err}");
            return;
        }
    }
    if let Err(err) = conexion.request_name(NOMBRE) {
        tracing::warn!("sin {NOMBRE}: {err}");
    }
}
