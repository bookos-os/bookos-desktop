//! Historial del portapapeles (`Meta+,`): captura selecciones Wayland, X11 y
//! propias del compositor con fotos y textos, y las repone o pega al elegirlas.

use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bookos_shell::{ContenidoPortapapeles, ElementoPortapapeles, MiniaturaPortapapeles};
use smithay::reexports::calloop::channel::{self, Event, Sender};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::{
    current_data_device_selection_userdata, request_data_device_client_selection,
};
use smithay::wayland::text_input::TextInputSeat;

use crate::handlers::Seleccion;
use crate::state::BookosComp;

const MAX_ENTRADAS: usize = 40;
const MAX_TEXTO_BYTES: usize = 256 * 1024;
const MAX_IMAGEN_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGENES_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const TIMEOUT_LECTURA_MS: i32 = 2000;
const HINT_CONTRASENA: &str = "x-kde-passwordManagerHint";

#[derive(Default)]
pub struct Estado {
    destino: Option<WlSurface>,
    entradas: Vec<Entrada>,
    siguiente_id: u64,
    generacion: u64,
    ignorar_propia: bool,
    canal: Option<Sender<Lectura>>,
}

struct Entrada {
    id: u64,
    fijada: bool,
    hora: String,
    origen: Option<String>,
    contenido: Contenido,
}

enum Contenido {
    Texto(String),
    Imagen {
        mime: String,
        formato: String,
        bytes: Arc<Vec<u8>>,
        miniatura: MiniaturaPortapapeles,
    },
}

enum Lectura {
    Texto {
        generacion: u64,
        texto: String,
        origen: Option<String>,
    },
    Imagen {
        generacion: u64,
        mime: String,
        formato: String,
        bytes: Arc<Vec<u8>>,
        miniatura: MiniaturaPortapapeles,
        origen: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TipoElegido {
    mime: String,
    es_imagen: bool,
    formato: String,
}

fn elegir_mime(tipos: &[String]) -> Option<TipoElegido> {
    if tipos.iter().any(|t| t == HINT_CONTRASENA) {
        return None;
    }
    for &(mime, formato) in &[
        ("image/png", "PNG"),
        ("image/jpeg", "JPEG"),
        ("image/webp", "WEBP"),
        ("PNG", "PNG"),
    ] {
        if let Some(encontrado) = tipos.iter().find(|t| t.as_str() == mime) {
            return Some(TipoElegido {
                mime: encontrado.clone(),
                es_imagen: true,
                formato: formato.to_string(),
            });
        }
    }
    for mime in &[
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "STRING",
        "TEXT",
    ] {
        if let Some(encontrado) = tipos.iter().find(|t| t.as_str() == *mime) {
            return Some(TipoElegido {
                mime: encontrado.clone(),
                es_imagen: false,
                formato: String::new(),
            });
        }
    }
    None
}

fn hora_actual() -> String {
    let (h, m) = bookos_shell::hora_local_ahora();
    format!("{h:02}:{m:02}")
}

fn nombre_origen_enfocado(state: &BookosComp) -> Option<String> {
    let ventana = state.ventana_con_foco()?;
    let app_id = crate::handlers::app_id(&ventana)?;
    bookos_shell::nombre_de_app(&app_id)
        .or_else(|| (!app_id.trim().is_empty()).then_some(app_id))
}

fn canal(state: &mut BookosComp) -> Sender<Lectura> {
    if let Some(tx) = &state.portapapeles.canal {
        return tx.clone();
    }
    let (tx, rx) = channel::channel::<Lectura>();
    let insertado = state.loop_handle.insert_source(rx, |evento, _, state| {
        if let Event::Msg(lectura) = evento {
            recibir_lectura(state, lectura);
        }
    });
    if let Err(err) = insertado {
        tracing::error!("no se pudo registrar el canal del historial del portapapeles: {err}");
    }
    state.portapapeles.canal = Some(tx.clone());
    tx
}

fn recibir_lectura(state: &mut BookosComp, lectura: Lectura) {
    match lectura {
        Lectura::Texto {
            generacion,
            texto,
            origen,
        } => {
            if generacion != state.portapapeles.generacion {
                return;
            }
            insertar_texto(state, texto, origen);
        }
        Lectura::Imagen {
            generacion,
            mime,
            formato,
            bytes,
            miniatura,
            origen,
        } => {
            if generacion != state.portapapeles.generacion {
                return;
            }
            insertar_imagen(state, mime, formato, bytes, miniatura, origen);
        }
    }
}

fn leer_pipe_con_timeout(
    fd: OwnedFd,
    max_bytes: usize,
    permitir_truncar: bool,
) -> Option<Vec<u8>> {
    use smithay::reexports::rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
    if let Ok(flags) = fcntl_getfl(&fd) {
        let _ = fcntl_setfl(&fd, flags | OFlags::NONBLOCK);
    }
    let mut archivo = std::fs::File::from(fd);
    let raw_fd = archivo.as_raw_fd();
    let mut salida = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    let inicio = Instant::now();
    let limite = Duration::from_millis(TIMEOUT_LECTURA_MS as u64);

    loop {
        let restante = limite.saturating_sub(inicio.elapsed());
        if restante.is_zero() {
            return (permitir_truncar && !salida.is_empty()).then_some(salida);
        }
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = restante.as_millis().min(i32::MAX as u128) as i32;
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if rc == 0 {
            return (permitir_truncar && !salida.is_empty()).then_some(salida);
        }
        loop {
            match archivo.read(&mut buf) {
                Ok(0) => return (!salida.is_empty()).then_some(salida),
                Ok(n) => {
                    if salida.len() + n > max_bytes {
                        if permitir_truncar {
                            let falta = max_bytes.saturating_sub(salida.len());
                            salida.extend_from_slice(&buf[..falta]);
                            return Some(salida);
                        }
                        return None;
                    }
                    salida.extend_from_slice(&buf[..n]);
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }
}

fn recortar_texto_utf8(bytes: &[u8]) -> String {
    let corte = bytes.len().min(MAX_TEXTO_BYTES);
    String::from_utf8_lossy(&bytes[..corte]).into_owned()
}

fn lanzar_lectura_pipe(
    tx: Sender<Lectura>,
    lectura: OwnedFd,
    elegido: TipoElegido,
    origen: Option<String>,
    generacion: u64,
) {
    let hilo = std::thread::Builder::new()
        .name("historial-leer".into())
        .spawn(move || {
            let max_bytes = if elegido.es_imagen {
                MAX_IMAGEN_BYTES
            } else {
                MAX_TEXTO_BYTES
            };
            let Some(bytes) = leer_pipe_con_timeout(lectura, max_bytes, !elegido.es_imagen) else {
                return;
            };
            if elegido.es_imagen {
                let Some(miniatura) = bookos_shell::miniatura_de_imagen(&bytes) else {
                    return;
                };
                let mime = if elegido.mime == "PNG" {
                    "image/png".to_string()
                } else {
                    elegido.mime
                };
                let _ = tx.send(Lectura::Imagen {
                    generacion,
                    mime,
                    formato: elegido.formato,
                    bytes: Arc::new(bytes),
                    miniatura,
                    origen,
                });
            } else {
                let texto = recortar_texto_utf8(&bytes);
                if texto.trim().is_empty() {
                    return;
                }
                let _ = tx.send(Lectura::Texto {
                    generacion,
                    texto,
                    origen,
                });
            }
        });
    if let Err(err) = hilo {
        tracing::warn!("no se pudo lanzar el hilo del historial de portapapeles: {err}");
    }
}

/// Programa la captura de una selección puesta por un cliente Wayland.
///
/// Smithay invoca `SelectionHandler::new_selection` antes de registrar la nueva
/// fuente en `seat_data`, así que se difiere al siguiente ciclo `idle`.
pub fn programar_captura_cliente(state: &mut BookosComp, tipos: Vec<String>) {
    if elegir_mime(&tipos).is_none() {
        return;
    }
    state.portapapeles.generacion = state.portapapeles.generacion.wrapping_add(1);
    let generacion = state.portapapeles.generacion;
    let origen = nombre_origen_enfocado(state);
    state.loop_handle.insert_idle(move |state| {
        leer_cliente(state, tipos, origen, generacion);
    });
}

fn leer_cliente(
    state: &mut BookosComp,
    tipos: Vec<String>,
    origen: Option<String>,
    generacion: u64,
) {
    if state.portapapeles.generacion != generacion {
        return;
    }
    let Some(elegido) = elegir_mime(&tipos) else {
        return;
    };
    let Ok((lectura, escritura)) = smithay::reexports::rustix::pipe::pipe() else {
        return;
    };
    if let Err(err) = request_data_device_client_selection(&state.seat, elegido.mime.clone(), escritura)
    {
        tracing::debug!(?err, mime = %elegido.mime, "no se pudo leer selección del cliente");
        return;
    }
    let tx = canal(state);
    lanzar_lectura_pipe(tx, lectura, elegido, origen, generacion);
}

/// Programa la captura de una selección puesta por un cliente X11/XWayland.
pub fn programar_captura_x11(state: &mut BookosComp, tipos: Vec<String>) {
    if elegir_mime(&tipos).is_none() {
        return;
    }
    state.portapapeles.generacion = state.portapapeles.generacion.wrapping_add(1);
    let generacion = state.portapapeles.generacion;
    let origen = nombre_origen_enfocado(state);
    state.loop_handle.insert_idle(move |state| {
        leer_x11(state, tipos, origen, generacion);
    });
}

fn leer_x11(
    state: &mut BookosComp,
    tipos: Vec<String>,
    origen: Option<String>,
    generacion: u64,
) {
    if state.portapapeles.generacion != generacion {
        return;
    }
    if !matches!(
        current_data_device_selection_userdata(&state.seat).as_deref(),
        Some(Seleccion::X11)
    ) {
        return;
    }
    let Some(elegido) = elegir_mime(&tipos) else {
        return;
    };
    let Some(xwm) = state.xwm.as_mut() else {
        return;
    };
    let Ok((lectura, escritura)) = smithay::reexports::rustix::pipe::pipe() else {
        return;
    };
    if let Err(err) = xwm.send_selection(
        SelectionTarget::Clipboard,
        elegido.mime.clone(),
        escritura,
        state.loop_handle.clone(),
    ) {
        tracing::debug!(?err, mime = %elegido.mime, "no se pudo leer selección X11");
        return;
    }
    let tx = canal(state);
    lanzar_lectura_pipe(tx, lectura, elegido, origen, generacion);
}

/// Registra una selección puesta por el propio compositor (captura, color, ficha, rutas).
pub(crate) fn registrar_propia(
    state: &mut BookosComp,
    tipos: &[String],
    datos: &Seleccion,
) {
    if state.portapapeles.ignorar_propia {
        return;
    }
    state.portapapeles.generacion = state.portapapeles.generacion.wrapping_add(1);
    let generacion = state.portapapeles.generacion;
    let Some(elegido) = elegir_mime(tipos) else {
        return;
    };
    let bytes = match datos {
        Seleccion::Bytes(arc) => arc.clone(),
        Seleccion::PorTipo(lista) => {
            let Some((_, b)) = lista.iter().find(|(t, _)| t == &elegido.mime) else {
                return;
            };
            Arc::new(b.clone())
        }
        Seleccion::X11 => return,
    };
    let origen = Some("BookOS".to_string());
    if elegido.es_imagen {
        if bytes.len() > MAX_IMAGEN_BYTES {
            return;
        }
        let tx = canal(state);
        let mime = if elegido.mime == "PNG" {
            "image/png".to_string()
        } else {
            elegido.mime
        };
        let formato = elegido.formato;
        let _ = std::thread::Builder::new()
            .name("historial-mini".into())
            .spawn(move || {
                let Some(miniatura) = bookos_shell::miniatura_de_imagen(&bytes) else {
                    return;
                };
                let _ = tx.send(Lectura::Imagen {
                    generacion,
                    mime,
                    formato,
                    bytes,
                    miniatura,
                    origen,
                });
            });
    } else {
        let texto = recortar_texto_utf8(&bytes);
        insertar_texto(state, texto, origen);
    }
}

fn insertar_texto(state: &mut BookosComp, texto: String, origen: Option<String>) {
    if texto.trim().is_empty() {
        return;
    }
    let hora = hora_actual();
    state
        .portapapeles
        .insertar_texto_interno(texto, hora, origen);
    sincronizar_vista(state);
}

fn insertar_imagen(
    state: &mut BookosComp,
    mime: String,
    formato: String,
    bytes: Arc<Vec<u8>>,
    miniatura: MiniaturaPortapapeles,
    origen: Option<String>,
) {
    let hora = hora_actual();
    state
        .portapapeles
        .insertar_imagen_interno(mime, formato, bytes, miniatura, hora, origen);
    sincronizar_vista(state);
}

impl Estado {
    fn nuevo_id(&mut self) -> u64 {
        self.siguiente_id = self.siguiente_id.wrapping_add(1);
        self.siguiente_id
    }

    fn insertar_texto_interno(&mut self, texto: String, hora: String, origen: Option<String>) {
        if texto.trim().is_empty() {
            return;
        }
        let fijada = self
            .entradas
            .iter()
            .position(|e| matches!(&e.contenido, Contenido::Texto(t) if t == &texto))
            .map(|pos| self.entradas.remove(pos).fijada)
            .unwrap_or(false);
        let id = self.nuevo_id();
        self.entradas.insert(
            0,
            Entrada {
                id,
                fijada,
                hora,
                origen,
                contenido: Contenido::Texto(texto),
            },
        );
        self.recortar_limites();
    }

    fn insertar_imagen_interno(
        &mut self,
        mime: String,
        formato: String,
        bytes: Arc<Vec<u8>>,
        miniatura: MiniaturaPortapapeles,
        hora: String,
        origen: Option<String>,
    ) {
        let fijada = self
            .entradas
            .iter()
            .position(|e| matches!(&e.contenido, Contenido::Imagen { bytes: b, .. } if b.as_slice() == bytes.as_slice()))
            .map(|pos| self.entradas.remove(pos).fijada)
            .unwrap_or(false);
        let id = self.nuevo_id();
        self.entradas.insert(
            0,
            Entrada {
                id,
                fijada,
                hora,
                origen,
                contenido: Contenido::Imagen {
                    mime,
                    formato,
                    bytes,
                    miniatura,
                },
            },
        );
        self.recortar_limites();
    }

    fn total_bytes_imagenes(&self) -> usize {
        self.entradas
            .iter()
            .map(|e| match &e.contenido {
                Contenido::Imagen { bytes, .. } => bytes.len(),
                Contenido::Texto(_) => 0,
            })
            .sum()
    }

    fn recortar_limites(&mut self) {
        while self.total_bytes_imagenes() > MAX_IMAGENES_TOTAL_BYTES {
            if let Some(pos) = self
                .entradas
                .iter()
                .rposition(|e| !e.fijada && matches!(e.contenido, Contenido::Imagen { .. }))
            {
                self.entradas.remove(pos);
            } else {
                break;
            }
        }
        while self.entradas.len() > MAX_ENTRADAS {
            if let Some(pos) = self.entradas.iter().rposition(|e| !e.fijada) {
                self.entradas.remove(pos);
            } else {
                break;
            }
        }
    }

    fn snapshot(&self) -> Vec<ElementoPortapapeles> {
        let mut fijadas = Vec::new();
        let mut recientes = Vec::new();
        for e in &self.entradas {
            let item = ElementoPortapapeles {
                id: e.id,
                fijada: e.fijada,
                hora: e.hora.clone(),
                origen: e.origen.clone(),
                contenido: match &e.contenido {
                    Contenido::Texto(tx) => ContenidoPortapapeles::Texto(tx.clone()),
                    Contenido::Imagen {
                        formato, miniatura, ..
                    } => ContenidoPortapapeles::Imagen {
                        formato: formato.clone(),
                        miniatura: miniatura.clone(),
                    },
                },
            };
            if e.fijada {
                fijadas.push(item);
            } else {
                recientes.push(item);
            }
        }
        fijadas.extend(recientes);
        fijadas
    }

    fn fijar_id(&mut self, id: u64) -> bool {
        if let Some(e) = self.entradas.iter_mut().find(|e| e.id == id) {
            e.fijada = !e.fijada;
            return true;
        }
        false
    }

    fn eliminar_id(&mut self, id: u64) -> bool {
        let antes = self.entradas.len();
        self.entradas.retain(|e| e.id != id);
        self.entradas.len() != antes
    }

    fn vaciar_interno(&mut self) -> bool {
        if self.entradas.is_empty() {
            return false;
        }
        let hay_no_fijadas = self.entradas.iter().any(|e| !e.fijada);
        if hay_no_fijadas {
            self.entradas.retain(|e| e.fijada);
        } else {
            self.entradas.clear();
        }
        true
    }
}

fn sincronizar_vista(state: &mut BookosComp) {
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.emergente_nombre() == Some("portapapeles"))
    {
        let elementos = state.portapapeles.snapshot();
        if let Some(shell) = state.shell.as_mut() {
            shell.actualizar_portapapeles(elementos);
        }
        state.needs_redraw = true;
    }
}

/// Abre o cierra la tarjeta del historial (`Meta+,`).
pub fn alternar(state: &mut BookosComp) {
    state.portapapeles.destino = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let campo = state.seat.text_input().campo_activo();
    let punto = campo
        .as_ref()
        .and_then(|campo| {
            let rect = campo.cursor_rectangle?;
            let ventana = state.window_for_surface(&campo.surface)?;
            let origen = state.space.element_location(&ventana)? - ventana.geometry().loc;
            Some((
                origen.x as f32 + rect.loc.x as f32,
                origen.y as f32 + (rect.loc.y + rect.size.h) as f32 + 8.0,
            ))
        })
        .unwrap_or((
            state.pointer_location.x as f32 + 8.0,
            state.pointer_location.y as f32 + 8.0,
        ));
    let elementos = state.portapapeles.snapshot();
    if let Some(shell) = state.shell.as_mut() {
        shell.alternar_portapapeles(punto.0, punto.1, elementos);
    }
    crate::keybinds::liberar_puntero_para_capa(state);
    state.needs_redraw = true;
}

/// Repone la entrada elegida en el portapapeles y la pega en la ventana enfocada.
pub fn pegar(state: &mut BookosComp, id: u64) {
    let Some(pos) = state.portapapeles.entradas.iter().position(|e| e.id == id) else {
        return;
    };
    let mut entrada = state.portapapeles.entradas.remove(pos);
    entrada.hora = hora_actual();
    let (tipos, datos) = match &entrada.contenido {
        Contenido::Texto(texto) => (
            vec![
                "text/plain;charset=utf-8".to_string(),
                "text/plain".to_string(),
                "UTF8_STRING".to_string(),
                "STRING".to_string(),
                "TEXT".to_string(),
            ],
            Seleccion::Bytes(Arc::new(texto.as_bytes().to_vec())),
        ),
        Contenido::Imagen { mime, bytes, .. } => {
            let mut tipos = vec![mime.clone()];
            if mime == "image/png" {
                tipos.push("PNG".to_string());
            }
            (tipos, Seleccion::Bytes(bytes.clone()))
        }
    };
    state.portapapeles.entradas.insert(0, entrada);

    state.portapapeles.ignorar_propia = true;
    crate::backend::poner_seleccion_con(state, tipos, datos);
    state.portapapeles.ignorar_propia = false;

    let destino = state.portapapeles.destino.take();
    let pegado = destino.as_ref().is_some_and(|d| {
        state
            .seat
            .get_keyboard()
            .is_some_and(|k| k.current_focus().as_ref() == Some(d))
            && crate::emojis::enviar_pegado(state, d)
    });
    if !pegado
        && let Some(shell) = state.shell.as_mut()
    {
        shell.notificar(
            bookos_shell::notificaciones::Notificacion::nueva(
                0,
                "BookOS".into(),
                bookos_shell::t("Copiado al portapapeles", "Copied to clipboard").into(),
                bookos_shell::t(
                    "Puedes pegarlo en cualquier aplicación",
                    "You can paste it in any app",
                )
                .into(),
                "edit-copy",
                false,
            ),
            2500,
        );
    }
    state.needs_redraw = true;
}

pub fn fijar(state: &mut BookosComp, id: u64) {
    if state.portapapeles.fijar_id(id) {
        sincronizar_vista(state);
    }
}

pub fn eliminar(state: &mut BookosComp, id: u64) {
    if state.portapapeles.eliminar_id(id) {
        sincronizar_vista(state);
    }
}

pub fn vaciar(state: &mut BookosComp) {
    if state.portapapeles.vaciar_interno() {
        sincronizar_vista(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mini_prueba(w: u32, h: u32) -> MiniaturaPortapapeles {
        MiniaturaPortapapeles {
            ancho_original: w,
            alto_original: h,
            ancho: 10,
            alto: 10,
            rgba: Arc::new(vec![255; 10 * 10 * 4]),
        }
    }

    #[test]
    fn elegir_mime_prioriza_imagen_e_ignora_contrasenas() {
        assert_eq!(
            elegir_mime(&[
                "text/plain".into(),
                "image/png".into(),
                HINT_CONTRASENA.into()
            ]),
            None
        );
        assert_eq!(
            elegir_mime(&["text/plain;charset=utf-8".into(), "image/png".into()]),
            Some(TipoElegido {
                mime: "image/png".into(),
                es_imagen: true,
                formato: "PNG".into()
            })
        );
        assert_eq!(
            elegir_mime(&["UTF8_STRING".into(), "text/plain;charset=utf-8".into()]),
            Some(TipoElegido {
                mime: "text/plain;charset=utf-8".into(),
                es_imagen: false,
                formato: String::new()
            })
        );
    }

    #[test]
    fn deduplica_preservando_fijada_y_ordena_fijadas_primero() {
        let mut estado = Estado::default();
        estado.insertar_texto_interno("uno".into(), "10:00".into(), None);
        estado.insertar_texto_interno("dos".into(), "10:01".into(), None);
        let id_uno = estado.entradas[1].id;
        assert!(estado.fijar_id(id_uno));
        // Volver a copiar "uno" conserva su estado fijado.
        estado.insertar_texto_interno("uno".into(), "10:05".into(), Some("Notas".into()));
        let snap = estado.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap[0].fijada);
        assert!(matches!(&snap[0].contenido, ContenidoPortapapeles::Texto(t) if t == "uno"));
    }

    #[test]
    fn limites_y_vaciar_respetan_entradas_fijadas() {
        let mut estado = Estado::default();
        estado.insertar_texto_interno("fija".into(), "09:00".into(), None);
        let id_fija = estado.entradas[0].id;
        estado.fijar_id(id_fija);

        for i in 0..50 {
            estado.insertar_texto_interno(format!("txt-{i}"), "09:01".into(), None);
        }
        assert_eq!(estado.entradas.len(), MAX_ENTRADAS);
        assert!(estado.entradas.iter().any(|e| e.id == id_fija));

        // Vaciar conserva las fijadas si hay no fijadas; una segunda vez limpia todo.
        assert!(estado.vaciar_interno());
        assert_eq!(estado.entradas.len(), 1);
        assert_eq!(estado.entradas[0].id, id_fija);
        assert!(estado.vaciar_interno());
        assert!(estado.entradas.is_empty());
    }

    #[test]
    fn limite_de_memoria_de_imagenes_purga_las_antiguas_no_fijadas() {
        let mut estado = Estado::default();
        let bloque = Arc::new(vec![1u8; 15 * 1024 * 1024]);
        for i in 0..6 {
            let mut b = (*bloque).clone();
            b[0] = i as u8;
            estado.insertar_imagen_interno(
                "image/png".into(),
                "PNG".into(),
                Arc::new(b),
                mini_prueba(100, 100),
                "12:00".into(),
                None,
            );
        }
        assert!(estado.total_bytes_imagenes() <= MAX_IMAGENES_TOTAL_BYTES);
        assert_eq!(estado.entradas.len(), 4);
    }
}
