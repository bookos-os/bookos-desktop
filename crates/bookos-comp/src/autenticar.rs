//! Autenticación PAM para salir del bloqueo.
//!
//! PAM se ejecuta en un worker: una contraseña incorrecta tarda segundos a
//! propósito y nunca puede bloquear el bucle de frames. `libpam.so.0` se carga
//! dinámicamente para compilar sin exigir las cabeceras de desarrollo.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_CONV_ERR: c_int = 19;
/// `pam_set_item` con esto fija la función de espera tras un fallo.
const PAM_FAIL_DELAY: c_int = 10;
/// «No puedo recoger la información de autenticación». Es lo que contesta
/// `pam_fprintd` en una máquina **sin lector de huellas**, y no es lo mismo que
/// un rechazo: no hay nada que rechazar. Distinguirlo importa porque tratarlo
/// como fallo hacía que el bloqueo se abriera anunciando «no se pudo verificar
/// la huella» en un portátil que nunca ha tenido sensor.
const PAM_AUTHINFO_UNAVAIL: c_int = 9;

/// Qué contestó PAM, con el único matiz que cambia lo que hay que enseñar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Veredicto {
    Correcto,
    /// La identidad no cuela: contraseña incorrecta, o huella que no casa.
    Rechazado,
    /// No se pudo ni preguntar: no hay lector, o el servicio no está.
    NoDisponible,
}

impl Veredicto {
    // 0.1.0
    pub fn correcto(self) -> bool {
        self == Self::Correcto
    }
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type Conversar = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<Conversar>,
    appdata_ptr: *mut c_void,
}

struct Secretos {
    usuario: CString,
    contrasena: CString,
}

/// Sobrescribe a ceros y se asegura de que el compilador no lo tire.
///
/// La contraseña pasa por varias copias antes de llegar a PAM, y el compositor
/// es un proceso que vive toda la sesión y puede acabar en un volcado: lo que no
/// se borre queda ahí legible. `write_volatile` es lo que impide que se optimice
/// el bucle, porque escribir algo que nadie vuelve a leer es justo lo que un
/// optimizador está deseando quitar.
// 0.1.0
pub(crate) fn borrar(bytes: &mut [u8]) {
    for b in bytes {
        // SAFETY: el puntero sale de una referencia mutable viva y válida.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

pub struct Comprobacion {
    resultado: Receiver<Veredicto>,
}

/// Eventos que emite la vigilancia continua del lector de huellas hacia el bucle
/// principal del compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventoHuella {
    /// El lector está armado y esperando que el usuario coloque el dedo.
    Listo,
    /// La huella ha coincidido con una registrada del usuario.
    Coincide,
    /// El dedo colocado no coincide con ninguna huella registrada.
    NoCoincide,
    /// Lectura incompleta (dedo movido rápido, descentrado…); el lector sigue activo.
    ReintentarLectura,
    /// Varios fallos consecutivos: pausa breve antes de rearmar automáticamente.
    Enfriamiento,
    /// No hay lector o el usuario no tiene huellas registradas.
    SinLector,
}

type ActivaHuella = Option<(zbus::blocking::Connection, Option<zbus::zvariant::OwnedObjectPath>)>;

struct EstadoCompartidoHuella {
    cancelado: AtomicBool,
    despertar: (Mutex<bool>, Condvar),
    fin: (Mutex<bool>, Condvar),
    activa: Mutex<ActivaHuella>,
}

/// Vigilancia pasiva del lector de huellas sobre D-Bus (`net.reactivated.Fprint`).
///
/// A diferencia de `pam_fprintd` con `timeout=15`, `VerifyStart("any")` espera
/// sin límite de tiempo a que el usuario coloque el dedo (incluso tras minutos u
/// horas con la pantalla apagada), se cancela al instante al desbloquear con
/// contraseña y se rearma solo ante desconexiones o errores transitorios.
pub struct VigilanciaHuella {
    compartido: Arc<EstadoCompartidoHuella>,
}

impl Drop for VigilanciaHuella {
    // 0.2.3
    fn drop(&mut self) {
        self.cancelar();
    }
}

impl VigilanciaHuella {
    /// Arranca la escucha continua del lector para `usuario` y reenvía los
    /// eventos por `canal` etiquetados con `generacion`.
    // 0.2.3
    pub fn iniciar(
        usuario: &str,
        generacion: u64,
        canal: smithay::reexports::calloop::channel::Sender<(u64, EventoHuella)>,
    ) -> Option<Self> {
        if usuario.is_empty() {
            return None;
        }
        let compartido = Arc::new(EstadoCompartidoHuella {
            cancelado: AtomicBool::new(false),
            despertar: (Mutex::new(false), Condvar::new()),
            fin: (Mutex::new(false), Condvar::new()),
            activa: Mutex::new(None),
        });
        let hilo_compartido = Arc::clone(&compartido);
        let usuario = usuario.to_string();
        std::thread::Builder::new()
            .name("bookos-fingerprint".into())
            .spawn(move || vigilar_huella(&usuario, generacion, &canal, &hilo_compartido))
            .ok()?;
        Some(Self { compartido })
    }

    /// Cancela la espera en curso y libera el lector sin bloquear el hilo del
    /// compositor.
    // 0.2.3
    pub fn cancelar(&self) {
        if self.compartido.cancelado.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let (lock, cv) = &self.compartido.despertar;
            if let Ok(mut listo) = lock.lock() {
                *listo = true;
                cv.notify_all();
            }
        }
        let tomado = self
            .compartido
            .activa
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some((conexion, ruta)) = tomado {
            let compartido = Arc::clone(&self.compartido);
            let _ = std::thread::Builder::new()
                .name("bookos-huella-fin".into())
                .spawn(move || {
                    if let Some(ruta) = ruta {
                        let conn_stop = conexion.clone();
                        let _ = std::thread::Builder::new()
                            .name("bookos-huella-stop".into())
                            .spawn(move || {
                                if let Ok(proxy) = zbus::blocking::Proxy::new(
                                    &conn_stop,
                                    "net.reactivated.Fprint",
                                    ruta,
                                    "net.reactivated.Fprint.Device",
                                ) {
                                    let _ = proxy.call::<_, _, ()>("VerifyStop", &());
                                }
                            });
                    }
                    esperar_fin(&compartido.fin, std::time::Duration::from_millis(400));
                    let _ = conexion.close();
                });
        }
    }

    /// Cancela la vigilancia y espera hasta `limite` a que el hilo libere
    /// el lector `fprintd` (pensado para el hilo de `PrepareForSleep` antes
    /// de soltar el inhibidor de retardo al suspender).
    // añadido en la 0.2.3
    pub fn detener_y_esperar(self, limite: std::time::Duration) {
        self.cancelar();
        esperar_fin(&self.compartido.fin, limite);
    }
}

fn esperar_fin(fin: &(Mutex<bool>, Condvar), limite: std::time::Duration) {
    let (lock, cv) = fin;
    if let Ok(mut terminado) = lock.lock() {
        let inicio = std::time::Instant::now();
        while !*terminado {
            let restante = limite.saturating_sub(inicio.elapsed());
            if restante.is_zero() {
                break;
            }
            match cv.wait_timeout(terminado, restante) {
                Ok((guard, res)) => {
                    terminado = guard;
                    if res.timed_out() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }
}

struct FinGuard(Arc<EstadoCompartidoHuella>);

impl Drop for FinGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.0.activa.lock() {
            *g = None;
        }
        let (lock, cv) = &self.0.fin;
        if let Ok(mut terminado) = lock.lock() {
            *terminado = true;
            cv.notify_all();
        }
    }
}

fn dormir_cancelable(
    compartido: &EstadoCompartidoHuella,
    duracion: std::time::Duration,
) -> bool {
    if compartido.cancelado.load(Ordering::Acquire) {
        return false;
    }
    let (lock, cv) = &compartido.despertar;
    let Ok(mut despertado) = lock.lock() else {
        return !compartido.cancelado.load(Ordering::Acquire);
    };
    let inicio = std::time::Instant::now();
    while !*despertado && !compartido.cancelado.load(Ordering::Acquire) {
        let restante = duracion.saturating_sub(inicio.elapsed());
        if restante.is_zero() {
            break;
        }
        match cv.wait_timeout(despertado, restante) {
            Ok((guard, res)) => {
                despertado = guard;
                if res.timed_out() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    !compartido.cancelado.load(Ordering::Acquire)
}

fn nombre_error_dbus(err: &zbus::Error) -> Option<&str> {
    match err {
        zbus::Error::MethodError(nombre, _, _) => Some(nombre.as_str()),
        _ => None,
    }
}

fn es_error_fprint(err: &zbus::Error, sufijo: &str) -> bool {
    nombre_error_dbus(err)
        .and_then(|n| n.strip_prefix("net.reactivated.Fprint.Error."))
        == Some(sufijo)
}

fn es_servicio_ausente(err: &zbus::Error) -> bool {
    matches!(
        nombre_error_dbus(err),
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
                | "org.freedesktop.DBus.Error.Spawn.FileInvalid"
        )
    )
}

fn soltar_lector(lector: &zbus::blocking::Proxy<'_>) {
    let _ = lector.call::<_, _, ()>("VerifyStop", &());
    let _ = lector.call::<_, _, ()>("Release", &());
}

enum ResultadoSesionHuella {
    Salir,
    SinLector,
    Reintentar(std::time::Duration),
}

fn vigilar_huella(
    usuario: &str,
    generacion: u64,
    canal: &smithay::reexports::calloop::channel::Sender<(u64, EventoHuella)>,
    compartido: &Arc<EstadoCompartidoHuella>,
) {
    let _fin = FinGuard(Arc::clone(compartido));
    let mut sin_dispositivo = 0_u32;
    let mut errores_seguidos = 0_u32;
    let mut rondas_fallidas = 0_u32;

    while !compartido.cancelado.load(Ordering::Acquire) {
        let desde = std::time::Instant::now();
        match ejecutar_sesion_huella(
            usuario,
            generacion,
            canal,
            compartido,
            &mut rondas_fallidas,
        ) {
            ResultadoSesionHuella::Salir => return,
            ResultadoSesionHuella::SinLector => {
                sin_dispositivo = sin_dispositivo.saturating_add(1);
                // Al despertar de suspender, el lector USB puede tardar ~1 s
                // en reaparecer: dos comprobaciones silenciosas evitan darlo
                // por ausente antes de tiempo.
                if sin_dispositivo <= 2 {
                    let espera = std::time::Duration::from_millis(500 * u64::from(sin_dispositivo));
                    if !dormir_cancelable(compartido, espera) {
                        return;
                    }
                    continue;
                }
                let _ = canal.send((generacion, EventoHuella::SinLector));
                return;
            }
            ResultadoSesionHuella::Reintentar(sugerida) => {
                sin_dispositivo = 0;
                if desde.elapsed() >= std::time::Duration::from_secs(5) {
                    errores_seguidos = 0;
                } else {
                    errores_seguidos = errores_seguidos.saturating_add(1);
                }
                let espera = if !sugerida.is_zero() {
                    sugerida
                } else {
                    match errores_seguidos {
                        0 | 1 => std::time::Duration::from_millis(300),
                        2 => std::time::Duration::from_secs(1),
                        3 => std::time::Duration::from_secs(2),
                        _ => std::time::Duration::from_secs(5),
                    }
                };
                if !dormir_cancelable(compartido, espera) {
                    return;
                }
            }
        }
    }
}

fn ejecutar_sesion_huella(
    usuario: &str,
    generacion: u64,
    canal: &smithay::reexports::calloop::channel::Sender<(u64, EventoHuella)>,
    compartido: &EstadoCompartidoHuella,
    rondas_fallidas: &mut u32,
) -> ResultadoSesionHuella {
    const MAX_INTENTOS: u32 = 3;

    let Ok(conexion) = zbus::blocking::Connection::system() else {
        return ResultadoSesionHuella::Reintentar(std::time::Duration::from_secs(2));
    };
    if let Ok(mut g) = compartido.activa.lock() {
        *g = Some((conexion.clone(), None));
    }
    if compartido.cancelado.load(Ordering::Acquire) {
        return ResultadoSesionHuella::Salir;
    }

    let gestor = match zbus::blocking::Proxy::new(
        &conexion,
        "net.reactivated.Fprint",
        "/net/reactivated/Fprint/Manager",
        "net.reactivated.Fprint.Manager",
    ) {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!("no se pudo crear el proxy de fprintd: {err}");
            return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
        }
    };

    let ruta: zbus::zvariant::OwnedObjectPath = match gestor.call("GetDefaultDevice", &()) {
        Ok(ruta) => ruta,
        Err(err) if es_error_fprint(&err, "NoSuchDevice") => {
            return ResultadoSesionHuella::SinLector;
        }
        Err(err) if es_servicio_ausente(&err) => {
            tracing::info!("fprintd no está disponible en el sistema: {err}");
            let _ = canal.send((generacion, EventoHuella::SinLector));
            return ResultadoSesionHuella::Salir;
        }
        Err(err) => {
            tracing::warn!("GetDefaultDevice de fprintd falló: {err}");
            return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
        }
    };

    let lector = match zbus::blocking::Proxy::new(
        &conexion,
        "net.reactivated.Fprint",
        ruta.clone(),
        "net.reactivated.Fprint.Device",
    ) {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!("no se pudo abrir el dispositivo de huellas: {err}");
            return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
        }
    };

    match lector.call::<_, _, Vec<String>>("ListEnrolledFingers", &(usuario,)) {
        Ok(dedos) if dedos.is_empty() => {
            let _ = canal.send((generacion, EventoHuella::SinLector));
            return ResultadoSesionHuella::Salir;
        }
        Ok(_) => {}
        Err(err) if es_error_fprint(&err, "NoEnrolledPrints") => {
            let _ = canal.send((generacion, EventoHuella::SinLector));
            return ResultadoSesionHuella::Salir;
        }
        Err(err) if es_error_fprint(&err, "NoSuchDevice") => {
            return ResultadoSesionHuella::SinLector;
        }
        Err(err) => {
            tracing::debug!("ListEnrolledFingers falló ({err}); se intenta Claim");
        }
    }

    if let Err(err) = lector.call::<_, _, ()>("Claim", &(usuario,)) {
        if es_error_fprint(&err, "NoSuchDevice") {
            return ResultadoSesionHuella::SinLector;
        }
        if es_error_fprint(&err, "AlreadyInUse") || es_error_fprint(&err, "ClaimDevice") {
            tracing::debug!("lector de huellas ocupado al reclamarlo; se reintentará");
            return ResultadoSesionHuella::Reintentar(std::time::Duration::from_millis(300));
        }
        tracing::warn!("Claim del lector de huellas falló: {err}");
        return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
    }

    if let Ok(mut g) = compartido.activa.lock() {
        *g = Some((conexion.clone(), Some(ruta.clone())));
    }
    if compartido.cancelado.load(Ordering::Acquire) {
        soltar_lector(&lector);
        return ResultadoSesionHuella::Salir;
    }

    // Suscripción previa a VerifyStart: con lectores rápidos (egismoc ~53 ms)
    // la señal podría llegar antes de suscribirse después.
    let mensajes = zbus::blocking::MessageIterator::from(&conexion);
    if let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conexion) {
        if let Ok(regla_verify) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("net.reactivated.Fprint")
            .and_then(|b| b.interface("net.reactivated.Fprint.Device"))
            .and_then(|b| b.member("VerifyStatus"))
            .and_then(|b| b.path(ruta.as_ref()))
            .map(|b| b.build())
        {
            let _ = dbus.add_match_rule(regla_verify.into());
        }
        if let Ok(regla_dueno) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|b| b.interface("org.freedesktop.DBus"))
            .and_then(|b| b.member("NameOwnerChanged"))
            .and_then(|b| b.arg(0, "net.reactivated.Fprint"))
            .map(|b| b.build())
        {
            let _ = dbus.add_match_rule(regla_dueno.into());
        }
    }

    if let Err(err) = lector.call::<_, _, ()>("VerifyStart", &("any",)) {
        soltar_lector(&lector);
        if es_error_fprint(&err, "NoEnrolledPrints") {
            let _ = canal.send((generacion, EventoHuella::SinLector));
            return ResultadoSesionHuella::Salir;
        }
        if es_error_fprint(&err, "NoSuchDevice") {
            return ResultadoSesionHuella::SinLector;
        }
        tracing::warn!("VerifyStart falló: {err}");
        return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
    }

    let _ = canal.send((generacion, EventoHuella::Listo));
    let mut fallos = 0_u32;

    for mensaje in mensajes {
        if compartido.cancelado.load(Ordering::Acquire) {
            soltar_lector(&lector);
            return ResultadoSesionHuella::Salir;
        }
        let Ok(msg) = mensaje else {
            break;
        };
        if msg.message_type() != zbus::message::Type::Signal {
            continue;
        }
        let header = msg.header();
        let interfaz = header.interface().map(|i| i.as_str());
        let miembro = header.member().map(|m| m.as_str());
        let ruta_msg = header.path().map(|p| p.as_str());

        if interfaz == Some("org.freedesktop.DBus") && miembro == Some("NameOwnerChanged") {
            if let Ok((nombre, anterior, nuevo)) =
                msg.body().deserialize::<(String, String, String)>()
                && nombre == "net.reactivated.Fprint"
                && (nuevo.is_empty() || nuevo != anterior)
            {
                tracing::warn!("fprintd reinició o desapareció del bus; se rearmará el lector");
                soltar_lector(&lector);
                return ResultadoSesionHuella::Reintentar(std::time::Duration::from_millis(500));
            }
            continue;
        }

        if interfaz != Some("net.reactivated.Fprint.Device")
            || miembro != Some("VerifyStatus")
            || ruta_msg != Some(ruta.as_str())
        {
            continue;
        }

        let Ok((que, terminado)) = msg.body().deserialize::<(String, bool)>() else {
            continue;
        };

        match que.as_str() {
            "verify-match" => {
                *rondas_fallidas = 0;
                soltar_lector(&lector);
                let _ = canal.send((generacion, EventoHuella::Coincide));
                return ResultadoSesionHuella::Salir;
            }
            "verify-no-match" => {
                fallos += 1;
                let _ = canal.send((generacion, EventoHuella::NoCoincide));
                if fallos >= MAX_INTENTOS {
                    soltar_lector(&lector);
                    *rondas_fallidas = rondas_fallidas.saturating_add(1);
                    let enfriamiento = match *rondas_fallidas {
                        1 => std::time::Duration::from_secs(3),
                        2 => std::time::Duration::from_secs(6),
                        3 => std::time::Duration::from_secs(15),
                        _ => std::time::Duration::from_secs(30),
                    };
                    let _ = canal.send((generacion, EventoHuella::Enfriamiento));
                    return ResultadoSesionHuella::Reintentar(enfriamiento);
                }
                if terminado {
                    let _ = lector.call::<_, _, ()>("VerifyStop", &());
                    if let Err(err) = lector.call::<_, _, ()>("VerifyStart", &("any",)) {
                        tracing::warn!("no se pudo reanudar VerifyStart tras no coincidir: {err}");
                        soltar_lector(&lector);
                        return ResultadoSesionHuella::Reintentar(std::time::Duration::ZERO);
                    }
                }
            }
            "verify-retry-scan"
            | "verify-swipe-too-short"
            | "verify-finger-not-centered"
            | "verify-remove-and-retry"
            | "verify-too-fast"
                if !terminado =>
            {
                let _ = canal.send((generacion, EventoHuella::ReintentarLectura));
            }
            _ if !terminado => {}
            other => {
                tracing::warn!(estado = other, "VerifyStatus terminal inesperado; se rearmará el lector");
                soltar_lector(&lector);
                return ResultadoSesionHuella::Reintentar(std::time::Duration::from_millis(500));
            }
        }
    }

    soltar_lector(&lector);
    if compartido.cancelado.load(Ordering::Acquire) {
        ResultadoSesionHuella::Salir
    } else {
        ResultadoSesionHuella::Reintentar(std::time::Duration::from_millis(500))
    }
}

impl Comprobacion {
    // 0.1.0
    pub fn lanzar(usuario: &str, contrasena: &str) -> Option<Self> {
        let usuario = usuario.to_string();
        // En `Vec<u8>` y no en `String` porque hay que poder ponerlo a ceros al
        // terminar, y borrar un `String` por dentro obligaría a tocar sus bytes
        // saltándose la garantía de UTF-8.
        let mut contrasena = contrasena.as_bytes().to_vec();
        let (envio, resultado) = mpsc::sync_channel(1);
        let hilo = std::thread::Builder::new()
            .name("bookos-pam".into())
            .spawn(move || {
                let vale = autenticar(&usuario, &contrasena);
                borrar(&mut contrasena);
                let _ = envio.send(vale);
            });
        if hilo.is_err() {
            return None;
        }
        Some(Self { resultado })
    }

    // 0.1.0
    pub fn resultado(&mut self) -> Option<Veredicto> {
        match self.resultado.try_recv() {
            Ok(vale) => Some(vale),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Veredicto::Rechazado),
        }
    }
}

// 0.1.0
pub fn usuario() -> String {
    // La identidad autenticada es el UID de la sesión, nunca una variable
    // de entorno ni root como reserva. La variante reentrante sirve también
    // cuando hay trabajadores PAM activos.
    let mut entrada: libc::passwd = unsafe { std::mem::zeroed() };
    let mut resultado = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 65536];
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entrada,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut resultado,
        )
    };
    if status != 0 || resultado.is_null() || entrada.pw_name.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(entrada.pw_name) }
        .to_string_lossy()
        .into_owned()
}

// 0.1.0
fn servicio() -> &'static str {
    if std::path::Path::new("/etc/pam.d/bookos").is_file() {
        "bookos"
    } else {
        // Compatibilidad con instalaciones anteriores. Estas pilas generales
        // pueden incluir pam_fprintd y demorar el intento; el instalador nuevo
        // crea el servicio `bookos`, exclusivo para la contraseña.
        ["system-auth", "common-auth", "login"]
            .into_iter()
            .find(|nombre| std::path::Path::new("/etc/pam.d").join(nombre).is_file())
            .unwrap_or("login")
    }
}

// 0.1.0
// modified 0.1.1 -- acepta el PIN antes de preguntar a PAM
fn autenticar(usuario: &str, contrasena: &[u8]) -> Veredicto {
    if pin_correcto(usuario, contrasena) {
        return Veredicto::Correcto;
    }
    autenticar_servicio(servicio(), usuario, contrasena)
}

/// El PIN que pone Ajustes en `/etc/bookos/pin/<usuario>`: hash SHA-512 crypt,
/// de root y legible por el grupo del usuario. Va fuera de PAM a propósito:
/// en la pila `bookos` haría falta `pam_exec` y un ayudante instalado, y el PIN
/// solo debe abrir el bloqueo, no valer para sudo ni polkit. Lo que no son
/// cifras ni se mira: una contraseña normal va directa a PAM.
// 0.1.1
fn pin_correcto(usuario: &str, intento: &[u8]) -> bool {
    if intento.is_empty() || !intento.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Ok(guardado) = std::fs::read_to_string(format!("/etc/bookos/pin/{usuario}")) else {
        return false;
    };
    pin_casa(guardado.trim(), intento)
}

// 0.1.1
fn pin_casa(guardado: &str, intento: &[u8]) -> bool {
    let (Ok(hash), Ok(frase)) = (CString::new(guardado), CString::new(intento)) else {
        return false;
    };
    // libxcrypt: `.so.2` en Fedora, `.so.1` (compat) en Arch y Debian.
    // `crypt_ra` y no `crypt` porque este hilo puede coincidir con el de la
    // huella y `crypt` devuelve un búfer estático compartido.
    let Some(lib) = ["libcrypt.so.2", "libcrypt.so.1"]
        .into_iter()
        .find_map(|n| unsafe { libloading::Library::new(n) }.ok())
    else {
        tracing::error!("no se pudo cargar libcrypt para comprobar el PIN");
        return false;
    };
    type CryptRa = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *mut *mut c_void,
        *mut c_int,
    ) -> *const c_char;
    // SAFETY: firma de `crypt_ra` en crypt.h de libxcrypt; `datos` lo reserva
    // la biblioteca y se libera con `free` cuando ya no se lee el resultado.
    unsafe {
        let Ok(crypt_ra) = lib.get::<CryptRa>(b"crypt_ra\0") else {
            return false;
        };
        let mut datos: *mut c_void = std::ptr::null_mut();
        let mut tamano: c_int = 0;
        let r = crypt_ra(frase.as_ptr(), hash.as_ptr(), &mut datos, &mut tamano);
        // Un fallo devuelve NULL o una cadena que empieza por «*», que nunca
        // coincide con un hash `$6$`.
        let vale = !r.is_null() && CStr::from_ptr(r) == hash.as_c_str();
        libc::free(datos);
        borrar(&mut frase.into_bytes());
        vale
    }
}

// 0.1.0
fn autenticar_servicio(nombre: &str, usuario: &str, contrasena: &[u8]) -> Veredicto {
    if usuario.is_empty() {
        return Veredicto::Rechazado;
    }
    let Ok(usuario) = CString::new(usuario) else {
        return Veredicto::Rechazado;
    };
    let Ok(contrasena) = CString::new(contrasena) else {
        return Veredicto::Rechazado;
    };
    let Ok(servicio) = CString::new(nombre) else {
        return Veredicto::Rechazado;
    };

    // SAFETY: la biblioteca permanece viva hasta después de `pam_end`; las
    // firmas son las de la ABI estable de PAM.
    unsafe {
        let Ok(pam) = libloading::Library::new("libpam.so.0") else {
            tracing::error!("no se pudo cargar libpam.so.0");
            return Veredicto::NoDisponible;
        };
        type Start = unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const PamConv,
            *mut *mut c_void,
        ) -> c_int;
        type Authenticate = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type Account = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type End = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type SetItem = unsafe extern "C" fn(*mut c_void, c_int, *const c_void) -> c_int;
        type StrError = unsafe extern "C" fn(*mut c_void, c_int) -> *const c_char;
        let Ok(start) = pam.get::<Start>(b"pam_start\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(authenticate) = pam.get::<Authenticate>(b"pam_authenticate\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(account) = pam.get::<Account>(b"pam_acct_mgmt\0") else {
            return Veredicto::NoDisponible;
        };
        let Ok(end) = pam.get::<End>(b"pam_end\0") else {
            return Veredicto::NoDisponible;
        };
        // Para poder decir **por qué** falló. Sin esto, todos los caminos de
        // error son el mismo `false` y «el bloqueo no me deja entrar» no se
        // puede distinguir de «la contraseña estaba mal»: no hay nada en el
        // registro que separe una cosa de la otra.
        let porque = pam.get::<StrError>(b"pam_strerror\0").ok();
        let motivo = |handle: *mut c_void, codigo: c_int| -> String {
            match &porque {
                Some(f) => CStr::from_ptr(f(handle, codigo))
                    .to_string_lossy()
                    .into_owned(),
                None => format!("código {codigo}"),
            }
        };

        let mut secretos = Secretos {
            usuario,
            contrasena,
        };
        let conversacion = PamConv {
            conv: Some(conversar),
            appdata_ptr: (&mut secretos as *mut Secretos).cast(),
        };
        let mut handle: *mut c_void = std::ptr::null_mut();
        let inicio = start(
            servicio.as_ptr(),
            secretos.usuario.as_ptr(),
            &conversacion,
            &mut handle,
        );
        if inicio != PAM_SUCCESS || handle.is_null() {
            tracing::error!(
                servicio = nombre,
                "pam_start falló: {}",
                motivo(handle, inicio)
            );
            return Veredicto::NoDisponible;
        }
        // Medido con contraseña errónea en este portátil: 1,7-2,2 s por
        // intento, de los que la comprobación real (unix_chkpwd + yescrypt)
        // son 41-55 ms. El resto es la espera que pide `pam_unix` antes de
        // contestar un fallo. Con PAM_FAIL_DELAY la espera la gestiona la
        // aplicación, que es para lo que existe: el bloqueo ya tiene su propia
        // demora creciente (`demora_reintento`), así que esta sobraba y solo
        // hacía lento equivocarse una vez al teclear.
        if let Ok(poner) = pam.get::<SetItem>(b"pam_set_item\0") {
            poner(handle, PAM_FAIL_DELAY, sin_demora as *const c_void);
        }
        // Con la duración en la traza: tras dormir el procesador puede volver
        // a 400 MHz (PL1 de RAPL) y el yescrypt de 50 ms pasa a notarse.
        let desde = std::time::Instant::now();
        let autenticacion = authenticate(handle, 0);
        let resultado = if autenticacion == PAM_SUCCESS {
            account(handle, 0)
        } else {
            autenticacion
        };
        // La contraseña no aparece por ningún lado: lo que se registra es el
        // veredicto de PAM, que es exactamente lo que hace falta para saber si
        // el fallo es «no era la contraseña» o «esto ni siquiera pudo
        // preguntar».
        if resultado == PAM_SUCCESS {
            tracing::info!(
                servicio = nombre,
                ms = desde.elapsed().as_millis() as u64,
                "PAM autenticó"
            );
        } else if autenticacion == PAM_AUTHINFO_UNAVAIL {
            tracing::info!(
                servicio = nombre,
                "PAM no pudo preguntar: {}",
                motivo(handle, autenticacion)
            );
        } else if autenticacion != PAM_SUCCESS {
            tracing::info!(
                servicio = nombre,
                "PAM rechazó: {}",
                motivo(handle, autenticacion)
            );
        } else {
            tracing::warn!(
                servicio = nombre,
                "PAM autenticó pero rechazó la cuenta: {}",
                motivo(handle, resultado)
            );
        }
        let _ = end(handle, resultado);
        // Ya no queda nadie que la mire: ni PAM ni `conversar`. La copia que
        // `conversar` le entregó a PAM la libera PAM por su cuenta y no está en
        // nuestra mano; esta sí.
        let mut bytes = secretos.contrasena.into_bytes_with_nul();
        borrar(&mut bytes);
        match resultado {
            PAM_SUCCESS => Veredicto::Correcto,
            // El único que no es un «no»: significa que no había con qué
            // preguntar. Quien llame decide si eso merece un mensaje o si
            // simplemente se pasa al siguiente método.
            PAM_AUTHINFO_UNAVAIL => Veredicto::NoDisponible,
            _ => Veredicto::Rechazado,
        }
    }
}

/// La función que PAM llama en vez de dormir tras un fallo. No hace nada:
/// la demora la pone el bloqueo.
// 0.1.0
unsafe extern "C" fn sin_demora(
    _estado: c_int,
    _microsegundos: std::ffi::c_uint,
    _datos: *mut c_void,
) {
}

// 0.1.0
unsafe extern "C" fn conversar(
    cuantos: c_int,
    mensajes: *mut *const PamMessage,
    respuestas: *mut *mut PamResponse,
    datos: *mut c_void,
) -> c_int {
    if cuantos <= 0 || mensajes.is_null() || respuestas.is_null() || datos.is_null() {
        return PAM_CONV_ERR;
    }
    let secretos = unsafe { &*(datos.cast::<Secretos>()) };
    let bloque = unsafe {
        libc::calloc(cuantos as usize, std::mem::size_of::<PamResponse>()).cast::<PamResponse>()
    };
    if bloque.is_null() {
        return PAM_CONV_ERR;
    }

    for i in 0..cuantos as usize {
        let mensaje = unsafe { *mensajes.add(i) };
        if mensaje.is_null() {
            liberar_respuestas(bloque, i);
            return PAM_CONV_ERR;
        }
        let estilo = unsafe { (*mensaje).msg_style };
        let texto = match estilo {
            PAM_PROMPT_ECHO_OFF => Some(secretos.contrasena.as_c_str()),
            PAM_PROMPT_ECHO_ON => Some(secretos.usuario.as_c_str()),
            PAM_ERROR_MSG => {
                // El texto viene traducido según el idioma del sistema, así
                // que no se compara: se registra para poder leerlo.
                let texto = unsafe { (*mensaje).msg };
                if !texto.is_null() {
                    tracing::info!(
                        "PAM avisa: {}",
                        unsafe { CStr::from_ptr(texto) }.to_string_lossy()
                    );
                }
                None
            }
            PAM_TEXT_INFO => None,
            _ => {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
        };
        if let Some(texto) = texto {
            let copia = copiar_cadena(texto);
            if copia.is_null() {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
            unsafe { (*bloque.add(i)).resp = copia };
        }
    }
    unsafe { *respuestas = bloque };
    PAM_SUCCESS
}

// 0.1.0
fn copiar_cadena(texto: &CStr) -> *mut c_char {
    let bytes = texto.to_bytes_with_nul();
    let destino = unsafe { libc::malloc(bytes.len()).cast::<c_char>() };
    if !destino.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast(), destino, bytes.len()) };
    }
    destino
}

// 0.1.0
fn liberar_respuestas(respuestas: *mut PamResponse, cuantos: usize) {
    for i in 0..cuantos {
        unsafe { libc::free((*respuestas.add(i)).resp.cast()) };
    }
    unsafe { libc::free(respuestas.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_identidad_ausente_nunca_autentica() {
        assert!(!autenticar_servicio("bookos", "", b"").correcto());
        assert!(!autenticar_servicio("bookos", "usuario\0invalido", b"").correcto());
    }

    #[test]
    fn un_worker_desconectado_falla_cerrado() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut comprobacion = Comprobacion { resultado: rx };
        assert_eq!(comprobacion.resultado(), Some(Veredicto::Rechazado));
    }

    #[test]
    fn vigilancia_huella_rechaza_usuario_vacio() {
        let (tx, _rx) = smithay::reexports::calloop::channel::channel();
        assert!(VigilanciaHuella::iniciar("", 1, tx).is_none());
    }

    #[test]
    fn dormir_cancelable_despierta_al_instant_al_cancelar() {
        let compartido = Arc::new(EstadoCompartidoHuella {
            cancelado: AtomicBool::new(false),
            despertar: (Mutex::new(false), Condvar::new()),
            fin: (Mutex::new(false), Condvar::new()),
            activa: Mutex::new(None),
        });
        let vigilancia = VigilanciaHuella {
            compartido: Arc::clone(&compartido),
        };
        let inicio = std::time::Instant::now();
        let hilo = std::thread::spawn(move || {
            dormir_cancelable(&compartido, std::time::Duration::from_secs(30))
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        vigilancia.cancelar();
        let seguir = hilo.join().expect("el hilo de espera terminó");
        assert!(!seguir);
        assert!(
            inicio.elapsed() < std::time::Duration::from_secs(2),
            "dormir_cancelable no despertó al cancelar"
        );
    }

    #[test]
    fn detener_y_esperar_vuelve_al_terminar_el_hilo() {
        let compartido = Arc::new(EstadoCompartidoHuella {
            cancelado: AtomicBool::new(false),
            despertar: (Mutex::new(false), Condvar::new()),
            fin: (Mutex::new(false), Condvar::new()),
            activa: Mutex::new(None),
        });
        let vigilancia = VigilanciaHuella {
            compartido: Arc::clone(&compartido),
        };
        let hilo = std::thread::spawn(move || {
            let _fin = FinGuard(Arc::clone(&compartido));
            dormir_cancelable(&compartido, std::time::Duration::from_secs(30));
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        let inicio = std::time::Instant::now();
        vigilancia.detener_y_esperar(std::time::Duration::from_secs(2));
        assert!(
            inicio.elapsed() < std::time::Duration::from_secs(1),
            "detener_y_esperar no retornó al cerrar el worker"
        );
        hilo.join().expect("el worker terminó");
    }

    #[test]
    fn clasifica_errores_fprint_y_dbus() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("net.reactivated.Fprint.Error.NoSuchDevice")
                .unwrap(),
            None,
            zbus::Message::method_call("/", "Ping").unwrap().build(&()).unwrap(),
        );
        assert!(es_error_fprint(&err, "NoSuchDevice"));
        assert!(!es_error_fprint(&err, "AlreadyInUse"));
        assert!(!es_servicio_ausente(&err));

        let err_svc = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.ServiceUnknown")
                .unwrap(),
            None,
            zbus::Message::method_call("/", "Ping").unwrap().build(&()).unwrap(),
        );
        assert!(es_servicio_ausente(&err_svc));
    }

    #[test]
    fn el_pin_casa_con_un_hash_de_openssl() {
        // `echo 1234 | openssl passwd -6 -stdin`, como lo genera Ajustes.
        let hash = "$6$yxDdl7B1K2j0KxRU$/LIFIhW18mW1RvDLzHR0OLfzhfFhs2.DNHnglX4zcNAmop3IVI3CFDC7kFVPHZVpG0YjuaJUp7I2mXdaAXPVU1";
        assert!(pin_casa(hash, b"1234"));
        assert!(!pin_casa(hash, b"1235"));
        assert!(!pin_casa("", b"1234"));
    }

    #[test]
    fn una_contrasena_falsa_no_pasa() {
        let mut c = Comprobacion::lanzar(&usuario(), "no-es-esta-contrasena-12345")
            .expect("se puede crear el worker PAM");
        for _ in 0..100 {
            if let Some(veredicto) = c.resultado() {
                assert!(
                    !veredicto.correcto(),
                    "una contraseña inventada ha pasado PAM"
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("PAM no contestó en cinco segundos");
    }
}
