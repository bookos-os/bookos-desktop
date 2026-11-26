//! Agente de autenticación de Polkit nativo de BookOS.
//!
//! Publica `org.freedesktop.PolicyKit1.AuthenticationAgent` en el bus de
//! sistema y lo registra ante `org.freedesktop.PolicyKit1.Authority` para la
//! sesión activa. Cuando una aplicación pide privilegios (`pkexec`, Flatpak,
//! actualizaciones de aplicaciones, montaje de unidades…), abre el diálogo
//! HIG §9 en `bookos-shell` y conversa con `/usr/lib/polkit-1/polkit-agent-helper-1`
//! en un hilo secundario sin bloquear jamás ni el ejecutor D-Bus de `zbus` ni
//! el bucle de fotogramas del compositor.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, c_char};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use bookos_shell::{IdentidadPolkit, PeticionPolkit, t};
use smithay::reexports::calloop::channel::Sender;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const RUTA_AGENTE: &str = "/org/bookos/PolicyKit1/AuthenticationAgent";
const BUS_POLKIT: &str = "org.freedesktop.PolicyKit1";
const RUTA_AUTORIDAD: &str = "/org/freedesktop/PolicyKit1/Authority";
const INTERFAZ_AUTORIDAD: &str = "org.freedesktop.PolicyKit1.Authority";

const RUTAS_HELPER: [&str; 3] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum ErrorPolkit {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
    Failed(String),
}

pub enum ResultadoAuth {
    Autorizado,
    Cancelado,
    Fallo(String),
}

/// Secreto en memoria borrado con escritura volátil al soltarlo.
struct Secreto(Vec<u8>);

impl Drop for Secreto {
    fn drop(&mut self) {
        crate::autenticar::borrar(&mut self.0);
    }
}

/// Lo que llega por el canal de `calloop` al hilo principal del compositor.
pub enum Aviso {
    Autenticar {
        id: u64,
        cookie: String,
        action_id: String,
        mensaje: String,
        icono: String,
        detalle: Option<String>,
        identidades: Vec<IdentidadPolkit>,
        respuesta: crate::portal::Emisario<ResultadoAuth>,
    },
    Cancelar {
        cookie: String,
    },
    Helper {
        solicitud_id: u64,
        sesion_id: u64,
        evento: EventoHelper,
    },
}

pub enum EventoHelper {
    Prompt { texto: String, oculto: bool },
    Info(String),
    Error(String),
    Exito,
    Fallo(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LineaHelper {
    Prompt { texto: String, oculto: bool },
    Info(String),
    Error(String),
    Exito,
    Fallo,
}

fn parsear_linea_helper(linea: &str) -> Option<LineaHelper> {
    let limpia = linea.trim_end_matches(['\r', '\n']);
    if limpia == "SUCCESS" {
        return Some(LineaHelper::Exito);
    }
    if limpia == "FAILURE" {
        return Some(LineaHelper::Fallo);
    }
    if limpia == "PAM_PROMPT_ECHO_OFF" {
        return Some(LineaHelper::Prompt {
            texto: String::new(),
            oculto: true,
        });
    }
    if let Some(texto) = limpia.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        return Some(LineaHelper::Prompt {
            texto: texto.trim().to_string(),
            oculto: true,
        });
    }
    if limpia == "PAM_PROMPT_ECHO_ON" {
        return Some(LineaHelper::Prompt {
            texto: String::new(),
            oculto: false,
        });
    }
    if let Some(texto) = limpia.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        return Some(LineaHelper::Prompt {
            texto: texto.trim().to_string(),
            oculto: false,
        });
    }
    if let Some(texto) = limpia.strip_prefix("PAM_TEXT_INFO ") {
        let msg = texto.trim();
        if !msg.is_empty() {
            return Some(LineaHelper::Info(msg.to_string()));
        }
    }
    if let Some(texto) = limpia.strip_prefix("PAM_ERROR_MSG ") {
        let msg = texto.trim();
        if !msg.is_empty() {
            return Some(LineaHelper::Error(msg.to_string()));
        }
    }
    None
}

fn usuario_por_uid(uid: u32) -> Option<IdentidadPolkit> {
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0u8; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len(),
            &mut res,
        )
    };
    if rc != 0 || res.is_null() {
        return None;
    }
    // SAFETY: `getpwuid_r` devolvió 0 y `res` apunta a `pwd` con cadenas en `buf`.
    let pwd = unsafe { pwd.assume_init() };
    let usuario = unsafe { CStr::from_ptr(pwd.pw_name) }
        .to_string_lossy()
        .into_owned();
    if usuario.is_empty() {
        return None;
    }
    let completo = if pwd.pw_gecos.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pwd.pw_gecos) }
            .to_string_lossy()
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let etiqueta = if !completo.is_empty() && completo != usuario {
        format!("{completo} ({usuario})")
    } else {
        usuario.clone()
    };
    Some(IdentidadPolkit { usuario, etiqueta })
}

fn identidad_por_nombre(nombre: &str) -> Option<(u32, IdentidadPolkit)> {
    let c_nombre = std::ffi::CString::new(nombre).ok()?;
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0u8; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwnam_r(
            c_nombre.as_ptr(),
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len(),
            &mut res,
        )
    };
    if rc != 0 || res.is_null() {
        return None;
    }
    let pwd = unsafe { pwd.assume_init() };
    usuario_por_uid(pwd.pw_uid).map(|id| (pwd.pw_uid, id))
}

fn miembros_de_grupo(gid: u32) -> Vec<(u32, IdentidadPolkit)> {
    let mut grp = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut buf = vec![0u8; 4096];
    let mut res: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe {
        libc::getgrgid_r(
            gid,
            grp.as_mut_ptr(),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len(),
            &mut res,
        )
    };
    if rc != 0 || res.is_null() {
        return Vec::new();
    }
    let grp = unsafe { grp.assume_init() };
    let mut salida = Vec::new();
    if !grp.gr_mem.is_null() {
        let mut cursor = grp.gr_mem;
        // SAFETY: `gr_mem` es un array acabado en puntero nulo en `buf`.
        unsafe {
            while !(*cursor).is_null() {
                if let Ok(nombre) = CStr::from_ptr(*cursor).to_str()
                    && let Some(par) = identidad_por_nombre(nombre)
                {
                    salida.push(par);
                }
                cursor = cursor.add(1);
            }
        }
    }
    salida
}

fn valor_u32(v: &OwnedValue) -> Option<u32> {
    if let Ok(n) = u32::try_from(v) {
        return Some(n);
    }
    if let Ok(n) = i32::try_from(v) {
        return u32::try_from(n).ok();
    }
    if let Ok(n) = u64::try_from(v) {
        return u32::try_from(n).ok();
    }
    None
}

fn resolver_identidades(
    brutas: &[(String, HashMap<String, OwnedValue>)],
    uid_propio: u32,
) -> Vec<IdentidadPolkit> {
    let mut candidatos = Vec::new();
    for (tipo, props) in brutas {
        match tipo.as_str() {
            "unix-user" => {
                if let Some(uid) = props.get("uid").and_then(valor_u32)
                    && let Some(id) = usuario_por_uid(uid)
                {
                    candidatos.push((uid, id));
                }
            }
            "unix-group" => {
                if let Some(gid) = props.get("gid").and_then(valor_u32) {
                    candidatos.extend(miembros_de_grupo(gid));
                }
            }
            _ => {}
        }
    }
    priorizar_identidades(candidatos, uid_propio)
}

fn priorizar_identidades(
    candidatos: Vec<(u32, IdentidadPolkit)>,
    uid_propio: u32,
) -> Vec<IdentidadPolkit> {
    let mut vistas = HashSet::new();
    let mut primero = Vec::new();
    let mut resto = Vec::new();
    for (uid, id) in candidatos {
        if !vistas.insert(id.usuario.clone()) {
            continue;
        }
        if uid == uid_propio {
            primero.push(id);
        } else {
            resto.push(id);
        }
    }
    primero.extend(resto);
    primero
}

fn extraer_detalle(details: &HashMap<String, String>, action_id: &str) -> Option<String> {
    for clave in ["command_line", "program", "unit", "device"] {
        if let Some(v) = details.get(clave) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    let accion = action_id.trim();
    if accion.is_empty() {
        None
    } else {
        Some(accion.to_string())
    }
}

struct Agente {
    canal: Sender<Aviso>,
    siguiente_id: AtomicU64,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agente {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), ErrorPolkit> {
        let uid_propio = unsafe { libc::geteuid() };
        let identidades = resolver_identidades(&identities, uid_propio);
        if identidades.is_empty() {
            return Err(ErrorPolkit::Failed(
                "No valid identities to authenticate".into(),
            ));
        }
        let mensaje = if message.trim().is_empty() {
            action_id.clone()
        } else {
            message
        };
        let detalle = extraer_detalle(&details, &action_id);
        let id = self.siguiente_id.fetch_add(1, Ordering::Relaxed);
        let (respuesta, espera) = crate::portal::promesa();

        if self
            .canal
            .send(Aviso::Autenticar {
                id,
                cookie,
                action_id,
                mensaje,
                icono: icon_name,
                detalle,
                identidades,
                respuesta,
            })
            .is_err()
        {
            return Err(ErrorPolkit::Failed(
                "BookOS compositor channel is closed".into(),
            ));
        }

        match espera.await {
            Some(ResultadoAuth::Autorizado) => Ok(()),
            Some(ResultadoAuth::Cancelado) | None => {
                Err(ErrorPolkit::Cancelled("Authentication cancelled".into()))
            }
            Some(ResultadoAuth::Fallo(motivo)) => Err(ErrorPolkit::Failed(motivo)),
        }
    }

    fn cancel_authentication(&self, cookie: String) {
        let _ = self.canal.send(Aviso::Cancelar { cookie });
    }
}

/// Conexión D-Bus registrada ante `polkitd`. Al soltarse desregistra el agente.
pub struct Registro {
    conexion: zbus::blocking::Connection,
    session_id: String,
}

impl Drop for Registro {
    fn drop(&mut self) {
        let mut props = HashMap::new();
        props.insert("session-id", Value::from(self.session_id.as_str()));
        let sujeto = ("unix-session", props);
        let _ = self.conexion.call_method(
            Some(BUS_POLKIT),
            RUTA_AUTORIDAD,
            Some(INTERFAZ_AUTORIDAD),
            "UnregisterAuthenticationAgent",
            &(sujeto, RUTA_AGENTE),
        );
    }
}

fn detectar_session_id(conexion: &zbus::blocking::Connection) -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        let t = id.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    let ruta_sesion: OwnedObjectPath = conexion
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetSessionByPID",
            &(std::process::id(),),
        )
        .and_then(|m| m.body().deserialize())
        .or_else(|_| {
            conexion
                .call_method(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    Some("org.freedesktop.login1.Manager"),
                    "GetSession",
                    &("auto",),
                )
                .and_then(|m| m.body().deserialize())
        })
        .ok()?;
    let valor: OwnedValue = conexion
        .call_method(
            Some("org.freedesktop.login1"),
            ruta_sesion.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.Session", "Id"),
        )
        .and_then(|m| m.body().deserialize())
        .ok()?;
    String::try_from(valor).ok().filter(|s| !s.is_empty())
}

fn idioma_sesion() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let limpia = val.trim();
            if !limpia.is_empty() {
                return limpia.to_string();
            }
        }
    }
    "en_US.UTF-8".to_string()
}

fn es_sesion_propia() -> bool {
    let escritorio = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    escritorio.split(':').any(|item| item == "BookOS")
        || std::env::var("BOOKOS_POLKIT").as_deref() == Ok("1")
}

/// Conecta al bus del sistema y registra el agente de Polkit para la sesión
/// activa cuando corre dentro de una sesión BookOS (o con `BOOKOS_POLKIT=1`).
pub fn arrancar(canal: Sender<Aviso>) -> Option<Registro> {
    if !es_sesion_propia() {
        tracing::info!("agente Polkit omitido fuera de sesión BookOS (usa BOOKOS_POLKIT=1 para probar en anidado)");
        return None;
    }
    let agente = Agente {
        canal,
        siguiente_id: AtomicU64::new(1),
    };
    let conexion = match zbus::blocking::connection::Builder::system()
        .and_then(|b| b.serve_at(RUTA_AGENTE, agente))
        .and_then(|b| b.build())
    {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!("sin agente Polkit en el bus del sistema: {err}");
            return None;
        }
    };
    let Some(session_id) = detectar_session_id(&conexion) else {
        tracing::warn!("no se pudo resolver el identificador de sesión para Polkit");
        return None;
    };
    let locale = idioma_sesion();
    let mut props = HashMap::new();
    props.insert("session-id", Value::from(session_id.as_str()));
    let sujeto = ("unix-session", props);
    if let Err(err) = conexion.call_method(
        Some(BUS_POLKIT),
        RUTA_AUTORIDAD,
        Some(INTERFAZ_AUTORIDAD),
        "RegisterAuthenticationAgent",
        &(sujeto, locale.as_str(), RUTA_AGENTE),
    ) {
        tracing::warn!(%session_id, "no se pudo registrar el agente Polkit: {err}");
        return None;
    }
    tracing::info!(%session_id, "agente de autenticación Polkit de BookOS registrado");
    Some(Registro {
        conexion,
        session_id,
    })
}

fn encontrar_helper() -> Option<PathBuf> {
    RUTAS_HELPER
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

struct SesionHelper {
    sesion_id: u64,
    entrada: Option<mpsc::Sender<Secreto>>,
    cancelado: Arc<AtomicBool>,
}

impl Drop for SesionHelper {
    fn drop(&mut self) {
        self.cancelado.store(true, Ordering::Relaxed);
        // Al soltar el emisor se desbloquea `rx.recv()` y el hilo cierra
        // `ChildStdin` (EOF). Nunca llamamos `kill()` porque el helper es setuid
        // root y devolvería `EPERM`.
        self.entrada.take();
    }
}

impl SesionHelper {
    fn lanzar(
        solicitud_id: u64,
        sesion_id: u64,
        usuario: String,
        cookie: String,
        canal: Sender<Aviso>,
    ) -> Option<Self> {
        let helper = encontrar_helper()?;
        let (tx, rx) = mpsc::channel::<Secreto>();
        let cancelado = Arc::new(AtomicBool::new(false));
        let hilo_cancelado = Arc::clone(&cancelado);

        let spawned = std::thread::Builder::new()
            .name("bookos-polkit-helper".into())
            .spawn(move || {
                correr_helper(
                    helper,
                    solicitud_id,
                    sesion_id,
                    usuario,
                    cookie,
                    rx,
                    canal,
                    hilo_cancelado,
                );
            });
        if let Err(err) = spawned {
            tracing::warn!("no se pudo lanzar el hilo del helper Polkit: {err}");
            return None;
        }
        Some(Self {
            sesion_id,
            entrada: Some(tx),
            cancelado,
        })
    }

    fn responder(&self, secreto: Secreto) -> bool {
        self.entrada
            .as_ref()
            .is_some_and(|tx| tx.send(secreto).is_ok())
    }
}

#[allow(clippy::too_many_arguments)]
fn correr_helper(
    ruta: PathBuf,
    solicitud_id: u64,
    sesion_id: u64,
    usuario: String,
    cookie: String,
    rx: mpsc::Receiver<Secreto>,
    canal: Sender<Aviso>,
    cancelado: Arc<AtomicBool>,
) {
    let mut child = match Command::new(&ruta)
        .arg(&usuario)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(?ruta, "no se pudo ejecutar polkit-agent-helper-1: {err}");
            let _ = canal.send(Aviso::Helper {
                solicitud_id,
                sesion_id,
                evento: EventoHelper::Fallo(Some(
                    t(
                        "No se pudo iniciar el asistente de autenticación.",
                        "Failed to start authentication helper.",
                    )
                    .into(),
                )),
            });
            return;
        }
    };

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.wait();
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        drop(stdin);
        let _ = child.wait();
        return;
    };

    if writeln!(stdin, "{cookie}").is_err() || stdin.flush().is_err() {
        drop(stdin);
        let _ = child.wait();
        if !cancelado.load(Ordering::Relaxed) {
            let _ = canal.send(Aviso::Helper {
                solicitud_id,
                sesion_id,
                evento: EventoHelper::Fallo(None),
            });
        }
        return;
    }

    let mut lector = BufReader::new(stdout);
    let mut linea = String::new();
    let mut ultimo_error: Option<String> = None;
    let mut terminado = false;

    loop {
        linea.clear();
        match lector.read_line(&mut linea) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if cancelado.load(Ordering::Relaxed) {
            break;
        }
        let Some(orden) = parsear_linea_helper(&linea) else {
            continue;
        };
        match orden {
            LineaHelper::Prompt { texto, oculto } => {
                let _ = canal.send(Aviso::Helper {
                    solicitud_id,
                    sesion_id,
                    evento: EventoHelper::Prompt { texto, oculto },
                });
                let Ok(secreto) = rx.recv() else {
                    break;
                };
                if cancelado.load(Ordering::Relaxed) {
                    break;
                }
                if stdin.write_all(&secreto.0).is_err()
                    || stdin.write_all(b"\n").is_err()
                    || stdin.flush().is_err()
                {
                    break;
                }
            }
            LineaHelper::Info(msg) => {
                let _ = canal.send(Aviso::Helper {
                    solicitud_id,
                    sesion_id,
                    evento: EventoHelper::Info(msg),
                });
            }
            LineaHelper::Error(msg) => {
                ultimo_error = Some(msg.clone());
                let _ = canal.send(Aviso::Helper {
                    solicitud_id,
                    sesion_id,
                    evento: EventoHelper::Error(msg),
                });
            }
            LineaHelper::Exito => {
                terminado = true;
                let _ = canal.send(Aviso::Helper {
                    solicitud_id,
                    sesion_id,
                    evento: EventoHelper::Exito,
                });
                break;
            }
            LineaHelper::Fallo => {
                terminado = true;
                let _ = canal.send(Aviso::Helper {
                    solicitud_id,
                    sesion_id,
                    evento: EventoHelper::Fallo(ultimo_error.take()),
                });
                break;
            }
        }
    }

    drop(stdin);
    drop(lector);
    let _ = child.wait();
    if !terminado && !cancelado.load(Ordering::Relaxed) {
        let _ = canal.send(Aviso::Helper {
            solicitud_id,
            sesion_id,
            evento: EventoHelper::Fallo(ultimo_error),
        });
    }
}

struct Pendiente {
    id: u64,
    cookie: String,
    action_id: String,
    mensaje: String,
    icono: String,
    detalle: Option<String>,
    identidades: Vec<IdentidadPolkit>,
    respuesta: crate::portal::Emisario<ResultadoAuth>,
}

struct Activa {
    id: u64,
    cookie: String,
    #[allow(dead_code)]
    action_id: String,
    identidades: Vec<IdentidadPolkit>,
    identidad_actual: usize,
    respuesta: Option<crate::portal::Emisario<ResultadoAuth>>,
    helper: Option<SesionHelper>,
    siguiente_sesion: u64,
}

impl Activa {
    fn responder(&mut self, resultado: ResultadoAuth) {
        if let Some(em) = self.respuesta.take() {
            em.entregar(resultado);
        }
    }
}

impl Drop for Activa {
    fn drop(&mut self) {
        self.helper.take();
        if let Some(em) = self.respuesta.take() {
            em.entregar(ResultadoAuth::Cancelado);
        }
    }
}

/// Estado del agente Polkit dentro del compositor.
pub struct Estado {
    canal: Sender<Aviso>,
    activa: Option<Activa>,
    cola: VecDeque<Pendiente>,
}

impl Estado {
    pub fn new(canal: Sender<Aviso>) -> Self {
        Self {
            canal,
            activa: None,
            cola: VecDeque::new(),
        }
    }
}

fn spawn_helper_para(activa: &mut Activa, canal: &Sender<Aviso>) -> bool {
    let Some(id_cuenta) = activa.identidades.get(activa.identidad_actual) else {
        return false;
    };
    activa.siguiente_sesion = activa.siguiente_sesion.wrapping_add(1).max(1);
    let sesion_id = activa.siguiente_sesion;
    activa.helper = SesionHelper::lanzar(
        activa.id,
        sesion_id,
        id_cuenta.usuario.clone(),
        activa.cookie.clone(),
        canal.clone(),
    );
    activa.helper.is_some()
}

fn activar_peticion(state: &mut crate::state::BookosComp, p: Pendiente) {
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.esta_bloqueado())
    {
        p.respuesta.entregar(ResultadoAuth::Cancelado);
        return;
    }

    let peticion = PeticionPolkit {
        id: p.id,
        mensaje: p.mensaje,
        icono: p.icono,
        detalle: p.detalle,
        identidades: p.identidades.clone(),
    };

    let Some(shell) = state.shell.as_mut() else {
        p.respuesta.entregar(ResultadoAuth::Fallo(
            "BookOS shell unavailable".into(),
        ));
        return;
    };
    shell.abrir_polkit(peticion);
    if let Some(kbd) = state.seat.get_keyboard() {
        shell.polkit_caps_lock(kbd.modifier_state().caps_lock);
    }

    let mut activa = Activa {
        id: p.id,
        cookie: p.cookie,
        action_id: p.action_id,
        identidades: p.identidades,
        identidad_actual: 0,
        respuesta: Some(p.respuesta),
        helper: None,
        siguiente_sesion: 0,
    };

    let canal = state.polkit.canal.clone();
    if !spawn_helper_para(&mut activa, &canal) {
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_polkit_si(activa.id);
        }
        activa.responder(ResultadoAuth::Fallo(
            "polkit-agent-helper-1 not found".into(),
        ));
        state.needs_redraw = true;
        return;
    }

    state.polkit.activa = Some(activa);
    state.needs_redraw = true;
}

fn promover_siguiente(state: &mut crate::state::BookosComp) {
    if state.polkit.activa.is_some() {
        return;
    }
    while let Some(siguiente) = state.polkit.cola.pop_front() {
        if state
            .shell
            .as_ref()
            .is_some_and(|s| s.esta_bloqueado())
        {
            siguiente.respuesta.entregar(ResultadoAuth::Cancelado);
            continue;
        }
        activar_peticion(state, siguiente);
        if state.polkit.activa.is_some() {
            break;
        }
    }
}

/// Procesa un aviso del hilo D-Bus o del helper ya en el hilo del compositor.
pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    match aviso {
        Aviso::Autenticar {
            id,
            cookie,
            action_id,
            mensaje,
            icono,
            detalle,
            identidades,
            respuesta,
        } => {
            if state
                .shell
                .as_ref()
                .is_some_and(|s| s.esta_bloqueado())
            {
                respuesta.entregar(ResultadoAuth::Cancelado);
                return;
            }
            let p = Pendiente {
                id,
                cookie,
                action_id,
                mensaje,
                icono,
                detalle,
                identidades,
                respuesta,
            };
            if state.polkit.activa.is_none() {
                activar_peticion(state, p);
            } else {
                state.polkit.cola.push_back(p);
            }
        }
        Aviso::Cancelar { cookie } => {
            let coincide_activa = state
                .polkit
                .activa
                .as_ref()
                .is_some_and(|a| a.cookie == cookie);
            if coincide_activa {
                if let Some(mut activa) = state.polkit.activa.take() {
                    activa.responder(ResultadoAuth::Cancelado);
                    if let Some(shell) = state.shell.as_mut() {
                        shell.cerrar_polkit_si(activa.id);
                    }
                    state.needs_redraw = true;
                }
                promover_siguiente(state);
            } else {
                let mut restantes = VecDeque::with_capacity(state.polkit.cola.len());
                while let Some(item) = state.polkit.cola.pop_front() {
                    if item.cookie == cookie {
                        item.respuesta.entregar(ResultadoAuth::Cancelado);
                    } else {
                        restantes.push_back(item);
                    }
                }
                state.polkit.cola = restantes;
            }
        }
        Aviso::Helper {
            solicitud_id,
            sesion_id,
            evento,
        } => {
            let coincide = state
                .polkit
                .activa
                .as_ref()
                .is_some_and(|a| a.id == solicitud_id && a.helper.as_ref().is_some_and(|h| h.sesion_id == sesion_id));
            if !coincide {
                return;
            }
            match evento {
                EventoHelper::Prompt { texto, oculto } => {
                    let etiqueta = (!texto.is_empty()).then_some(texto);
                    if let Some(shell) = state.shell.as_mut() {
                        shell.polkit_prompt(solicitud_id, etiqueta, oculto);
                    }
                    state.needs_redraw = true;
                }
                EventoHelper::Info(msg) => {
                    if let Some(shell) = state.shell.as_mut() {
                        shell.polkit_info(solicitud_id, Some(msg));
                    }
                    state.needs_redraw = true;
                }
                EventoHelper::Error(msg) => {
                    if let Some(shell) = state.shell.as_mut() {
                        shell.polkit_info(solicitud_id, Some(msg));
                    }
                    state.needs_redraw = true;
                }
                EventoHelper::Exito => {
                    if let Some(mut activa) = state.polkit.activa.take() {
                        activa.helper.take();
                        if let Some(shell) = state.shell.as_mut() {
                            shell.cerrar_polkit_si(activa.id);
                        }
                        activa.responder(ResultadoAuth::Autorizado);
                        state.needs_redraw = true;
                    }
                    promover_siguiente(state);
                }
                EventoHelper::Fallo(motivo) => {
                    let texto = motivo.unwrap_or_else(|| {
                        t(
                            "Contraseña incorrecta. Inténtalo de nuevo.",
                            "Incorrect password. Try again.",
                        )
                        .into()
                    });
                    if let Some(shell) = state.shell.as_mut() {
                        shell.polkit_error(solicitud_id, texto);
                        shell.polkit_info(solicitud_id, None);
                    }
                    let canal = state.polkit.canal.clone();
                    if let Some(activa) = state.polkit.activa.as_mut() {
                        activa.helper.take();
                        if !spawn_helper_para(activa, &canal) {
                            let mut activa = state.polkit.activa.take().unwrap();
                            if let Some(shell) = state.shell.as_mut() {
                                shell.cerrar_polkit_si(activa.id);
                            }
                            activa.responder(ResultadoAuth::Fallo(
                                "polkit-agent-helper-1 failed".into(),
                            ));
                            promover_siguiente(state);
                        }
                    }
                    state.needs_redraw = true;
                }
            }
        }
    }
}

/// Envía la respuesta escrita por el usuario al asistente de Polkit activo.
pub fn enviar(state: &mut crate::state::BookosComp, id: u64) {
    let Some(bytes) = state
        .shell
        .as_mut()
        .and_then(|s| s.tomar_polkit_clave(id))
    else {
        return;
    };
    let mut secreto = Secreto(bytes);
    let canal = state.polkit.canal.clone();
    let Some(activa) = state.polkit.activa.as_mut() else {
        return;
    };
    if activa.id != id {
        return;
    }
    if activa.helper.is_none() && !spawn_helper_para(activa, &canal) {
        if let Some(shell) = state.shell.as_mut() {
            shell.polkit_error(
                id,
                t("No se pudo comprobar la contraseña.", "Could not verify password.").into(),
            );
        }
        state.needs_redraw = true;
        return;
    }
    let enviado = activa
        .helper
        .as_ref()
        .is_some_and(|h| h.responder(Secreto(std::mem::take(&mut secreto.0))));
    if !enviado {
        let _ = spawn_helper_para(activa, &canal);
        if let Some(shell) = state.shell.as_mut() {
            shell.polkit_error(
                id,
                t("La sesión expiró. Inténtalo de nuevo.", "Session expired. Try again.").into(),
            );
        }
    }
    state.needs_redraw = true;
}

/// Cancela explícitamente la petición de Polkit activa.
pub fn cancelar(state: &mut crate::state::BookosComp, id: u64) {
    if state.polkit.activa.as_ref().is_some_and(|a| a.id == id) {
        if let Some(mut activa) = state.polkit.activa.take() {
            activa.responder(ResultadoAuth::Cancelado);
        }
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_polkit_si(id);
        }
        state.needs_redraw = true;
        promover_siguiente(state);
    }
}

/// Reinicia el asistente de Polkit con otra de las identidades ofrecidas.
pub fn cambiar_identidad(state: &mut crate::state::BookosComp, id: u64, indice: usize) {
    let canal = state.polkit.canal.clone();
    if let Some(activa) = state.polkit.activa.as_mut()
        && activa.id == id
        && indice < activa.identidades.len()
    {
        activa.identidad_actual = indice;
        activa.helper.take();
        if !spawn_helper_para(activa, &canal) {
            if let Some(shell) = state.shell.as_mut() {
                shell.polkit_error(
                    id,
                    t("No se pudo cambiar de cuenta.", "Could not switch account.").into(),
                );
            }
        }
        state.needs_redraw = true;
    }
}

/// Red de seguridad: si la pantalla se bloquea o la emergente `polkit` se cierra
/// desde fuera, cancela de inmediato la espera pendiente para no colgar clientes.
pub fn cancelar_si_se_cerro(state: &mut crate::state::BookosComp) {
    let bloqueado = state
        .shell
        .as_ref()
        .is_some_and(|s| s.esta_bloqueado());
    if bloqueado {
        if let Some(mut activa) = state.polkit.activa.take() {
            activa.responder(ResultadoAuth::Cancelado);
            if let Some(shell) = state.shell.as_mut() {
                shell.cerrar_polkit_si(activa.id);
            }
            state.needs_redraw = true;
        }
        while let Some(item) = state.polkit.cola.pop_front() {
            item.respuesta.entregar(ResultadoAuth::Cancelado);
        }
        return;
    }

    if state.polkit.activa.is_none() {
        return;
    }
    let abierta = state
        .shell
        .as_ref()
        .and_then(|s| s.emergente_nombre())
        == Some("polkit");
    if abierta {
        return;
    }
    if let Some(mut activa) = state.polkit.activa.take() {
        activa.responder(ResultadoAuth::Cancelado);
    }
    promover_siguiente(state);
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn parsea_todas_las_lineas_del_helper() {
        assert_eq!(
            parsear_linea_helper("PAM_PROMPT_ECHO_OFF Password: \n"),
            Some(LineaHelper::Prompt {
                texto: "Password:".into(),
                oculto: true
            })
        );
        assert_eq!(
            parsear_linea_helper("PAM_PROMPT_ECHO_OFF\r\n"),
            Some(LineaHelper::Prompt {
                texto: String::new(),
                oculto: true
            })
        );
        assert_eq!(
            parsear_linea_helper("PAM_PROMPT_ECHO_ON Code: "),
            Some(LineaHelper::Prompt {
                texto: "Code:".into(),
                oculto: false
            })
        );
        assert_eq!(
            parsear_linea_helper("PAM_TEXT_INFO Place your finger on the reader"),
            Some(LineaHelper::Info("Place your finger on the reader".into()))
        );
        assert_eq!(
            parsear_linea_helper("PAM_ERROR_MSG Authentication failure"),
            Some(LineaHelper::Error("Authentication failure".into()))
        );
        assert_eq!(parsear_linea_helper("SUCCESS\n"), Some(LineaHelper::Exito));
        assert_eq!(parsear_linea_helper("FAILURE\n"), Some(LineaHelper::Fallo));
        assert_eq!(parsear_linea_helper("PAM_TEXT_INFO   "), None);
        assert_eq!(parsear_linea_helper(""), None);
    }

    #[test]
    fn extrae_detalle_por_prioridad() {
        let mut detalles = HashMap::new();
        assert_eq!(
            extraer_detalle(&detalles, "org.freedesktop.policykit.exec"),
            Some("org.freedesktop.policykit.exec".into())
        );
        assert_eq!(extraer_detalle(&detalles, ""), None);

        detalles.insert("program".into(), "/usr/bin/true".into());
        assert_eq!(
            extraer_detalle(&detalles, "org.freedesktop.policykit.exec"),
            Some("/usr/bin/true".into())
        );

        detalles.insert("command_line".into(), "/usr/bin/true --flag".into());
        assert_eq!(
            extraer_detalle(&detalles, "org.freedesktop.policykit.exec"),
            Some("/usr/bin/true --flag".into())
        );
    }

    #[test]
    fn prioriza_uid_propio_y_deduplica_identidades() {
        let candidatos = vec![
            (
                0,
                IdentidadPolkit {
                    usuario: "root".into(),
                    etiqueta: "root".into(),
                },
            ),
            (
                1000,
                IdentidadPolkit {
                    usuario: "eve".into(),
                    etiqueta: "Eve (eve)".into(),
                },
            ),
            (
                1000,
                IdentidadPolkit {
                    usuario: "eve".into(),
                    etiqueta: "Eve (eve)".into(),
                },
            ),
        ];
        let ordenadas = priorizar_identidades(candidatos, 1000);
        assert_eq!(ordenadas.len(), 2);
        assert_eq!(ordenadas[0].usuario, "eve");
        assert_eq!(ordenadas[1].usuario, "root");
    }

    #[test]
    fn secreto_borra_bytes_al_soltar() {
        let mut buf = b"clave-secreta".to_vec();
        crate::autenticar::borrar(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }
}
