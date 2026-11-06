//! El canal de control: perfil de BlueZ, apertura con límite de tiempo y los
//! hilos que leen y mantienen vivo el socket.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, fdo, interface};

use crate::protocolo::{self, BudsStatus};

const SAMSUNG_UUID: &str = "2e73a4ad-332d-41fc-90e2-16bef06523f2";
const SPP_UUID: &str = "00001101-0000-1000-8000-00805f9b34fb";
/// Solo lo anuncian los Buds originales, que hablan las tramas antiguas.
const LEGACY_UUID: &str = "00001102-0000-1000-8000-00805f9b34fd";

/// Un objeto por UUID, registrados a la vez y para siempre. Ajustes daba de
/// baja y volvía a registrar el mismo objeto con otro UUID en cada intento, y
/// entre medias BlueZ podía entregar el descriptor a un perfil ya borrado.
const PERFILES: [(&str, &str); 2] = [
    ("/org/bookos/Buds/perfil/samsung", SAMSUNG_UUID),
    ("/org/bookos/Buds/perfil/spp", SPP_UUID),
];

/// Lo que dura como mucho un intento: `ConnectProfile` más la entrega del
/// descriptor. Un `connect` RFCOMM a unos Buds que no contestan tardó 4,97 s
/// en fallar y 0,15 s en conectar con el enlace ya levantado (medido con los
/// Buds en el estuche abierto), así que 10 s deja margen para el aviso de
/// radio sin colgar la reconexión.
const LIMITE_INTENTO: Duration = Duration::from_secs(10);

pub type Pendiente = Arc<Mutex<Option<oneshot::Sender<OwnedFd>>>>;

struct Perfil {
    pendiente: Pendiente,
}

#[interface(name = "org.bluez.Profile1")]
impl Perfil {
    // 0.1.0
    async fn new_connection(
        &self,
        _device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        _props: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        let fd = OwnedFd::from(fd);
        // BlueZ lo entrega no bloqueante, y el lector hace `recv` bloqueante.
        // SAFETY: fcntl sobre un descriptor que es nuestro.
        unsafe {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL, 0);
            if flags >= 0 {
                libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }
        let quien = self
            .pendiente
            .lock()
            .expect("nadie entra en pánico con el pendiente tomado")
            .take();
        match quien {
            Some(tx) => {
                let _ = tx.send(fd);
            }
            // Nadie lo pidió (o el intento ya caducó): se cierra al soltarlo.
            None => eprintln!("budsd: BlueZ entregó un canal que nadie esperaba; se cierra"),
        }
        Ok(())
    }

    // 0.1.0
    async fn release(&self) {}

    // 0.1.0
    async fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

/// Publica los objetos de perfil en el bus de sistema. El registro en BlueZ
/// se hace en cada intento (ver [`abrir`]) para sobrevivir a un reinicio de
/// `bluetoothd`, que olvida los perfiles registrados.
// 0.1.0
pub async fn publicar_perfiles(sistema: &Connection) -> zbus::Result<Pendiente> {
    let pendiente: Pendiente = Arc::new(Mutex::new(None));
    for (ruta, _) in PERFILES {
        sistema
            .object_server()
            .at(
                ruta,
                Perfil {
                    pendiente: Arc::clone(&pendiente),
                },
            )
            .await?;
    }
    Ok(pendiente)
}

/// Lo que se sabe del aparato antes de abrir el canal.
pub struct Aparato {
    pub ruta: OwnedObjectPath,
    pub mac: String,
    pub nombre: String,
    pub legacy: bool,
    /// Si es de la familia Galaxy Buds. Por el UUID de Samsung y, para los
    /// originales que no lo anuncian, por el nombre, como hacía Ajustes.
    pub buds: bool,
}

// 0.1.0
pub async fn aparato(sistema: &Connection, ruta: OwnedObjectPath) -> Result<Aparato, String> {
    let dev = zbus::Proxy::new(sistema, "org.bluez", ruta.clone(), "org.bluez.Device1")
        .await
        .map_err(|e| e.to_string())?;
    let mac: String = dev
        .get_property("Address")
        .await
        .map_err(|e| e.to_string())?;
    let nombre = dev
        .get_property::<String>("Name")
        .await
        .unwrap_or_else(|_| "Galaxy Buds".into());
    let uuids: Vec<String> = dev.get_property("UUIDs").await.unwrap_or_default();
    let legacy = uuids.iter().any(|u| u.eq_ignore_ascii_case(LEGACY_UUID));
    let buds = uuids.iter().any(|u| u.eq_ignore_ascii_case(SAMSUNG_UUID))
        || nombre.to_lowercase().contains("buds");
    Ok(Aparato {
        ruta,
        mac,
        nombre,
        legacy,
        buds,
    })
}

/// La ruta de BlueZ de un aparato por su MAC, sin suponer `hci0`.
// 0.1.0
pub async fn ruta_de(sistema: &Connection, mac: &str) -> Result<OwnedObjectPath, String> {
    let objetos: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        zbus::Proxy::new(
            sistema,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await
        .map_err(|e| e.to_string())?
        .call("GetManagedObjects", &())
        .await
        .map_err(|e| e.to_string())?;
    objetos
        .into_iter()
        .find(|(_, ifaces)| {
            ifaces
                .get("org.bluez.Device1")
                .and_then(|d| d.get("Address"))
                .and_then(|v| <&str>::try_from(v).ok())
                .is_some_and(|a| a.eq_ignore_ascii_case(mac))
        })
        .map(|(ruta, _)| ruta)
        .ok_or_else(|| format!("BlueZ no conoce {mac}: ¿están emparejados?"))
}

/// Un intento de abrir el canal: registra los perfiles y pide a BlueZ que
/// conecte, primero el UUID de Samsung y si no el SPP genérico.
///
/// No hay barrido de canales RFCOMM 1-30 como en Ajustes: cada canal que no
/// contesta cuesta 4,97 s (medido), así que el barrido podía bloquear más de
/// dos minutos, y BlueZ ya sabe por SDP qué canal es el bueno.
// 0.1.0
pub async fn abrir(
    sistema: &Connection,
    pendiente: &Pendiente,
    ruta: &ObjectPath<'_>,
) -> Result<OwnedFd, String> {
    let pm = zbus::Proxy::new(
        sistema,
        "org.bluez",
        "/org/bluez",
        "org.bluez.ProfileManager1",
    )
    .await
    .map_err(|e| e.to_string())?;
    let dev = zbus::Proxy::new(sistema, "org.bluez", ruta, "org.bluez.Device1")
        .await
        .map_err(|e| e.to_string())?;

    let mut errores = Vec::new();
    for (objeto, uuid) in PERFILES {
        let opciones: HashMap<&str, Value> = HashMap::from([
            ("Role", Value::from("client")),
            ("RequireAuthentication", Value::from(false)),
            ("RequireAuthorization", Value::from(false)),
            ("AutoConnect", Value::from(false)),
        ]);
        let objeto = ObjectPath::from_static_str_unchecked(objeto);
        match pm
            .call::<_, _, ()>("RegisterProfile", &(&objeto, uuid, opciones))
            .await
        {
            Ok(()) => {}
            Err(zbus::Error::MethodError(nombre, _, _))
                if nombre.as_str() == "org.bluez.Error.AlreadyExists" => {}
            Err(e) => {
                errores.push(e.to_string());
                continue;
            }
        }

        let (tx, rx) = oneshot::channel();
        *pendiente
            .lock()
            .expect("nadie entra en pánico con el pendiente tomado") = Some(tx);
        let inicio = Instant::now();
        let resultado = tokio::time::timeout(LIMITE_INTENTO, async {
            dev.call::<_, _, ()>("ConnectProfile", &(uuid,))
                .await
                .map_err(|e| e.to_string())?;
            rx.await
                .map_err(|_| "BlueZ cerró la conexión sin entregar el canal".to_string())
        })
        .await
        .unwrap_or_else(|_| Err(format!("sin respuesta en {} s", LIMITE_INTENTO.as_secs())));
        pendiente
            .lock()
            .expect("nadie entra en pánico con el pendiente tomado")
            .take();
        eprintln!(
            "budsd: ConnectProfile({uuid}) en {} ms: {}",
            inicio.elapsed().as_millis(),
            resultado.as_ref().map_or_else(|e| e.as_str(), |_| "ok")
        );
        match resultado {
            Ok(fd) => return Ok(fd),
            Err(e) => errores.push(e),
        }
    }
    Err(errores.join("; "))
}

/// Traduce el error de BlueZ a algo que se pueda leer en Ajustes, sin
/// esconder el original.
// 0.1.0
pub fn error_legible(crudo: &str) -> String {
    let c = crudo.to_ascii_lowercase();
    let motivo = if c.contains("refused") || c.contains("busy") {
        // Hipótesis aún sin confirmar con los registros (ver el resumen del
        // cambio): el firmware solo admite un cliente del canal de control, y
        // Galaxy Wearable en el móvil lo tiene ocupado.
        "Los auriculares rechazaron el canal de control. Suele pasar si el móvil \
         tiene Galaxy Wearable conectado a ellos."
    } else if c.contains("host is down")
        || c.contains("page timeout")
        || c.contains("sin respuesta")
    {
        "Los auriculares no responden. ¿Están fuera del estuche y cerca?"
    } else if c.contains("not connected")
        || c.contains("notavailable")
        || c.contains("not available")
    {
        "Los auriculares no están conectados por Bluetooth."
    } else {
        "No se pudo abrir el canal de control de los auriculares."
    };
    format!("{motivo} (BlueZ: {crudo})")
}

/// Lo que BlueZ y los hilos del canal cuentan al servicio.
pub enum Aviso {
    /// Cambió `Connected` en un aparato de BlueZ: el audio, no el canal.
    Audio {
        ruta: OwnedObjectPath,
        conectado: bool,
    },
    /// Llegó un estado nuevo (trae la batería).
    Estado(String),
    /// El socket se cerró por el otro lado.
    Caido(String),
}

pub struct Conexion {
    fd: OwnedFd,
    pub legacy: bool,
    pub mac: String,
    pub status: Arc<Mutex<BudsStatus>>,
    stop: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Conexion {
    // 0.1.0
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // `shutdown` actúa sobre el socket, no sobre este descriptor: despierta
        // el `recv` del lector y hace fallar el `send` del keepalive aunque
        // cada uno tenga su propia copia.
        // SAFETY: el descriptor es nuestro y sigue abierto.
        unsafe { libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_RDWR) };
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

// 0.1.0
fn enviar_fd(fd: &OwnedFd, msg_id: u8, payload: &[u8], legacy: bool) -> std::io::Result<()> {
    let pkt = protocolo::encode(msg_id, payload, legacy);
    // SAFETY: búfer válido durante la llamada; MSG_NOSIGNAL evita SIGPIPE si
    // los Buds ya cerraron.
    let n = unsafe {
        libc::send(
            fd.as_raw_fd(),
            pkt.as_ptr().cast(),
            pkt.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Conexion {
    /// Hace el saludo, reaplica las preferencias guardadas y arranca lector y
    /// keepalive.
    // 0.1.0
    pub fn iniciar(
        fd: OwnedFd,
        aparato: &Aparato,
        avisos: mpsc::UnboundedSender<Aviso>,
    ) -> Result<Self, String> {
        let legacy = aparato.legacy;
        let lector = fd.try_clone().map_err(|e| e.to_string())?;
        let vivo = fd.try_clone().map_err(|e| e.to_string())?;

        // MANAGER_INFO: [1, 2 (ClientDeviceTypes::Other), 34 (SDK de Android)].
        // Sin él el firmware no contesta.
        enviar_fd(&fd, protocolo::MSG_MANAGER_INFO, &[1, 2, 34], legacy)
            .map_err(|e| e.to_string())?;
        enviar_fd(&fd, protocolo::MSG_EXTENDED_STATUS, &[], legacy).map_err(|e| e.to_string())?;

        // Los Buds no guardan EQ, ANC ni bloqueo táctil entre encendidos.
        let prefs = protocolo::get_prefs(&aparato.mac);
        enviar_fd(
            &fd,
            protocolo::MSG_EQUALIZER,
            &payload_eq(prefs.eq_preset, prefs.eq_enabled, legacy),
            legacy,
        )
        .map_err(|e| e.to_string())?;
        enviar_fd(
            &fd,
            protocolo::MSG_NOISE_CONTROLS,
            &[prefs.anc_mode],
            legacy,
        )
        .map_err(|e| e.to_string())?;
        enviar_fd(
            &fd,
            protocolo::MSG_LOCK_TOUCHPAD,
            &[prefs.touchpad_locked as u8],
            legacy,
        )
        .map_err(|e| e.to_string())?;

        let status = Arc::new(Mutex::new(BudsStatus {
            connected: true,
            model: aparato.nombre.clone(),
            mac: aparato.mac.clone(),
            anc_mode: prefs.anc_mode,
            eq_preset: prefs.eq_preset,
            eq_enabled: prefs.eq_enabled,
            touchpad_locked: prefs.touchpad_locked,
            battery_case: prefs.bateria_estuche,
            battery_case_guardada: prefs.bateria_estuche > 0,
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let reader = {
            let status = Arc::clone(&status);
            let stop = Arc::clone(&stop);
            let mac = aparato.mac.clone();
            std::thread::spawn(move || leer(lector, mac, status, stop, avisos))
        };

        // Sin tráfico, el firmware cierra el canal por inactividad.
        let stop_ka = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop_ka.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(5));
                if stop_ka.load(Ordering::SeqCst)
                    || enviar_fd(&vivo, protocolo::MSG_MANAGER_INFO, &[1, 2, 34], legacy).is_err()
                    || enviar_fd(&vivo, protocolo::MSG_EXTENDED_STATUS, &[], legacy).is_err()
                {
                    break;
                }
            }
        });

        Ok(Self {
            fd,
            legacy,
            mac: aparato.mac.clone(),
            status,
            stop,
            reader: Some(reader),
        })
    }

    // 0.1.0
    pub fn enviar(&self, msg_id: u8, payload: &[u8]) -> Result<(), String> {
        enviar_fd(&self.fd, msg_id, payload, self.legacy)
            .map_err(|e| format!("No se pudo enviar a los auriculares: {e}"))
    }

    // 0.1.0
    pub fn estado(&self) -> BudsStatus {
        self.status
            .lock()
            .expect("nadie entra en pánico con el estado tomado")
            .clone()
    }
}

/// Las tramas antiguas llevan `[activado, preset + 5]`; el resto, `0` para
/// plano o `preset + 1`.
// 0.1.0
pub fn payload_eq(preset: u8, enabled: bool, legacy: bool) -> Vec<u8> {
    if legacy {
        vec![enabled as u8, preset.saturating_add(5)]
    } else {
        vec![if enabled { preset.saturating_add(1) } else { 0 }]
    }
}

// 0.1.0
fn leer(
    fd: OwnedFd,
    mac: String,
    status: Arc<Mutex<BudsStatus>>,
    stop: Arc<AtomicBool>,
    avisos: mpsc::UnboundedSender<Aviso>,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    while !stop.load(Ordering::SeqCst) {
        // SAFETY: `tmp` vive durante la llamada y su longitud es la que se pasa.
        let n = unsafe { libc::recv(fd.as_raw_fd(), tmp.as_mut_ptr().cast(), tmp.len(), 0) };
        if n <= 0 {
            if n < 0 {
                eprintln!("budsd: recv: {}", std::io::Error::last_os_error());
            }
            break;
        }
        buf.extend_from_slice(&tmp[..n as usize]);
        let mut hubo_estado = false;
        {
            let mut g = status
                .lock()
                .expect("nadie entra en pánico con el estado tomado");
            for p in protocolo::parse_packets(&mut buf) {
                hubo_estado |= protocolo::aplicar(&p, &mut g);
            }
            g.connected = true;
        }
        if hubo_estado {
            let _ = avisos.send(Aviso::Estado(mac.clone()));
        }
    }
    status
        .lock()
        .expect("nadie entra en pánico con el estado tomado")
        .connected = false;
    // Si lo cerramos nosotros no es una caída.
    if !stop.load(Ordering::SeqCst) {
        let _ = avisos.send(Aviso::Caido(mac));
    }
}
