//! Punto de extensión pasivo para `bookos-extras`.
//!
//! No incluye ninguna UI ni lógica de gestos propios en el escritorio base:
//! solo escucha un socket Unix de sesión (`$XDG_RUNTIME_DIR/bookos-extras-<display>.sock`).
//! Si ningún cliente `bookos-extras` está conectado y activo, no intercepta ningún
//! toque ni consume CPU.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use smithay::backend::input::{KeyState, TouchSlot};
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use smithay::utils::{Logical, Point};

use crate::gestos::{Movimiento, Suelta};
use crate::state::BookosComp;

/// Margen mínimo y máximo que el compositor acepta como zona de borde (en píxeles lógicos).
const MARGEN_MIN: f64 = 8.0;
const MARGEN_MAX: f64 = 64.0;
const MARGEN_PREDETERMINADO: f64 = 24.0;
/// Desplazamiento mínimo antes de decidir si un toque de borde es gesto o toque normal.
const UMBRAL_GESTO: f64 = 10.0;
const LIMITE_COLA_ESCRITURA: usize = 512 * 1024;
/// Evdev `KEY_BACK` (158) y `KEY_FORWARD` (159) + 8 (offset XKB).
const XKB_KEY_BACK: u32 = 158 + 8;
const XKB_KEY_FORWARD: u32 = 159 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Borde {
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccionNotifDto {
    pub clave: String,
    pub etiqueta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagenNotifDto {
    pub ancho: u32,
    pub alto: u32,
    pub rgba_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificacionDto {
    pub id: u32,
    pub app: String,
    pub resumen: String,
    pub cuerpo: String,
    pub critica: bool,
    pub leida: bool,
    pub progreso: Option<u8>,
    pub segundos: u64,
    pub acciones: Vec<AccionNotifDto>,
    #[serde(default)]
    pub tiene_default: bool,
    #[serde(default)]
    pub es_captura: bool,
    #[serde(default)]
    pub imagen: Option<ImagenNotifDto>,
}

/// Mensajes que el compositor envía al cliente `bookos-extras`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Evento {
    Pantalla {
        ancho: f64,
        alto: f64,
        escala: f64,
        oscuro: bool,
        efectos_reducidos: bool,
    },
    ToqueInicio {
        slot: u32,
        x: f64,
        y: f64,
        ancho: f64,
        alto: f64,
        borde: Borde,
        panel_visible: bool,
        #[serde(default)]
        exposicion_abierta: bool,
        tiempo: u32,
    },
    ToqueMueve {
        slot: u32,
        x: f64,
        y: f64,
        tiempo: u32,
    },
    ToqueFin {
        slot: u32,
        tiempo: u32,
    },
    ToqueCancelar {
        slot: u32,
        tiempo: u32,
    },
    ToqueFuera,
    Notificaciones {
        lista: Vec<NotificacionDto>,
        #[serde(default)]
        no_molestar: bool,
    },
}

/// Peticiones que `bookos-extras` envía al compositor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Peticion {
    Config {
        activo: bool,
        margen_borde: f64,
        #[serde(default = "horizontal_activo")]
        gesto_inferior_horizontal: bool,
    },
    MenuBookos,
    CentroControl,
    MostrarPanel,
    VistaInicio,
    VistaProgreso {
        progreso: f32,
    },
    VistaSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    ExposicionProgreso {
        progreso: f32,
        #[serde(default)]
        desplazamiento_x: f32,
    },
    ExposicionSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    ExposicionEscritorioProgreso {
        progreso: f32,
        #[serde(default)]
        desplazamiento_x: f32,
    },
    ExposicionEscritorioSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    DespejarEscritorio,
    DespejeProgreso {
        progreso: f32,
    },
    DespejeSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    EscritorioInicio,
    EscritorioProgreso {
        progreso: f32,
    },
    EscritorioSoltar {
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    },
    Atras,
    Adelante,
    PedirNotificaciones,
    CerrarNotificacion {
        id: u32,
    },
    LimpiarNotificaciones,
    InvocarAccionNotificacion {
        id: u32,
        clave: String,
    },
    MarcarNotificacionesLeidas,
    CortinaAbierta,
    AlternarNoMolestar,
    AbrirAjustesNotificaciones,
}

#[derive(Debug, Clone, Copy)]
enum ToqueBorde {
    Pendiente {
        slot: TouchSlot,
        origen: Point<f64, Logical>,
        borde: Borde,
        tiempo: u32,
        ancho: f64,
        alto: f64,
        origen_salida: Point<f64, Logical>,
        sin_extras: bool,
    },
    Capturado {
        slot: TouchSlot,
        origen_salida: Point<f64, Logical>,
    },
    Inferior {
        slot: TouchSlot,
        gesto: GestoInferior,
    },
    Barra {
        slot: TouchSlot,
    },
}

impl ToqueBorde {
    fn slot(self) -> TouchSlot {
        match self {
            Self::Pendiente { slot, .. }
            | Self::Capturado { slot, .. }
            | Self::Inferior { slot, .. }
            | Self::Barra { slot } => slot,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RecientesDesdeBorde {
    Pendiente,
    EnCurso,
    NoDisponible,
}

#[derive(Debug, Clone, Copy)]
enum DockDesdeBorde {
    Oculto,
    Temporal,
    Visible,
}

#[derive(Debug, Clone, Copy)]
struct GestoInferior {
    origen: Point<f64, Logical>,
    inicio_recientes: f64,
    dock: DockDesdeBorde,
    recorrido: f64,
    ultimo_recorrido: f64,
    ultimo_tiempo: u32,
    recientes: RecientesDesdeBorde,
}

fn tiempo_cruce_umbral(
    ultimo_tiempo: u32,
    tiempo: u32,
    ultimo_recorrido: f64,
    recorrido: f64,
    umbral: f64,
) -> u32 {
    let tramo = recorrido - ultimo_recorrido;
    let frac = if tramo > 1e-6 {
        ((umbral - ultimo_recorrido) / tramo).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let dt = tiempo.wrapping_sub(ultimo_tiempo) as f64;
    ultimo_tiempo.wrapping_add((dt * frac).round() as u32)
}

impl GestoInferior {
    fn nuevo(state: &BookosComp, origen: Point<f64, Logical>, tiempo: u32, alto: f64) -> Self {
        let dock = if crate::tactil::puede_revelar_barra(state, crate::shell::Barra::Dock) {
            DockDesdeBorde::Oculto
        } else if state.vista_tactil.is_none()
            && state
                .shell
                .as_ref()
                .is_some_and(|s| s.dock_tactil_temporal())
        {
            DockDesdeBorde::Temporal
        } else {
            DockDesdeBorde::Visible
        };
        Self {
            origen,
            // Deja sitio al recorrido corto también con escalas altas. La
            // vista existente siempre puede volver a agarrarse sin ese margen.
            inicio_recientes: if state.vista_tactil.is_some() {
                0.0
            } else {
                (alto * 0.065).clamp(40.0, 64.0)
            },
            dock,
            recorrido: 0.0,
            ultimo_recorrido: 0.0,
            ultimo_tiempo: tiempo,
            recientes: RecientesDesdeBorde::Pendiente,
        }
    }

    fn mover(&mut self, state: &mut BookosComp, punto: Point<f64, Logical>, time: u32) {
        let delta = punto - self.origen;
        self.recorrido = -delta.y;
        if matches!(self.recientes, RecientesDesdeBorde::Pendiente)
            && self.recorrido > self.inicio_recientes
        {
            let origen = self.origen + delta.upscale(self.inicio_recientes / self.recorrido);
            let tiempo_inicio = tiempo_cruce_umbral(
                self.ultimo_tiempo,
                time,
                self.ultimo_recorrido,
                self.recorrido,
                self.inicio_recientes,
            );
            self.recientes = if crate::multitarea_tactil::iniciar_borde(state, origen, tiempo_inicio)
            {
                RecientesDesdeBorde::EnCurso
            } else {
                RecientesDesdeBorde::NoDisponible
            };
        }
        if matches!(self.recientes, RecientesDesdeBorde::EnCurso) {
            crate::multitarea_tactil::mover_borde(state, punto, time);
        }
        self.ultimo_recorrido = self.recorrido;
        self.ultimo_tiempo = time;
        if matches!(self.dock, DockDesdeBorde::Oculto)
            && let Some(shell) = state.shell.as_mut()
        {
            let altura = self.inicio_recientes.max(1.0);
            let entra = (self.recorrido / altura).clamp(0.0, 1.0);
            // Al continuar, el dock se retira a medida que asoman las tarjetas.
            // Volver hacia abajo recorre las dos transiciones en sentido inverso.
            let sale = if matches!(self.recientes, RecientesDesdeBorde::EnCurso) {
                ((self.recorrido - altura) / altura).clamp(0.0, 1.0)
            } else {
                0.0
            };
            shell.previsualizar_dock((1.0 - entra + sale) as f32);
            state.needs_redraw = true;
        }
    }

    fn soltar(self, state: &mut BookosComp, time: u32, cancelar: bool) {
        let abre_recientes = matches!(self.recientes, RecientesDesdeBorde::EnCurso)
            && crate::multitarea_tactil::soltar_borde(state, time, cancelar);
        let mostrar_dock = match self.dock {
            DockDesdeBorde::Oculto => !cancelar && self.recorrido >= UMBRAL_GESTO,
            // Al deshacer Recientes se conserva el dock que estaba a la vista.
            DockDesdeBorde::Temporal => {
                !cancelar || matches!(self.recientes, RecientesDesdeBorde::EnCurso)
            }
            DockDesdeBorde::Visible => false,
        };
        if !abre_recientes && mostrar_dock {
            crate::tactil::revelar_barra(state, crate::shell::Barra::Dock);
        }
        if let Some(shell) = state.shell.as_mut() {
            shell.terminar_previa_dock();
        }
        state.needs_redraw = true;
    }
}

#[derive(Default)]
struct ColaEscritura {
    bytes: VecDeque<u8>,
}

impl ColaEscritura {
    fn encolar(&mut self, datos: &[u8]) -> Result<(), ()> {
        if self.bytes.len().saturating_add(datos.len()) > LIMITE_COLA_ESCRITURA {
            return Err(());
        }
        self.bytes.extend(datos);
        Ok(())
    }

    /// Intenta vaciar los bytes pendientes sin partir mensajes NDJSON al recibir `WouldBlock`.
    fn drenar(&mut self, writer: &mut impl Write) -> std::io::Result<bool> {
        while !self.bytes.is_empty() {
            let (front, _) = self.bytes.as_slices();
            match writer.write(front) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "escritura nula en socket de extras",
                    ));
                }
                Ok(n) => {
                    self.bytes.drain(..n);
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(err) => return Err(err),
            }
        }
        Ok(true)
    }
}

struct Cliente {
    escritor: UnixStream,
    token: RegistrationToken,
    write_token: Option<RegistrationToken>,
    cola: ColaEscritura,
    activo: bool,
    margen_borde: f64,
    gesto_inferior_horizontal: bool,
}

#[derive(Default)]
pub struct Estado {
    ruta: Option<PathBuf>,
    cliente: Option<Cliente>,
    toque: Option<ToqueBorde>,
    dedos_ignorados: Vec<TouchSlot>,
}

impl Drop for Estado {
    fn drop(&mut self) {
        if let Some(ruta) = self.ruta.take() {
            let _ = std::fs::remove_file(ruta);
        }
    }
}

impl Estado {
    pub fn inactivo() -> Self {
        Self::default()
    }
}

/// Ruta del socket de extensión para esta instancia de compositor.
pub fn ruta_socket(wayland_display: &str) -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("BOOKOS_EXTRAS_SOCK")
        && !custom.is_empty()
    {
        return Some(PathBuf::from(custom));
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(dir).join(format!("bookos-extras-{wayland_display}.sock")))
}

/// Arranca el listener pasivo en `calloop`. Si falla (p. ej. sin `XDG_RUNTIME_DIR`),
/// el compositor sigue funcionando sin extensión.
pub fn arrancar(loop_handle: &LoopHandle<'static, BookosComp>, socket_name: &str) -> Estado {
    let Some(ruta) = ruta_socket(socket_name) else {
        return Estado::inactivo();
    };
    let _ = std::fs::remove_file(&ruta);
    let listener = match UnixListener::bind(&ruta) {
        Ok(l) => l,
        Err(err) => {
            tracing::warn!(ruta = %ruta.display(), "no se pudo abrir socket de extras: {err}");
            return Estado::inactivo();
        }
    };
    if let Err(err) = listener.set_nonblocking(true) {
        tracing::warn!("socket de extras sin nonblocking: {err}");
        let _ = std::fs::remove_file(&ruta);
        return Estado::inactivo();
    }
    let fuente = Generic::new(listener, Interest::READ, Mode::Level);
    let res = loop_handle.insert_source(fuente, |_, listener, state| {
        loop {
            match listener.accept() {
                Ok((stream, _)) => aceptar_cliente(state, stream),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    tracing::warn!("error aceptando cliente de extras: {err}");
                    break;
                }
            }
        }
        Ok(PostAction::Continue)
    });
    if let Err(err) = res {
        tracing::warn!("no se pudo registrar socket de extras en calloop: {err}");
        let _ = std::fs::remove_file(&ruta);
        return Estado::inactivo();
    }
    tracing::debug!(ruta = %ruta.display(), "punto de extensión bookos-extras listo");
    Estado {
        ruta: Some(ruta),
        cliente: None,
        toque: None,
        dedos_ignorados: Vec::new(),
    }
}

fn aceptar_cliente(state: &mut BookosComp, stream: UnixStream) {
    if let Err(err) = stream.set_nonblocking(true) {
        tracing::warn!("cliente extras nonblocking: {err}");
        return;
    }
    let Ok(escritor) = stream.try_clone() else {
        return;
    };
    // Si ya había un cliente conectado, se reemplaza limpiamente.
    desconectar_cliente(state);

    let fuente = Generic::new(stream, Interest::READ, Mode::Level);
    let mut pendiente = Vec::<u8>::new();
    let token = match state.loop_handle.insert_source(fuente, move |_, stream, state| {
        let mut cerrar = false;
        let mut buf = [0u8; 4096];
        loop {
            match (&**stream).read(&mut buf) {
                Ok(0) => {
                    cerrar = true;
                    break;
                }
                Ok(n) => {
                    pendiente.extend_from_slice(&buf[..n]);
                    while let Some(pos) = pendiente.iter().position(|&b| b == b'\n') {
                        let linea = pendiente.drain(..=pos).collect::<Vec<_>>();
                        if let Ok(texto) = std::str::from_utf8(&linea) {
                            let texto = texto.trim();
                            if !texto.is_empty() {
                                match serde_json::from_str::<Peticion>(texto) {
                                    Ok(peticion) => ejecutar_peticion(state, peticion),
                                    Err(err) => {
                                        tracing::debug!("petición de extras inválida: {err}");
                                    }
                                }
                            }
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) => {
                    tracing::debug!("lectura de extras cerrada: {err}");
                    cerrar = true;
                    break;
                }
            }
        }
        if cerrar {
            cerrar_cliente(state, false);
            tracing::info!("cliente bookos-extras desconectado");
            Ok(PostAction::Remove)
        } else {
            Ok(PostAction::Continue)
        }
    }) {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!("no se pudo registrar cliente de extras: {err}");
            return;
        }
    };

    state.extras.cliente = Some(Cliente {
        escritor,
        token,
        write_token: None,
        cola: ColaEscritura::default(),
        activo: true,
        margen_borde: MARGEN_PREDETERMINADO,
        gesto_inferior_horizontal: true,
    });
    tracing::info!("cliente bookos-extras conectado");
    enviar_pantalla(state);
    enviar_notificaciones(state);
}

fn desconectar_cliente(state: &mut BookosComp) {
    cerrar_cliente(state, true);
}

fn cerrar_cliente(state: &mut BookosComp, quitar_lectura: bool) {
    if let Some(mut cliente) = state.extras.cliente.take() {
        if quitar_lectura {
            state.loop_handle.remove(cliente.token);
        }
        if let Some(write_token) = cliente.write_token.take() {
            state.loop_handle.remove(write_token);
        }
    }
    interceptar_toque_cancelar(state, 0);
}

fn asegurar_fuente_escritura(state: &mut BookosComp) {
    let Some(cliente) = state.extras.cliente.as_ref() else {
        return;
    };
    if cliente.write_token.is_some() {
        return;
    }
    let Ok(escritor_async) = cliente.escritor.try_clone() else {
        desconectar_cliente(state);
        return;
    };
    let fuente = Generic::new(escritor_async, Interest::WRITE, Mode::Level);
    match state.loop_handle.insert_source(fuente, |_, stream, state| {
        let resultado = if let Some(cliente) = state.extras.cliente.as_mut() {
            cliente.cola.drenar(&mut &**stream)
        } else {
            return Ok(PostAction::Remove);
        };
        match resultado {
            Ok(true) => {
                if let Some(cliente) = state.extras.cliente.as_mut() {
                    cliente.write_token = None;
                }
                Ok(PostAction::Remove)
            }
            Ok(false) => Ok(PostAction::Continue),
            Err(err) => {
                tracing::debug!("drenaje de escritura a extras fallido, desconectando: {err}");
                if let Some(mut cliente) = state.extras.cliente.take() {
                    cliente.write_token = None;
                    state.loop_handle.remove(cliente.token);
                }
                interceptar_toque_cancelar(state, 0);
                Ok(PostAction::Remove)
            }
        }
    }) {
        Ok(token) => {
            if let Some(cliente) = state.extras.cliente.as_mut() {
                cliente.write_token = Some(token);
            }
        }
        Err(err) => {
            tracing::warn!("no se pudo registrar drenaje de extras en calloop: {err}");
            desconectar_cliente(state);
        }
    }
}

fn enviar(state: &mut BookosComp, evento: &Evento) {
    let Ok(mut linea) = serde_json::to_string(evento) else {
        return;
    };
    linea.push('\n');
    let resultado = {
        let Some(cliente) = state.extras.cliente.as_mut() else {
            return;
        };
        if cliente.cola.encolar(linea.as_bytes()).is_err() {
            tracing::warn!("cola de escritura de extras saturada, desconectando");
            Err(std::io::Error::other("cola saturada"))
        } else {
            cliente.cola.drenar(&mut cliente.escritor)
        }
    };
    match resultado {
        Ok(true) => {
            if let Some(cliente) = state.extras.cliente.as_mut()
                && let Some(write_token) = cliente.write_token.take()
            {
                state.loop_handle.remove(write_token);
            }
        }
        Ok(false) => asegurar_fuente_escritura(state),
        Err(err) => {
            tracing::debug!("envío a extras fallido, desconectando: {err}");
            desconectar_cliente(state);
        }
    }
}

/// Envía al cliente `bookos-extras` la geometría y tema actuales.
pub fn enviar_pantalla(state: &mut BookosComp) {
    if state.extras.cliente.is_none() {
        return;
    }
    let (ancho, alto, escala) = geometria_tactil(state).unwrap_or((1280.0, 800.0, 1.0));
    let oscuro = !bookos_shell::tema::es_claro();
    let efectos_reducidos = bookos_shell::tema::efectos_reducidos();
    enviar(
        state,
        &Evento::Pantalla {
            ancho,
            alto,
            escala,
            oscuro,
            efectos_reducidos,
        },
    );
}

/// Notifica a `bookos-extras` que la lista de notificaciones cambió.
pub fn notificaciones_cambiaron(state: &mut BookosComp) {
    if state.extras.cliente.is_none() {
        return;
    }
    enviar_notificaciones(state);
}

/// Avisa a `bookos-extras` de un toque fuera de sus capas para cerrar la cortina.
pub fn toque_fuera(state: &mut BookosComp) {
    if state.extras.cliente.is_none() {
        return;
    }
    enviar(state, &Evento::ToqueFuera);
}

fn encode_base64(datos: &[u8]) -> String {
    const TABLA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(datos.len().div_ceil(3) * 4);
    for chunk in datos.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLA[((n >> 18) & 63) as usize] as char);
        out.push(TABLA[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLA[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLA[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn reducir_rgba_dto(ancho: u32, alto: u32, rgba: &[u8], max_lado: u32) -> Option<ImagenNotifDto> {
    if ancho == 0 || alto == 0 || rgba.len() < (ancho as usize).checked_mul(alto as usize)?.checked_mul(4)? {
        return None;
    }
    let (dw, dh) = if ancho.max(alto) <= max_lado {
        (ancho, alto)
    } else if ancho >= alto {
        (max_lado, ((alto * max_lado) / ancho).max(1))
    } else {
        (((ancho * max_lado) / alto).max(1), max_lado)
    };
    let mut out = Vec::with_capacity((dw * dh * 4) as usize);
    for y in 0..dh {
        let sy = ((y as u64 * alto as u64) / dh as u64).min(alto as u64 - 1) as usize;
        for x in 0..dw {
            let sx = ((x as u64 * ancho as u64) / dw as u64).min(ancho as u64 - 1) as usize;
            let idx = (sy * ancho as usize + sx) * 4;
            out.extend_from_slice(&rgba[idx..idx + 4]);
        }
    }
    Some(ImagenNotifDto {
        ancho: dw,
        alto: dh,
        rgba_b64: encode_base64(&out),
    })
}

fn enviar_notificaciones(state: &mut BookosComp) {
    let (lista, no_molestar) = state
        .shell
        .as_mut()
        .map(|shell| {
            let lista = shell
                .notificaciones_lista()
                .iter()
                .map(|n| {
                    let tiene_default = n.acciones.iter().any(|a| a.clave == "default");
                    let imagen = n
                        .icono
                        .as_ref()
                        .and_then(bookos_shell::icono::rgba_de)
                        .and_then(|(w, h, rgba)| reducir_rgba_dto(w, h, rgba, 48));
                    NotificacionDto {
                        id: n.id,
                        app: n.app.clone(),
                        resumen: n.resumen.clone(),
                        cuerpo: n.cuerpo.clone(),
                        critica: n.critica,
                        leida: n.leida,
                        progreso: n.progreso,
                        segundos: n.llegada.elapsed().as_secs(),
                        acciones: n
                            .acciones
                            .iter()
                            .map(|a| AccionNotifDto {
                                clave: a.clave.clone(),
                                etiqueta: a.etiqueta.clone(),
                            })
                            .collect(),
                        tiene_default,
                        es_captura: n.ruta.is_some(),
                        imagen,
                    }
                })
                .collect();
            (lista, shell.no_molestar(None))
        })
        .unwrap_or_default();
    enviar(state, &Evento::Notificaciones { lista, no_molestar });
}

fn horizontal_activo() -> bool {
    true
}

fn ejecutar_peticion(state: &mut BookosComp, peticion: Peticion) {
    match peticion {
        Peticion::Config {
            activo,
            margen_borde,
            gesto_inferior_horizontal,
        } => {
            if let Some(cliente) = state.extras.cliente.as_mut() {
                cliente.activo = activo;
                cliente.gesto_inferior_horizontal = gesto_inferior_horizontal;
                cliente.margen_borde = margen_borde.clamp(MARGEN_MIN, MARGEN_MAX);
                if !activo {
                    interceptar_toque_cancelar(state, 0);
                }
            }
        }
        Peticion::MenuBookos => {
            toque_fuera(state);
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::MenuBookos);
        }
        Peticion::CentroControl => {
            toque_fuera(state);
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::CentroControl);
        }
        Peticion::MostrarPanel => {
            crate::tactil::revelar_barra(state, crate::shell::Barra::Panel);
        }
        Peticion::VistaInicio => {
            // La apertura continua arranca al primer `VistaProgreso`.
            state.needs_redraw = true;
        }
        Peticion::VistaProgreso { progreso } => {
            let activo = crate::escritorios::activo_aqui(state);
            let nombres = state.escritorios.nombres().to_vec();
            if let Some(shell) = state.shell.as_mut() {
                shell.seguir_vista(progreso, activo, nombres);
            }
            state.revisar_barras();
            state.needs_redraw = true;
        }
        Peticion::VistaSoltar {
            progreso,
            velocidad,
            cancelada,
        } => {
            if let Some(shell) = state.shell.as_mut() {
                shell.soltar_vista(Suelta {
                    movimiento: Movimiento::Vista,
                    progreso,
                    velocidad,
                    cancelada,
                });
            }
            state.revisar_barras();
            state.needs_redraw = true;
        }
        Peticion::ExposicionProgreso {
            progreso,
            desplazamiento_x,
        } => {
            crate::input::seguir_exposicion_con_x(state, progreso, desplazamiento_x);
        }
        Peticion::ExposicionSoltar {
            progreso,
            velocidad,
            cancelada,
        } => {
            crate::input::soltar_exposicion(
                state,
                Suelta {
                    movimiento: Movimiento::Exposicion,
                    progreso,
                    velocidad,
                    cancelada,
                },
            );
        }
        Peticion::ExposicionEscritorioProgreso {
            progreso,
            desplazamiento_x,
        } => {
            crate::input::seguir_exposicion_escritorio(state, progreso, desplazamiento_x);
        }
        Peticion::ExposicionEscritorioSoltar {
            progreso,
            velocidad,
            cancelada,
        } => {
            crate::input::soltar_exposicion_escritorio(
                state,
                Suelta {
                    movimiento: Movimiento::Despeje,
                    progreso,
                    velocidad,
                    cancelada,
                },
            );
        }
        Peticion::DespejarEscritorio => {
            if exposicion_abierta(state) {
                cerrar_exposicion_inmediata(state);
            }
            if state
                .shell
                .as_ref()
                .is_some_and(|s| s.vista_escritorios_abierta())
                && let Some(shell) = state.shell.as_mut()
            {
                shell.cerrar_emergente();
            }
            crate::escritorios::despejar(state);
            state.revisar_barras();
            state.needs_redraw = true;
        }
        Peticion::DespejeProgreso { progreso } => {
            if exposicion_abierta(state) {
                cerrar_exposicion_inmediata(state);
            }
            crate::escritorios::seguir_despeje(state, progreso);
        }
        Peticion::DespejeSoltar {
            progreso,
            velocidad,
            cancelada,
        } => {
            crate::escritorios::soltar_despeje(
                state,
                Suelta {
                    movimiento: Movimiento::Despeje,
                    progreso,
                    velocidad,
                    cancelada,
                },
            );
        }
        Peticion::EscritorioInicio => {
            crate::multitarea_tactil::cancelar(state);
            if exposicion_abierta(state) {
                cerrar_exposicion_inmediata(state);
            }
            crate::escritorios::agarrar_dedos(state);
        }
        Peticion::EscritorioProgreso { progreso } => {
            crate::escritorios::seguir_dedos(state, progreso);
        }
        Peticion::EscritorioSoltar {
            progreso,
            velocidad,
            cancelada,
        } => {
            crate::escritorios::soltar_dedos(
                state,
                Suelta {
                    movimiento: Movimiento::Escritorio,
                    progreso,
                    velocidad,
                    cancelada,
                },
            );
        }
        Peticion::Atras => ejecutar_atras(state),
        Peticion::Adelante => enviar_tecla_navegacion(state, XKB_KEY_FORWARD),
        Peticion::PedirNotificaciones => enviar_notificaciones(state),
        Peticion::CerrarNotificacion { id } => {
            let cerrada = state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_notificacion(id));
            if cerrada {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
                state.needs_redraw = true;
            }
            enviar_notificaciones(state);
        }
        Peticion::LimpiarNotificaciones => {
            let ids = state
                .shell
                .as_mut()
                .map(|s| s.borrar_notificaciones())
                .unwrap_or_default();
            for id in ids {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
            enviar_notificaciones(state);
        }
        Peticion::InvocarAccionNotificacion { id, clave } => {
            if clave == "inline-reply" {
                toque_fuera(state);
                if let Some(shell) = state.shell.as_mut() {
                    shell.abrir_respuesta(id);
                }
                state.needs_redraw = true;
            } else {
                if !crate::notificaciones::abrir_ruta_de_notificacion(state, id) {
                    crate::notificaciones::accion_invocada(
                        state.bus_notificaciones.as_ref(),
                        id,
                        &clave,
                    );
                }
                if state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.cerrar_notificacion(id))
                {
                    crate::notificaciones::cerrada(
                        state.bus_notificaciones.as_ref(),
                        id,
                        crate::notificaciones::CERRADA_POR_EL_USUARIO,
                    );
                }
                if clave == "default" {
                    toque_fuera(state);
                }
                state.needs_redraw = true;
                crate::notificaciones::despertar_para_la_salida(state);
                enviar_notificaciones(state);
            }
        }
        Peticion::MarcarNotificacionesLeidas => {
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.marcar_notificaciones_leidas())
            {
                state.needs_redraw = true;
            }
            enviar_notificaciones(state);
        }
        Peticion::CortinaAbierta => {
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_emergente())
            {
                state.revisar_barras();
                state.needs_redraw = true;
            }
        }
        Peticion::AlternarNoMolestar => {
            if let Some(shell) = state.shell.as_mut() {
                let nuevo = !shell.no_molestar(None);
                shell.no_molestar(Some(nuevo));
                state.needs_redraw = true;
            }
            enviar_notificaciones(state);
        }
        Peticion::AbrirAjustesNotificaciones => {
            toque_fuera(state);
            crate::keybinds::lanzar(state, "bookos-settings --page notificaciones");
        }
    }
}

/// Gesto «Atrás»: primero cierra cualquier emergente del shell; si no había
/// ninguno abierto, envía `XF86Back` a la ventana con foco.
fn ejecutar_atras(state: &mut BookosComp) {
    if state
        .shell
        .as_mut()
        .is_some_and(|s| s.cerrar_emergente())
    {
        state.needs_redraw = true;
        return;
    }
    enviar_tecla_navegacion(state, XKB_KEY_BACK);
}

fn enviar_tecla_navegacion(state: &mut BookosComp, xkb_code: u32) {
    let Some(kbd) = state.seat.get_keyboard() else {
        return;
    };
    let tiempo = state.start_time.elapsed().as_millis() as u32;
    let codigo = Keycode::new(xkb_code);
    let serial_down = smithay::utils::SERIAL_COUNTER.next_serial();
    kbd.input::<(), _>(state, codigo, KeyState::Pressed, serial_down, tiempo, |_, _, _| {
        smithay::input::keyboard::FilterResult::Forward
    });
    let serial_up = smithay::utils::SERIAL_COUNTER.next_serial();
    kbd.input::<(), _>(
        state,
        codigo,
        KeyState::Released,
        serial_up,
        tiempo.saturating_add(1),
        |_, _, _| smithay::input::keyboard::FilterResult::Forward,
    );
}

fn geometria_tactil(state: &BookosComp) -> Option<(f64, f64, f64)> {
    let salida = crate::input::salida_tactil(state)?;
    let geo = state.space.output_geometry(salida)?;
    let escala = salida.current_scale().fractional_scale();
    Some((geo.size.w as f64, geo.size.h as f64, escala))
}

fn exposicion_abierta(state: &BookosComp) -> bool {
    state.conmutador_pegado
        && state.conmutador_salida.is_none()
        && state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
}

fn cerrar_exposicion_inmediata(state: &mut BookosComp) {
    state.conmutador_pegado = false;
    state.conmutador_salida = None;
    state.conmutador_entrada = None;
    state.exposicion_gesto = None;
    crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarCancelar);
}

fn puede_interceptar(state: &BookosComp) -> Option<f64> {
    if state.modo.kiosko() || state.cuentagotas {
        return None;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.esta_bloqueado() || s.hay_captura())
    {
        return None;
    }
    match &state.extras.cliente {
        Some(cliente) => cliente.activo.then_some(cliente.margen_borde),
        None => Some(MARGEN_PREDETERMINADO),
    }
}

/// Detecta si un punto lógico cae dentro de alguno de los cuatro bordes.
pub fn detectar_borde(x: f64, y: f64, ancho: f64, alto: f64, margen: f64) -> Option<Borde> {
    if ancho <= 0.0 || alto <= 0.0 {
        return None;
    }
    if y <= margen {
        Some(Borde::Arriba)
    } else if y >= alto - margen {
        Some(Borde::Abajo)
    } else if x <= margen {
        Some(Borde::Izquierda)
    } else if x >= ancho - margen {
        Some(Borde::Derecha)
    } else {
        None
    }
}

/// Decide si el desplazamiento `(dx, dy)` desde un borde corresponde a un gesto
/// del sistema (hacia dentro, o horizontal en el borde inferior).
pub fn es_gesto_desde_borde(borde: Borde, dx: f64, dy: f64) -> bool {
    match borde {
        Borde::Arriba => dy > 0.0 && dy.abs() >= dx.abs() * 0.8,
        Borde::Abajo => dy < 0.0 || dx.abs() >= dy.abs() * 1.1,
        Borde::Izquierda => dx > 0.0 && dx.abs() >= dy.abs() * 1.1,
        Borde::Derecha => dx < 0.0 && dx.abs() >= dy.abs() * 1.1,
    }
}

/// Retiene el toque hasta distinguir un gesto del sistema de un toque normal.
pub fn interceptar_toque_abajo(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Point<f64, Logical>,
    time: u32,
) -> bool {
    state.extras.dedos_ignorados.retain(|&s| s != slot);
    let Some(margen) = puede_interceptar(state) else {
        return false;
    };
    // Si llega un toque nuevo con el mismo slot que había quedado huérfano, se limpia.
    if let Some(prev) = state.extras.toque {
        if prev.slot() == slot {
            interceptar_toque_cancelar(state, time);
        } else if let ToqueBorde::Pendiente {
            slot: prev_slot,
            origen,
            tiempo,
            ..
        } = prev
        {
            // Un segundo dedo descarta la espera de borde y reenvía primero el dedo 0
            // en orden cronológico para no desordenar pellizcos o scrolls.
            state.extras.toque = None;
            state.extras.dedos_ignorados.clear();
            crate::input::toque_abajo_normal(state, prev_slot, origen, tiempo);
            return false;
        } else {
            if !state.extras.dedos_ignorados.contains(&slot) {
                state.extras.dedos_ignorados.push(slot);
            }
            return true;
        }
    }
    let Some(geo) = crate::input::salida_tactil(state)
        .and_then(|salida| state.space.output_geometry(salida))
        .map(|r| r.to_f64())
    else {
        return false;
    };
    let (ancho, alto) = (geo.size.w, geo.size.h);
    let local = punto - geo.loc;
    let Some(borde) = detectar_borde(local.x, local.y, ancho, alto, margen) else {
        return false;
    };
    let sin_extras = state.extras.cliente.is_none();
    if sin_extras
        && borde != Borde::Abajo
        && (borde != Borde::Arriba
            || !crate::tactil::puede_revelar_barra(state, crate::shell::Barra::Panel))
    {
        return false;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.teclado_en(punto.x, punto.y).is_some())
    {
        return false;
    }
    if borde == Borde::Abajo
        && local.y < alto - 8.0
        && state.shell_contiene(punto)
        && state
            .shell
            .as_ref()
            .is_some_and(|s| s.en_el_dock(punto.x, punto.y))
    {
        return false;
    }
    // Con el conmutador/exposición abierto solo se intercepta el borde inferior
    // (para el segundo gesto de abajo→arriba hacia el escritorio o cambio de escritorio).
    if (state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) || state.vista_tactil.is_some())
        && borde != Borde::Abajo
    {
        return false;
    }
    state.extras.toque = Some(ToqueBorde::Pendiente {
        slot,
        origen: punto,
        borde,
        tiempo: time,
        ancho,
        alto,
        origen_salida: geo.loc,
        sin_extras,
    });
    true
}

fn slot_id(slot: TouchSlot) -> u32 {
    i32::from(slot).max(0) as u32
}

/// Devuelve `true` si el movimiento lo consume `bookos-extras`.
pub fn interceptar_toque_mueve(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Point<f64, Logical>,
    time: u32,
) -> bool {
    if state.extras.dedos_ignorados.contains(&slot) {
        return true;
    }
    let Some(actual) = state.extras.toque.filter(|t| t.slot() == slot) else {
        return false;
    };
    match actual {
        ToqueBorde::Barra { .. } => true,
        ToqueBorde::Inferior { mut gesto, .. } => {
            gesto.mover(state, punto, time);
            state.extras.toque = Some(ToqueBorde::Inferior { slot, gesto });
            true
        }
        ToqueBorde::Capturado { origen_salida, .. } => {
            let local = punto - origen_salida;
            enviar(
                state,
                &Evento::ToqueMueve {
                    slot: slot_id(slot),
                    x: local.x,
                    y: local.y,
                    tiempo: time,
                },
            );
            true
        }
        ToqueBorde::Pendiente {
            origen,
            borde,
            tiempo,
            ancho,
            alto,
            origen_salida,
            sin_extras,
            ..
        } => {
            let dx = punto.x - origen.x;
            let dy = punto.y - origen.y;
            if dx.hypot(dy) < UMBRAL_GESTO {
                return true;
            }
            let horizontal = state
                .extras
                .cliente
                .as_ref()
                .is_some_and(|c| c.gesto_inferior_horizontal);
            if borde == Borde::Arriba
                && es_gesto_desde_borde(borde, dx, dy)
                && crate::tactil::revelar_barra_si_oculta(state, crate::shell::Barra::Panel)
            {
                state.extras.toque = Some(ToqueBorde::Barra { slot });
                return true;
            }
            if borde == Borde::Abajo && dy < 0.0 && dx.abs() <= -dy * 1.15 {
                let mut gesto = GestoInferior::nuevo(state, origen, tiempo, alto);
                gesto.mover(state, punto, time);
                state.extras.toque = Some(ToqueBorde::Inferior { slot, gesto });
                return true;
            }
            if !sin_extras
                && (borde != Borde::Abajo || horizontal)
                && es_gesto_desde_borde(borde, dx, dy)
            {
                let panel_visible = state
                    .shell
                    .as_ref()
                    .is_none_or(|s| s.panel_visible());
                let exposicion = exposicion_abierta(state);
                state.extras.toque = Some(ToqueBorde::Capturado {
                    slot,
                    origen_salida,
                });
                let local = origen - origen_salida;
                enviar(
                    state,
                    &Evento::ToqueInicio {
                        slot: slot_id(slot),
                        x: local.x,
                        y: local.y,
                        ancho,
                        alto,
                        borde,
                        panel_visible,
                        exposicion_abierta: exposicion,
                        tiempo,
                    },
                );
                let local = punto - origen_salida;
                enviar(
                    state,
                    &Evento::ToqueMueve {
                        slot: slot_id(slot),
                        x: local.x,
                        y: local.y,
                        tiempo: time,
                    },
                );
                true
            } else {
                // No era un gesto de borde (p. ej. scroll vertical pegado al margen):
                // se reenvía el toque original de forma transparente al escritorio/app.
                state.extras.toque = None;
                crate::input::toque_abajo_normal(state, slot, origen, tiempo);
                crate::input::toque_mueve_normal(state, slot, punto, time);
                true
            }
        }
    }
}

/// Devuelve `true` si la suelta la consume `bookos-extras` (o se reenvió como toque normal).
pub fn interceptar_toque_arriba(state: &mut BookosComp, slot: TouchSlot, time: u32) -> bool {
    if let Some(pos) = state.extras.dedos_ignorados.iter().position(|&s| s == slot) {
        state.extras.dedos_ignorados.swap_remove(pos);
        return true;
    }
    let Some(actual) = state.extras.toque.filter(|t| t.slot() == slot) else {
        return false;
    };
    state.extras.toque = None;
    match actual {
        ToqueBorde::Barra { .. } => true,
        ToqueBorde::Inferior { gesto, .. } => {
            gesto.soltar(state, time, false);
            true
        }
        ToqueBorde::Capturado { .. } => {
            enviar(
                state,
                &Evento::ToqueFin {
                    slot: slot_id(slot),
                    tiempo: time,
                },
            );
            true
        }
        ToqueBorde::Pendiente { origen, tiempo, .. } => {
            // Toque rápido en el borde (panel, dock o botón de una app): se reproduce normal.
            crate::input::toque_abajo_normal(state, slot, origen, tiempo);
            crate::input::toque_arriba_normal(state, slot, time);
            true
        }
    }
}

pub fn interceptar_toque_cancelar(state: &mut BookosComp, time: u32) {
    state.extras.dedos_ignorados.clear();
    match state.extras.toque.take() {
        Some(ToqueBorde::Inferior { gesto, .. }) => gesto.soltar(state, time, true),
        Some(ToqueBorde::Capturado { slot, .. }) => enviar(
            state,
            &Evento::ToqueCancelar {
                slot: slot_id(slot),
                tiempo: time,
            },
        ),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bordes_detectados_correctamente() {
        assert_eq!(detectar_borde(600.0, 5.0, 1280.0, 800.0, 24.0), Some(Borde::Arriba));
        assert_eq!(detectar_borde(600.0, 790.0, 1280.0, 800.0, 24.0), Some(Borde::Abajo));
        assert_eq!(detectar_borde(10.0, 400.0, 1280.0, 800.0, 24.0), Some(Borde::Izquierda));
        assert_eq!(detectar_borde(1270.0, 400.0, 1280.0, 800.0, 24.0), Some(Borde::Derecha));
        assert_eq!(detectar_borde(640.0, 400.0, 1280.0, 800.0, 24.0), None);
    }

    #[test]
    fn direccion_de_gesto_por_borde() {
        assert!(es_gesto_desde_borde(Borde::Arriba, 2.0, 25.0));
        assert!(!es_gesto_desde_borde(Borde::Arriba, 30.0, 5.0));
        assert!(es_gesto_desde_borde(Borde::Abajo, 0.0, -25.0));
        assert!(es_gesto_desde_borde(Borde::Abajo, 30.0, 2.0));
        assert!(es_gesto_desde_borde(Borde::Izquierda, 25.0, 3.0));
        assert!(!es_gesto_desde_borde(Borde::Izquierda, 3.0, 25.0));
        assert!(es_gesto_desde_borde(Borde::Derecha, -25.0, 3.0));
        assert!(!es_gesto_desde_borde(Borde::Derecha, -3.0, 25.0));
    }

    #[test]
    fn protocolo_json_ida_y_vuelta() {
        let ev = Evento::ToqueInicio {
            slot: 0,
            x: 10.0,
            y: 2.0,
            ancho: 1280.0,
            alto: 800.0,
            borde: Borde::Arriba,
            panel_visible: true,
            exposicion_abierta: false,
            tiempo: 100,
        };
        let s = serde_json::to_string(&ev).unwrap();
        let dec: Evento = serde_json::from_str(&s).unwrap();
        assert!(matches!(dec, Evento::ToqueInicio { borde: Borde::Arriba, .. }));

        let pet = Peticion::ExposicionProgreso {
            progreso: 0.42,
            desplazamiento_x: 0.1,
        };
        let s2 = serde_json::to_string(&pet).unwrap();
        let dec2: Peticion = serde_json::from_str(&s2).unwrap();
        assert!(matches!(dec2, Peticion::ExposicionProgreso { .. }));
    }

    #[test]
    fn tiempo_al_cruzar_umbral_no_arrastra_espera_previa_en_el_dock() {
        // El usuario esperó 500 ms en y=40 antes de dar un tirón rápido a y=64 en 16 ms.
        let t = tiempo_cruce_umbral(500, 516, 40.0, 64.0, 52.0);
        assert_eq!(t, 508);
    }

    #[test]
    fn cola_escritura_conserva_mensajes_tras_would_block_y_escritura_parcial() {
        struct EscritorParcial {
            salida: Vec<u8>,
            paso: usize,
        }
        impl Write for EscritorParcial {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.paso += 1;
                match self.paso {
                    1 => {
                        let n = buf.len().min(7);
                        self.salida.extend_from_slice(&buf[..n]);
                        Ok(n)
                    }
                    2 => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
                    _ => {
                        self.salida.extend_from_slice(buf);
                        Ok(buf.len())
                    }
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut cola = ColaEscritura::default();
        cola.encolar(b"{\"tipo\":\"toque_fuera\"}\n").unwrap();
        let mut escritor = EscritorParcial {
            salida: Vec::new(),
            paso: 0,
        };
        assert!(!cola.drenar(&mut escritor).unwrap());
        cola.encolar(b"{\"tipo\":\"pantalla\"}\n").unwrap();
        assert!(cola.drenar(&mut escritor).unwrap());
        assert_eq!(
            String::from_utf8(escritor.salida).unwrap(),
            "{\"tipo\":\"toque_fuera\"}\n{\"tipo\":\"pantalla\"}\n"
        );
    }

    #[test]
    fn cola_escritura_rechaza_desbordamiento() {
        let mut cola = ColaEscritura::default();
        let bloque = vec![b'a'; LIMITE_COLA_ESCRITURA];
        assert!(cola.encolar(&bloque).is_ok());
        assert!(cola.encolar(b"b").is_err());
    }
}
