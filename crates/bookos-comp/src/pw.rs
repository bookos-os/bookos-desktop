//! El hilo de PipeWire: convierte fotogramas del compositor en nodos de vídeo.
//!
//! Es la mitad de abajo de «compartir pantalla». La de arriba es [`crate::portal`],
//! que habla D-Bus con `xdg-desktop-portal`; aquí solo se sirven píxeles.
//!
//! ## Por qué un hilo y no el bucle de frames
//!
//! Igual que el servidor de notificaciones: PipeWire trae su propio bucle de
//! eventos y casarlo con `calloop` costaría más que un hilo con dos canales.
//! El compositor le manda órdenes por [`pipewire::channel`] —que escribe en una
//! tubería y despierta el bucle de pw— y las respuestas vuelven por
//! `std::sync::mpsc`. **Este hilo no toca nada del compositor**, ni al revés.
//!
//! ## El compositor es quien marca el ritmo
//!
//! El stream se conecta con `DRIVER`, así que PipeWire no tiene reloj propio:
//! cada fotograma sale porque el compositor acaba de dibujar uno y llama a
//! `trigger_process()`. Sin `DRIVER`, pw despertaría al proceso a 60 Hz aunque
//! el escritorio estuviera quieto, que es exactamente lo que esta casa no hace.
//! El formato anuncia `framerate = 0/1` —variable— para que el consumidor no
//! espere un fotograma cada 16,6 ms.
//!
//! ## Formato
//!
//! `BGRx` y no otro: el readback del compositor pide `Fourcc::Xrgb8888`, que en
//! little-endian son los bytes B,G,R,X, y eso es literalmente
//! `SPA_VIDEO_FORMAT_BGRx`. No hay conversión de color en ningún punto del
//! camino, que es lo que hace que esto sea barato pese a pasar por la CPU.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::rc::Rc;
use std::sync::mpsc;

use libspa::param::ParamType;
use libspa::param::format::{FormatProperties, MediaSubtype, MediaType};
use libspa::param::video::VideoFormat;
use libspa::pod::{self, Pod};
use libspa::utils::{Fraction, Rectangle, SpaTypes};
use pipewire as pw;

/// Lo que el compositor le pide al hilo.
pub enum Orden {
    /// Crea un nodo para una sesión. El `node_id` vuelve por `respuesta` cuando
    /// PipeWire lo asigna, que no es en el momento de conectar.
    Abrir {
        sesion: u32,
        ancho: u32,
        alto: u32,
        fps: u32,
        respuesta: crate::portal::Emisario<u32>,
    },
    /// Un fotograma ya compuesto, en BGRx, sin relleno entre filas.
    ///
    /// No lleva marca de tiempo: sin `SPA_META_Header` en el buffer, PipeWire
    /// fecha el fotograma con el reloj del grafo al encolarlo, que es lo que
    /// quiere un consumidor de vídeo en vivo.
    Marco {
        sesion: u32,
        datos: Vec<u8>,
    },
    /// Un descriptor de conexión a PipeWire para dárselo al cliente del portal.
    Descriptor {
        respuesta: crate::portal::Emisario<OwnedFd>,
    },
    Cerrar {
        sesion: u32,
    },
}

/// El extremo del compositor: por aquí se le habla al hilo.
#[derive(Clone)]
pub struct Emisor {
    canal: pw::channel::Sender<Orden>,
}

impl Emisor {
    // 0.1.0
    pub fn enviar(&self, orden: Orden) {
        // Si el canal está roto es que el hilo se cayó; el compositor sigue sin
        // compartir pantalla, que es mejor que llevárselo por delante.
        let _ = self.canal.send(orden);
    }
}

/// Lo que el hilo guarda de cada sesión.
struct Nodo {
    stream: pw::stream::StreamRc,
    _oyente: pw::stream::StreamListener<()>,
    /// El último fotograma recibido y todavía no entregado. Solo uno: si llegan
    /// dos antes de que PipeWire pida, el viejo sobra —enseñar un fotograma
    /// atrasado es peor que saltárselo— y su `Vec` se recicla.
    pendiente: Option<Vec<u8>>,
    /// Tamaño y stride negociados, o `None` mientras no hay formato.
    formato: Option<(u32, u32, u32)>,
    /// Los `Vec` vacíos vuelven al compositor por aquí para no asignar 8 MB por
    /// fotograma. Si el otro extremo desapareció, se tiran.
    reciclado: mpsc::Sender<(u32, Vec<u8>)>,
}

type Nodos = Rc<RefCell<HashMap<u32, Nodo>>>;
/// Los `Vec` de vuelta, con el nodo al que pertenecían, para reutilizarlos.
pub type Reciclado = mpsc::Receiver<(u32, Vec<u8>)>;

/// Arranca el hilo. Devuelve por dónde hablarle y por dónde recoger los `Vec`
/// reciclados.
// 0.1.0
pub fn arrancar() -> Option<(Emisor, Reciclado)> {
    let (canal, receptor) = pw::channel::channel::<Orden>();
    let (devolver, reciclado) = mpsc::channel::<(u32, Vec<u8>)>();

    let hilo = std::thread::Builder::new()
        .name("bookos-pipewire".into())
        .spawn(move || {
            if let Err(err) = bucle(receptor, devolver) {
                tracing::warn!("el hilo de PipeWire se paró: {err}");
            }
        });

    match hilo {
        Ok(_) => Some((Emisor { canal }, reciclado)),
        Err(err) => {
            tracing::warn!("no se pudo arrancar el hilo de PipeWire: {err}");
            None
        }
    }
}

// 0.1.0
fn bucle(
    receptor: pw::channel::Receiver<Orden>,
    devolver: mpsc::Sender<(u32, Vec<u8>)>,
) -> anyhow::Result<()> {
    pw::init();

    let bucle = pw::main_loop::MainLoopRc::new(None)?;
    let contexto = pw::context::ContextRc::new(&bucle, None)?;
    let core = contexto.connect_rc(None)?;

    let nodos: Nodos = Rc::new(RefCell::new(HashMap::new()));

    let _adjunto = {
        let nodos = nodos.clone();
        let core = core.clone();
        let contexto = contexto.clone();
        receptor.attach(bucle.loop_(), move |orden| match orden {
            Orden::Abrir {
                sesion,
                ancho,
                alto,
                fps,
                respuesta,
            } => {
                match abrir(
                    &core,
                    &nodos,
                    sesion,
                    ancho,
                    alto,
                    fps,
                    devolver.clone(),
                    respuesta,
                ) {
                    Ok(()) => {}
                    Err(err) => tracing::warn!(sesion, "no se pudo abrir el nodo: {err}"),
                }
            }
            Orden::Marco { sesion, datos } => {
                // El `RefCell` se suelta **antes** de disparar: siendo driver,
                // `trigger_process` puede llamar a `process` en el momento y
                // ese vuelve a pedir el mismo préstamo. Con el `borrow_mut`
                // todavía vivo, eso es un pánico.
                let stream = {
                    let mut nodos = nodos.borrow_mut();
                    let Some(nodo) = nodos.get_mut(&sesion) else {
                        let _ = devolver.send((sesion, datos));
                        return;
                    };
                    if let Some(viejo) = nodo.pendiente.replace(datos) {
                        let _ = nodo.reciclado.send((sesion, viejo));
                    }
                    nodo.stream.clone()
                };
                // Solo tiene sentido si somos el reloj del grafo; si PipeWire
                // todavía no nos ha hecho driver, el fotograma espera al
                // siguiente `process`.
                if stream.is_driving() {
                    let _ = stream.trigger_process();
                }
            }
            Orden::Descriptor { respuesta } => match descriptor(&contexto) {
                Ok(fd) => respuesta.entregar(fd),
                // Soltar el emisario sin entregar nada es el «no se pudo»; el
                // portal contesta un error en vez de quedarse esperando.
                Err(err) => tracing::warn!("no se pudo abrir el descriptor de PipeWire: {err}"),
            },
            Orden::Cerrar { sesion } => {
                if let Some(nodo) = nodos.borrow_mut().remove(&sesion) {
                    let _ = nodo.stream.disconnect();
                }
            }
        })
    };

    bucle.run();
    Ok(())
}

/// Un descriptor de conexión nuevo para el cliente del portal.
///
/// `OpenPipeWireRemote` tiene que devolver un socket ya conectado a PipeWire
/// sobre el que el cliente hace su propio saludo. No vale abrirlo a mano con
/// `connect(2)`: la conexión lleva credenciales y propiedades que negocia la
/// propia biblioteca. Lo que sí vale es conectar un `core` de usar y tirar y
/// **robarle** el descriptor, que es para lo que existe `pw_core_steal_fd`:
/// deja el `core` sin socket, así que al soltarlo no se cierra nada.
// 0.1.0
fn descriptor(contexto: &pw::context::ContextRc) -> anyhow::Result<OwnedFd> {
    let core = contexto.connect_rc(None)?;
    let fd: RawFd = unsafe { pw::sys::pw_core_steal_fd(core.as_raw_ptr()) };
    drop(core);
    if fd < 0 {
        anyhow::bail!("pw_core_steal_fd devolvió {fd}");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[allow(clippy::too_many_arguments)]
// 0.1.0
fn abrir(
    core: &pw::core::CoreRc,
    nodos: &Nodos,
    sesion: u32,
    ancho: u32,
    alto: u32,
    fps: u32,
    reciclado: mpsc::Sender<(u32, Vec<u8>)>,
    respuesta: crate::portal::Emisario<u32>,
) -> anyhow::Result<()> {
    let props = pw::properties::properties! {
        *pw::keys::MEDIA_CLASS => "Video/Source",
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
        *pw::keys::NODE_NAME => "bookos-screencast",
        *pw::keys::NODE_DESCRIPTION => "Pantalla de BookOS",
    };
    let stream = pw::stream::StreamRc::new(core.clone(), "bookos-screencast", props)?;

    // El identificador se entrega **una sola vez**: `Emisario::entregar` lo
    // consume, y el callback de estado se llama en cada transición.
    let mut respuesta = Some(respuesta);
    let oyente = {
        let nodos_estado = nodos.clone();
        let nodos_formato = nodos.clone();
        let nodos_proceso = nodos.clone();
        stream
            .add_local_listener::<()>()
            .state_changed(move |stream, _, vieja, nueva| {
                tracing::debug!(sesion, ?vieja, ?nueva, "estado del nodo de captura");
                // El id solo existe una vez conectado. `Start` está esperándolo
                // al otro lado, así que se manda en cuanto aparece; el `Sender`
                // se cierra solo cuando el receptor se va.
                let id = stream.node_id();
                if id != pw::constants::ID_ANY
                    && let Some(respuesta) = respuesta.take()
                {
                    respuesta.entregar(id);
                }
                if matches!(nueva, pw::stream::StreamState::Error(_)) {
                    nodos_estado.borrow_mut().remove(&sesion);
                }
            })
            .param_changed(move |stream, _, id, param| {
                if id != ParamType::Format.as_raw() {
                    return;
                }
                let Some(param) = param else {
                    nodos_formato
                        .borrow_mut()
                        .entry(sesion)
                        .and_modify(|n| n.formato = None);
                    return;
                };
                let Ok((tipo, subtipo)) = libspa::param::format_utils::parse_format(param) else {
                    return;
                };
                if tipo != MediaType::Video || subtipo != MediaSubtype::Raw {
                    return;
                }
                let mut info = libspa::param::video::VideoInfoRaw::new();
                if info.parse(param).is_err() {
                    return;
                }
                let tam = info.size();
                let stride = tam.width * 4;
                let bytes = stride * tam.height;
                // El préstamo se suelta aquí, antes de `update_params`, por lo
                // mismo que en `Orden::Marco`: la llamada puede volver a entrar.
                if let Some(nodo) = nodos_formato.borrow_mut().get_mut(&sesion) {
                    nodo.formato = Some((tam.width, tam.height, stride));
                }
                let buffers = pod_buffers(stride, bytes);
                let mut params =
                    [Pod::from_bytes(&buffers).expect("pod de Buffers recién serializado")];
                if let Err(err) = stream.update_params(&mut params) {
                    tracing::warn!(sesion, "no se pudieron fijar los buffers: {err}");
                }
            })
            .process(move |stream, _| {
                let mut nodos = nodos_proceso.borrow_mut();
                let Some(nodo) = nodos.get_mut(&sesion) else {
                    return;
                };
                let Some((_, _, stride)) = nodo.formato else {
                    return;
                };
                // Sin fotograma pendiente no se encola nada: encolar un buffer
                // vacío haría que el consumidor pintara negro.
                let Some(datos) = nodo.pendiente.take() else {
                    return;
                };
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    let _ = nodo.reciclado.send((sesion, datos));
                    return;
                };
                let devolver = {
                    let destinos = buffer.datas_mut();
                    let Some(destino) = destinos.first_mut() else {
                        nodo.pendiente = Some(datos);
                        return;
                    };
                    let escritos = match destino.data() {
                        Some(hueco) => {
                            let n = hueco.len().min(datos.len());
                            hueco[..n].copy_from_slice(&datos[..n]);
                            n
                        }
                        None => 0,
                    };
                    let chunk = destino.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = stride as i32;
                    *chunk.size_mut() = escritos as u32;
                    datos
                };
                let _ = nodo.reciclado.send((sesion, devolver));
            })
            .register()?
    };

    // El nodo entra en el mapa **antes** de conectar: `connect` dispara
    // `state_changed` y `param_changed` en el momento, y esos callbacks buscan
    // la sesión aquí. Registrándolo después, el primer `param_changed` no
    // encontraba nada y el formato se quedaba sin fijar para siempre.
    nodos.borrow_mut().insert(
        sesion,
        Nodo {
            stream: stream.clone(),
            _oyente: oyente,
            pendiente: None,
            formato: None,
            reciclado,
        },
    );

    let formato = pod_formato(ancho, alto, fps);
    let mut params = [Pod::from_bytes(&formato).expect("pod de EnumFormat recién serializado")];
    if let Err(err) = stream.connect(
        libspa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    ) {
        nodos.borrow_mut().remove(&sesion);
        return Err(err.into());
    }
    Ok(())
}

/// El único formato que se ofrece: BGRx del tamaño de la salida, a ritmo libre.
///
/// El tamaño va fijo y no como rango porque no hay escalador en este camino: lo
/// que sale del compositor mide lo que mide la pantalla. El `framerate` a 0/1
/// dice «variable»; el tope real va en `maxFramerate`.
// 0.1.0
fn pod_formato(ancho: u32, alto: u32, fps: u32) -> Vec<u8> {
    let obj = pod::object! {
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(FormatProperties::VideoFormat, Id, VideoFormat::BGRx),
        pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle { width: ancho, height: alto }
        ),
        pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction { num: 0, denom: 1 }
        ),
        pod::property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: fps, denom: 1 },
            Fraction { num: 1, denom: 1 },
            Fraction { num: fps, denom: 1 }
        ),
    };
    serializar(pod::Value::Object(obj))
}

/// La respuesta a la negociación del formato: cuántos buffers y de qué tamaño.
///
/// Tres buffers, no uno: con uno solo el compositor se queda esperando a que el
/// consumidor suelte el que tiene y se pierden fotogramas en cada hipo. Se
/// aceptan `MemFd` y `MemPtr`, que con `MAP_BUFFERS` llegan ya mapeados.
// 0.1.0
fn pod_buffers(stride: u32, bytes: u32) -> Vec<u8> {
    use libspa::sys as spa_sys;
    use libspa::utils::{Choice, ChoiceEnum, ChoiceFlags};

    let prop = |key: u32, value: pod::Value| pod::Property {
        key,
        flags: pod::PropertyFlags::empty(),
        value,
    };
    let tipos = (1 << spa_sys::SPA_DATA_MemFd) | (1 << spa_sys::SPA_DATA_MemPtr);
    let obj = pod::Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: ParamType::Buffers.as_raw(),
        properties: vec![
            prop(
                spa_sys::SPA_PARAM_BUFFERS_buffers,
                pod::Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 3,
                        min: 2,
                        max: 8,
                    },
                ))),
            ),
            prop(spa_sys::SPA_PARAM_BUFFERS_blocks, pod::Value::Int(1)),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_size,
                pod::Value::Int(bytes as i32),
            ),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_stride,
                pod::Value::Int(stride as i32),
            ),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_dataType,
                pod::Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: tipos,
                        flags: Vec::new(),
                    },
                ))),
            ),
        ],
    };
    serializar(pod::Value::Object(obj))
}

// 0.1.0
fn serializar(valor: pod::Value) -> Vec<u8> {
    pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &valor)
        .expect("serializar a un Vec en memoria no puede fallar")
        .0
        .into_inner()
}
