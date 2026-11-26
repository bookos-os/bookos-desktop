//! El backend de `xdg-desktop-portal` para compartir pantalla y capturarla.
//!
//! Es lo que hace que «compartir pantalla» funcione desde un navegador: Meet,
//! Discord u OBS no hablan Wayland para esto, hablan
//! `org.freedesktop.portal.ScreenCast`, y `xdg-desktop-portal` reenvía a la
//! interfaz `impl` que implementa este fichero.
//!
//! ## Por qué va dentro del compositor y no en un proceso aparte
//!
//! Un backend externo —`xdg-desktop-portal-wlr`, `-gnome`, `-kde`— es un
//! cliente Wayland más: para ver la pantalla tiene que pedirla por
//! `zwlr_screencopy`, recibirla en memoria compartida y volver a subirla a
//! PipeWire. Eso es una copia de ida y otra de vuelta sobre la que ya duele.
//! Aquí el fotograma sale del mismo composite que ya se hace ([`crate::backend::servir_capturas`])
//! y va directo al hilo de PipeWire.
//!
//! Y el diálogo de permiso lo dibuja el propio escritorio. Un backend externo
//! tendría que traerse GTK o Qt para enseñar una tarjeta; el shell ya está en
//! este proceso.
//!
//! ## Cómo se reparte el trabajo
//!
//! Igual que [`crate::notificaciones`]: `zbus` bloqueante en su propio hilo y
//! un canal de `calloop` hacia el bucle de frames. Este hilo **no toca nada
//! gráfico**; manda un [`Aviso`] y espera la respuesta.
//!
//! Esperar **no puede ser bloqueando**. zbus atiende toda la conexión en un
//! ejecutor de un solo hilo, así que un método síncrono parado deja muerto el
//! portal entero: medido, con el diálogo del permiso abierto ni siquiera se
//! podía leer una propiedad —`busctl ... Properties Get` daba tiempo de espera
//! a los 8 s—, y por tanto `xdg-desktop-portal` tampoco podía cancelar ni
//! abrir una segunda sesión. Por eso los métodos que esperan son `async` y
//! esperan sobre [`Promesa`], que es un `Future` de verdad: mientras el
//! usuario mira el diálogo, la conexión sigue atendiendo lo demás.
//!
//! También sirve `org.freedesktop.impl.portal.Settings`, que es por donde las
//! aplicaciones **de fuera** —GTK, Qt, Tauri— se enteran del tema, del acento y
//! de los efectos reducidos de BookOS. Ver [`Ajustes`].
//!
//! ## Lo que todavía no hay
//!
//! - **Solo pantallas enteras**, no ventanas sueltas: compartir una ventana
//!   pide componerla en su propio buffer, con su propio seguimiento de daño.
//!   Por eso `AvailableSourceTypes` anuncia solo `MONITOR`.
//! - **El puntero siempre sale.** `escena()` lo pinta como un elemento más;
//!   quitarlo obligaría a componer la salida dos veces. `AvailableCursorModes`
//!   anuncia solo `EMBEDDED`, que es la verdad.
//! - La interacción de `ScreenCast.Start` publica un objeto `Request`: cerrar
//!   la petición retira inmediatamente el diálogo y el nodo en preparación.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use smithay::reexports::calloop::channel::Sender;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const NOMBRE: &str = "org.bookos.portal.desktop";
const RUTA: &str = "/org/freedesktop/portal/desktop";

/// Códigos de respuesta del protocolo: 0 correcto, 1 cancelado por el usuario,
/// 2 fallo.
const CORRECTO: u32 = 0;
const CANCELADO: u32 = 1;
const FALLO: u32 = 2;

/// El buzón compartido entre quien espera y quien contesta.
struct Buzon<T> {
    valor: Option<T>,
    /// El emisario se fue sin dejar nada. No es un error: significa «no», y es
    /// lo que ocurre cuando el diálogo se cierra de cualquier otra forma.
    cerrado: bool,
    waker: Option<Waker>,
}

/// El extremo que contesta. Vive en el hilo del compositor.
///
/// Contesta una sola vez —[`Self::entregar`] lo consume— y si se suelta sin
/// contestar, quien esperaba recibe `None`. Eso es lo que hace que ninguna
/// espera pueda quedarse colgada sin necesidad de un plazo inventado: todos los
/// caminos o contestan o sueltan.
pub struct Emisario<T>(Arc<Mutex<Buzon<T>>>);

impl<T> Emisario<T> {
    // 0.1.0
    pub fn entregar(self, valor: T) {
        // El bloque cierra el préstamo antes de que corra el `Drop` de `self`,
        // que vuelve a coger el mismo mutex.
        {
            let mut buzon = self
                .0
                .lock()
                .expect("el buzón no se comparte con nada que entre en pánico");
            buzon.valor = Some(valor);
            if let Some(waker) = buzon.waker.take() {
                waker.wake();
            }
        }
    }
}

impl<T> Drop for Emisario<T> {
    // 0.1.0
    fn drop(&mut self) {
        let mut buzon = self
            .0
            .lock()
            .expect("el buzón no se comparte con nada que entre en pánico");
        buzon.cerrado = true;
        if let Some(waker) = buzon.waker.take() {
            waker.wake();
        }
    }
}

/// El extremo que espera. Se `.await`ea desde un método del portal.
pub struct Promesa<T>(Arc<Mutex<Buzon<T>>>);

impl<T> std::future::Future for Promesa<T> {
    type Output = Option<T>;

    // 0.1.0
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut buzon = self
            .0
            .lock()
            .expect("el buzón no se comparte con nada que entre en pánico");
        if let Some(valor) = buzon.valor.take() {
            return Poll::Ready(Some(valor));
        }
        if buzon.cerrado {
            return Poll::Ready(None);
        }
        buzon.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

// 0.1.0
pub fn promesa<T>() -> (Emisario<T>, Promesa<T>) {
    let buzon = Arc::new(Mutex::new(Buzon {
        valor: None,
        cerrado: false,
        waker: None,
    }));
    (Emisario(buzon.clone()), Promesa(buzon))
}

/// Lo que el hilo del portal le pide al bucle de frames.
pub enum Aviso {
    /// Enseña el diálogo de permiso y, si el usuario acepta, abre el nodo.
    Consentir {
        sesion: u32,
        ruta: OwnedObjectPath,
        app: String,
        /// El tamaño en píxeles del buffer de la salida elegida. Soltarlo sin
        /// contestar es la negativa.
        respuesta: Emisario<(u32, u32)>,
        /// Va tal cual al hilo de PipeWire, que contesta con el `node_id`.
        nodo: Emisario<u32>,
    },
    /// La aplicación cerró la sesión.
    Cerrar { sesion: u32 },
    /// El portal canceló la interacción antes de que `Start` terminara.
    Cancelar { sesion: u32 },
    /// Un socket de PipeWire ya conectado para dárselo al cliente.
    Descriptor { respuesta: Emisario<OwnedFd> },
    /// Una captura de la pantalla entera a un fichero.
    Captura { respuesta: Emisario<PathBuf> },
    /// Elegir un píxel con el cuentagotas del compositor.
    Color {
        id: u32,
        respuesta: Emisario<[u8; 3]>,
    },
    /// Cancelar un `PickColor` en curso.
    CancelarColor { id: u32 },
    /// Diálogo de permiso `org.freedesktop.impl.portal.Access`.
    Acceso {
        id: u32,
        peticion: bookos_shell::PeticionAcceso,
        respuesta: Emisario<Vec<(String, String)>>,
    },
    /// Cancelar un diálogo `Access` abierto.
    CancelarAcceso { id: u32 },
    /// Selector «Abrir con…» (`org.freedesktop.impl.portal.AppChooser`).
    ElegirApp {
        id: u32,
        peticion: bookos_shell::PeticionElegirApp,
        respuesta: Emisario<String>,
    },
    /// Actualización dinámica de opciones de un `AppChooser` abierto.
    ActualizarApps { id: u32, opciones: Vec<String> },
    /// Cancelar un selector `AppChooser` abierto.
    CancelarElegirApp { id: u32 },
}

/// Lo que el hilo guarda entre llamadas.
struct Estado {
    canal: Sender<Aviso>,
    /// De la ruta de sesión del portal al identificador que conoce el hilo de
    /// PipeWire. Se comparte con las tarjetas de `Session`, que solo saben la
    /// suya.
    sesiones: Mutex<HashMap<OwnedObjectPath, u32>>,
    /// Peticiones `AppChooser` vivas para resolver `UpdateChoices(handle, choices)`.
    elegir_apps: Mutex<HashMap<OwnedObjectPath, u32>>,
    siguiente: AtomicU32,
}

impl Estado {
    // 0.1.0
    fn avisar(&self, aviso: Aviso) -> bool {
        self.canal.send(aviso).is_ok()
    }

    fn siguiente_id(&self) -> u32 {
        loop {
            let id = self.siguiente.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }
}

struct ScreenCast {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    /// Solo pantallas enteras (`MONITOR`).
    #[zbus(property)]
    // 0.1.0
    fn available_source_types(&self) -> u32 {
        1
    }

    /// Solo `EMBEDDED`: el cursor va pintado dentro del fotograma y no hay
    /// forma de sacarlo aparte sin componer la salida dos veces.
    #[zbus(property)]
    // 0.1.0
    fn available_cursor_modes(&self) -> u32 {
        2
    }

    /// En minúscula: así se llama la propiedad en el XML del portal, y zbus
    /// convertiría `version` en `Version` si no se le dice.
    #[zbus(property, name = "version")]
    // 0.1.0
    fn version(&self) -> u32 {
        4
    }

    // 0.1.0
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let sesion = self.estado.siguiente_id();
        let ruta: OwnedObjectPath = session_handle.to_owned().into();
        self.estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .insert(ruta.clone(), sesion);

        // La tarjeta `Session` vive en la ruta que manda el portal; es por
        // donde llega el `Close` cuando la aplicación termina la llamada.
        let tarjeta = Sesion {
            estado: self.estado.clone(),
            ruta: ruta.clone(),
        };
        if let Err(err) = servidor.at(ruta.as_str(), tarjeta).await {
            self.estado
                .sesiones
                .lock()
                .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
                .remove(&ruta);
            tracing::warn!("no se pudo publicar la sesión del portal: {err}");
            return (FALLO, HashMap::new());
        }
        tracing::info!(sesion, %ruta, "sesión de captura creada");
        (CORRECTO, HashMap::new())
    }

    /// No hay nada que elegir todavía: la pantalla se elige en el diálogo de
    /// `Start`, que es donde el usuario está mirando. Se acepta y ya.
    // 0.1.0
    fn select_sources(
        &self,
        _handle: ObjectPath<'_>,
        _session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        (CORRECTO, HashMap::new())
    }

    // 0.1.0
    async fn start(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let ruta: OwnedObjectPath = session_handle.to_owned().into();
        let Some(sesion) = self
            .estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .get(&ruta)
            .copied()
        else {
            tracing::warn!(%ruta, "Start sobre una sesión que no existe");
            return (FALLO, HashMap::new());
        };

        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            ruta: ruta_solicitud.clone(),
            cancelada: cancelada.clone(),
            al_cerrar: AlCerrar::Captura {
                estado: self.estado.clone(),
                sesion,
            },
        };
        if let Err(err) = servidor.at(ruta_solicitud.as_str(), solicitud).await {
            tracing::warn!(%ruta_solicitud, "no se pudo publicar Request: {err}");
            return (FALLO, HashMap::new());
        }

        let (respuesta, espera) = promesa();
        let (nodo, espera_nodo) = promesa();
        let app = nombre_legible(app_id);
        if !self.estado.avisar(Aviso::Consentir {
            sesion,
            ruta,
            app,
            respuesta,
            nodo,
        }) {
            let _ = servidor
                .remove::<Solicitud, _>(ruta_solicitud.as_str())
                .await;
            return (FALLO, HashMap::new());
        }

        let respuesta_usuario = espera.await;
        let resultado = if cancelada.load(Ordering::Acquire) {
            (CANCELADO, HashMap::new())
        } else if let Some((ancho, alto)) = respuesta_usuario {
            match espera_nodo.await {
                Some(node_id) if !cancelada.load(Ordering::Acquire) => {
                    respuesta_start(sesion, node_id, ancho, alto)
                }
                Some(_) => {
                    self.estado.avisar(Aviso::Cerrar { sesion });
                    (CANCELADO, HashMap::new())
                }
                None => {
                    tracing::warn!(sesion, "PipeWire no dio identificador de nodo");
                    self.estado.avisar(Aviso::Cerrar { sesion });
                    (FALLO, HashMap::new())
                }
            }
        } else {
            (CANCELADO, HashMap::new())
        };

        if let Err(err) = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await
        {
            tracing::debug!(%ruta_solicitud, "Request ya retirado: {err}");
        }
        resultado
    }

    // 0.1.0
    async fn open_pipe_wire_remote(
        &self,
        _session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Descriptor { respuesta }) {
            return Err(zbus::fdo::Error::Failed(
                "el compositor se está cerrando".into(),
            ));
        }
        match espera.await {
            Some(fd) => Ok(fd.into()),
            None => Err(zbus::fdo::Error::Failed(
                "no se pudo abrir una conexión a PipeWire".into(),
            )),
        }
    }
}

// 0.1.0
fn respuesta_start(
    sesion: u32,
    node_id: u32,
    ancho: u32,
    alto: u32,
) -> (u32, HashMap<String, OwnedValue>) {
    // `streams` es `a(ua{sv})`: una tupla por flujo con su nodo y sus
    // propiedades. `source_type = 1` es MONITOR.
    let mut props: HashMap<String, Value<'_>> = HashMap::new();
    props.insert("size".into(), Value::from((ancho as i32, alto as i32)));
    props.insert("source_type".into(), Value::from(1u32));
    let flujos = vec![(node_id, props)];
    let mut resultados = HashMap::new();
    match OwnedValue::try_from(Value::from(flujos)) {
        Ok(v) => {
            resultados.insert("streams".to_string(), v);
        }
        Err(err) => {
            tracing::warn!("no se pudo empaquetar la lista de flujos: {err}");
            return (FALLO, HashMap::new());
        }
    }
    tracing::info!(sesion, node_id, ancho, alto, "compartiendo pantalla");
    (CORRECTO, resultados)
}

/// Objeto efímero que permite a xdg-desktop-portal cancelar una interacción
/// —el diálogo de `Start` o el selector de archivos— mientras su llamada
/// asíncrona sigue esperando.
struct Solicitud {
    ruta: OwnedObjectPath,
    cancelada: Arc<AtomicBool>,
    al_cerrar: AlCerrar,
}

/// Qué hay que deshacer cuando el portal cancela.
enum AlCerrar {
    /// Retirar el diálogo de permiso y el nodo en preparación.
    Captura { estado: Arc<Estado>, sesion: u32 },
    /// Cerrar la ventana del selector, que es un proceso aparte.
    Selector { pid: u32 },
    /// Cancelar el cuentagotas lanzado para `PickColor`.
    Color { estado: Arc<Estado>, id: u32 },
    /// Cerrar el diálogo modal de `Access`.
    Acceso { estado: Arc<Estado>, id: u32 },
    /// Cerrar el selector «Abrir con…» (`AppChooser`).
    ElegirApp { estado: Arc<Estado>, id: u32 },
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Solicitud {
    // 0.1.0
    async fn close(&self, #[zbus(object_server)] servidor: &zbus::ObjectServer) {
        self.cancelada.store(true, Ordering::Release);
        match &self.al_cerrar {
            AlCerrar::Captura { estado, sesion } => {
                estado.avisar(Aviso::Cancelar { sesion: *sesion });
            }
            // Hay una ventana entre que el hilo recoge al hijo y se retira esta
            // `Solicitud` en la que el pid ya no es nuestro. Es de
            // microsegundos y el kernel no recicla un pid tan deprisa; cerrarla
            // del todo pediría `pidfd`, y no compensa para un `SIGTERM`.
            AlCerrar::Selector { pid } => {
                // SAFETY: `kill` no toca memoria; el peor caso es ESRCH si el
                // hijo acaba de salir por su cuenta.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGTERM) };
            }
            AlCerrar::Color { estado, id } => {
                estado.avisar(Aviso::CancelarColor { id: *id });
            }
            AlCerrar::Acceso { estado, id } => {
                estado.avisar(Aviso::CancelarAcceso { id: *id });
            }
            AlCerrar::ElegirApp { estado, id } => {
                estado.avisar(Aviso::CancelarElegirApp { id: *id });
            }
        }
        if let Err(err) = servidor.remove::<Solicitud, _>(self.ruta.as_str()).await {
            tracing::debug!(ruta = %self.ruta, "Request ya retirado: {err}");
        }
    }
}

/// La sesión que el portal cierra cuando la aplicación termina.
struct Sesion {
    estado: Arc<Estado>,
    ruta: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Sesion {
    #[zbus(property, name = "version")]
    // 0.1.0
    fn version(&self) -> u32 {
        1
    }

    // 0.1.0
    async fn close(&self, #[zbus(object_server)] servidor: &zbus::ObjectServer) {
        let sesion = self
            .estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .remove(&self.ruta);
        if let Some(sesion) = sesion {
            tracing::info!(sesion, "la aplicación cerró la sesión de captura");
            // modified 0.2.0 -- `Cancelar` y no `Cerrar`: si la tarjeta de
            // permiso de esta sesión sigue abierta, hay que retirarla. Vesktop
            // cierra una sesión y abre otra en 65 ms; con la tarjeta vieja
            // puesta, el `Start` nuevo fallaba por «un diálogo cada vez» y lo
            // que se aceptaba era la sesión muerta: pantalla negra en Discord.
            self.estado.avisar(Aviso::Cancelar { sesion });
        }
        // Se retira la tarjeta de su ruta: si no, una sesión nueva con la misma
        // ruta —el portal las reutiliza— chocaría con esta.
        if let Err(err) = servidor.remove::<Sesion, _>(self.ruta.as_str()).await {
            tracing::warn!(ruta = %self.ruta, "no se pudo retirar la sesión del portal: {err}");
        }
    }
}

/// El aspecto del sistema, para las aplicaciones que no son de BookOS.
///
/// Es lo que hace que una app **GTK o Tauri** siga el tema y el acento del
/// escritorio sin instalarle nada: GTK 4 y libadwaita ≥ 1.6 preguntan aquí al
/// arrancar y se quedan escuchando la señal, así que cambiar el acento en
/// Apariencia les llega en caliente. Tauri va sobre WebKitGTK, o sea que
/// también le llega —su CSS ve el `prefers-color-scheme` correcto—.
///
/// Es mejor que el tema GTK de `BookOS-GTK-Theme`, que se instala copiando
/// ficheros y solo trae dos acentos fijos: aquí van los diez y cambian sin
/// reiniciar la aplicación.
///
/// **Los valores no se guardan aquí.** El tema, el acento, el contraste y los efectos
/// reducidos son estáticos atómicos del proceso (`tema::actual()`,
/// `tema::acento()`, `tema::efectos_reducidos()`), así que este hilo los lee
/// directamente sin canal ni copia que se pueda quedar vieja.
struct Ajustes;

/// El espacio de nombres que miran GTK, Qt y quien siga la especificación.
const APARIENCIA: &str = "org.freedesktop.appearance";

impl Ajustes {
    /// `color-scheme`: 0 sin preferencia, 1 oscuro, 2 claro.
    // 0.1.0
    fn color_scheme() -> u32 {
        if bookos_shell::tema::es_claro() { 2 } else { 1 }
    }

    /// `accent-color` es una tupla de tres dobles de 0 a 1, no un entero de 32
    /// bits: comprobado contra lo que sirve el portal de KDE en esta máquina.
    // 0.1.0
    fn accent_color() -> (f64, f64, f64) {
        let c = bookos_shell::tema::acento();
        (c.r as f64, c.g as f64, c.b as f64)
    }

    /// `reduced-motion`: 0 no, 1 sí. Sale de los efectos reducidos de BookOS,
    /// así que apagarlos en el escritorio también calma las animaciones de las
    /// aplicaciones que lo respetan.
    // 0.1.0
    fn reduced_motion() -> u32 {
        u32::from(bookos_shell::tema::efectos_reducidos())
    }

    // 0.1.0
    fn valores() -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        let mete = |m: &mut HashMap<String, OwnedValue>, k: &str, v: Value<'_>| {
            match OwnedValue::try_from(v) {
                Ok(v) => {
                    m.insert(k.to_string(), v);
                }
                Err(err) => tracing::warn!(k, "no se pudo empaquetar el ajuste: {err}"),
            }
        };
        mete(&mut m, "color-scheme", Value::from(Self::color_scheme()));
        mete(&mut m, "accent-color", Value::from(Self::accent_color()));
        mete(
            &mut m,
            "reduced-motion",
            Value::from(Self::reduced_motion()),
        );
        mete(
            &mut m,
            "contrast",
            Value::from(u32::from(bookos_shell::tema::alto_contraste())),
        );
        m
    }

    // 0.1.0
    fn uno(clave: &str) -> Option<OwnedValue> {
        let v = match clave {
            "color-scheme" => Value::from(Self::color_scheme()),
            "accent-color" => Value::from(Self::accent_color()),
            "reduced-motion" => Value::from(Self::reduced_motion()),
            "contrast" => Value::from(u32::from(bookos_shell::tema::alto_contraste())),
            _ => return None,
        };
        OwnedValue::try_from(v).ok()
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Ajustes {
    #[zbus(property, name = "version")]
    // 0.1.0
    fn version(&self) -> u32 {
        2
    }

    // 0.1.0
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        // La lista admite comodines por prefijo y la vacía significa «todo».
        let quiere = namespaces.is_empty()
            || namespaces
                .iter()
                .any(|n| n.is_empty() || APARIENCIA.starts_with(n.trim_end_matches('*')));
        let mut fuera = HashMap::new();
        if quiere {
            fuera.insert(APARIENCIA.to_string(), Self::valores());
        }
        fuera
    }

    /// `ReadOne` es la de la versión 2. `Read` hace lo mismo y sigue viva
    /// porque las aplicaciones antiguas solo conocen esa.
    // 0.1.0
    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace != APARIENCIA {
            return Err(zbus::fdo::Error::UnknownProperty(format!(
                "BookOS no sirve el espacio «{namespace}»"
            )));
        }
        Self::uno(key)
            .ok_or_else(|| zbus::fdo::Error::UnknownProperty(format!("no hay ajuste «{key}»")))
    }

    // 0.1.0
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        self.read_one(namespace, key)
    }

    #[zbus(signal)]
    // 0.1.0
    async fn setting_changed(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

/// Avisa a las aplicaciones de que el aspecto ha cambiado.
///
/// Se llama desde el bucle de frames al aplicar apariencia o efectos. Sin la
/// señal, una app GTK ya abierta se queda con el tema con el que arrancó.
// modified 0.2.2 -- avisa también por `org.bookos.Desktop.AppearanceChanged`
pub fn apariencia_cambiada(state: &crate::state::BookosComp, sincronizar_iconos: bool) {
    tema_gtk(sincronizar_iconos);
    crate::ajustes::aspecto_cambiado(state.bus_ajustes.as_ref());
    let Some(conexion) = state.bus_portal.as_ref() else {
        return;
    };
    for (clave, valor) in [
        ("color-scheme", Value::from(Ajustes::color_scheme())),
        ("accent-color", Value::from(Ajustes::accent_color())),
        ("reduced-motion", Value::from(Ajustes::reduced_motion())),
        (
            "contrast",
            Value::from(u32::from(bookos_shell::tema::alto_contraste())),
        ),
    ] {
        if let Err(err) = conexion.emit_signal(
            None::<&str>,
            RUTA,
            "org.freedesktop.impl.portal.Settings",
            "SettingChanged",
            &(APARIENCIA, clave, valor),
        ) {
            tracing::warn!(clave, "no se pudo avisar del cambio de aspecto: {err}");
        }
    }
}

/// Pone el tema GTK que corresponde al tema y al acento del escritorio.
///
/// El `color-scheme` del portal no basta: GTK 3 no lo lee. LibreOffice —que va
/// con `libvclplug_gtk3lo`— y el resto de apps GTK 3 se guían por el **nombre**
/// del tema en GSettings y en `settings.ini`, y ahí se quedaba escrito
/// `BookOS-Dark-Blue` para siempre: cambiar a claro no les llegaba. GSettings
/// avisa en caliente a las que ya están abiertas; `settings.ini` es lo que leen
/// las que arrancan sin dconf.
///
/// Solo en la sesión de BookOS: el compositor anidado corre dentro de KDE y
/// esto reescribiría el tema GTK del anfitrión.
// 0.1.0
// modified 0.1.1 -- pasa el tema y tamaño de cursor a GTK y recoge los gsettings sin dejar zombis
// modified 0.2.3 -- consulta gsettings, carga config y reescribe settings.ini en segundo plano para no bloquear el hilo del compositor
static GENERACION_GTK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ICONOS_GTK_PENDIENTES: AtomicBool = AtomicBool::new(false);
static BLOQUEO_GTK: Mutex<()> = Mutex::new(());

fn elegir_iconos_gtk(claro: bool, actual: Option<String>, sincronizar_iconos: bool) -> String {
    if sincronizar_iconos {
        match actual {
            Some(t) if t.starts_with("BookOS-Tinted") => t,
            _ => bookos_shell::icono::tema_por_defecto(claro).to_string(),
        }
    } else {
        actual.unwrap_or_else(|| bookos_shell::icono::tema_por_defecto(claro).to_string())
    }
}

pub fn tema_gtk(sincronizar_iconos: bool) {
    if std::env::var("XDG_CURRENT_DESKTOP").as_deref() != Ok("BookOS") {
        return;
    }
    if sincronizar_iconos {
        ICONOS_GTK_PENDIENTES.store(true, Ordering::Release);
    }
    let generacion = GENERACION_GTK.fetch_add(1, Ordering::AcqRel) + 1;
    let claro = bookos_shell::tema::es_claro();
    let acento = bookos_shell::tema::acento_actual();
    let reducidos = bookos_shell::tema::efectos_reducidos();
    // GTK guarda sus propias preferencias y puede ignorar XCURSOR_THEME:
    // dejarlas en Breeze cambiaba el puntero al entrar en una ventana GTK.
    let cursor = crate::cursor::nombre_tema();
    let _ = std::thread::Builder::new()
        .name("tema-gtk".into())
        .spawn(move || aplicar_tema_gtk(generacion, claro, acento, reducidos, cursor));
}

fn aplicar_tema_gtk(
    generacion: u64,
    claro: bool,
    acento: bookos_shell::tema::Acento,
    reducidos: bool,
    cursor: String,
) {
    let _guard = BLOQUEO_GTK.lock().unwrap_or_else(|e| e.into_inner());
    if GENERACION_GTK.load(Ordering::Acquire) != generacion {
        return;
    }
    let sincronizar_iconos = ICONOS_GTK_PENDIENTES.swap(false, Ordering::AcqRel);
    // Del tema GTK solo hay variantes azul y verde; los acentos que no son
    // verdosos se quedan con la azul, que es la de serie.
    use bookos_shell::tema::Acento;
    let color = match acento {
        Acento::Verde | Acento::Turquesa => "Green",
        _ => "Blue",
    };
    let tema = format!("BookOS-{}-{color}", if claro { "Light" } else { "Dark" });
    // Si cambió el tema claro/oscuro sincronizamos el pack por defecto salvo que
    // estén puestos los tintados; en cambios solo de acento/efectos conservamos
    // el pack que el usuario tuviera puesto en Ajustes.
    let iconos = elegir_iconos_gtk(
        claro,
        bookos_shell::icono::tema_elegido(),
        sincronizar_iconos,
    );
    let iconos = iconos.as_str();
    let config = bookos_shell::Config::cargar();
    let tamano_cursor = config.cursor.to_string();
    // GTK 3 y 4 releen este factor en vivo; es la forma de agrandar el texto de
    // las aplicaciones GTK sin tocar su geometría.
    // añadido en la 0.2.3
    let factor_texto = format!("{:.2}", f64::from(config.texto_tamano) / 100.0);
    let mut hijos = Vec::new();
    for (clave, valor) in [
        ("gtk-theme", tema.as_str()),
        ("icon-theme", iconos),
        ("cursor-theme", cursor.as_str()),
        ("cursor-size", tamano_cursor.as_str()),
        // Las apps GTK no leen `reduced-motion` del portal: lo suyo es esta
        // clave de gsettings.
        // añadido en la 0.2.3
        (
            "enable-animations",
            if reducidos { "false" } else { "true" },
        ),
        ("text-scaling-factor", factor_texto.as_str()),
        (
            "color-scheme",
            if claro { "prefer-light" } else { "prefer-dark" },
        ),
    ] {
        // Sin esperar en el hilo del compositor: `gsettings` habla con dconf por
        // D-Bus y ya estamos en un hilo propio que recoge los procesos al final.
        if let Ok(hijo) = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", clave, valor])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            hijos.push(hijo);
        }
    }
    let sonido = tema_sonido();
    if let Some(sonido) = sonido
        && let Ok(hijo) = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.sound", "theme-name", sonido])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    {
        hijos.push(hijo);
    }
    let oscuro = if claro { "false" } else { "true" };
    if let Some(config_dir) =
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
    {
        for version in ["gtk-3.0", "gtk-4.0"] {
            let ruta = config_dir.join(version).join("settings.ini");
            let mut valores = vec![
                ("gtk-theme-name", tema.as_str()),
                ("gtk-icon-theme-name", iconos),
                ("gtk-cursor-theme-name", cursor.as_str()),
                ("gtk-cursor-theme-size", tamano_cursor.as_str()),
                // Si se quedara en `true`, GTK cargaría `gtk-dark.css` del tema
                // claro y seguiría saliendo oscuro.
                ("gtk-application-prefer-dark-theme", oscuro),
            ];
            if let Some(sonido) = sonido {
                valores.push(("gtk-sound-theme-name", sonido));
            }
            if let Err(err) = reescribir_ini(&ruta, &valores) {
                tracing::warn!(?ruta, "no se pudo escribir el tema GTK: {err}");
            }
        }
    }
    for mut hijo in hijos {
        let _ = hijo.wait();
    }
}

/// El tema de sonido de BookOS, si está instalado.
///
/// Si no, `None` y no se toca nada: nombrar un tema que no existe dejaría a
/// libcanberra cayendo a freedesktop, pero también pisaría el que el usuario
/// tuviera puesto (en esta máquina, `ocean` de KDE).
// 0.1.0
fn tema_sonido() -> Option<&'static str> {
    let mut bases = vec![std::path::PathBuf::from("/usr/share/sounds")];
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(std::path::PathBuf::from(home).join(".local/share/sounds"));
    }
    bases
        .iter()
        .any(|b| b.join("bookos/index.theme").is_file())
        .then_some("bookos")
}

/// Sustituye esas claves en un `settings.ini` de GTK, o las añade bajo
/// `[Settings]`, sin tocar el resto: lo escriben también KDE y el usuario.
// 0.1.0
fn reescribir_ini(ruta: &std::path::Path, valores: &[(&str, &str)]) -> std::io::Result<()> {
    let anterior = match std::fs::read_to_string(ruta) {
        Ok(t) => t,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "[Settings]\n".to_string(),
        Err(err) => return Err(err),
    };
    let mut vistos = vec![false; valores.len()];
    let mut salida: Vec<String> = anterior
        .lines()
        .map(|linea| {
            let clave = linea.split_once('=').map(|(k, _)| k.trim());
            match valores.iter().position(|(k, _)| Some(*k) == clave) {
                Some(i) => {
                    vistos[i] = true;
                    format!("{}={}", valores[i].0, valores[i].1)
                }
                None => linea.to_string(),
            }
        })
        .collect();
    let seccion = salida
        .iter()
        .position(|l| l.trim() == "[Settings]")
        .map_or(salida.len(), |i| i + 1);
    for (i, (clave, valor)) in valores.iter().enumerate().rev() {
        if !vistos[i] {
            salida.insert(seccion, format!("{clave}={valor}"));
        }
    }
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(ruta, salida.join("\n") + "\n")
}

struct Screenshot {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    /// Versión 2: añade `PickColor` apoyado en el cuentagotas nativo.
    #[zbus(property, name = "version")]
    // 0.1.0
    fn version(&self) -> u32 {
        2
    }

    // 0.1.0
    async fn screenshot(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Captura { respuesta }) {
            return (FALLO, HashMap::new());
        }
        let Some(ruta) = espera.await else {
            return (FALLO, HashMap::new());
        };
        let mut resultados = HashMap::new();
        let uri = format!("file://{}", ruta.to_string_lossy());
        match OwnedValue::try_from(Value::from(uri)) {
            Ok(v) => {
                resultados.insert("uri".to_string(), v);
            }
            Err(err) => {
                tracing::warn!("no se pudo empaquetar la ruta de la captura: {err}");
                return (FALLO, HashMap::new());
            }
        }
        (CORRECTO, resultados)
    }

    async fn pick_color(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let id = self.estado.siguiente_id();
        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            ruta: ruta_solicitud.clone(),
            cancelada: cancelada.clone(),
            al_cerrar: AlCerrar::Color {
                estado: self.estado.clone(),
                id,
            },
        };
        let _ = servidor.at(ruta_solicitud.as_str(), solicitud).await;

        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Color { id, respuesta }) {
            let _ = servidor
                .remove::<Solicitud, _>(ruta_solicitud.as_str())
                .await;
            return (FALLO, HashMap::new());
        }

        let rgb = espera.await;
        let _ = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await;

        if cancelada.load(Ordering::Acquire) {
            return (CANCELADO, HashMap::new());
        }
        let Some(rgb) = rgb else {
            return (CANCELADO, HashMap::new());
        };
        match OwnedValue::try_from(Value::from(color_a_rgb_f64(rgb))) {
            Ok(v) => (CORRECTO, HashMap::from([("color".to_string(), v)])),
            Err(err) => {
                tracing::warn!("no se pudo empaquetar el color elegido: {err}");
                (FALLO, HashMap::new())
            }
        }
    }
}

struct Acceso {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl Acceso {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        mut options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let id = self.estado.siguiente_id();
        let denegar = cadena(&mut options, "deny_label")
            .map(|s| limpiar_mnemonico(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| bookos_shell::t("Denegar", "Deny").to_string());
        let permitir = cadena(&mut options, "grant_label")
            .map(|s| limpiar_mnemonico(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| bookos_shell::t("Permitir", "Allow").to_string());
        let icono = cadena(&mut options, "icon").unwrap_or_default();
        let opciones = parsear_opciones_acceso(&mut options);

        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            ruta: ruta_solicitud.clone(),
            cancelada: cancelada.clone(),
            al_cerrar: AlCerrar::Acceso {
                estado: self.estado.clone(),
                id,
            },
        };
        let _ = servidor.at(ruta_solicitud.as_str(), solicitud).await;

        let peticion = bookos_shell::PeticionAcceso {
            id,
            app_id: app_id.to_string(),
            titulo: title.to_string(),
            subtitulo: subtitle.to_string(),
            cuerpo: body.to_string(),
            icono,
            denegar,
            permitir,
            opciones,
        };
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Acceso {
            id,
            peticion,
            respuesta,
        }) {
            let _ = servidor
                .remove::<Solicitud, _>(ruta_solicitud.as_str())
                .await;
            return (FALLO, HashMap::new());
        }

        let elecciones = espera.await;
        let _ = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await;

        if cancelada.load(Ordering::Acquire) {
            return (CANCELADO, HashMap::new());
        }
        let Some(elecciones) = elecciones else {
            return (CANCELADO, HashMap::new());
        };
        let mut resultados = HashMap::new();
        if !elecciones.is_empty()
            && let Ok(v) = OwnedValue::try_from(Value::from(elecciones))
        {
            resultados.insert("choices".to_string(), v);
        }
        (CORRECTO, resultados)
    }
}

struct ElegirApp {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.AppChooser")]
impl ElegirApp {
    async fn choose_application(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        choices: Vec<String>,
        mut options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let id = self.estado.siguiente_id();
        let ultima = cadena(&mut options, "last_choice");
        let filename = cadena(&mut options, "filename");
        let uri = cadena(&mut options, "uri");
        let content_type = cadena(&mut options, "content_type");
        let token = cadena(&mut options, "activation_token");
        let subtitulo = construir_subtitulo_app(
            filename.as_deref(),
            uri.as_deref(),
            content_type.as_deref(),
        );

        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        self.estado
            .elegir_apps
            .lock()
            .expect("el mutex de AppChooser no se comparte con nada que entre en pánico")
            .insert(ruta_solicitud.clone(), id);

        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            ruta: ruta_solicitud.clone(),
            cancelada: cancelada.clone(),
            al_cerrar: AlCerrar::ElegirApp {
                estado: self.estado.clone(),
                id,
            },
        };
        let _ = servidor.at(ruta_solicitud.as_str(), solicitud).await;

        let peticion = bookos_shell::PeticionElegirApp {
            id,
            subtitulo,
            opciones: choices,
            ultima,
        };
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::ElegirApp {
            id,
            peticion,
            respuesta,
        }) {
            self.estado
                .elegir_apps
                .lock()
                .expect("el mutex de AppChooser no se comparte con nada que entre en pánico")
                .remove(&ruta_solicitud);
            let _ = servidor
                .remove::<Solicitud, _>(ruta_solicitud.as_str())
                .await;
            return (FALLO, HashMap::new());
        }

        let eleccion = espera.await;
        self.estado
            .elegir_apps
            .lock()
            .expect("el mutex de AppChooser no se comparte con nada que entre en pánico")
            .remove(&ruta_solicitud);
        let _ = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await;

        if cancelada.load(Ordering::Acquire) {
            return (CANCELADO, HashMap::new());
        }
        let Some(eleccion) = eleccion else {
            return (CANCELADO, HashMap::new());
        };
        let mut resultados = HashMap::new();
        match OwnedValue::try_from(Value::from(eleccion)) {
            Ok(v) => {
                resultados.insert("choice".to_string(), v);
            }
            Err(err) => {
                tracing::warn!("no se pudo empaquetar la aplicación elegida: {err}");
                return (FALLO, HashMap::new());
            }
        }
        if let Some(token) = token
            && let Ok(v) = OwnedValue::try_from(Value::from(token))
        {
            resultados.insert("activation_token".to_string(), v);
        }
        (CORRECTO, resultados)
    }

    fn update_choices(&self, handle: ObjectPath<'_>, choices: Vec<String>) {
        let ruta: OwnedObjectPath = handle.to_owned().into();
        let id = self
            .estado
            .elegir_apps
            .lock()
            .expect("el mutex de AppChooser no se comparte con nada que entre en pánico")
            .get(&ruta)
            .copied();
        if let Some(id) = id {
            self.estado.avisar(Aviso::ActualizarApps {
                id,
                opciones: choices,
            });
        }
    }
}

/// El diálogo de abrir y guardar archivos, con la cara de BookOS.
///
/// No lo dibuja el shell: es `bookos-explorer` en modo selector, que ya tiene
/// la navegación, las miniaturas, la barra lateral y el tema. Rehacer todo eso
/// en iced para un diálogo sería mantener dos exploradores. Como proceso
/// aparte, además, un explorador que se cuelga no se lleva al compositor.
///
/// El explorador escribe las rutas elegidas, una por línea, en un fichero que
/// se le pasa; si se cierra sin escribir nada, es que el usuario canceló.
struct Selector {
    /// El socket Wayland de **este** compositor. El proceso hereda el entorno
    /// del compositor y, en anidado, su `WAYLAND_DISPLAY` es el del anfitrión:
    /// sin fijarlo, el diálogo se abriría en la otra sesión.
    socket: std::ffi::OsString,
    siguiente: AtomicU32,
}

/// Lo que se le pide al explorador.
enum Modo {
    Abrir {
        multiple: bool,
    },
    Guardar,
    /// `directory` en `OpenFile`, y también `SaveFiles`, que solo necesita
    /// saber la carpeta: los nombres los trae la aplicación.
    Carpeta,
}

/// Un filtro del portal, `(sa(us))`: nombre y lista de `(tipo, patrón)`, donde
/// el tipo 0 es un glob y el 1 un tipo MIME.
type Filtro = (String, Vec<(u32, String)>);

impl Selector {
    // 0.1.0
    async fn elegir(
        &self,
        handle: ObjectPath<'_>,
        titulo: &str,
        modo: Modo,
        mut opciones: HashMap<String, OwnedValue>,
        servidor: &zbus::ObjectServer,
    ) -> Result<Vec<PathBuf>, u32> {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        let mut par = |clave: &str, valor: std::ffi::OsString| {
            args.push(clave.into());
            args.push(valor);
        };
        par(
            "--pick-mode",
            match modo {
                Modo::Abrir { .. } => "open",
                Modo::Guardar => "save",
                Modo::Carpeta => "folder",
            }
            .into(),
        );
        if !titulo.is_empty() {
            par("--pick-title", titulo.into());
        }
        if let Some(etiqueta) = cadena(&mut opciones, "accept_label") {
            // El guion bajo es el mnemónico de GTK («_Abrir»); en un botón
            // HTML se vería tal cual.
            par("--pick-accept", etiqueta.replacen('_', "", 1).into());
        }
        if let Some(nombre) = cadena(&mut opciones, "current_name") {
            par("--pick-name", nombre.into());
        }
        // `current_file` es un archivo que ya existe y se está volviendo a
        // guardar: manda sobre `current_folder` y `current_name` porque dice
        // las dos cosas a la vez.
        let actual = bytes_ruta(&mut opciones, "current_file");
        let carpeta = actual
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(PathBuf::from)
            .or_else(|| bytes_ruta(&mut opciones, "current_folder"));
        if let Some(carpeta) = carpeta {
            par("--pick-start", carpeta.into_os_string());
        }
        if let Some(nombre) = actual.as_deref().and_then(std::path::Path::file_name) {
            par("--pick-name", nombre.to_owned());
        }
        let filtro = filtro_inicial(&mut opciones);
        if let Some((nombre, patrones)) = &filtro {
            // El nombre también: «Imágenes» se entiende; `*.png *.jpg`, menos.
            par("--pick-filter-name", nombre.into());
            // Solo los globs. Un filtro MIME pediría la base de datos de
            // shared-mime-info para decidir; sin ella, no filtrar es mejor que
            // esconder archivos que la aplicación sí acepta.
            for (_, glob) in patrones.iter().filter(|(tipo, _)| *tipo == 0) {
                par("--pick-pattern", glob.into());
            }
        }
        if let Modo::Abrir { multiple: true } = modo {
            args.push("--pick-multiple".into());
        }

        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let salida = dir.join(format!(
            "bookos-selector-{}-{}",
            std::process::id(),
            self.siguiente.fetch_add(1, Ordering::Relaxed)
        ));
        let hijo = std::process::Command::new("bookos-explorer")
            .arg("--bookos-portal")
            .arg(&salida)
            .args(&args)
            .env("WAYLAND_DISPLAY", &self.socket)
            .spawn();
        let mut hijo = match hijo {
            Ok(hijo) => hijo,
            Err(err) => {
                tracing::warn!("no se pudo abrir el selector de archivos: {err}");
                return Err(FALLO);
            }
        };

        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            ruta: ruta_solicitud.clone(),
            cancelada: cancelada.clone(),
            al_cerrar: AlCerrar::Selector { pid: hijo.id() },
        };
        if let Err(err) = servidor.at(ruta_solicitud.as_str(), solicitud).await {
            // Sin `Request` el portal no podría cancelar, pero el usuario sí
            // puede cerrar la ventana: se sigue.
            tracing::debug!(%ruta_solicitud, "no se pudo publicar Request: {err}");
        }

        // `wait` bloquea y aquí no se puede bloquear (ver la cabecera del
        // módulo): lo espera un hilo y la conexión sigue atendiendo.
        let (emisario, espera) = promesa();
        std::thread::spawn(move || {
            if let Err(err) = hijo.wait() {
                tracing::warn!("no se pudo esperar al selector de archivos: {err}");
                return;
            }
            // Sin fichero, el emisario se suelta sin contestar: cancelado.
            if let Ok(texto) = std::fs::read_to_string(&salida) {
                let _ = std::fs::remove_file(&salida);
                emisario.entregar(texto);
            }
        });
        let texto = espera.await;
        if let Err(err) = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await
        {
            tracing::debug!(%ruta_solicitud, "Request ya retirado: {err}");
        }

        let rutas: Vec<PathBuf> = texto
            .iter()
            .flat_map(|t| t.lines())
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        if cancelada.load(Ordering::Acquire) || rutas.is_empty() {
            return Err(CANCELADO);
        }
        Ok(rutas)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl Selector {
    // 0.1.0
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        mut options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let modo = if booleano(&mut options, "directory") {
            Modo::Carpeta
        } else {
            Modo::Abrir {
                multiple: booleano(&mut options, "multiple"),
            }
        };
        match self.elegir(handle, title, modo, options, servidor).await {
            Ok(rutas) => respuesta_uris(&rutas),
            Err(codigo) => (codigo, HashMap::new()),
        }
    }

    // 0.1.0
    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        match self
            .elegir(handle, title, Modo::Guardar, options, servidor)
            .await
        {
            Ok(rutas) => respuesta_uris(&rutas[..1]),
            Err(codigo) => (codigo, HashMap::new()),
        }
    }

    /// Se elige una carpeta y cada nombre de `files` se cuelga de ella, en el
    /// mismo orden, que es lo que pide la especificación.
    // 0.1.0
    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        mut options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let nombres: Vec<Vec<u8>> = options
            .remove("files")
            .and_then(|v| Vec::try_from(v).ok())
            .unwrap_or_default();
        // Se rechaza antes de enseñar el diálogo: `join` con "/tmp/x" o "../x"
        // devolvería permiso sobre una ruta que el usuario nunca eligió.
        let Some(nombres) = nombres
            .iter()
            .map(|n| nombre_base(n))
            .collect::<Option<Vec<PathBuf>>>()
        else {
            return (FALLO, HashMap::new());
        };
        match self
            .elegir(handle, title, Modo::Carpeta, options, servidor)
            .await
        {
            Ok(rutas) => {
                let rutas: Vec<PathBuf> = nombres.iter().map(|n| rutas[0].join(n)).collect();
                respuesta_uris(&rutas)
            }
            Err(codigo) => (codigo, HashMap::new()),
        }
    }
}

// 0.1.0
fn cadena(opciones: &mut HashMap<String, OwnedValue>, clave: &str) -> Option<String> {
    opciones
        .remove(clave)
        .and_then(|v| String::try_from(v).ok())
        .filter(|s| !s.is_empty())
}

// 0.1.0
fn booleano(opciones: &mut HashMap<String, OwnedValue>, clave: &str) -> bool {
    opciones
        .remove(clave)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

/// Las rutas del portal viajan como `ay` terminado en NUL: no tienen por qué
/// ser UTF-8.
// 0.1.0
fn bytes_ruta(opciones: &mut HashMap<String, OwnedValue>, clave: &str) -> Option<PathBuf> {
    let bytes: Vec<u8> = opciones.remove(clave).and_then(|v| Vec::try_from(v).ok())?;
    let ruta = ruta_de_bytes(&bytes);
    (!ruta.as_os_str().is_empty()).then_some(ruta)
}

// 0.1.0
fn ruta_de_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let hasta = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..hasta]))
}

/// Un nombre de archivo sin más: un único componente normal, ni absoluto, ni
/// `..`, ni con barras.
// 0.1.0
fn nombre_base(bytes: &[u8]) -> Option<PathBuf> {
    let ruta = ruta_de_bytes(bytes);
    let mut componentes = ruta.components();
    match (componentes.next(), componentes.next()) {
        (Some(std::path::Component::Normal(_)), None) => Some(ruta),
        _ => None,
    }
}

/// El que pide la aplicación con `current_filter` y, si no dice nada, el
/// primero de la lista, que es el que enseñaría cualquier diálogo.
// 0.1.0
fn filtro_inicial(opciones: &mut HashMap<String, OwnedValue>) -> Option<Filtro> {
    let actual = opciones
        .remove("current_filter")
        .and_then(|v| Filtro::try_from(v).ok());
    actual.or_else(|| {
        opciones
            .remove("filters")
            .and_then(|v| Vec::<Filtro>::try_from(v).ok())
            .and_then(|filtros| filtros.into_iter().next())
    })
}

// 0.1.0
fn respuesta_uris(rutas: &[PathBuf]) -> (u32, HashMap<String, OwnedValue>) {
    let uris: Vec<String> = rutas.iter().map(|r| uri_de_archivo(r)).collect();
    match OwnedValue::try_from(Value::from(uris)) {
        Ok(v) => (CORRECTO, HashMap::from([("uris".to_string(), v)])),
        Err(err) => {
            tracing::warn!("no se pudieron empaquetar las rutas elegidas: {err}");
            (FALLO, HashMap::new())
        }
    }
}

/// `file://` con los bytes escapados. Una ruta con espacios o acentos metida
/// tal cual no es una URI: GLib la rechaza y la aplicación no abre nada.
// 0.1.0
fn uri_de_archivo(ruta: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in ruta.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Convierte un píxel `u8` en terna `(ddd)` de `[0.0, 1.0]` para `PickColor`.
fn color_a_rgb_f64([r, g, b]: [u8; 3]) -> (f64, f64, f64) {
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

/// Quita mnemónicos de GTK (`_Permitir` → `Permitir`, `__` → `_`).
fn limpiar_mnemonico(texto: &str) -> String {
    let mut salida = String::with_capacity(texto.len());
    let mut chars = texto.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                salida.push('_');
                chars.next();
            }
        } else {
            salida.push(c);
        }
    }
    salida
}

type OpcionAccesoCruda = (String, String, Vec<(String, String)>, String);

fn parsear_opciones_acceso(
    opciones: &mut HashMap<String, OwnedValue>,
) -> Vec<bookos_shell::OpcionAcceso> {
    let crudas = opciones
        .remove("choices")
        .and_then(|v| Vec::<OpcionAccesoCruda>::try_from(v).ok())
        .unwrap_or_default();
    crudas
        .into_iter()
        .map(|(id, etiqueta, subopciones, elegida)| {
            let sub_limpias: Vec<(String, String)> = subopciones
                .into_iter()
                .map(|(k, v)| (k, limpiar_mnemonico(&v)))
                .collect();
            let elegida = if sub_limpias.is_empty() {
                if elegida == "true" {
                    "true".into()
                } else {
                    "false".into()
                }
            } else if elegida.is_empty() || !sub_limpias.iter().any(|(k, _)| k == &elegida) {
                sub_limpias[0].0.clone()
            } else {
                elegida
            };
            bookos_shell::OpcionAcceso {
                id,
                etiqueta: limpiar_mnemonico(&etiqueta),
                opciones: sub_limpias,
                elegida,
            }
        })
        .collect()
}

fn construir_subtitulo_app(
    filename: Option<&str>,
    uri: Option<&str>,
    content_type: Option<&str>,
) -> String {
    match (filename, uri, content_type) {
        (Some(f), _, Some(ct)) => format!("{f} · {ct}"),
        (Some(f), _, None) => f.to_string(),
        (None, Some(u), Some(ct)) => format!("{u} · {ct}"),
        (None, Some(u), None) => u.to_string(),
        (None, None, Some(ct)) => ct.to_string(),
        (None, None, None) => {
            bookos_shell::t("Elige una aplicación", "Choose an application").to_string()
        }
    }
}

/// El nombre que se le enseña al usuario en el diálogo.
///
/// El `app_id` que llega es el del `.desktop` —`org.mozilla.firefox`— o la
/// cadena vacía si la aplicación no está en un contenedor y el portal no pudo
/// averiguarlo. Sin nombre, «Una aplicación»: es más honesto que enseñar un
/// hueco.
// 0.1.0
fn nombre_legible(app_id: &str) -> String {
    if app_id.is_empty() {
        return bookos_shell::t("Una aplicación", "An app").into();
    }
    let corto = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut letras = corto.chars();
    match letras.next() {
        Some(primera) => primera.to_uppercase().collect::<String>() + letras.as_str(),
        None => bookos_shell::t("Una aplicación", "An app").into(),
    }
}

/// Devuelve la conexión, que hay que **guardar**: al soltarla se cierra el bus
/// y el nombre se pierde.
// 0.1.0
pub fn arrancar(
    canal: Sender<Aviso>,
    socket: &std::ffi::OsStr,
) -> Option<zbus::blocking::Connection> {
    let estado = Arc::new(Estado {
        canal,
        sesiones: Mutex::new(HashMap::new()),
        elegir_apps: Mutex::new(HashMap::new()),
        siguiente: AtomicU32::new(1),
    });
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| {
            b.serve_at(
                RUTA,
                ScreenCast {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| {
            b.serve_at(
                RUTA,
                Screenshot {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| {
            b.serve_at(
                RUTA,
                Acceso {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| {
            b.serve_at(
                RUTA,
                ElegirApp {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| b.serve_at(RUTA, Ajustes))
        .and_then(|b| {
            b.serve_at(
                RUTA,
                Selector {
                    socket: socket.to_owned(),
                    siguiente: AtomicU32::new(1),
                },
            )
        })
        // El nombre se pide **al final**: hasta que todas las interfaces están
        // publicadas, un `xdg-desktop-portal` que ya estuviera esperando podría
        // preguntar por una que todavía no existe.
        .and_then(|b| b.name(NOMBRE))
        .and_then(|b| b.build());
    match conexion {
        Ok(conexion) => {
            tracing::info!("portal de escritorio de BookOS en el bus");
            Some(conexion)
        }
        Err(err) => {
            tracing::warn!("sin portal de escritorio: {err}");
            None
        }
    }
}

/// Lo que llega del hilo del portal, ya en el hilo del compositor.
// 0.1.0
pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    match aviso {
        Aviso::Consentir {
            sesion,
            ruta,
            app,
            respuesta,
            nodo,
        } => {
            consentir(state, sesion, ruta, app, respuesta, nodo);
        }
        Aviso::Cerrar { sesion } => {
            state.emisiones.quitar(sesion);
            crate::backend::actualizar_privacidad(state);
        }
        Aviso::Cancelar { sesion } => cancelar(state, sesion),
        Aviso::Descriptor { respuesta } => {
            // Si el hilo de PipeWire no arranca, el portal se queda sin
            // respuesta y vence su plazo: mejor eso que contestarle con un
            // descriptor inventado.
            if let Some(emisor) = state.emisiones.hilo() {
                emisor.enviar(crate::pw::Orden::Descriptor { respuesta });
            }
        }
        Aviso::Captura { respuesta } => capturar(state, respuesta),
        Aviso::Color { id, respuesta } => elegir_color(state, id, respuesta),
        Aviso::CancelarColor { id } => {
            if state
                .color_portal
                .as_ref()
                .is_some_and(|(actual, _)| *actual == id)
            {
                crate::cuentagotas::cancelar(state);
            }
        }
        Aviso::Acceso {
            id,
            peticion,
            respuesta,
        } => abrir_acceso(state, id, peticion, respuesta),
        Aviso::CancelarAcceso { id } => {
            if state
                .acceso_portal
                .as_ref()
                .is_some_and(|(actual, _)| *actual == id)
            {
                state.acceso_portal = None;
                if let Some(shell) = state.shell.as_mut() {
                    shell.cerrar_acceso_si(id);
                }
                state.needs_redraw = true;
            }
        }
        Aviso::ElegirApp {
            id,
            peticion,
            respuesta,
        } => abrir_elegir_app(state, id, peticion, respuesta),
        Aviso::ActualizarApps { id, opciones } => {
            if state
                .elegir_app_portal
                .as_ref()
                .is_some_and(|(actual, _)| *actual == id)
                && let Some(shell) = state.shell.as_mut()
            {
                shell.elegir_app_actualizar(id, opciones);
                state.needs_redraw = true;
            }
        }
        Aviso::CancelarElegirApp { id } => {
            if state
                .elegir_app_portal
                .as_ref()
                .is_some_and(|(actual, _)| *actual == id)
            {
                state.elegir_app_portal = None;
                if let Some(shell) = state.shell.as_mut() {
                    shell.cerrar_elegir_app_si(id);
                }
                state.needs_redraw = true;
            }
        }
    }
}

fn elegir_color(state: &mut crate::state::BookosComp, id: u32, respuesta: Emisario<[u8; 3]>) {
    if state.color_portal.is_some()
        || state.cuentagotas
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return;
    }
    if crate::cuentagotas::activar(state) {
        state.color_portal = Some((id, respuesta));
    }
}

fn abrir_acceso(
    state: &mut crate::state::BookosComp,
    id: u32,
    peticion: bookos_shell::PeticionAcceso,
    respuesta: Emisario<Vec<(String, String)>>,
) {
    if state.acceso_portal.is_some()
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return;
    }
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    shell.abrir_acceso(peticion);
    state.acceso_portal = Some((id, respuesta));
    state.needs_redraw = true;
}

/// Respuesta del usuario al diálogo `org.freedesktop.impl.portal.Access`.
pub fn responder_acceso(
    state: &mut crate::state::BookosComp,
    id: u32,
    permitido: bool,
    opciones: Vec<(String, String)>,
) {
    let Some((actual, respuesta)) = state.acceso_portal.take() else {
        return;
    };
    if actual != id {
        state.acceso_portal = Some((actual, respuesta));
        return;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_acceso_si(id);
    }
    state.needs_redraw = true;
    if permitido {
        respuesta.entregar(opciones);
    }
}

fn abrir_elegir_app(
    state: &mut crate::state::BookosComp,
    id: u32,
    peticion: bookos_shell::PeticionElegirApp,
    respuesta: Emisario<String>,
) {
    if state.elegir_app_portal.is_some()
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return;
    }
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    shell.abrir_elegir_app(peticion);
    state.elegir_app_portal = Some((id, respuesta));
    state.needs_redraw = true;
}

/// Respuesta del usuario al selector «Abrir con…» (`org.freedesktop.impl.portal.AppChooser`).
pub fn responder_elegir_app(
    state: &mut crate::state::BookosComp,
    id: u32,
    eleccion: Option<String>,
) {
    let Some((actual, respuesta)) = state.elegir_app_portal.take() else {
        return;
    };
    if actual != id {
        state.elegir_app_portal = Some((actual, respuesta));
        return;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_elegir_app_si(id);
    }
    state.needs_redraw = true;
    if let Some(id_app) = eleccion {
        respuesta.entregar(id_app);
    }
}

// 0.1.0
fn cancelar(state: &mut crate::state::BookosComp, sesion: u32) {
    if state
        .consentimiento
        .as_ref()
        .is_some_and(|consentimiento| consentimiento.sesion == sesion)
    {
        // Soltar los emisarios despierta `Start` con cancelación.
        state.consentimiento = None;
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        state.needs_redraw = true;
    }
    state.emisiones.quitar(sesion);
    crate::backend::actualizar_privacidad(state);
}

// 0.1.0
fn consentir(
    state: &mut crate::state::BookosComp,
    sesion: u32,
    ruta: OwnedObjectPath,
    app: String,
    respuesta: Emisario<(u32, u32)>,
    nodo: Emisario<u32>,
) {
    // Un diálogo cada vez. Dos tarjetas de permiso superpuestas no se pueden
    // contestar por separado, y la segunda taparía a la primera.
    if state.consentimiento.is_some() {
        return;
    }
    let salidas: Vec<smithay::output::Output> = state.space.outputs().cloned().collect();
    if salidas.is_empty() {
        return;
    }
    let pantallas: Vec<bookos_shell::PantallaCompartible> = salidas
        .iter()
        .map(|output| {
            let tamano = crate::captura::tamano_de(output);
            bookos_shell::PantallaCompartible {
                nombre: crate::pantallas::nombre_visible(
                    &output.name(),
                    &output.physical_properties().model,
                ),
                ancho: tamano.w.max(0) as u32,
                alto: tamano.h.max(0) as u32,
            }
        })
        .collect();
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    shell.abrir_compartir(sesion, app, pantallas);
    state.consentimiento = Some(crate::state::Consentimiento {
        sesion,
        ruta,
        respuesta,
        nodo,
        salidas,
    });
    state.needs_redraw = true;
}

/// La respuesta del usuario a la tarjeta.
// 0.1.0
pub fn responder(state: &mut crate::state::BookosComp, sesion: u32, pantalla: Option<usize>) {
    let Some(consentimiento) = state.consentimiento.take() else {
        return;
    };
    // Una tarjeta que ya no es la que está abierta: se deja como estaba.
    if consentimiento.sesion != sesion {
        state.consentimiento = Some(consentimiento);
        return;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
    }
    state.needs_redraw = true;

    let elegida = pantalla.and_then(|i| consentimiento.salidas.get(i).cloned());
    let Some(output) = elegida else {
        // Los dos emisarios se sueltan aquí sin contestar, que es la negativa.
        return;
    };

    let tamano = crate::captura::tamano_de(&output);
    let (ancho, alto) = (tamano.w.max(0) as u32, tamano.h.max(0) as u32);
    // modified 0.2.0 -- techo subido de 60 a 120 para que el panel de 120 Hz
    // salga entero. Es el tope que se anuncia: el consumidor puede pedir menos
    // (Discord se queda en 60). El camino sigue pasando por la CPU, así que a
    // 120 fps y 2880×1800 son ~2,4 GB/s de copia mientras haya movimiento.
    // `refresh` viene en milihercios.
    let fps = output
        .current_mode()
        .map(|modo| (modo.refresh as u32).div_ceil(1000))
        .unwrap_or(60)
        .clamp(1, 120);
    let periodo = std::time::Duration::from_micros(1_000_000 / fps as u64);

    let Some(emisor) = state.emisiones.hilo() else {
        return;
    };
    emisor.enviar(crate::pw::Orden::Abrir {
        sesion,
        ancho,
        alto,
        fps,
        respuesta: consentimiento.nodo,
    });
    state.emisiones.anadir(crate::emision::Emision {
        sesion,
        ruta: consentimiento.ruta,
        output,
        tamano,
        periodo,
        // Atrasado un periodo para que el primer fotograma salga en el
        // siguiente dibujo y no haya un hueco negro al empezar la llamada.
        ultimo: std::time::Instant::now() - periodo,
    });
    crate::backend::actualizar_privacidad(state);
    consentimiento.respuesta.entregar((ancho, alto));
}

/// Avisa al portal cuando el compositor tiene que cortar una sesión por su
/// cuenta, por ejemplo porque el modo de la salida ya no coincide con el
/// formato negociado con PipeWire.
// 0.1.0
pub fn sesion_cerrada(conexion: Option<&zbus::blocking::Connection>, ruta: &OwnedObjectPath) {
    let Some(conexion) = conexion else { return };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        ruta.as_str(),
        "org.freedesktop.impl.portal.Session",
        "Closed",
        &(),
    ) {
        tracing::warn!(%ruta, "no se pudo emitir Session.Closed: {err}");
    }
}

/// La red de seguridad: si la tarjeta ya no está pero nadie contestó, se
/// contesta que no.
///
/// Pasa con un clic fuera de la tarjeta, que la cierra sin pasar por ninguna
/// acción, y pasaría también si el shell se cayera con la tarjeta abierta. El
/// hilo del portal está bloqueado esperando y dejarlo hasta que venza el plazo
/// se ve, desde la aplicación, como que el escritorio se ha colgado.
// 0.1.0
pub fn denegar_si_se_cerro(state: &mut crate::state::BookosComp) {
    let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    if state.consentimiento.is_some() {
        let abierta = state
            .shell
            .as_ref()
            .and_then(|shell| shell.emergente_nombre())
            == Some("compartir");
        if !abierta || bloqueado {
            state.consentimiento = None;
        }
    }
    if state.acceso_portal.is_some() {
        let abierta = state
            .shell
            .as_ref()
            .and_then(|shell| shell.emergente_nombre())
            == Some("acceso");
        if !abierta || bloqueado {
            state.acceso_portal = None;
        }
    }
    if state.elegir_app_portal.is_some() {
        let abierta = state
            .shell
            .as_ref()
            .and_then(|shell| shell.emergente_nombre())
            == Some("elegir-app");
        if !abierta || bloqueado {
            state.elegir_app_portal = None;
        }
    }
    if state.color_portal.is_some() {
        let esperando_pixel = state.cuentagotas
            || state
                .captura_pedida
                .is_some_and(|p| p.destino == crate::state::DestinoCaptura::Color);
        if !esperando_pixel || bloqueado {
            state.color_portal = None;
        }
    }
}

/// La captura de pantalla que pide el portal: la principal entera, a fichero.
// 0.1.0
// modified 0.1.1 -- usa DestinoCaptura en vez de un booleano
fn capturar(state: &mut crate::state::BookosComp, respuesta: Emisario<PathBuf>) {
    // La principal tiene el origen en (0,0); es la única que sabe capturar
    // `servir_captura_propia`, y sus coordenadas son las de esa pantalla.
    let principal = state
        .space
        .outputs()
        .find(|o| o.current_location() == (0, 0).into())
        .cloned();
    let Some(principal) = principal else {
        return;
    };
    let Some(geometria) = state.space.output_geometry(&principal) else {
        return;
    };
    state.captura_portal = Some(respuesta);
    state.captura_pedida = Some(crate::state::CapturaPedida {
        x: 0,
        y: 0,
        ancho: geometria.size.w,
        alto: geometria.size.h,
        destino: crate::state::DestinoCaptura::Archivo,
    });
    state.needs_redraw = true;
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Cambia las claves en su sitio, añade las que faltan bajo `[Settings]` y
    /// deja intacto lo que escribió KDE.
    #[test]
    fn el_ini_de_gtk_conserva_lo_ajeno() {
        let dir = std::env::temp_dir().join(format!("bookos-ini-{}", std::process::id()));
        let ruta = dir.join("settings.ini");
        std::fs::create_dir_all(&dir).expect("directorio temporal");
        std::fs::write(
            &ruta,
            "[Settings]\ngtk-application-prefer-dark-theme=true\ngtk-font-name=Noto Sans,  10\n",
        )
        .expect("escribir el ini de partida");
        reescribir_ini(
            &ruta,
            &[
                ("gtk-theme-name", "BookOS-Light-Blue"),
                ("gtk-application-prefer-dark-theme", "false"),
            ],
        )
        .expect("reescribir");
        let texto = std::fs::read_to_string(&ruta).expect("leer");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            texto,
            "[Settings]\ngtk-theme-name=BookOS-Light-Blue\ngtk-application-prefer-dark-theme=false\ngtk-font-name=Noto Sans,  10\n"
        );
    }

    #[test]
    fn save_files_solo_acepta_nombres_base() {
        assert_eq!(
            nombre_base(b"informe.pdf\0"),
            Some(PathBuf::from("informe.pdf"))
        );
        for malo in [&b"/tmp/x"[..], b"../x", b"a/b", b"..", b".", b"", b"\0"] {
            assert_eq!(
                nombre_base(malo),
                None,
                "{:?}",
                String::from_utf8_lossy(malo)
            );
        }
    }

    /// Lo que se le sirve a GTK es lo que el escritorio está pintando.
    ///
    /// Los números son los del propio tema, no una copia: si alguien cambia el
    /// azul de BookOS, esto lo sigue. Lo que se fija aquí es la **forma**, que
    /// es lo que la especificación obliga y lo que se comprobó contra el portal
    /// de KDE de esta máquina: `color-scheme` entero con 1 oscuro y 2 claro, y
    /// `accent-color` tres dobles de 0 a 1.
    #[test]
    fn el_aspecto_que_se_sirve_es_el_del_escritorio() {
        use bookos_shell::tema::{self, Acento, Tema};

        let antes = (tema::actual(), tema::acento_actual());

        tema::aplicar(Tema::Oscuro);
        tema::aplicar_acento(Acento::Verde);
        assert_eq!(Ajustes::color_scheme(), 1, "oscuro es 1");
        let c = tema::acento();
        assert_eq!(
            Ajustes::accent_color(),
            (c.r as f64, c.g as f64, c.b as f64),
            "el acento servido no es el del tema"
        );

        tema::aplicar(Tema::Claro);
        assert_eq!(Ajustes::color_scheme(), 2, "claro es 2");
        assert_ne!(
            Ajustes::accent_color(),
            (c.r as f64, c.g as f64, c.b as f64),
            "el verde claro y el oscuro son colores distintos"
        );

        // Las cuatro claves están y `ReadOne` da lo mismo que `ReadAll`.
        let todo = Ajustes::valores();
        for clave in ["color-scheme", "accent-color", "reduced-motion", "contrast"] {
            let uno = Ajustes::uno(clave).unwrap_or_else(|| panic!("falta «{clave}»"));
            assert_eq!(
                todo.get(clave),
                Some(&uno),
                "«{clave}» no cuadra entre las dos"
            );
        }
        assert!(
            Ajustes::uno("lo-que-sea").is_none(),
            "una clave que no existe no puede dar valor"
        );

        tema::aplicar(antes.0);
        tema::aplicar_acento(antes.1);
    }

    #[test]
    fn el_nombre_de_la_aplicacion_sale_legible() {
        assert_eq!(nombre_legible("org.mozilla.firefox"), "Firefox");
        assert_eq!(nombre_legible("chromium"), "Chromium");
        assert_eq!(
            nombre_legible(""),
            bookos_shell::t("Una aplicación", "An app")
        );
    }

    #[test]
    fn la_ruta_elegida_sale_como_uri_escapada() {
        assert_eq!(
            uri_de_archivo(std::path::Path::new("/home/eve/Mis fotos/año.png")),
            "file:///home/eve/Mis%20fotos/a%C3%B1o.png"
        );
    }

    #[test]
    fn la_ruta_del_portal_se_corta_en_el_nul() {
        assert_eq!(ruta_de_bytes(b"/tmp/x\0basura"), PathBuf::from("/tmp/x"));
        assert_eq!(ruta_de_bytes(b"/tmp/y"), PathBuf::from("/tmp/y"));
    }

    /// `current_filter` manda; sin él, el primero de `filters`. Los valores se
    /// construyen con la misma firma que manda xdg-desktop-portal.
    #[test]
    fn el_filtro_inicial_es_el_que_pide_la_aplicacion() {
        let imagenes: Filtro = (
            "Imágenes".into(),
            vec![(0, "*.png".into()), (1, "image/*".into())],
        );
        let texto: Filtro = ("Texto".into(), vec![(0, "*.txt".into())]);
        let empaqueta = |v: Value<'_>| OwnedValue::try_from(v).expect("valor sin descriptores");

        let mut opciones = HashMap::from([(
            "filters".to_string(),
            empaqueta(Value::from(vec![imagenes.clone(), texto.clone()])),
        )]);
        assert_eq!(filtro_inicial(&mut opciones), Some(imagenes.clone()));

        let mut opciones = HashMap::from([
            (
                "filters".to_string(),
                empaqueta(Value::from(vec![imagenes, texto.clone()])),
            ),
            (
                "current_filter".to_string(),
                empaqueta(Value::from(texto.clone())),
            ),
        ]);
        assert_eq!(filtro_inicial(&mut opciones), Some(texto));

        assert_eq!(filtro_inicial(&mut HashMap::new()), None);
    }

    #[test]
    fn elegir_iconos_gtk_sincroniza_al_cambiar_tema_y_respeta_tintados_o_ajuste_manual() {
        let claro_defecto = bookos_shell::icono::tema_por_defecto(true);
        let oscuro_defecto = bookos_shell::icono::tema_por_defecto(false);

        assert_eq!(
            elegir_iconos_gtk(false, Some("BookOS-Light".into()), true),
            oscuro_defecto
        );
        assert_eq!(
            elegir_iconos_gtk(true, Some("BookOS-Dark".into()), true),
            claro_defecto
        );
        assert_eq!(
            elegir_iconos_gtk(false, Some("Adwaita".into()), true),
            oscuro_defecto
        );
        assert_eq!(elegir_iconos_gtk(false, None, true), oscuro_defecto);
        assert_eq!(
            elegir_iconos_gtk(false, Some("BookOS-Tinted".into()), true),
            "BookOS-Tinted"
        );
        assert_eq!(
            elegir_iconos_gtk(true, Some("BookOS-Tinted-B".into()), true),
            "BookOS-Tinted-B"
        );

        assert_eq!(
            elegir_iconos_gtk(true, Some("BookOS-Dark".into()), false),
            "BookOS-Dark"
        );
        assert_eq!(elegir_iconos_gtk(true, None, false), claro_defecto);
    }

    #[test]
    fn convierte_rgb_u8_a_ddd_portal() {
        let (r, g, b) = color_a_rgb_f64([0, 128, 255]);
        assert!((r - 0.0).abs() < 1e-9);
        assert!((g - (128.0 / 255.0)).abs() < 1e-9);
        assert!((b - 1.0).abs() < 1e-9);
    }

    #[test]
    fn limpia_mnemonicos_de_gtk() {
        assert_eq!(limpiar_mnemonico("_Permitir"), "Permitir");
        assert_eq!(limpiar_mnemonico("No _volver__nunca"), "No volver_nunca");
        assert_eq!(limpiar_mnemonico("Allow"), "Allow");
    }

    #[test]
    fn parsea_choices_de_access_booleanos_y_selector() {
        let crudas: Vec<OpcionAccesoCruda> = vec![
            (
                "remember".into(),
                "_Recordar".into(),
                Vec::new(),
                String::new(),
            ),
            (
                "duration".into(),
                "Duración".into(),
                vec![
                    ("once".into(), "Una vez".into()),
                    ("always".into(), "_Siempre".into()),
                ],
                String::new(),
            ),
        ];
        let mut opciones = HashMap::from([(
            "choices".to_string(),
            OwnedValue::try_from(Value::from(crudas)).expect("choices"),
        )]);
        let parsed = parsear_opciones_acceso(&mut opciones);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].etiqueta, "Recordar");
        assert_eq!(parsed[0].elegida, "false");
        assert_eq!(parsed[1].elegida, "once");
        assert_eq!(parsed[1].opciones[1].1, "Siempre");
    }

    #[test]
    fn construye_subtitulo_de_app_chooser_por_prioridad() {
        assert_eq!(
            construir_subtitulo_app(Some("foto.png"), Some("file:///foto.png"), Some("image/png")),
            "foto.png · image/png"
        );
        assert_eq!(
            construir_subtitulo_app(None, Some("https://bookos.org"), None),
            "https://bookos.org"
        );
        assert_eq!(
            construir_subtitulo_app(None, None, Some("application/pdf")),
            "application/pdf"
        );
    }

    #[test]
    fn siguiente_id_portal_evita_cero_al_desbordar() {
        let (tx, _rx) = smithay::reexports::calloop::channel::channel();
        let estado = Estado {
            canal: tx,
            sesiones: Mutex::new(HashMap::new()),
            elegir_apps: Mutex::new(HashMap::new()),
            siguiente: AtomicU32::new(u32::MAX),
        };
        assert_eq!(estado.siguiente_id(), u32::MAX);
        assert_eq!(estado.siguiente_id(), 1);
    }
}
