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
pub const ID_INTERNO_BASE: u32 = 0xF000_0000;

static SIGUIENTE_INTERNO: AtomicU32 = AtomicU32::new(ID_INTERNO_BASE);

/// Genera un identificador único para notificaciones internas del compositor
/// (como capturas guardadas) que no colisiona con los IDs de D-Bus.
pub fn siguiente_id_interno() -> u32 {
    let id = SIGUIENTE_INTERNO.fetch_add(1, Ordering::Relaxed);
    if id < ID_INTERNO_BASE {
        SIGUIENTE_INTERNO.store(ID_INTERNO_BASE + 1, Ordering::Relaxed);
        ID_INTERNO_BASE
    } else {
        id
    }
}

/// Abre en el visor del sistema la ruta local asociada a una notificación (si la tiene).
pub fn abrir_ruta_de_notificacion(state: &mut crate::state::BookosComp, id: u32) -> bool {
    let ruta = state.shell.as_ref().and_then(|s| {
        s.notificaciones_lista()
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.ruta.clone())
    });
    let Some(ruta) = ruta else {
        return false;
    };
    let visor = bookos_shell::visor_imagenes();
    let mut cmd = match visor.as_ref() {
        Some(v) => crate::keybinds::comando_directo(state, &v.programa, &v.argumentos),
        None => crate::keybinds::comando_directo(state, "xdg-open", &[]),
    };
    cmd.arg(&ruta);
    match cmd.spawn() {
        Ok(hijo) => {
            crate::keybinds::registrar_hijo(state, hijo, None);
            true
        }
        Err(err) => {
            tracing::warn!(?ruta, "no se pudo abrir la captura desde la notificación: {err}");
            false
        }
    }
}

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
            siguiente_id(&self.siguiente)
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
        // Prioridad según la especificación 1.2:
        // 1. `image-data` / `image_data` (píxeles crudos: avatares, carátulas)
        // 2. `image-path` / `image_path` (ruta o nombre del tema)
        // 3. `app_icon`
        // 4. `icon_data` (legado)
        let imagen_datos = hints
            .get("image-data")
            .or_else(|| hints.get("image_data"))
            .and_then(decodificar_imagen);
        let imagen = hints
            .get("image-path")
            .or_else(|| hints.get("image_path"))
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .filter(|s| !s.is_empty());
        let icono = imagen
            .as_deref()
            .map(|s| s.strip_prefix("file://").unwrap_or(s))
            .unwrap_or(&app_icon);
        let mut notificacion = bookos_shell::notificaciones::Notificacion::nueva_con_datos(
            id, app_name, summary, body, icono, critica, acciones, progreso, false,
        );
        if let Some(ic) = imagen_datos.or_else(|| {
            if imagen.is_none() && app_icon.is_empty() {
                hints.get("icon_data").and_then(decodificar_imagen)
            } else {
                None
            }
        }) {
            notificacion = notificacion.con_icono(ic);
        }
        // añadido en la 0.2.2 -- avisos con dibujo propio (Buds, batería baja)
        let texto = |clave: &str| {
            hints
                .get(clave)
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        };
        if let (Some(forma), Some(medidas)) = (texto("x-bookos-forma"), texto("x-bookos-medidas")) {
            notificacion.forma = bookos_shell::notificaciones::Forma::parsear(&forma, &medidas);
        }
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
    // Modo de concentración efectivo: el elegido a mano o, si está activado
    // `concentracion_auto_juego`, el perfil «juego» al estar en pantalla completa.
    let modo_foco = if !state.modo_concentracion.is_empty() {
        Some(state.modo_concentracion.clone())
    } else if completa && state.concentracion_auto_juego {
        Some("juego".to_string())
    } else {
        None
    };
    let permitida_en_foco = |app: &str| -> bool {
        let Some(modo) = modo_foco.as_deref() else {
            return true;
        };
        state
            .concentracion_permitidas
            .get(modo)
            .is_some_and(|lista| lista.iter().any(|a| a.eq_ignore_ascii_case(app)))
    };
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
            // Sobre un vídeo o un juego a pantalla completa, en un modo de
            // concentración (salvo apps permitidas) o si el usuario ha
            // silenciado esa aplicación, el aviso queda en la campana sin
            // molestar; las críticas —batería a punto de agotarse— salen igual.
            let bloqueada_por_foco = modo_foco.is_some() && !permitida_en_foco(&notificacion.app);
            if (completa || bloqueada_por_foco || shell.app_silenciada(&notificacion.app))
                && !notificacion.critica
            {
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
    crate::extras::notificaciones_cambiaron(state);
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
pub(crate) fn despertar_para_la_salida(state: &mut crate::state::BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    for token in state.toast_timers.drain(..) {
        state.loop_handle.remove(token);
    }
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
        match resultado {
            Ok(token) => state.toast_timers.push(token),
            Err(err) => tracing::error!("no se pudo programar la salida del aviso: {err}"),
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
    if id == 0 || id >= ID_INTERNO_BASE {
        return;
    }
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
    if id == 0 || id >= ID_INTERNO_BASE {
        return;
    }
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

/// Decodifica la pista D-Bus `image-data` (`(iiibiiay)`) en un icono RGBA.
///
/// Acepta 3 canales (RGB) o 4 canales (RGBA) a 8 bits por muestra, respetando
/// `rowstride`. Acota el tamaño a 1024×1024 para evitar asignaciones absurdas
/// si un cliente manda una imagen desproporcionada.
// añadido en la 0.2.4
fn decodificar_imagen(valor: &OwnedValue) -> Option<bookos_shell::icono::Icono> {
    let (ancho, alto, rowstride, tiene_alfa, bits, canales, datos): (
        i32,
        i32,
        i32,
        bool,
        i32,
        i32,
        Vec<u8>,
    ) = valor.try_clone().ok()?.try_into().ok()?;
    let rgba = imagen_a_rgba(ancho, alto, rowstride, tiene_alfa, bits, canales, &datos)?;
    bookos_shell::icono::desde_rgba(ancho as u32, alto as u32, rgba)
}

// añadido en la 0.2.4
fn imagen_a_rgba(
    ancho: i32,
    alto: i32,
    rowstride: i32,
    tiene_alfa: bool,
    bits: i32,
    canales: i32,
    datos: &[u8],
) -> Option<Vec<u8>> {
    const MAX_LADO: i32 = 1024;
    if ancho <= 0 || alto <= 0 || ancho > MAX_LADO || alto > MAX_LADO {
        return None;
    }
    if bits != 8 || !matches!(canales, 3 | 4) {
        return None;
    }
    let w = ancho as usize;
    let h = alto as usize;
    let ch = canales as usize;
    let fila_min = w.checked_mul(ch)?;
    let paso = usize::try_from(rowstride).ok()?;
    if paso < fila_min {
        return None;
    }
    let necesario = (h - 1).checked_mul(paso)?.checked_add(fila_min)?;
    if datos.len() < necesario {
        return None;
    }
    let mut rgba = Vec::with_capacity(w.checked_mul(h)?.checked_mul(4)?);
    for y in 0..h {
        let inicio = y * paso;
        let fila = &datos[inicio..inicio + fila_min];
        if ch == 4 {
            if tiene_alfa {
                rgba.extend_from_slice(fila);
            } else {
                for px in fila.as_chunks::<4>().0 {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        } else {
            for px in fila.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
    }
    Some(rgba)
}

fn siguiente_id(contador: &AtomicU32) -> u32 {
    loop {
        let actual = contador.load(Ordering::Relaxed);
        let nuevo = if actual == u32::MAX { 1 } else { actual + 1 };
        if contador
            .compare_exchange_weak(actual, nuevo, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return actual.max(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_data_rgb_con_rowstride_y_padding() {
        // 2×2 RGB con 2 bytes de relleno al final de cada fila (rowstride = 8).
        let datos = vec![
            10, 20, 30, 40, 50, 60, 0, 0, // fila 0
            70, 80, 90, 100, 110, 120, 0, 0, // fila 1
        ];
        let rgba = imagen_a_rgba(2, 2, 8, false, 8, 3, &datos).expect("RGB válido");
        assert_eq!(
            rgba,
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255
            ]
        );
    }

    #[test]
    fn image_data_rgba_y_rechazos() {
        let rgba = imagen_a_rgba(1, 1, 4, true, 8, 4, &[1, 2, 3, 128]).expect("RGBA válido");
        assert_eq!(rgba, vec![1, 2, 3, 128]);
        assert!(imagen_a_rgba(0, 1, 4, true, 8, 4, &[1, 2, 3, 4]).is_none());
        assert!(imagen_a_rgba(2000, 1, 8000, true, 8, 4, &[]).is_none());
        assert!(imagen_a_rgba(2, 1, 4, true, 8, 4, &[0; 8]).is_none());
    }

    #[test]
    fn siguiente_id_nunca_devuelve_cero_al_desbordar() {
        let contador = AtomicU32::new(u32::MAX);
        assert_eq!(siguiente_id(&contador), u32::MAX);
        assert_eq!(siguiente_id(&contador), 1);
        assert_eq!(siguiente_id(&contador), 2);
        let desde_cero = AtomicU32::new(0);
        assert_eq!(siguiente_id(&desde_cero), 1);
    }
}
