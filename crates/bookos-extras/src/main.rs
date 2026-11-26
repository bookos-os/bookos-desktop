//! `bookos-extras`: paquete opcional de navegación táctil estilo Android/iOS
//! y cortina ampliada de notificaciones para BookOS.

mod config;
mod fisica;
mod gestos;
mod paleta;
mod protocolo;
mod ui;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init};

use crate::config::Config;
use crate::gestos::{EfectoLocal, MotorGestos};
use crate::protocolo::{Evento, Peticion};
use crate::ui::{AccionUi, UiState};

static RECARGAR: AtomicBool = AtomicBool::new(false);

extern "C" fn manejar_sigusr1(_: libc::c_int) {
    RECARGAR.store(true, Ordering::SeqCst);
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bookos_extras=info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!(
                "Uso: bookos-extras [daemon|status|enable|disable|toggle]\n\n\
                 Paquete opcional de navegación táctil estilo Android/iOS para BookOS.\n\
                   daemon    ejecuta el servicio de gestos y cortina de notificaciones\n\
                   status    muestra si la navegación táctil está activada\n\
                   enable    activa la navegación táctil y recarga el servicio\n\
                   disable   desactiva la navegación táctil y recarga el servicio\n\
                   toggle    alterna la navegación táctil y recarga el servicio"
            );
            Ok(())
        }
        Some("status") => {
            let cfg = config::cargar();
            println!(
                "bookos-extras: {}",
                if cfg.activo { "activado" } else { "desactivado" }
            );
            Ok(())
        }
        Some("enable") => {
            let mut cfg = config::cargar();
            cfg.activo = true;
            config::guardar(&cfg)?;
            config::notificar_recarga();
            println!("bookos-extras: activado");
            Ok(())
        }
        Some("disable") => {
            let mut cfg = config::cargar();
            cfg.activo = false;
            config::guardar(&cfg)?;
            config::notificar_recarga();
            println!("bookos-extras: desactivado");
            Ok(())
        }
        Some("toggle") => {
            let mut cfg = config::cargar();
            cfg.activo = !cfg.activo;
            config::guardar(&cfg)?;
            config::notificar_recarga();
            println!(
                "bookos-extras: {}",
                if cfg.activo { "activado" } else { "desactivado" }
            );
            Ok(())
        }
        None | Some("daemon") => ejecutar_demonio(),
        Some(otro) => anyhow::bail!("subcomando desconocido: {otro} (usa --help)"),
    }
}

const LIMITE_COLA_ESCRITURA: usize = 512 * 1024;

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

    fn drenar(&mut self, writer: &mut impl Write) -> std::io::Result<bool> {
        while !self.bytes.is_empty() {
            let (front, _) = self.bytes.as_slices();
            match writer.write(front) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "escritura nula hacia compositor",
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

    fn limpiar(&mut self) {
        self.bytes.clear();
    }
}

struct App {
    cfg: Config,
    motor: MotorGestos,
    ui: UiState,
    qh: QueueHandle<UiState>,
    handle: LoopHandle<'static, App>,
    escritor: Option<UnixStream>,
    cola: ColaEscritura,
    read_token: Option<RegistrationToken>,
    write_token: Option<RegistrationToken>,
    conectado: bool,
    timer_anim_activo: bool,
}

impl App {
    fn desconectar(&mut self, quitar_lectura: bool) {
        self.escritor = None;
        self.cola.limpiar();
        self.conectado = false;
        if quitar_lectura && let Some(t) = self.read_token.take() {
            self.handle.remove(t);
        } else {
            self.read_token = None;
        }
        if let Some(t) = self.write_token.take() {
            self.handle.remove(t);
        }
        let salida = self.motor.cancelar_todo();
        self.aplicar_salida(salida);
        self.ui.cerrar_cortina_animada(-1.8);
        self.asegurar_timer_animacion();
    }

    fn asegurar_fuente_escritura(&mut self) {
        if self.write_token.is_some() {
            return;
        }
        let Some(stream) = self.escritor.as_ref() else {
            return;
        };
        let Ok(escritor_async) = stream.try_clone() else {
            self.desconectar(true);
            return;
        };
        let fuente = Generic::new(escritor_async, Interest::WRITE, Mode::Level);
        match self.handle.insert_source(fuente, |_, stream, app| {
            match app.cola.drenar(&mut &**stream) {
                Ok(true) => {
                    app.write_token = None;
                    Ok(PostAction::Remove)
                }
                Ok(false) => Ok(PostAction::Continue),
                Err(err) => {
                    tracing::warn!("error drenando hacia compositor: {err}");
                    app.write_token = None;
                    app.desconectar(true);
                    Ok(PostAction::Remove)
                }
            }
        }) {
            Ok(token) => self.write_token = Some(token),
            Err(err) => {
                tracing::warn!("no se pudo registrar drenaje de escritura: {err}");
                self.desconectar(true);
            }
        }
    }

    fn enviar(&mut self, peticion: &Peticion) {
        let Some(stream) = self.escritor.as_mut() else {
            return;
        };
        let Ok(mut linea) = serde_json::to_string(peticion) else {
            return;
        };
        linea.push('\n');
        if self.cola.encolar(linea.as_bytes()).is_err() {
            tracing::warn!("cola de escritura saturada hacia compositor, desconectando");
            self.desconectar(true);
            return;
        }
        match self.cola.drenar(stream) {
            Ok(true) => {
                if let Some(t) = self.write_token.take() {
                    self.handle.remove(t);
                }
            }
            Ok(false) => self.asegurar_fuente_escritura(),
            Err(err) => {
                tracing::warn!("error escribiendo al compositor: {err}");
                self.desconectar(true);
            }
        }
    }

    fn enviar_config(&mut self) {
        let pet = Peticion::Config {
            activo: self.cfg.activo,
            margen_borde: self.cfg.margen_borde,
            gesto_inferior_horizontal: self.cfg.gesto_inferior_horizontal,
        };
        self.enviar(&pet);
        self.ui.actualizar_gestos_activos(self.cfg.activo);
        self.asegurar_timer_animacion();
    }

    fn recargar_config(&mut self) {
        self.cfg = config::cargar();
        self.enviar_config();
        tracing::info!(activo = self.cfg.activo, "configuración de bookos-extras recargada");
    }

    /// Arranca el reloj de animación (~120 Hz) SOLO mientras haya algún muelle activo.
    /// Al asentarse, se apaga por completo (HIG §16.3: reposo = cero trabajo).
    fn asegurar_timer_animacion(&mut self) {
        if self.timer_anim_activo || !self.ui.animando() {
            return;
        }
        self.timer_anim_activo = true;
        let _ = self
            .handle
            .insert_source(Timer::from_duration(Duration::from_millis(8)), |_, _, app| {
                app.ui.tick_animaciones();
                if app.ui.animando() {
                    TimeoutAction::ToDuration(Duration::from_millis(8))
                } else {
                    app.timer_anim_activo = false;
                    TimeoutAction::Drop
                }
            });
    }

    fn procesar_evento(&mut self, evento: Evento) {
        let salida = match evento {
            Evento::Pantalla {
                ancho,
                alto,
                escala,
                oscuro,
                efectos_reducidos,
            } => {
                self.ui
                    .actualizar_pantalla(ancho, alto, escala, oscuro, efectos_reducidos);
                return;
            }
            Evento::Notificaciones { lista, no_molestar } => {
                self.ui.actualizar_notificaciones(lista, no_molestar);
                return;
            }
            Evento::ToqueFuera => {
                self.ui.cerrar_cortina_animada(-1.8);
                self.asegurar_timer_animacion();
                return;
            }
            Evento::ToqueInicio {
                slot,
                x,
                y,
                ancho,
                alto,
                borde,
                panel_visible,
                exposicion_abierta,
                tiempo,
            } => self.motor.inicio(
                &self.cfg,
                slot,
                x,
                y,
                ancho,
                alto,
                borde,
                panel_visible,
                exposicion_abierta,
                tiempo,
            ),
            Evento::ToqueMueve {
                slot,
                x,
                y,
                tiempo,
            } => self.motor.mueve(slot, x, y, tiempo),
            Evento::ToqueFin { slot, tiempo } => self.motor.fin(slot, tiempo, false),
            Evento::ToqueCancelar { slot, tiempo } => self.motor.fin(slot, tiempo, true),
        };

        self.aplicar_salida(salida);
    }

    fn aplicar_salida(&mut self, salida: crate::gestos::SalidaGesto) {
        for pet in salida.peticiones {
            self.enviar(&pet);
        }
        for ef in salida.locales {
            match ef {
                EfectoLocal::CortinaProgreso(p) => self.ui.cortina_progreso(&self.qh, p),
                EfectoLocal::CortinaSoltar {
                    progreso,
                    velocidad,
                    cancelada,
                } => self.ui.cortina_soltar(&self.qh, progreso, velocidad, cancelada),
                EfectoLocal::IndicadorFlotante {
                    lado,
                    y,
                    desplazamiento_x,
                    progreso,
                    armado,
                } => self
                    .ui
                    .indicador_flotante(&self.qh, lado, y, desplazamiento_x, progreso, armado),
                EfectoLocal::IndicadorSoltar { disparado } => self.ui.indicador_soltar(disparado),
            }
        }
        self.asegurar_timer_animacion();
    }

    fn drenar_acciones_ui(&mut self) {
        while let Some(accion) = self.ui.acciones_pendientes.pop_front() {
            match accion {
                AccionUi::AlternarGestos => {
                    self.cfg.activo = !self.cfg.activo;
                    let _ = config::guardar(&self.cfg);
                    self.enviar_config();
                }
                AccionUi::AlternarNoMolestar => {
                    self.enviar(&Peticion::AlternarNoMolestar);
                }
                AccionUi::AbrirAjustesNotificaciones => {
                    self.ui.cerrar_cortina_animada(-1.8);
                    self.enviar(&Peticion::AbrirAjustesNotificaciones);
                }
                AccionUi::CerrarNotificacion(id) => {
                    self.enviar(&Peticion::CerrarNotificacion { id });
                }
                AccionUi::LimpiarNotificaciones => {
                    self.enviar(&Peticion::LimpiarNotificaciones);
                }
                AccionUi::InvocarAccion(id, clave) => {
                    if clave == "default" || clave == "inline-reply" {
                        self.ui.cerrar_cortina_animada(-1.8);
                    }
                    self.enviar(&Peticion::InvocarAccionNotificacion { id, clave });
                }
            }
        }
        self.asegurar_timer_animacion();
    }
}

fn conectar_compositor(loop_handle: &LoopHandle<'static, App>, app: &mut App) -> bool {
    let Some(ruta) = protocolo::ruta_socket() else {
        return false;
    };
    let Ok(stream) = UnixStream::connect(&ruta) else {
        return false;
    };
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let Ok(escritor) = stream.try_clone() else {
        return false;
    };
    let fuente = Generic::new(stream, Interest::READ, Mode::Level);
    let mut pendiente = Vec::<u8>::new();
    let res = loop_handle.insert_source(fuente, move |_, stream, app| {
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
                            if !texto.is_empty()
                                && let Ok(ev) = serde_json::from_str::<Evento>(texto)
                            {
                                app.procesar_evento(ev);
                            }
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    cerrar = true;
                    break;
                }
            }
        }
        if cerrar {
            app.desconectar(false);
            tracing::info!("desconectado de bookos-comp; esperando reconexión");
            Ok(PostAction::Remove)
        } else {
            Ok(PostAction::Continue)
        }
    });
    let Ok(read_token) = res else {
        return false;
    };
    app.read_token = Some(read_token);
    app.escritor = Some(escritor);
    app.cola.limpiar();
    app.conectado = true;
    app.enviar_config();
    app.enviar(&Peticion::PedirNotificaciones);
    tracing::info!(ruta = %ruta.display(), "conectado a bookos-comp");
    true
}

fn ejecutar_demonio() -> anyhow::Result<()> {
    // SAFETY: manejador simple que solo escribe un AtomicBool.
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            manejar_sigusr1 as *const () as libc::sighandler_t,
        );
    }
    let _ = std::fs::write(config::ruta_pid(), std::process::id().to_string());

    let cfg = config::cargar();
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<UiState>(&conn)?;
    let qh = event_queue.handle();
    let ui = UiState::nuevo(&globals, &qh, cfg.activo)?;

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    handle
        .insert_source(WaylandSource::new(conn, event_queue), |_, queue, app| {
            queue.dispatch_pending(&mut app.ui)
        })
        .map_err(|e| anyhow::anyhow!("WaylandSource: {e}"))?;

    let mut app = App {
        cfg,
        motor: MotorGestos::default(),
        ui,
        qh,
        handle: handle.clone(),
        escritor: None,
        cola: ColaEscritura::default(),
        read_token: None,
        write_token: None,
        conectado: false,
        timer_anim_activo: false,
    };

    conectar_compositor(&handle, &mut app);

    let handle_recon = handle.clone();
    handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(750)),
            move |_, _, app| {
                if RECARGAR.swap(false, Ordering::SeqCst) {
                    app.recargar_config();
                }
                if !app.conectado {
                    conectar_compositor(&handle_recon, app);
                }
                TimeoutAction::ToDuration(Duration::from_secs(1))
            },
        )
        .map_err(|e| anyhow::anyhow!("Timer: {e}"))?;

    loop {
        event_loop.dispatch(None, &mut app)?;
        if RECARGAR.swap(false, Ordering::SeqCst) {
            app.recargar_config();
        }
        app.drenar_acciones_ui();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cola_escritura_reintenta_tras_would_block_y_parcial() {
        struct Escritor {
            salida: Vec<u8>,
            paso: usize,
        }
        impl Write for Escritor {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.paso += 1;
                match self.paso {
                    1 => {
                        let n = buf.len().min(5);
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
        cola.encolar(b"{\"tipo\":\"atras\"}\n").unwrap();
        let mut escritor = Escritor {
            salida: Vec::new(),
            paso: 0,
        };
        assert!(!cola.drenar(&mut escritor).unwrap());
        cola.encolar(b"{\"tipo\":\"adelante\"}\n").unwrap();
        assert!(cola.drenar(&mut escritor).unwrap());
        assert_eq!(
            String::from_utf8(escritor.salida).unwrap(),
            "{\"tipo\":\"atras\"}\n{\"tipo\":\"adelante\"}\n"
        );
    }
}
