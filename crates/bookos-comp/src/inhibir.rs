//! Inhibición de inactividad por D-Bus: las apps piden que no se apague la pantalla.
//!
//! Además de `zwp_idle_inhibit_v1` de Wayland, navegadores y reproductores
//! consultan tres servicios clásicos de escritorio:
//! - `org.freedesktop.ScreenSaver` (Firefox, Chromium, Electron, WebKitGTK)
//! - `org.freedesktop.PowerManagement.Inhibit` (Firefox, Chromium, VLC, Qt)
//! - `org.gnome.SessionManager` (aplicaciones GTK/GNOME cuando pasan flags de
//!   suspensión/inactividad `4 | 8`)
//!
//! Cada `Inhibit` devuelve una cookie y se apunta con el nombre único de quien
//! la pidió: si la app se cierra o se cuelga sin llamar a `UnInhibit`, la
//! inhibición se suelta al irse su conexión del bus (ver
//! `Aviso::RemitenteCaido`), y no deja la pantalla encendida para siempre.

use smithay::reexports::calloop::channel::Sender;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::ajustes::Aviso;

// añadido en la 0.2.2
pub const NOMBRE: &str = "org.freedesktop.ScreenSaver";
pub const NOMBRE_POWER: &str = "org.freedesktop.PowerManagement.Inhibit";
pub const NOMBRE_GNOME: &str = "org.gnome.SessionManager";

/// Las dos rutas de ScreenSaver: KDE y Chromium van a la larga, Firefox y GNOME a la corta.
const RUTAS_SCREENSAVER: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
const RUTA_POWER: &str = "/org/freedesktop/PowerManagement/Inhibit";
const RUTA_GNOME: &str = "/org/gnome/SessionManager";

/// Flags de `org.gnome.SessionManager.Inhibit`: 4 = suspender, 8 = inactivo.
const GNOME_INHIBIR_SUSPENSION: u32 = 4;
const GNOME_INHIBIR_INACTIVO: u32 = 8;

#[derive(Clone)]
pub(crate) struct GestorCookies {
    canal: Sender<Aviso>,
    siguiente: Arc<AtomicU32>,
    activas: Arc<Mutex<HashMap<u32, String>>>,
}

impl GestorCookies {
    fn nueva(canal: Sender<Aviso>) -> Self {
        Self {
            canal,
            // Empieza en 1: algunas apps toman la cookie 0 como «falló».
            siguiente: Arc::new(AtomicU32::new(1)),
            activas: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn siguiente_cookie(&self) -> u32 {
        loop {
            let cookie = self.siguiente.fetch_add(1, Ordering::Relaxed);
            if cookie != 0 {
                return cookie;
            }
        }
    }

    fn registrar_con_remitente(&self, remitente: String, app: &str, motivo: &str) -> u32 {
        let cookie = self.siguiente_cookie();
        self.activas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(cookie, remitente.clone());
        tracing::info!(
            app,
            motivo,
            cookie,
            "una app pide que no se apague la pantalla"
        );
        let _ = self.canal.send(Aviso::Inhibir(cookie, remitente));
        cookie
    }

    fn registrar(
        &self,
        cabecera: &zbus::message::Header<'_>,
        app: String,
        motivo: String,
    ) -> u32 {
        let remitente = cabecera.sender().map(|u| u.to_string()).unwrap_or_default();
        self.registrar_con_remitente(remitente, &app, &motivo)
    }

    fn soltar(&self, cookie: u32) {
        if cookie == 0 {
            return;
        }
        let estaba = self
            .activas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&cookie)
            .is_some();
        if estaba {
            let _ = self.canal.send(Aviso::Desinhibir(cookie));
        }
    }

    pub(crate) fn retirar_remitente(&self, remitente: &str) {
        self.activas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, owner| owner != remitente);
    }
}

struct ScreenSaver {
    gestor: GestorCookies,
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
        self.gestor
            .registrar(&cabecera, application_name, reason_for_inhibit)
    }

    // añadido en la 0.2.2
    fn un_inhibit(&self, cookie: u32) {
        self.gestor.soltar(cookie);
    }

    /// El salvapantallas de BookOS no se pregunta por aquí; se contesta para
    /// que las apps que lo consultan antes de inhibir no den error.
    // añadido en la 0.2.2
    fn get_active(&self) -> bool {
        false
    }
}

struct PowerManagementInhibit {
    gestor: GestorCookies,
}

#[zbus::interface(name = "org.freedesktop.PowerManagement.Inhibit")]
impl PowerManagementInhibit {
    fn inhibit(
        &self,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        application: String,
        reason: String,
    ) -> u32 {
        self.gestor.registrar(&cabecera, application, reason)
    }

    fn un_inhibit(&self, cookie: u32) {
        self.gestor.soltar(cookie);
    }

    fn has_inhibit(&self) -> bool {
        !self
            .gestor
            .activas
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

struct GnomeSessionManager {
    gestor: GestorCookies,
}

#[zbus::interface(name = "org.gnome.SessionManager")]
impl GnomeSessionManager {
    fn inhibit(
        &self,
        #[zbus(header)] cabecera: zbus::message::Header<'_>,
        app_id: String,
        _toplevel_xid: u32,
        reason: String,
        flags: u32,
    ) -> u32 {
        if flags & (GNOME_INHIBIR_SUSPENSION | GNOME_INHIBIR_INACTIVO) == 0 {
            return 0;
        }
        self.gestor.registrar(&cabecera, app_id, reason)
    }

    #[zbus(name = "Uninhibit")]
    fn uninhibit(&self, cookie: u32) {
        self.gestor.soltar(cookie);
    }
}

/// Atiende `org.freedesktop.ScreenSaver`, `org.freedesktop.PowerManagement.Inhibit`
/// y `org.gnome.SessionManager` en la conexión de ajustes. Si otro escritorio
/// ya tiene el nombre (desarrollando anidado sobre Plasma), se avisa y se sigue:
/// `vigilar_nombres` lo volverá a pedir.
// añadido en la 0.2.2
pub(crate) fn arrancar(conexion: &zbus::blocking::Connection, canal: Sender<Aviso>) -> GestorCookies {
    let gestor = GestorCookies::nueva(canal);
    for ruta in RUTAS_SCREENSAVER {
        if let Err(err) = conexion.object_server().at(
            ruta,
            ScreenSaver {
                gestor: gestor.clone(),
            },
        ) {
            tracing::warn!("sin {NOMBRE} en {ruta}: {err}");
        }
    }
    if let Err(err) = conexion.object_server().at(
        RUTA_POWER,
        PowerManagementInhibit {
            gestor: gestor.clone(),
        },
    ) {
        tracing::warn!("sin {NOMBRE_POWER} en {RUTA_POWER}: {err}");
    }
    if let Err(err) = conexion.object_server().at(
        RUTA_GNOME,
        GnomeSessionManager {
            gestor: gestor.clone(),
        },
    ) {
        tracing::warn!("sin {NOMBRE_GNOME} en {RUTA_GNOME}: {err}");
    }
    for nombre in [NOMBRE, NOMBRE_POWER, NOMBRE_GNOME] {
        if let Err(err) = conexion.request_name(nombre) {
            tracing::warn!("sin {nombre}: {err}");
        }
    }
    gestor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_evitan_cero_y_soltar_ajeno_o_caida_mantienen_has_inhibit() {
        let (tx, rx) = smithay::reexports::calloop::channel::channel();
        let gestor = GestorCookies::nueva(tx);
        gestor.siguiente.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(gestor.siguiente_cookie(), u32::MAX);
        assert_eq!(gestor.siguiente_cookie(), 1);

        let power = PowerManagementInhibit {
            gestor: gestor.clone(),
        };
        let c1 = gestor.registrar_con_remitente(":1.10".into(), "mpv", "video");
        let _ = rx.try_recv();
        assert!(power.has_inhibit());

        // Soltar una cookie inexistente o repetida no baja la inhibición activa.
        gestor.soltar(999_999);
        assert!(rx.try_recv().is_err());
        assert!(power.has_inhibit());

        // Si el cliente dueño muere sin UnInhibit, retirar_remitente limpia el estado.
        gestor.retirar_remitente(":1.20");
        assert!(power.has_inhibit());
        gestor.retirar_remitente(":1.10");
        assert!(!power.has_inhibit());
        gestor.soltar(c1);
    }
}
