//! El servidor de notificaciones: `org.freedesktop.Notifications`.
//!
//! Es la única interfaz que las aplicaciones conocen para avisar de algo, y es
//! D-Bus, así que aquí sí hay bus. Lo que **no** hay es un ejecutor asíncrono
//! metido en el bucle de frames: se usa la API bloqueante de zbus, que arranca
//! su propio hilo por dentro, y lo que llega se manda al bucle por un canal de
//! `calloop`. Así el compositor sigue despertando solo por eventos —el canal es
//! un descriptor más en el `epoll`— y el código de dibujo no sabe que existe
//! D-Bus.
//!
//! ## Qué se implementa y qué no
//!
//! Los cuatro métodos de la especificación (`Notify`, `CloseNotification`,
//! `GetCapabilities`, `GetServerInformation`) y la señal `NotificationClosed`,
//! que es la que espera una aplicación para saber que su aviso ya no está.
//!
//! Las acciones se dibujan en el toast y vuelven por `ActionInvoked`. El
//! progreso estándar (`value`) también se conserva para descargas y copias.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use smithay::reexports::calloop::channel::Sender;
use zbus::zvariant::OwnedValue;

/// Lo que el servidor le manda al bucle del compositor.
pub enum Aviso {
    /// La notificación y su `expire_timeout` tal cual: lo interpreta el aviso
    /// que la enseña, no el servidor.
    Nueva(bookos_shell::notificaciones::Notificacion, i32),
    /// La aplicación ha retirado la suya.
    Cerrar(u32),
}

/// Por qué se cerró una notificación, en los códigos de la especificación.
///
/// Solo se usan dos: nadie mira el resto y no hay caducidad automática todavía.
pub const CERRADA_POR_LA_APP: u32 = 3;
pub const CERRADA_POR_EL_USUARIO: u32 = 2;

pub const RUTA: &str = "/org/freedesktop/Notifications";
pub const INTERFAZ: &str = "org.freedesktop.Notifications";

struct Servidor {
    canal: Sender<Aviso>,
    /// El siguiente identificador que se reparte. Empieza en 1 porque el 0
    /// significa «ninguna» en la especificación.
    siguiente: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Servidor {
    /// El método que usa todo el mundo. Devuelve el identificador con el que la
    /// aplicación podrá cerrarla o sustituirla.
    #[allow(clippy::too_many_arguments)]
    // 0.1.0
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        // `replaces_id` distinto de cero es «actualiza aquella», y entonces se
        // conserva el identificador: es lo que hace que un reproductor no llene
        // la lista con una notificación por canción.
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.siguiente.fetch_add(1, Ordering::Relaxed)
        };
        // La urgencia viene en los `hints` como un byte: 0 baja, 1 normal, 2
        // crítica. Lo que no se entienda cuenta como normal, que es lo que hay
        // que hacer con una pista opcional.
        let critica = hints
            .get("urgency")
            .and_then(|v| u8::try_from(v).ok())
            .is_some_and(|u| u >= 2);
        let acciones = actions
            .as_chunks::<2>()
            .0
            .iter()
            .map(|par| bookos_shell::notificaciones::Accion {
                clave: par[0].clone(),
                etiqueta: par[1].clone(),
            })
            .collect();
        let progreso = hints
            .get("value")
            .and_then(|v| i32::try_from(v).ok())
            .map(|v| v.clamp(0, 100) as u8);
        // La especificación pone `image-path` por delante de `app_icon`, y no
        // es un caso raro: `notify-send -i` lo manda **solo** ahí, con
        // `app_icon` vacío (visto en el servidor: llegaba `app_icon=""` y
        // `image-path` entre las pistas). Puede ser un nombre del tema o una
        // ruta, con o sin `file://`.
        let imagen = hints
            .get("image-path")
            .or_else(|| hints.get("image_path"))
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .filter(|s| !s.is_empty());
        let icono = imagen
            .as_deref()
            .map(|s| s.strip_prefix("file://").unwrap_or(s))
            .unwrap_or(&app_icon);
        let notificacion = bookos_shell::notificaciones::Notificacion::nueva_con_datos(
            id, app_name, summary, body, icono, critica, acciones, progreso, false,
        );
        // Si el canal está roto es que el compositor se está cerrando; se
        // contesta igual con el identificador para no dejar colgada a la
        // aplicación que llamó.
        if let Err(err) = self.canal.send(Aviso::Nueva(notificacion, expire_timeout)) {
            tracing::warn!("no se pudo entregar la notificación: {err}");
        }
        id
    }

    // 0.1.0
    fn close_notification(&self, id: u32) {
        let _ = self.canal.send(Aviso::Cerrar(id));
    }

    /// Capacidades que se pueden usar sin quedarse esperando una respuesta.
    // 0.1.0
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "persistence".into(),
            "icon-static".into(),
            "actions".into(),
            // La convención de KDE: una acción `inline-reply` pide texto, y la
            // respuesta vuelve por `NotificationReplied`. Sin anunciarla, las
            // aplicaciones ni ofrecen el botón.
            "inline-reply".into(),
        ]
    }

    // 0.1.0
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "BookOS".into(),
            "BookOS".into(),
            env!("CARGO_PKG_VERSION").into(),
            // La versión de la especificación que se sigue.
            "1.2".into(),
        )
    }

    /// La señal que espera una aplicación para saber que su aviso ya no está.
    /// La emite el compositor desde el bucle, con [`cerrada`].
    #[zbus(signal)]
    // 0.1.0
    async fn notification_closed(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        motivo: u32,
    ) -> zbus::Result<()>;

    /// La respuesta escrita a una notificación con `inline-reply`. Es la señal
    /// de KDE —no está en la especificación— y la que ya escuchan KDE Connect y
    /// Lumina.
    #[zbus(signal)]
    // 0.1.0
    async fn notification_replied(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        texto: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    // 0.1.0
    async fn action_invoked(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        accion: &str,
    ) -> zbus::Result<()>;
}

/// Lo que llega del bus, ya en el bucle del compositor.
// modified 0.1.1 -- sobre pantalla completa la notificación va a la campana sin aviso
pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    let completa = state.hay_pantalla_completa();
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    // Se apunta y se emite **después** del `match`: dentro, `shell` tiene
    // prestado a `state` en exclusiva y la conexión al bus vive ahí también.
    let mut confirmar_cierre = None;
    let repintar = match aviso {
        Aviso::Nueva(notificacion, caducidad) => {
            tracing::debug!(
                app = %notificacion.app,
                resumen = %notificacion.resumen,
                "notificación"
            );
            // Sobre un vídeo o un juego a pantalla completa el aviso tapaba
            // justo lo que se miraba. Queda en la campana; las críticas
            // —batería a punto de agotarse— salen igual.
            if completa && !notificacion.critica {
                shell.archivar_notificacion(notificacion)
            } else {
                shell.notificar(notificacion, caducidad)
            }
        }
        Aviso::Cerrar(id) => {
            let estaba = shell.cerrar_notificacion(id);
            // La especificación pide confirmar el cierre **también** cuando lo
            // pide la propia aplicación: es como sabe que la orden llegó.
            confirmar_cierre = Some(id);
            estaba
        }
    };
    if let Some(id) = confirmar_cierre {
        cerrada(state.bus_notificaciones.as_ref(), id, CERRADA_POR_LA_APP);
    }
    if repintar {
        state.needs_redraw = true;
    }
    despertar_para_la_salida(state);
}

/// Programa un despertar para cuando cada aviso empiece a irse.
///
/// Lo mismo que hace el aviso de volumen y por lo mismo: sin esto el compositor
/// se duerme con el aviso puesto y no vuelve a dibujar, así que se quedaría en
/// pantalla hasta que otra cosa provocara un frame. Uno **por aviso**: con la
/// pila, cada uno se va a su hora, y el despertar del primero no sirve para el
/// segundo.
// 0.1.0
fn despertar_para_la_salida(state: &mut crate::state::BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let quedan = state
        .shell
        .as_ref()
        .map(|s| s.toasts_quedan())
        .unwrap_or_default();
    for queda in quedan {
        let hasta_la_salida = queda.saturating_sub(bookos_shell::toast::SALIDA);
        let resultado = state.loop_handle.insert_source(
            Timer::from_duration(hasta_la_salida),
            |_, _, state| {
                state.needs_redraw = true;
                TimeoutAction::Drop
            },
        );
        if let Err(err) = resultado {
            tracing::error!("no se pudo programar la salida del aviso: {err}");
        }
    }
}

/// Registra el servidor en el bus de sesión.
///
/// Devuelve la conexión, que hay que **guardar**: al soltarla se cierra el bus
/// y el nombre se pierde. Si no hay bus de sesión —una sesión de prueba en un
/// TTY pelado— no es un error de los que tumban el escritorio: se avisa y se
/// sigue sin notificaciones.
// 0.1.0
pub fn arrancar(canal: Sender<Aviso>) -> Option<zbus::blocking::Connection> {
    let servidor = Servidor {
        canal,
        siguiente: Arc::new(AtomicU32::new(1)),
    };
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(INTERFAZ))
        .and_then(|b| b.serve_at(RUTA, servidor))
        .and_then(|b| b.build());
    match conexion {
        Ok(conexion) => {
            tracing::info!("servidor de notificaciones en el bus");
            Some(conexion)
        }
        // El caso normal de fallo es que ya haya otro servidor —una sesión de
        // Plasma por debajo, mientras se desarrolla anidado—: entonces el
        // nombre está cogido y aquí no llegan avisos, que es lo correcto.
        Err(err) => {
            tracing::warn!("sin servidor de notificaciones: {err}");
            None
        }
    }
}

/// Le dice a la aplicación que su notificación se ha cerrado.
// 0.1.0
pub fn cerrada(conexion: Option<&zbus::blocking::Connection>, id: u32, motivo: u32) {
    let Some(conexion) = conexion else {
        return;
    };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        RUTA,
        INTERFAZ,
        "NotificationClosed",
        &(id, motivo),
    ) {
        tracing::warn!(id, "no se pudo avisar del cierre: {err}");
    }
}

// 0.1.0
pub fn accion_invocada(conexion: Option<&zbus::blocking::Connection>, id: u32, accion: &str) {
    let Some(conexion) = conexion else {
        return;
    };
    if let Err(err) =
        conexion.emit_signal(None::<&str>, RUTA, INTERFAZ, "ActionInvoked", &(id, accion))
    {
        tracing::warn!(id, %accion, "no se pudo avisar de la acción de notificación: {err}");
    }
}

// 0.1.0
pub fn respondida(conexion: Option<&zbus::blocking::Connection>, id: u32, texto: &str) {
    let Some(conexion) = conexion else {
        return;
    };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        RUTA,
        INTERFAZ,
        "NotificationReplied",
        &(id, texto),
    ) {
        tracing::warn!(id, "no se pudo entregar la respuesta: {err}");
    }
}
