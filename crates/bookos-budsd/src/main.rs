//! bookos-budsd: el canal de control de los Galaxy Buds, fuera de Ajustes.
//!
//! El sonido lo conecta BlueZ por su cuenta; esto abre aparte el canal SPP de
//! Samsung (batería, ANC, ecualizador…) en cuanto BlueZ conecta unos Buds, sin
//! ventanas ni foco, y enseña la batería en la isla. Ajustes pasa a ser un
//! cliente más.
//!
//! Es un proceso aparte y no parte de `bookos-comp` porque abrir el canal
//! espera a la radio durante segundos, y en el compositor eso congelaría el
//! escritorio.
//!
//! **Contrato** (`org.bookos.Buds` en `/org/bookos/Buds`, bus de sesión). Los
//! nombres y el JSON son los de los antiguos comandos Tauri `buds_*`:
//!
//! ```text
//! Connect(s mac) -> s estado      error con el motivo legible si no abre
//! Disconnect()
//! GetStatus() -> s estado         siempre contesta; `error` y `connecting` dicen por qué no hay canal
//! SetAnc(y modo)  SetEq(y preset, b activado)  SetTouchLock(b)
//! SetEasyPairing(b)  FitTestStart()  FitTestStop()  RequestInfo()
//! SetAutoReconnect(s mac, b)  GetPrefs(s mac) -> s
//! ```

mod conexion;
mod protocolo;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use conexion::{Aviso, Conexion, Pendiente};
use protocolo::BudsStatus;

const NOMBRE: &str = "org.bookos.Buds";
const RUTA: &str = "/org/bookos/Buds";
/// Con este identificador publica en la isla; `bookos-comp` lo tiene en su
/// lista cerrada como clase «auriculares».
const APP_ID: &str = "com.bookos.buds";

/// Lo que se ve la batería en la isla al sacar los Buds.
const ISLA_VISIBLE: Duration = Duration::from_secs(6);
/// Si vuelven antes de esto desde que se guardaron, no se avisa: sacarlos un
/// momento para recolocarlos no es una novedad.
const RECIENTE: Duration = Duration::from_secs(5 * 60);
/// El umbral de batería baja que ya usaba Ajustes.
const BATERIA_BAJA: u8 = 20;
/// Pausas entre los tres intentos de abrir el canal. La primera es corta para
/// la carrera con BlueZ, que anuncia `Connected` antes de terminar de negociar
/// el audio; la segunda da tiempo a que otro cliente suelte el canal.
const ESPERAS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

#[derive(Default)]
struct Estado {
    conexion: Option<Conexion>,
    /// Lo que se contesta sin canal: el último error, si se está conectando y
    /// de qué aparato.
    suelto: BudsStatus,
    conectando: bool,
    /// Cuándo se desconectó el audio de cada MAC.
    guardados: HashMap<String, Instant>,
    /// La MAC que acaba de conectar audio: su primer estado va a la isla.
    isla_pendiente: Option<String>,
    baja_avisada: bool,
    /// Para que el cierre programado de una publicación vieja no cierre una
    /// más nueva.
    publicacion: u64,
}

#[derive(Clone)]
struct Servicio {
    estado: Arc<Mutex<Estado>>,
    sistema: Connection,
    pendiente: Pendiente,
    avisos: mpsc::UnboundedSender<Aviso>,
}

impl Servicio {
    // 0.1.0
    fn estado(&self) -> MutexGuard<'_, Estado> {
        self.estado
            .lock()
            .expect("nadie entra en pánico con el estado tomado")
    }

    // 0.1.0
    fn con<T>(&self, f: impl FnOnce(&Conexion) -> Result<T, String>) -> zbus::fdo::Result<T> {
        let e = self.estado();
        let c = e.conexion.as_ref().ok_or_else(|| {
            zbus::fdo::Error::Failed("No hay conexión activa con los Buds".into())
        })?;
        f(c).map_err(zbus::fdo::Error::Failed)
    }

    /// Abre el canal con hasta tres intentos. Si ya hay uno en marcha, espera
    /// a que acabe en vez de lanzar otro: dos `ConnectProfile` a la vez se
    /// pisan el descriptor pendiente.
    // 0.1.0
    async fn conectar(&self, ruta: OwnedObjectPath) -> Result<BudsStatus, String> {
        let aparato = conexion::aparato(&self.sistema, ruta).await?;
        loop {
            {
                let mut e = self.estado();
                if let Some(c) = &e.conexion
                    && c.mac == aparato.mac
                    && c.estado().connected
                {
                    return Ok(c.estado());
                }
                if !e.conectando {
                    e.conectando = true;
                    e.suelto = BudsStatus {
                        connecting: true,
                        mac: aparato.mac.clone(),
                        model: aparato.nombre.clone(),
                        ..Default::default()
                    };
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // BlueZ anuncia `Connected` en cuanto hay enlace, antes de montar el
        // audio. Abrir el canal de Samsung en ese momento se cruzaba con la
        // negociación: A2DP fallaba con «Input/output error» y los Buds
        // quedaban conectados pero sin sonido (visto en bluetoothd el
        // 2026-10-08). Se espera a que exista el transporte de audio.
        let inicio = Instant::now();
        if esperar_audio(&self.sistema, &aparato.ruta).await {
            eprintln!("budsd: audio listo en {} ms", inicio.elapsed().as_millis());
        } else {
            eprintln!(
                "budsd: sin transporte de audio tras {} s; se abre igual",
                AUDIO_TOPE.as_secs()
            );
        }

        let mut crudo = String::new();
        for intento in 0..=ESPERAS.len() {
            if intento > 0 {
                tokio::time::sleep(ESPERAS[intento - 1]).await;
            }
            let inicio = Instant::now();
            let abierto = conexion::abrir(&self.sistema, &self.pendiente, &aparato.ruta)
                .await
                .and_then(|fd| Conexion::iniciar(fd, &aparato, self.avisos.clone()));
            match abierto {
                Ok(c) => {
                    eprintln!(
                        "budsd: canal abierto con {} al intento {} en {} ms",
                        aparato.mac,
                        intento + 1,
                        inicio.elapsed().as_millis()
                    );
                    let st = c.estado();
                    let vieja = {
                        let mut e = self.estado();
                        e.conectando = false;
                        e.suelto = BudsStatus::default();
                        e.conexion.replace(c)
                    };
                    // Fuera del cerrojo: soltarla espera a que acabe su lector.
                    drop(vieja);
                    return Ok(st);
                }
                Err(e) => {
                    eprintln!(
                        "budsd: intento {} con {} fallido en {} ms: {e}",
                        intento + 1,
                        aparato.mac,
                        inicio.elapsed().as_millis()
                    );
                    crudo = e;
                }
            }
        }
        let legible = conexion::error_legible(&crudo);
        let mut e = self.estado();
        e.conectando = false;
        e.suelto = BudsStatus {
            error: Some(legible.clone()),
            mac: aparato.mac,
            model: aparato.nombre,
            ..Default::default()
        };
        Err(legible)
    }

    // 0.1.0
    fn soltar(&self, mac: &str) {
        let vieja = {
            let mut e = self.estado();
            if e.conexion.as_ref().is_none_or(|c| c.mac != mac) {
                return;
            }
            e.suelto = BudsStatus::default();
            e.conexion.take()
        };
        drop(vieja);
    }
}

// 0.1.0
fn json(s: &BudsStatus) -> String {
    serde_json::to_string(s).expect("BudsStatus solo tiene tipos que serde sabe escribir")
}

#[zbus::interface(name = "org.bookos.Buds")]
impl Servicio {
    // 0.1.0
    async fn connect(&self, mac: &str) -> zbus::fdo::Result<String> {
        let ruta = conexion::ruta_de(&self.sistema, mac)
            .await
            .map_err(zbus::fdo::Error::Failed)?;
        self.conectar(ruta)
            .await
            .map_err(zbus::fdo::Error::Failed)?;
        // Recién abierto, el estado aún no trae batería y Ajustes pintaría
        // 0 %. El primer estado llega en unos cientos de ms; con 1,5 s de
        // espera como mucho, si no llega se devuelve lo que haya.
        for _ in 0..15 {
            let s = self.con(|c| Ok(c.estado()))?;
            if s.battery_l > 0 || s.battery_r > 0 {
                return Ok(json(&s));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.con(|c| Ok(json(&c.estado())))
    }

    // 0.1.0
    async fn disconnect(&self) {
        let vieja = self.estado().conexion.take();
        drop(vieja);
    }

    // 0.1.0
    async fn get_status(&self) -> String {
        let e = self.estado();
        match &e.conexion {
            Some(c) => {
                // Pide un estado fresco; llega por el lector para la próxima.
                let _ = c.enviar(protocolo::MSG_EXTENDED_STATUS, &[]);
                json(&c.estado())
            }
            None => json(&e.suelto),
        }
    }

    /// 0 apagado, 1 cancelación, 2 ambiente, 3 adaptativo.
    // 0.1.0
    async fn set_anc(&self, mode: u8) -> zbus::fdo::Result<()> {
        let mac = self.con(|c| {
            c.enviar(protocolo::MSG_NOISE_CONTROLS, &[mode])?;
            // En los Buds3 Pro la cancelación al nivel normal apenas se nota:
            // con 1 y 3 se sube al alto.
            if mode == 1 || mode == 3 {
                c.enviar(protocolo::MSG_NOISE_REDUCTION_LEVEL, &[1])?;
            }
            c.status
                .lock()
                .expect("nadie entra en pánico con el estado tomado")
                .anc_mode = mode;
            Ok(c.mac.clone())
        })?;
        protocolo::update_prefs(&mac, |p| p.anc_mode = mode);
        Ok(())
    }

    /// 0 graves, 1 suave, 2 dinámico, 3 claro, 4 agudos.
    // 0.1.0
    async fn set_eq(&self, preset: u8, enabled: bool) -> zbus::fdo::Result<()> {
        let mac = self.con(|c| {
            c.enviar(
                protocolo::MSG_EQUALIZER,
                &conexion::payload_eq(preset, enabled, c.legacy),
            )?;
            let mut g = c
                .status
                .lock()
                .expect("nadie entra en pánico con el estado tomado");
            g.eq_preset = preset;
            g.eq_enabled = enabled;
            Ok(c.mac.clone())
        })?;
        protocolo::update_prefs(&mac, |p| {
            p.eq_preset = preset;
            p.eq_enabled = enabled;
        });
        Ok(())
    }

    // 0.1.0
    async fn set_touch_lock(&self, locked: bool) -> zbus::fdo::Result<()> {
        let mac = self.con(|c| {
            c.enviar(protocolo::MSG_LOCK_TOUCHPAD, &[locked as u8])?;
            c.status
                .lock()
                .expect("nadie entra en pánico con el estado tomado")
                .touchpad_locked = locked;
            Ok(c.mac.clone())
        })?;
        protocolo::update_prefs(&mac, |p| p.touchpad_locked = locked);
        Ok(())
    }

    // 0.1.0
    async fn set_easy_pairing(&self, enable: bool) -> zbus::fdo::Result<()> {
        self.con(|c| c.enviar(protocolo::MSG_SET_EASY_PAIRING, &[enable as u8]))
    }

    // 0.1.0
    async fn fit_test_start(&self) -> zbus::fdo::Result<()> {
        self.con(|c| {
            {
                let mut g = c
                    .status
                    .lock()
                    .expect("nadie entra en pánico con el estado tomado");
                g.fit_l = 255;
                g.fit_r = 255;
            }
            c.enviar(protocolo::MSG_FIT_TEST_START, &[])
        })
    }

    // 0.1.0
    async fn fit_test_stop(&self) -> zbus::fdo::Result<()> {
        self.con(|c| c.enviar(protocolo::MSG_FIT_TEST_STOP, &[]))
    }

    // 0.1.0
    async fn request_info(&self) -> zbus::fdo::Result<()> {
        self.con(|c| {
            c.enviar(protocolo::MSG_DEBUG_BUILD_INFO_REQ, &[])?;
            c.enviar(protocolo::MSG_DEBUG_SERIAL_REQ, &[])
        })
    }

    /// Se guarda para Ajustes, que la usa en su cambio automático de audio
    /// entre portátil y móvil. El canal de control se abre siempre que BlueZ
    /// conecta unos Buds, esté o no activada.
    // 0.1.0
    async fn set_auto_reconnect(&self, mac: &str, enable: bool) {
        protocolo::update_prefs(mac, |p| p.auto_reconnect = enable);
    }

    // 0.1.0
    async fn get_prefs(&self, mac: &str) -> String {
        serde_json::to_string(&protocolo::get_prefs(mac))
            .expect("BudsPrefs solo tiene tipos simples")
    }
}

/// Escucha `Connected` de todos los `org.bluez.Device1`. Reactivo: Ajustes
/// miraba la lista entera cada 4 s.
// 0.1.0
async fn escuchar_bluez(sistema: Connection, avisos: mpsc::UnboundedSender<Aviso>) {
    loop {
        let regla = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.bluez")
            .and_then(|r| r.interface("org.freedesktop.DBus.Properties"))
            .and_then(|r| r.member("PropertiesChanged"))
            .and_then(|r| r.arg(0, "org.bluez.Device1"))
            .expect("regla con nombres fijos y válidos")
            .build();
        match zbus::MessageStream::for_match_rule(regla, &sistema, Some(64)).await {
            Ok(mut flujo) => {
                // Los que ya estaban conectados al arrancar (reinicio del
                // servicio o de la sesión) no van a mandar señal.
                for ruta in buds_conectados(&sistema).await {
                    let _ = avisos.send(Aviso::Audio {
                        ruta,
                        conectado: true,
                    });
                }
                while let Some(Ok(m)) = flujo.next().await {
                    let Some(ruta) = m
                        .header()
                        .path()
                        .map(|p| OwnedObjectPath::from(p.to_owned()))
                    else {
                        continue;
                    };
                    let Ok((_, cambios, _)) =
                        m.body()
                            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    else {
                        continue;
                    };
                    if let Some(conectado) = cambios
                        .get("Connected")
                        .and_then(|v| bool::try_from(v).ok())
                    {
                        let _ = avisos.send(Aviso::Audio { ruta, conectado });
                    }
                }
            }
            Err(e) => eprintln!("budsd: no se pudo escuchar a BlueZ: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

type Objetos = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// Lo más que se espera al audio. Pasado esto se abre el canal igualmente:
/// unos Buds solo con perfil de llamadas no llegan a tener transporte A2DP.
const AUDIO_TOPE: Duration = Duration::from_secs(10);

/// Espera a que BlueZ cuelgue un `MediaTransport1` del aparato, que es la
/// señal de que el audio ya está negociado. `true` si llegó a tiempo.
// 0.1.0
async fn esperar_audio(sistema: &Connection, ruta: &OwnedObjectPath) -> bool {
    let Ok(p) = zbus::Proxy::new(
        sistema,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    else {
        return false;
    };
    let prefijo = format!("{}/", ruta.as_str());
    let tope = Instant::now() + AUDIO_TOPE;
    while Instant::now() < tope {
        let objetos: Objetos = p.call("GetManagedObjects", &()).await.unwrap_or_default();
        if objetos.iter().any(|(r, i)| {
            r.as_str().starts_with(&prefijo) && i.contains_key("org.bluez.MediaTransport1")
        }) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

// 0.1.0
async fn buds_conectados(sistema: &Connection) -> Vec<OwnedObjectPath> {
    let Ok(p) = zbus::Proxy::new(
        sistema,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    else {
        return Vec::new();
    };
    let objetos: Objetos = p.call("GetManagedObjects", &()).await.unwrap_or_default();
    objetos
        .into_iter()
        .filter(|(_, i)| {
            i.get("org.bluez.Device1")
                .and_then(|d| d.get("Connected"))
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false)
        })
        .map(|(ruta, _)| ruta)
        .collect()
}

// 0.1.0
fn en_castellano() -> bool {
    std::env::var("LANG").is_ok_and(|l| l.starts_with("es"))
}

// 0.1.0
fn baja(s: &BudsStatus) -> bool {
    [s.battery_l, s.battery_r]
        .iter()
        .any(|&b| b > 0 && b <= BATERIA_BAJA)
}

// 0.1.0
async fn llamar_isla<B>(sesion: &Connection, metodo: &str, cuerpo: &B)
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    // Sin sesión BookOS no hay isla: se anota y sigue.
    if let Err(e) = sesion
        .call_method(
            Some("org.bookos.Desktop"),
            "/org/bookos/Desktop",
            Some("org.bookos.Desktop"),
            metodo,
            cuerpo,
        )
        .await
    {
        eprintln!("budsd: {metodo}: {e}");
    }
}

// 0.1.0
async fn publicar_isla(svc: &Servicio, sesion: &Connection, s: &BudsStatus) {
    let es = en_castellano();
    let fila = |etiqueta: &str, v: u8| {
        serde_json::json!({
            "etiqueta": etiqueta,
            "valor": f32::from(v) / 100.0,
            "texto": format!("{v} %"),
            "critica": v > 0 && v <= BATERIA_BAJA,
        })
    };
    let mut metricas = vec![
        fila(if es { "Izquierdo" } else { "Left" }, s.battery_l),
        fila(if es { "Derecho" } else { "Right" }, s.battery_r),
    ];
    // Con un solo auricular fuera, el estuche puede no informar: un 0 % ahí
    // asustaría sin motivo.
    if s.battery_case > 0 {
        metricas.push(fila(if es { "Estuche" } else { "Case" }, s.battery_case));
    }
    let cuerpo = serde_json::json!({ "activo": true, "titulo": s.model, "metricas": metricas });
    let n = {
        let mut e = svc.estado();
        e.publicacion += 1;
        e.publicacion
    };
    llamar_isla(
        sesion,
        "PublishActivity",
        &(APP_ID, "auriculares", cuerpo.to_string()),
    )
    .await;
    let (svc, sesion) = (svc.clone(), sesion.clone());
    tokio::spawn(async move {
        tokio::time::sleep(ISLA_VISIBLE).await;
        if svc.estado().publicacion == n {
            llamar_isla(&sesion, "CloseActivity", &(APP_ID,)).await;
        }
    });
}

/// Tocar la isla abre Ajustes en la página de los Buds.
// 0.1.0
async fn escuchar_isla(sesion: Connection, svc: Servicio) {
    let regla = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.bookos.Desktop")
        .and_then(|r| r.member("ActivityAction"))
        .and_then(|r| r.arg(0, APP_ID))
        .expect("regla con nombres fijos y válidos")
        .build();
    let mut flujo = match zbus::MessageStream::for_match_rule(regla, &sesion, Some(8)).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("budsd: sin acciones de la isla: {e}");
            return;
        }
    };
    while let Some(Ok(_)) = flujo.next().await {
        svc.estado().publicacion += 1;
        llamar_isla(&sesion, "CloseActivity", &(APP_ID,)).await;
        // Si Ajustes ya está abierto, esta segunda copia le pasa la página por
        // su socket y sale.
        match tokio::process::Command::new("bookos-settings")
            .args(["--page", "buds"])
            .spawn()
        {
            Ok(mut hijo) => {
                tokio::spawn(async move { hijo.wait().await });
            }
            Err(e) => eprintln!("budsd: no se pudo abrir Ajustes: {e}"),
        }
    }
}

// 0.1.0
async fn supervisar(svc: Servicio, sesion: Connection, mut avisos: mpsc::UnboundedReceiver<Aviso>) {
    while let Some(aviso) = avisos.recv().await {
        match aviso {
            Aviso::Audio { ruta, conectado } => {
                let Ok(aparato) = conexion::aparato(&svc.sistema, ruta.clone()).await else {
                    continue;
                };
                if !aparato.buds {
                    continue;
                }
                if conectado {
                    eprintln!("budsd: audio conectado con {}", aparato.mac);
                    {
                        let mut e = svc.estado();
                        e.isla_pendiente = Some(aparato.mac.clone());
                        e.baja_avisada = false;
                    }
                    let svc = svc.clone();
                    tokio::spawn(async move { svc.conectar(ruta).await });
                } else {
                    eprintln!("budsd: audio desconectado de {}", aparato.mac);
                    svc.soltar(&aparato.mac);
                    svc.estado().guardados.insert(aparato.mac, Instant::now());
                }
            }
            Aviso::Estado(mac) => {
                let publicar = {
                    let mut e = svc.estado();
                    let Some(s) = e
                        .conexion
                        .as_ref()
                        .filter(|c| c.mac == mac)
                        .map(Conexion::estado)
                    else {
                        continue;
                    };
                    let es_baja = baja(&s);
                    let mostrar = if e.isla_pendiente.as_deref() == Some(mac.as_str()) {
                        e.isla_pendiente = None;
                        let reciente = e
                            .guardados
                            .get(&mac)
                            .is_some_and(|t| t.elapsed() < RECIENTE);
                        !reciente || es_baja
                    } else {
                        es_baja && !e.baja_avisada
                    };
                    e.baja_avisada = es_baja;
                    mostrar.then_some(s)
                };
                if let Some(s) = publicar {
                    publicar_isla(&svc, &sesion, &s).await;
                }
            }
            Aviso::Caido(mac) => {
                eprintln!("budsd: los Buds {mac} cerraron el canal");
                svc.soltar(&mac);
                // Si el audio sigue, el canal se cayó solo (el móvil lo tomó,
                // o el firmware lo cerró): se vuelve a abrir en silencio.
                let Ok(ruta) = conexion::ruta_de(&svc.sistema, &mac).await else {
                    continue;
                };
                let sigue =
                    zbus::Proxy::new(&svc.sistema, "org.bluez", ruta.clone(), "org.bluez.Device1")
                        .await
                        .ok();
                let sigue = match sigue {
                    Some(p) => p.get_property::<bool>("Connected").await.unwrap_or(false),
                    None => false,
                };
                if sigue {
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(ESPERAS[0]).await;
                        svc.conectar(ruta).await
                    });
                }
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
// 0.1.0
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sistema = Connection::system().await?;
    let pendiente = conexion::publicar_perfiles(&sistema).await?;
    let (tx, rx) = mpsc::unbounded_channel();
    let svc = Servicio {
        estado: Arc::new(Mutex::new(Estado::default())),
        sistema: sistema.clone(),
        pendiente,
        avisos: tx.clone(),
    };
    let sesion = zbus::connection::Builder::session()?
        .name(NOMBRE)?
        .serve_at(RUTA, svc.clone())?
        .build()
        .await?;
    tokio::spawn(escuchar_bluez(sistema, tx));
    tokio::spawn(escuchar_isla(sesion.clone(), svc.clone()));
    supervisar(svc, sesion, rx).await;
    Ok(())
}
