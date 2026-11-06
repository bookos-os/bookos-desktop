//! El fondo del escritorio.
//!
//! Es una textura y no un color liso por dos razones que van más allá de lo
//! bonito: la pantalla de bloqueo necesita algo detrás —si no, se ve el
//! escritorio a través— y el cristal del panel y del dock desenfoca **el
//! fondo**, no las ventanas, que es lo que hace que se lea igual de bien tenga
//! lo que tenga debajo.
//!
//! ## Una subida por imagen visible
//!
//! Los fondos estáticos conservan el mismo buffer durante toda la sesión. En
//! un WebP animado solo se conserva y sube el fotograma visible; el temporizador
//! despierta al llegar su duración para no mantener la animación entera en RAM.
//!
//! ## Vídeo
//!
//! Un vídeo no se convierte a WebP: el decodificador WebP de `image` tarda
//! 67-80 ms por fotograma a 1920×1200 (medido con `image` 0.25.10), y aquí corre
//! en el hilo del compositor, así que a 24 fps el escritorio entero bajaba a
//! unos 12. El vídeo lo decodifica un `ffmpeg` aparte y un hilo lector deja
//! cada fotograma ya en buffer y con su miniatura; al compositor solo le toca
//! cambiar de buffer y subirlo. Lo que no sea AV1 ni VP9 se convierte una vez a
//! AV1 en segundo plano; el porqué, en [`convertir`].

use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use bookos_shell::EncajeFondo;
use image::AnimationDecoder as _;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryBuffer, MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::Transform;

/// A qué ancho se reduce el fondo para las miniaturas de la vista de
/// escritorios.
///
/// La previa más grande mide 250 px lógicos, que a escala 2 son 500 físicos:
/// 640 llega de sobra y cuesta 1 MB por copia, frente a los 20 MB del original.
const MINIATURA_ANCHO: i32 = 640;

/// Ancho máximo al que se decodifica un vídeo.
///
/// Cada fotograma se sube entero a la GPU dos veces —el fondo y el cristal con
/// mipmaps—, así que el coste va con los píxeles: un 4K serían 33 MB por
/// fotograma. A 1920 son 9 MB, y en un fondo con ventanas encima no se nota.
const VIDEO_ANCHO: u32 = 1920;

/// Fotogramas por segundo a los que se limita un vídeo. Por lo mismo que el
/// ancho: un vídeo de 60 fps son el doble de subidas para un fondo.
const VIDEO_FPS: f64 = 30.0;

/// Extensiones que se abren como vídeo. Coinciden con las que acepta Ajustes.
pub const EXTENSIONES_VIDEO: [&str; 5] = ["mp4", "webm", "mkv", "mov", "m4v"];

/// Qué fondo ha pedido el usuario, antes de decidir cuál toca.
///
/// Son tres rutas y no una porque el tema cambia en caliente: `claro` y
/// `oscuro` son la pareja —los fondos de BookOS vienen así, `blue.png` y
/// `blue_dark.png`— y `ambos` es el de toda la vida, el mismo con los dos
/// temas. Con las tres vacías se busca el que venga instalado.
#[derive(Debug, Default, Clone)]
pub struct Eleccion {
    /// `fondo`: el mismo con los dos temas.
    pub ambos: Option<String>,
    /// `fondo_claro`. Manda sobre `ambos` cuando el tema es claro.
    pub claro: Option<String>,
    /// `fondo_oscuro`. Manda sobre `ambos` cuando el tema es oscuro.
    pub oscuro: Option<String>,
    /// `fondo_encaje`.
    pub encaje: EncajeFondo,
}

/// Un rectángulo en fracciones de su contenedor, de 0 a 1.
///
/// En fracciones porque el mismo encuadre vale para el fondo entero, para su
/// miniatura y para la textura reducida del cristal, que miden distinto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trozo {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Trozo {
    const ENTERO: Self = Self {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };
}

/// Qué trozo de la imagen se ve y en qué trozo de la pantalla cae.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Encuadre {
    imagen: Trozo,
    pantalla: Trozo,
}

impl Encuadre {
    // añadido en la 0.2.0
    fn calcular(encaje: EncajeFondo, imagen: (i32, i32), pantalla: (i32, i32)) -> Self {
        let (iw, ih) = (imagen.0.max(1) as f64, imagen.1.max(1) as f64);
        let (pw, ph) = (pantalla.0.max(1) as f64, pantalla.1.max(1) as f64);
        let escala = match encaje {
            EncajeFondo::Estirar => {
                return Self {
                    imagen: Trozo::ENTERO,
                    pantalla: Trozo::ENTERO,
                };
            }
            EncajeFondo::Rellenar => (pw / iw).max(ph / ih),
            EncajeFondo::Ajustar => (pw / iw).min(ph / ih),
            EncajeFondo::Ancho => pw / iw,
            EncajeFondo::Centrar => 1.0,
        };
        // Por eje: lo que no cabe se recorta de la imagen, y lo que no llega
        // deja banda en la pantalla. Las dos cosas, centradas.
        let eje = |imagen: f64, pantalla: f64| {
            let escalada = imagen * escala;
            let visible = escalada.min(pantalla);
            let (de_imagen, de_pantalla) = (visible / escalada, visible / pantalla);
            (
                (1.0 - de_imagen) / 2.0,
                de_imagen,
                (1.0 - de_pantalla) / 2.0,
                de_pantalla,
            )
        };
        let (ix, iw, px, pw) = eje(iw, pw);
        let (iy, ih, py, ph) = eje(ih, ph);
        Self {
            imagen: Trozo {
                x: ix,
                y: iy,
                w: iw,
                h: ih,
            },
            pantalla: Trozo {
                x: px,
                y: py,
                w: pw,
                h: ph,
            },
        }
    }

    /// ¿Hay imagen detrás de esta zona? Sobre una banda no la hay, y desenfocar
    /// el borde más cercano teñía el dock de un color que no se ve alrededor.
    // añadido en la 0.2.0
    pub fn tiene_imagen_detras(&self, zona: Trozo) -> bool {
        let p = self.pantalla;
        zona.x < p.x + p.w && p.x < zona.x + zona.w && zona.y < p.y + p.h && p.y < zona.y + zona.h
    }

    /// El trozo de imagen que queda detrás de una zona de la pantalla. Lo que
    /// asome sobre una banda se queda con el borde más cercano.
    // añadido en la 0.2.0
    pub fn detras_de(&self, zona: Trozo) -> Trozo {
        // Un trozo vacío dejaría a cero `uv_tam`, por el que divide el shader.
        const MINIMO: f64 = 0.001;
        let eje = |inicio: f64, largo: f64, imagen: (f64, f64), pantalla: (f64, f64)| {
            let aqui =
                |p: f64| (imagen.0 + (p - pantalla.0) / pantalla.1 * imagen.1).clamp(0.0, 1.0);
            let a = aqui(inicio).min(1.0 - MINIMO);
            (a, (aqui(inicio + largo) - a).max(MINIMO))
        };
        let (x, w) = eje(
            zona.x,
            zona.w,
            (self.imagen.x, self.imagen.w),
            (self.pantalla.x, self.pantalla.w),
        );
        let (y, h) = eje(
            zona.y,
            zona.h,
            (self.imagen.y, self.imagen.h),
            (self.pantalla.y, self.pantalla.h),
        );
        Trozo { x, y, w, h }
    }
}

/// El rectángulo en píxeles de un trozo de una imagen: origen y tamaño.
// añadido en la 0.2.0
fn en_pixeles(trozo: Trozo, (w, h): (i32, i32)) -> ((i32, i32), (i32, i32)) {
    let x = ((trozo.x * w as f64).round() as i32).clamp(0, w - 1);
    let y = ((trozo.y * h as f64).round() as i32).clamp(0, h - 1);
    let cw = ((trozo.w * w as f64).round() as i32).clamp(1, w - x);
    let ch = ((trozo.h * h as f64).round() as i32).clamp(1, h - y);
    ((x, y), (cw, ch))
}

impl Eleccion {
    /// La ruta que toca con el tema que está puesto **ahora**.
    ///
    /// El específico gana al común: quien se molesta en poner `fondo_oscuro`
    /// está diciendo justo eso, y si además tiene un `fondo` suelto es el de
    /// respaldo para el otro tema.
    // 0.1.0
    pub fn para_el_tema(&self) -> Option<&str> {
        self.para(bookos_shell::tema::es_claro())
    }

    /// La decisión, sin preguntarle al tema global.
    ///
    /// Va aparte para poder probarla: el tema es un `static` del proceso y
    /// `Shell::con_config` lo escribe, así que un test que lo moviera para
    /// leerlo después competiría con cualquier otro del mismo binario.
    // 0.1.0
    fn para(&self, claro: bool) -> Option<&str> {
        let propio = if claro {
            self.claro.as_deref()
        } else {
            self.oscuro.as_deref()
        };
        propio.or(self.ambos.as_deref())
    }

    /// ¿Cambia la imagen al cambiar de tema? Si no, recargar no sirve de nada.
    // 0.1.0
    pub fn depende_del_tema(&self) -> bool {
        self.claro.is_some() || self.oscuro.is_some() || self.ambos.is_none()
    }
}

/// El fondo ya decodificado y listo para dibujar.
pub struct Fondo {
    /// De dónde salió. Solo para la traza y el autotest: es la única forma de
    /// comprobar desde fuera que el cambio de tema se llevó el fondo con él.
    ruta: PathBuf,
    /// Va con la imagen y no se lee de la elección al dibujar: el fondo saliente
    /// de un fundido tiene que seguir cayendo como caía.
    encaje: EncajeFondo,
    buffer: MemoryRenderBuffer,
    /// Tamaño de la imagen en píxeles, para el recorte.
    pixeles: (i32, i32),
    /// Los píxeles en crudo, que hacen falta otra vez para la textura con
    /// mipmaps que usa el cristal. Son 20 MB: se guardan porque volver a
    /// decodificar el PNG al cambiar de resolución costaría más.
    ///
    /// Es la **misma** memoria que la de `buffer` y la del gemelo: el
    /// `MemoryBuffer` va en un `Arc` y `from_memory` no copia. Antes eran tres
    /// `Vec` iguales, 60 de los 70 MB del heap del compositor en reposo.
    rgba: MemoryBuffer,
    /// El fondo reducido, y una copia **por miniatura**.
    ///
    /// Hace falta una copia por cada una porque `MemoryRenderBufferRenderElement`
    /// hereda el identificador de su buffer: cinco elementos del mismo buffer
    /// serían cinco elementos con el mismo id, y el seguimiento de daño los
    /// tomaría por el mismo rectángulo moviéndose. Se crean la primera vez que
    /// se abre la vista y se quedan puestas: son 1 MB cada una.
    mini_rgba: Vec<u8>,
    mini_pixeles: (i32, i32),
    mini: std::cell::RefCell<Vec<MemoryRenderBuffer>>,
    /// Una segunda copia del fondo entero, para el cambio de escritorio.
    ///
    /// Hace falta porque durante el deslizamiento se ven **dos** fondos a la
    /// vez —el que se va y el que entra por el otro lado— y no pueden ser el
    /// mismo buffer: `MemoryRenderBufferRenderElement` hereda el identificador
    /// del buffer, así que dos elementos del mismo serían para el seguimiento
    /// de daño el mismo rectángulo teletransportándose, y uno de los dos no se
    /// dibujaría.
    ///
    /// Solo el id es distinto: los píxeles son los de `rgba`, sin copia. La
    /// textura en GPU sí es otra, y esa se crea la primera vez que se cambia de
    /// escritorio.
    gemelo: std::cell::RefCell<Option<MemoryRenderBuffer>>,
    /// Decodificador incremental. Solo conserva el fotograma actual y el
    /// estado comprimido: guardar todos los RGBA de un fondo largo ocuparía
    /// cientos de megas sin mejorar el dibujo.
    animacion: Option<Animacion>,
}

/// Un fotograma listo para poner: los píxeles y su miniatura.
struct Fotograma {
    rgba: MemoryBuffer,
    pixeles: (i32, i32),
    mini_rgba: Vec<u8>,
    mini_pixeles: (i32, i32),
}

impl Fotograma {
    // 0.1.0
    fn de_rgba(rgba: &[u8], pixeles: (i32, i32)) -> Self {
        // Abgr8888: en little-endian son los bytes R,G,B,A, que es justo lo que
        // devuelve el decodificador. Con Argb8888 —el del shell— saldría con el
        // rojo y el azul cambiados; ya pasó una vez con el panel.
        let (mini_rgba, mini_pixeles) = reducir(rgba, pixeles, MINIATURA_ANCHO);
        Self {
            rgba: MemoryBuffer::from_slice(rgba, Fourcc::Abgr8888, pixeles),
            pixeles,
            mini_rgba,
            mini_pixeles,
        }
    }
}

enum Animacion {
    Webp {
        ruta: PathBuf,
        frames: image::Frames<'static>,
        siguiente: Instant,
    },
    Video(Video),
}

/// Lo que deja un paso de la animación.
enum Paso {
    Nuevo(Fotograma),
    /// El vídeo no tenía listo el siguiente: se queda el que hay.
    Mismo,
    /// La animación se acabó por un error; el último fotograma se queda fijo.
    Fin,
}

impl Animacion {
    // 0.1.0
    fn abrir(ruta: &Path) -> Option<(Self, Fotograma)> {
        if es_video(ruta) {
            let (video, primero) = Video::abrir(ruta)?;
            return Some((Self::Video(video), primero));
        }
        let mut frames = frames_webp(ruta)?;
        let frame = frames.next()?.ok()?;
        let retraso = retraso(frame.delay());
        let rgba = frame.into_buffer();
        let (w, h) = rgba.dimensions();
        Some((
            Self::Webp {
                ruta: ruta.to_path_buf(),
                frames,
                siguiente: Instant::now() + retraso,
            },
            Fotograma::de_rgba(&rgba, (w as i32, h as i32)),
        ))
    }

    // 0.1.0
    fn siguiente(&self) -> Instant {
        match self {
            Self::Webp { siguiente, .. } => *siguiente,
            Self::Video(video) => video.siguiente,
        }
    }

    // 0.1.0
    fn avanzar(&mut self) -> Paso {
        let (ruta, frames, siguiente) = match self {
            Self::Video(video) => return video.avanzar(),
            Self::Webp {
                ruta,
                frames,
                siguiente,
            } => (ruta, frames, siguiente),
        };
        let frame = match frames.next() {
            Some(Ok(frame)) => frame,
            Some(Err(err)) => {
                tracing::warn!(?ruta, %err, "fotograma WebP inválido");
                return Paso::Fin;
            }
            None => {
                let Some(frame) = frames_webp(ruta).and_then(|nuevos| {
                    *frames = nuevos;
                    frames.next()?.ok()
                }) else {
                    return Paso::Fin;
                };
                frame
            }
        };
        *siguiente = Instant::now() + retraso(frame.delay());
        let rgba = frame.into_buffer();
        let (w, h) = rgba.dimensions();
        Paso::Nuevo(Fotograma::de_rgba(&rgba, (w as i32, h as i32)))
    }
}

// 0.1.0
// modified 0.1.1 -- pública para otros módulos
pub fn es_video(ruta: &Path) -> bool {
    ruta.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES_VIDEO.contains(&e.to_ascii_lowercase().as_str()))
}

/// Códecs que se reproducen tal cual. El resto se convierte a AV1 la primera
/// vez que se elige el vídeo; ver [`convertir`].
///
/// Son los dos que el ffmpeg de Fedora (`ffmpeg-free`) decodifica con su propio
/// decodificador —y por VAAPI—, y los dos que siguen bien al dar la vuelta.
const DIRECTOS: [&str; 2] = ["av1", "vp9"];

/// Un vídeo en bucle decodificado por `ffmpeg`.
struct Video {
    proceso: Child,
    fotogramas: Receiver<Fotograma>,
    periodo: Duration,
    siguiente: Instant,
    /// La conversión a AV1, mientras dura. Entretanto se reproduce el original.
    conversion: Option<Conversion>,
}

enum Conversion {
    /// ffmpeg convirtiendo. El sondeo es el del original: el convertido sale
    /// ya con ese tamaño y ese ritmo, y así no hace falta otro ffprobe.
    Convirtiendo {
        proceso: Child,
        destino: PathBuf,
        sondeo: Sondeo,
    },
    /// Ya convertido, y un hilo abriendo el resultado. Va en un hilo porque
    /// ffprobe y el primer fotograma de ffmpeg tardan entre los dos un segundo
    /// (medido: 0,3-0,4 s y 0,6-0,8 s), y en el del compositor eso es un
    /// segundo con la pantalla y la entrada paradas.
    Abriendo(Receiver<Option<(Video, Fotograma)>>),
}

impl Video {
    // 0.1.0
    fn abrir(ruta: &Path) -> Option<(Self, Fotograma)> {
        let sondeo = sondear(ruta)?;
        let directo = DIRECTOS.contains(&sondeo.codec.as_str());
        let (w, h, _) = sondeo.salida();
        // Con algunos tamaños diminutos SVT-AV1 se queda colgado, sin gastar
        // CPU y sin atender SIGTERM: 8×4 y 16×64 sí, 64×16 y 32×32 no. Con los
        // dos lados de 64 o más no se colgó ninguno de los probados. Ningún
        // fondo es así de pequeño, pero un proceso colgado para siempre es peor
        // que reproducir el original.
        let destino = ruta_convertida(ruta).filter(|_| !directo && w.min(h) >= 64);
        let Some(destino) = destino else {
            return Self::reproducir(ruta, &sondeo, directo);
        };
        if destino.is_file()
            && let Some(abierto) = Self::reproducir(&destino, &sondeo, true)
        {
            return Some(abierto);
        }
        // La conversión se lanza **después** de comprobar que el original se
        // puede reproducir: si este ffmpeg no lo decodifica (HEVC en
        // ffmpeg-free) tampoco lo podría convertir.
        let (mut video, primero) = Self::reproducir(ruta, &sondeo, false)?;
        video.conversion =
            convertir(ruta, &sondeo, &destino).map(|proceso| Conversion::Convirtiendo {
                proceso,
                destino,
                sondeo,
            });
        Some((video, primero))
    }

    /// Con `vaapi`, primero por la GPU y, si no arranca, por software. No hay
    /// forma barata de preguntar antes si la GPU decodifica este códec: ffmpeg
    /// falla al abrir y eso se ve aquí como un vídeo sin primer fotograma. Sin
    /// `vaapi` ni se intenta: con el H.264 de ffmpeg-free fallaría siempre, y
    /// el intento cuesta lo que tarda ffmpeg en arrancar.
    // 0.1.0
    fn reproducir(ruta: &Path, sondeo: &Sondeo, vaapi: bool) -> Option<(Self, Fotograma)> {
        let (w, h, fps) = sondeo.salida();
        let (proceso, fotogramas, primero) = vaapi
            .then(|| lanzar(ruta, (w, h), fps, true))
            .flatten()
            .or_else(|| lanzar(ruta, (w, h), fps, false))?;
        let periodo = Duration::from_secs_f64(1.0 / fps);
        let video = Self {
            proceso,
            fotogramas,
            periodo,
            siguiente: Instant::now() + periodo,
            conversion: None,
        };
        Some((video, primero))
    }

    // 0.1.0
    fn avanzar(&mut self) -> Paso {
        // El convertido entra en cuanto tiene su primer fotograma, sin fundido:
        // es un corte del punto por el que iba el original al principio del
        // vídeo, y pasa una vez en la vida de cada fondo.
        if let Some((convertido, primero)) = self.relevo() {
            *self = convertido;
            return Paso::Nuevo(primero);
        }
        // Sin acumular retraso: si el temporizador llega tarde se sigue desde
        // ahora, no se intenta recuperar fotogramas que ya no se van a ver.
        self.siguiente = (self.siguiente + self.periodo).max(Instant::now());
        match self.fotogramas.try_recv() {
            Ok(fotograma) => Paso::Nuevo(fotograma),
            Err(TryRecvError::Empty) => Paso::Mismo,
            Err(TryRecvError::Disconnected) => Paso::Fin,
        }
    }

    /// Sigue la conversión un paso. Devuelve el vídeo convertido cuando ya se
    /// puede poner en lugar de este.
    // 0.1.0
    fn relevo(&mut self) -> Option<(Video, Fotograma)> {
        match self.conversion.as_mut()? {
            Conversion::Convirtiendo { proceso, .. } => {
                let bien = match proceso.try_wait() {
                    Ok(None) => return None,
                    Ok(Some(estado)) => estado.success(),
                    Err(err) => {
                        tracing::warn!(%err, "no se pudo consultar la conversión del fondo");
                        false
                    }
                };
                let Some(Conversion::Convirtiendo {
                    destino, sondeo, ..
                }) = self.conversion.take()
                else {
                    unreachable!("comprobada arriba");
                };
                let parcial = parcial(&destino);
                if !bien
                    || std::fs::rename(&parcial, &destino)
                        .inspect_err(|err| {
                            tracing::warn!(?destino, %err, "no se pudo guardar el vídeo convertido");
                        })
                        .is_err()
                {
                    tracing::warn!(
                        ?destino,
                        "la conversión del vídeo del fondo falló: se sigue con el original"
                    );
                    // Puede no existir si ffmpeg falló antes de escribir nada.
                    let _ = std::fs::remove_file(&parcial);
                    return None;
                }
                tracing::info!(?destino, "vídeo del fondo convertido");
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                // Si el fondo se suelta antes de que acabe, `send` falla y el
                // vídeo abierto se suelta aquí, con su ffmpeg.
                std::thread::Builder::new()
                    .name("fondo-relevo".into())
                    .spawn(move || {
                        let _ = tx.send(Self::reproducir(&destino, &sondeo, true));
                    })
                    .inspect_err(|err| tracing::warn!(%err, "no se pudo abrir el convertido"))
                    .ok()?;
                self.conversion = Some(Conversion::Abriendo(rx));
                None
            }
            Conversion::Abriendo(rx) => match rx.try_recv() {
                Err(TryRecvError::Empty) => None,
                Ok(abierto) => {
                    self.conversion = None;
                    abierto
                }
                Err(TryRecvError::Disconnected) => {
                    self.conversion = None;
                    None
                }
            },
        }
    }
}

impl Drop for Video {
    // 0.1.0
    fn drop(&mut self) {
        // Al morir ffmpeg el lector ve fin de tubería y termina solo. `wait`
        // es para no dejar un zombi por cada cambio de fondo. La conversión
        // también muere: si se cambia de fondo a medias, el `.parcial` se
        // sobrescribe la próxima vez que se elija este vídeo.
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
        if let Some(Conversion::Convirtiendo { proceso, .. }) = self.conversion.as_mut() {
            let _ = proceso.kill();
            let _ = proceso.wait();
        }
    }
}

/// Arranca ffmpeg y espera el primer fotograma. `vaapi` decodifica en la GPU.
///
/// Por VAAPI los fotogramas se bajan como NV12 y se pasan a I420 **antes** de
/// RGBA: el escalador de ffmpeg tiene camino rápido de I420 a RGBA y no de NV12.
/// Medido con el fondo de 1920×1080 en AV1, 10 s a 30 fps: 4,6 s de CPU de NV12
/// directo a RGBA, 2,3 s pasando por I420. Por software, el mismo vídeo en
/// H.264 por openh264 eran 11 s: un núcleo entero solo para el fondo.
// 0.1.0
fn lanzar(
    ruta: &Path,
    (w, h): (u32, u32),
    fps: f64,
    vaapi: bool,
) -> Option<(Child, Receiver<Fotograma>, Fotograma)> {
    let mut ffmpeg = Command::new("ffmpeg");
    ffmpeg.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if vaapi {
        ffmpeg.args(["-hwaccel", "vaapi", "-hwaccel_output_format", "vaapi"]);
    }
    ffmpeg
        .args(["-stream_loop", "-1", "-i"])
        .arg(ruta)
        .args(["-an", "-vf"]);
    let bajada = if vaapi {
        "hwdownload,format=nv12,format=yuv420p,"
    } else {
        ""
    };
    let mut proceso = ffmpeg
        .arg(format!("{bajada}fps={fps},scale={w}:{h}:flags=bilinear"))
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .inspect_err(|err| tracing::warn!(?ruta, %err, "no se pudo lanzar ffmpeg"))
        .ok()?;
    let mut salida = proceso.stdout.take().expect("stdout pedido como tubería");
    // Capacidad uno: si el compositor no recoge —movimiento reducido, o el
    // fondo saliente de un fundido—, el hilo se bloquea al mandar, ffmpeg al
    // escribir en la tubería, y ninguno de los dos gasta CPU.
    let (tx, fotogramas) = std::sync::mpsc::sync_channel(1);
    let pixeles = (w as i32, h as i32);
    let lector = std::thread::Builder::new()
        .name("fondo-video".into())
        .spawn(move || {
            let mut crudo = vec![0u8; w as usize * h as usize * 4];
            while salida.read_exact(&mut crudo).is_ok() {
                if tx.send(Fotograma::de_rgba(&crudo, pixeles)).is_err() {
                    break;
                }
            }
        });
    // Arrancar ffmpeg y sacar el primero tarda unos cientos de ms; un códec
    // que este ffmpeg no tiene (HEVC en ffmpeg-free), o una GPU que no lo
    // decodifica, cierra la tubería y aquí se ve como desconexión.
    let primero = match lector {
        Ok(_) => fotogramas.recv_timeout(Duration::from_secs(5)),
        Err(err) => {
            tracing::warn!(?ruta, %err, "no se pudo crear el hilo del vídeo");
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        }
    };
    match primero {
        Ok(primero) => Some((proceso, fotogramas, primero)),
        Err(err) => {
            if vaapi {
                tracing::debug!(
                    ?ruta,
                    "VAAPI no decodifica este vídeo: se prueba por software"
                );
            } else {
                tracing::warn!(?ruta, %err, "el vídeo no dio ningún fotograma");
            }
            let _ = proceso.kill();
            let _ = proceso.wait();
            None
        }
    }
}

/// Convierte el vídeo a AV1, en segundo plano y con la prioridad más baja.
///
/// Es por el ffmpeg de Fedora: `ffmpeg-free` no trae decodificador de H.264
/// propio, solo openh264, y openh264 no aguanta el salto de `-stream_loop`.
/// Con el fondo de 10 s a 60 fps, en cada vuelta fallaban 247 paquetes y el
/// vídeo saltaba de 10 s a 14,17 s; el filtro `fps` rellena ese hueco
/// repitiendo el último fotograma, así que el fondo se quedaba **congelado
/// cuatro segundos** de cada diez. Tampoco hay VAAPI para H.264 sin el
/// decodificador propio. El AV1 resultante da la vuelta sin un error y se
/// decodifica en la GPU.
///
/// Se aprovecha la pasada única para lo que sería caro en cada fotograma:
/// `deband` alisa los escalones de los degradados oscuros —el H.264 de 4,5
/// Mbit/s del fondo los trae de serie, y a 2880 px de panel se ven—, y el
/// escalado va con lanczos. Con `preset 10` el fondo de 10 s tarda 11 s.
// 0.1.0
fn convertir(ruta: &Path, sondeo: &Sondeo, destino: &Path) -> Option<Child> {
    let carpeta = destino.parent().expect("la ruta convertida tiene carpeta");
    std::fs::create_dir_all(carpeta)
        .inspect_err(|err| tracing::warn!(?carpeta, %err, "no se pudo crear la caché de fondos"))
        .ok()?;
    let (w, h, fps) = sondeo.salida();
    let mut ffmpeg = Command::new("nice");
    ffmpeg
        .args([
            "-n",
            "19",
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
        ])
        .arg("-i")
        .arg(ruta)
        .args(["-an", "-vf"])
        .arg(format!(
            "deband=1thr=0.02:2thr=0.02:3thr=0.02:range=16:blur=1,\
             fps={fps},scale={w}:{h}:flags=lanczos"
        ))
        .args([
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libsvtav1",
            "-preset",
            "10",
            "-crf",
            "20",
        ]);
    // Sin etiqueta de color, el escalador de ffmpeg convierte con BT.601, que es
    // el de la definición estándar; los reproductores dan por hecho BT.709 en
    // HD, y con 601 el morado del fondo salía azulado.
    if sondeo.sin_color && h >= 720 {
        ffmpeg.args([
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ]);
    }
    let proceso = ffmpeg
        .arg(parcial(destino))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .inspect_err(|err| tracing::warn!(?ruta, %err, "no se pudo lanzar la conversión del fondo"))
        .ok()?;
    tracing::info!(?ruta, ?destino, "convirtiendo el vídeo del fondo");
    Some(proceso)
}

/// Dónde va la conversión de `ruta`. El tamaño y la fecha van en el nombre para
/// que un vídeo sobrescrito con otro del mismo nombre no reutilice la vieja.
// 0.1.0
fn ruta_convertida(ruta: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(ruta).ok()?;
    let fecha = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let cache = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    let nombre = ruta.file_stem()?.to_string_lossy();
    Some(cache.join("bookos/fondos").join(format!(
        "{nombre}-{}-{}.mkv",
        meta.len(),
        fecha.as_secs()
    )))
}

/// Mientras se escribe va con otro nombre: si el compositor muere a medias, el
/// fichero a medio hacer no se toma por bueno la próxima vez.
// 0.1.0
fn parcial(destino: &Path) -> PathBuf {
    destino.with_extension("parcial.mkv")
}

/// Lo que se sabe del vídeo antes de abrirlo.
struct Sondeo {
    /// Ancho y alto ya girados.
    ancho: u32,
    alto: u32,
    fps: f64,
    codec: String,
    /// El vídeo no dice con qué matriz se pasa de YUV a RGB.
    sin_color: bool,
}

impl Sondeo {
    /// Tamaño y ritmo a los que se decodifica, con los topes aplicados.
    // 0.1.0
    fn salida(&self) -> (u32, u32, f64) {
        // Pares: con rgba no hace falta, pero el escalador de ffmpeg redondea
        // las impares a su manera y el tamaño leído dejaría de cuadrar.
        let w = self.ancho.min(VIDEO_ANCHO) & !1;
        let h = ((self.alto as f64 * w as f64 / self.ancho as f64).round() as u32).max(2) & !1;
        (w, h, self.fps.min(VIDEO_FPS))
    }
}

// 0.1.0
fn sondear(ruta: &Path) -> Option<Sondeo> {
    let salida = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args([
            "stream=codec_name,width,height,avg_frame_rate,color_space:stream_side_data=rotation",
        ])
        .args(["-of", "default=nw=1"])
        .arg(ruta)
        .stdin(Stdio::null())
        .output()
        .inspect_err(|err| tracing::warn!(?ruta, %err, "no se pudo lanzar ffprobe"))
        .ok()?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    let campo = |nombre: &str| {
        texto
            .lines()
            .find_map(|l| l.strip_prefix(nombre)?.strip_prefix('='))
    };
    let ancho: u32 = campo("width")?.parse().ok()?;
    let alto: u32 = campo("height")?.parse().ok()?;
    // `30/1`, `30000/1001`. Sin dato fiable (`0/0` en algunos webm) se toma el
    // tope: duplicar algún fotograma es mejor que no animar.
    let fps = campo("avg_frame_rate")
        .and_then(|f| f.split_once('/'))
        .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(VIDEO_FPS);
    // ffmpeg gira solo los vídeos de móvil; ffprobe da el tamaño sin girar.
    let girado = campo("rotation")
        .and_then(|r| r.parse::<i32>().ok())
        .is_some_and(|r| r.rem_euclid(180) == 90);
    let (ancho, alto) = if girado { (alto, ancho) } else { (ancho, alto) };
    (ancho > 0 && alto > 0).then(|| Sondeo {
        ancho,
        alto,
        fps,
        codec: campo("codec_name").unwrap_or_default().to_owned(),
        sin_color: campo("color_space").is_none_or(|c| c == "unknown"),
    })
}

// 0.1.0
fn retraso(delay: image::Delay) -> Duration {
    let (numerador, denominador) = delay.numer_denom_ms();
    let ms = numerador as f64 / denominador.max(1) as f64;
    // Cero produciría un bucle ocupado con ficheros mal formados. 16 ms es
    // el fotograma más corto que tiene sentido dibujar en una pantalla normal.
    Duration::from_secs_f64((ms / 1000.0).max(0.016))
}

// 0.1.0
fn frames_webp(ruta: &Path) -> Option<image::Frames<'static>> {
    if ruta.extension().and_then(|e| e.to_str()) != Some("webp") {
        return None;
    }
    let fichero = std::fs::File::open(ruta).ok()?;
    let decoder = image::codecs::webp::WebPDecoder::new(BufReader::new(fichero)).ok()?;
    decoder.has_animation().then(|| decoder.into_frames())
}

impl Fondo {
    /// Carga el fondo. `None` si no hay ninguno donde mirar, y entonces el
    /// escritorio se queda con su color liso de siempre.
    // 0.1.0
    pub fn cargar(eleccion: &Eleccion) -> Option<Self> {
        let ruta = match eleccion.para_el_tema() {
            Some(r) => PathBuf::from(r),
            None => buscar()?,
        };
        let (animacion, primero) = match Animacion::abrir(&ruta) {
            Some((animacion, primero)) => (Some(animacion), primero),
            None if es_video(&ruta) => return None,
            None => {
                let (pixeles, w, h) = decodificar_estatico(&ruta)?;
                (None, Fotograma::de_rgba(&pixeles, (w as i32, h as i32)))
            }
        };
        tracing::info!(
            ?ruta,
            w = primero.pixeles.0,
            h = primero.pixeles.1,
            animado = animacion.is_some(),
            "fondo del escritorio"
        );
        Some(Self {
            ruta,
            encaje: eleccion.encaje,
            buffer: compartido(&primero.rgba),
            pixeles: primero.pixeles,
            rgba: primero.rgba,
            mini_rgba: primero.mini_rgba,
            mini_pixeles: primero.mini_pixeles,
            mini: std::cell::RefCell::new(Vec::new()),
            gemelo: std::cell::RefCell::new(None),
            animacion,
        })
    }

    /// Tiempo hasta el siguiente fotograma. `None` también cubre movimiento
    /// reducido: congelar el primero evita despertar para no cambiar nada.
    // 0.1.0
    pub fn hasta_siguiente(&self) -> Option<Duration> {
        if bookos_shell::tema::efectos_reducidos() {
            return None;
        }
        Some(
            self.animacion
                .as_ref()?
                .siguiente()
                .saturating_duration_since(Instant::now()),
        )
    }

    /// Cambia de fotograma solo cuando vence su retraso.
    // 0.1.0
    // modified 0.1.1 -- reutiliza el buffer si el fotograma mide lo mismo
    pub fn avanzar(&mut self) -> bool {
        if self
            .hasta_siguiente()
            .is_none_or(|espera| !espera.is_zero())
        {
            return false;
        }
        let Some(animacion) = self.animacion.as_mut() else {
            return false;
        };
        let fotograma = match animacion.avanzar() {
            Paso::Nuevo(fotograma) => fotograma,
            Paso::Mismo => return false,
            Paso::Fin => {
                self.animacion = None;
                return false;
            }
        };
        let mismo_tamano = self.pixeles == fotograma.pixeles;
        self.rgba = fotograma.rgba;
        self.pixeles = fotograma.pixeles;
        self.mini_rgba = fotograma.mini_rgba;
        self.mini_pixeles = fotograma.mini_pixeles;
        self.mini.get_mut().clear();
        *self.gemelo.get_mut() = None;
        if mismo_tamano {
            let mut ctx = self.buffer.render();
            let tam = self.pixeles;
            let src = &self.rgba[..];
            let _ = ctx.draw(|dst| {
                let n = dst.len().min(src.len());
                dst[..n].copy_from_slice(&src[..n]);
                Ok::<_, std::convert::Infallible>(vec![smithay::utils::Rectangle::from_size(
                    tam.into(),
                )])
            });
        } else {
            self.buffer = compartido(&self.rgba);
        }
        true
    }

    /// De qué fichero salió.
    // 0.1.0
    pub fn ruta(&self) -> &std::path::Path {
        &self.ruta
    }

    /// Los píxeles y el tamaño, para quien necesite subirlos por su cuenta.
    // 0.1.0
    pub fn rgba(&self) -> (&[u8], (i32, i32)) {
        (&self.rgba, self.pixeles)
    }

    /// El fondo encajado en un rectángulo cualquiera.
    ///
    /// Lo normal es `(0, 0)` y el tamaño lógico de la pantalla. Con otro
    /// rectángulo sirve para dos cosas: la miniatura de un escritorio vacío en
    /// la vista de Meta+W —sin ella la previa es un rectángulo negro— y el
    /// empujón del fondo al cambiar de escritorio.
    ///
    /// Se le pasa el `src` completo por lo mismo que al cursor: sin él,
    /// `from_buffer` usa el tamaño de destino como recorte y se vería una
    /// esquina de la imagen ampliada.
    // 0.1.0
    // modificado en la 0.2.0 -- respeta el encaje elegido
    pub fn elemento_en(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
        escala: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        self.elemento_con_alfa(renderer, posicion, logico, escala, 1.0)
    }

    /// Cómo cae la imagen en una pantalla de este tamaño en físicos.
    // añadido en la 0.2.0
    pub fn encuadre(&self, pantalla: (i32, i32)) -> Encuadre {
        Encuadre::calcular(self.encaje, self.pixeles, pantalla)
    }

    /// Dónde va el elemento, qué trozo del buffer lleva y a qué tamaño lógico.
    ///
    /// `posicion` va en físicos y `logico` en lógicos, la mezcla de `from_buffer`.
    // añadido en la 0.2.0
    fn colocar(
        &self,
        posicion: (i32, i32),
        logico: (i32, i32),
        escala: f64,
    ) -> (
        (f64, f64),
        smithay::utils::Rectangle<f64, smithay::utils::Logical>,
        (i32, i32),
    ) {
        let encuadre = self.encuadre((
            (logico.0 as f64 * escala).round() as i32,
            (logico.1 as f64 * escala).round() as i32,
        ));
        let (i, p) = (encuadre.imagen, encuadre.pantalla);
        let (w, h) = (self.pixeles.0 as f64, self.pixeles.1 as f64);
        (
            (
                posicion.0 as f64 + (p.x * logico.0 as f64 * escala).round(),
                posicion.1 as f64 + (p.y * logico.1 as f64 * escala).round(),
            ),
            smithay::utils::Rectangle::new((i.x * w, i.y * h).into(), (i.w * w, i.h * h).into()),
            (
                ((p.w * logico.0 as f64).round() as i32).max(1),
                ((p.h * logico.1 as f64).round() as i32).max(1),
            ),
        )
    }

    /// El fondo con alfa, para el fundido entre dos imágenes.
    ///
    /// El que entra se dibuja **encima** del que sale y va de cero a uno; el
    /// que sale se queda opaco todo el rato. Es el orden correcto para una
    /// mezcla normal: bajarle el alfa al saliente a la vez dejaría ver el
    /// escritorio a través de los dos a mitad de camino.
    // 0.1.0
    // modificado en la 0.2.0 -- respeta el encaje elegido
    pub fn elemento_con_alfa(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
        escala: f64,
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let (donde, src, tam) = self.colocar(posicion, logico, escala);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            donde,
            &self.buffer,
            Some(alfa),
            Some(src),
            Some(tam.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir el fondo: {err}"))
        .ok()
    }

    /// El segundo fondo, el que entra por el otro lado al cambiar de
    /// escritorio. Ver [`Fondo::gemelo`].
    // 0.1.0
    // modificado en la 0.2.0 -- respeta el encaje elegido
    pub fn elemento_gemelo_en(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
        escala: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let mut gemelo = self.gemelo.borrow_mut();
        let buffer = gemelo.get_or_insert_with(|| compartido(&self.rgba));
        let (donde, src, tam) = self.colocar(posicion, logico, escala);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            donde,
            buffer,
            None,
            Some(src),
            Some(tam.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir el segundo fondo: {err}"))
        .ok()
    }

    /// El fondo reducido para la miniatura número `indice`.
    ///
    /// `posicion` va en píxeles **físicos** y `logico` en lógicos, que es la
    /// mezcla que pide `from_buffer` —y la misma que usa el shell para sus
    /// superficies—. Ponerlos los dos en lógicos dejaba la miniatura desplazada
    /// hacia arriba y a la izquierda en cuanto la escala no era 1.
    // 0.1.0
    // modificado en la 0.2.0 -- enseña el mismo encuadre que la pantalla
    #[allow(clippy::too_many_arguments)]
    pub fn miniatura(
        &self,
        renderer: &mut GlesRenderer,
        indice: usize,
        posicion: (f64, f64),
        logico: (i32, i32),
        pantalla: (i32, i32),
        escala: f64,
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        use smithay::utils::Rectangle;
        let encuadre = self.encuadre(pantalla);
        let p = encuadre.pantalla;
        let tam = (
            ((p.w * logico.0 as f64).round() as i32).max(1),
            ((p.h * logico.1 as f64).round() as i32).max(1),
        );
        // El recorte se hace en los píxeles y no con `src`: las esquinas
        // redondeadas van grabadas en la copia, y con `src` se quedarían en la
        // parte recortada y la miniatura volvería a asomar en pico.
        let ((x, y), (cw, ch)) = en_pixeles(encuadre.imagen, self.mini_pixeles);
        let mut copias = self.mini.borrow_mut();
        while copias.len() <= indice {
            // Con las esquinas recortadas al radio del marco: compuesta en
            // rectángulo, la miniatura asomaba en pico por las cuatro esquinas
            // del borde redondeado de la vista de escritorios. El radio va en
            // píxeles de la imagen, que no es del tamaño del hueco.
            let mut rgba: Vec<u8> = self
                .mini_rgba
                .chunks_exact(self.mini_pixeles.0 as usize * 4)
                .skip(y as usize)
                .take(ch as usize)
                .flat_map(|fila| &fila[x as usize * 4..(x + cw) as usize * 4])
                .copied()
                .collect();
            let por_pixel = cw as f32 / tam.0 as f32;
            bookos_shell::redondear_esquinas(
                &mut rgba,
                cw as u32,
                ch as u32,
                bookos_shell::RADIO_MINIATURA_ESCRITORIO * por_pixel,
                bookos_shell::Alfa::Premultiplicado,
            );
            copias.push(MemoryRenderBuffer::from_slice(
                &rgba,
                Fourcc::Abgr8888,
                (cw, ch),
                1,
                Transform::Normal,
                None,
            ));
        }
        let src = Rectangle::from_size((cw as f64, ch as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (
                posicion.0 + (p.x * logico.0 as f64 * escala).round(),
                posicion.1 + (p.y * logico.1 as f64 * escala).round(),
            ),
            &copias[indice],
            Some(alfa),
            Some(src),
            Some(tam.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir la miniatura del fondo: {err}"))
        .ok()
    }
}

/// Un buffer de dibujo nuevo —con su propio id para el seguimiento de daño—
/// sobre los mismos píxeles, sin copiarlos.
// 0.1.0
fn compartido(rgba: &MemoryBuffer) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_memory(rgba.clone(), 1, Transform::Normal, None)
}

// 0.1.0
fn decodificar_estatico(ruta: &Path) -> Option<(Vec<u8>, u32, u32)> {
    if ruta.extension().and_then(|e| e.to_str()) != Some("svg") {
        return bookos_shell::decodificar_rgba(ruta);
    }
    let mut opciones = resvg::usvg::Options {
        resources_dir: ruta.parent().map(Path::to_path_buf),
        ..Default::default()
    };
    opciones.fontdb_mut().load_system_fonts();
    let datos = std::fs::read(ruta).ok()?;
    let arbol = resvg::usvg::Tree::from_data(&datos, &opciones).ok()?;
    let tamano = arbol.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(tamano.width(), tamano.height())?;
    // Un fondo es opaco por definición. Esto evita que un SVG con zonas
    // transparentes deje ver el color de seguridad distinto en cada backend.
    pixmap.fill(resvg::tiny_skia::Color::BLACK);
    resvg::render(
        &arbol,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some((pixmap.take(), tamano.width(), tamano.height()))
}

/// Reduce una imagen RGBA a `ancho` píxeles de ancho, promediando cada bloque.
///
/// Promedio y no vecino más próximo: el fondo tiene degradados y bandas finas,
/// y saltarse píxeles deja escalones visibles en una previa de 250 px. El coste
/// es una pasada por la imagen al arrancar.
// 0.1.0
fn reducir(rgba: &[u8], (w, h): (i32, i32), ancho: i32) -> (Vec<u8>, (i32, i32)) {
    if w <= ancho || w <= 0 || h <= 0 {
        return (rgba.to_vec(), (w, h));
    }
    let nw = ancho;
    let nh = ((h as f64 * ancho as f64 / w as f64).round() as i32).max(1);
    let mut destino = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let y0 = (y as i64 * h as i64 / nh as i64) as i32;
        let y1 = (((y + 1) as i64 * h as i64 / nh as i64) as i32)
            .max(y0 + 1)
            .min(h);
        for x in 0..nw {
            let x0 = (x as i64 * w as i64 / nw as i64) as i32;
            let x1 = (((x + 1) as i64 * w as i64 / nw as i64) as i32)
                .max(x0 + 1)
                .min(w);
            let mut suma = [0u32; 4];
            let mut cuenta = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy as usize * w as usize) + sx as usize) * 4;
                    for c in 0..4 {
                        suma[c] += rgba[i + c] as u32;
                    }
                    cuenta += 1;
                }
            }
            let d = ((y as usize * nw as usize) + x as usize) * 4;
            for c in 0..4 {
                destino[d + c] = (suma[c] / cuenta.max(1)) as u8;
            }
        }
    }
    (destino, (nw, nh))
}

/// Dónde buscar un fondo si la configuración no dice cuál.
///
/// Primero el del propio escritorio y luego los del sistema: así una
/// instalación de BookOS trae su fondo puesto, y una máquina donde solo está
/// el compositor no se queda en negro.
/// El fondo sigue al tema: los wallpapers vienen en pareja —`blue.png` claro y
/// `blue_dark.png` oscuro—, y arrancar en claro con el fondo de noche deja el
/// panel translúcido blanco sobre azul marino, que es justo lo que el tema
/// claro no quiere.
/// La familia de fondos que se usa cuando nadie ha elegido ninguna.
///
/// Los de BookOS vienen en cuatro familias —`blue`, `ember`, `pine`,
/// `purple`— y cada una con su pareja: `blue.png` en `Light/` y
/// `blue_dark.png` en `Dark/`. Aquí solo se nombra la familia; el sufijo y la
/// carpeta los pone [`buscar`] según el tema que esté puesto, que es lo que
/// permite que cambiar de tema se lleve el fondo con él.
const FAMILIA: &str = "blue";

/// El fondo que le toca al tema de ahora.
///
/// Se llama **cada vez que se recarga**, no solo al arrancar: es lo que hace
/// que pasar a claro cambie también la imagen. Antes tenía `blue` escrito a
/// fuego y las rutas de las cuatro familias instaladas no se miraban siquiera.
// 0.1.0
// modified 0.1.1 -- la búsqueda pasa a buscar_para, que sirve a los dos temas
fn buscar() -> Option<PathBuf> {
    buscar_para(bookos_shell::tema::es_claro())
}

/// El fondo de un tema concreto, lo elija el usuario o venga de serie. Lo pide
/// quien publica los dos a la vez para la pantalla de acceso.
// 0.1.1
pub fn ruta_para(eleccion: &Eleccion, claro: bool) -> Option<PathBuf> {
    eleccion
        .para(claro)
        .map(PathBuf::from)
        .or_else(|| buscar_para(claro))
}

// 0.1.1
fn buscar_para(claro: bool) -> Option<PathBuf> {
    let (carpeta, archivo) = if claro {
        ("Light", format!("{FAMILIA}.png"))
    } else {
        ("Dark", format!("{FAMILIA}_dark.png"))
    };
    let mut candidatos: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let base = home.join(".local/share/wallpapers/BookOS");
        // Con la carpeta del tema primero: es como los deja el instalador. La
        // ruta plana se sigue mirando después para no romper una instalación
        // hecha con la versión anterior, que las volcaba todas juntas.
        candidatos.push(base.join(carpeta).join(&archivo));
        candidatos.push(base.join(&archivo));
        // El repositorio de wallpapers, tal cual se clona para desarrollar.
        candidatos.push(
            home.join("Descargas/BookOS/BookOS-Wallpapers/Wallpapers-0.6")
                .join(carpeta)
                .join(&archivo),
        );
    }
    let sistema = PathBuf::from("/usr/share/wallpapers/BookOS");
    candidatos.push(sistema.join(carpeta).join(&archivo));
    candidatos.push(sistema.join(&archivo));
    candidatos.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporal(extension: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bookos-fondo-{}-{}.{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("prueba"),
            extension
        ))
    }

    #[test]
    fn un_svg_se_rasteriza_con_su_tamano() {
        let ruta = temporal("svg");
        std::fs::write(
            &ruta,
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4">
                 <rect width="8" height="4" fill="#ff0000"/>
               </svg>"##,
        )
        .expect("se puede crear el SVG temporal");
        let (rgba, w, h) = decodificar_estatico(&ruta).expect("SVG válido");
        std::fs::remove_file(&ruta).expect("se puede retirar el SVG temporal");
        assert_eq!((w, h), (8, 4));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn un_webp_animado_avanza_y_vuelve_al_principio() {
        use base64::Engine as _;

        // Dos cuadrados 2×2, rojo y azul, a 100 y 50 ms. Tener el fichero
        // incrustado hace que la prueba no dependa de ImageMagick ni de red.
        const WEBP: &str = "UklGRsAAAABXRUJQVlA4WAoAAAACAAAAAQAAAQAAQU5JTQYAAAD/////AABBTk1GSAAAAAAAAAAAAAEAAAEAAGQAAAJWUDggMAAAANABAJ0BKgIAAgACADQloAJ0ugH4AAOwAP7wxAv/ILlhdcjX/yA/5Af8gP/48gAAAEFOTUZEAAAAAAAAAAAAAQAAAQAAMgAAAFZQOCAsAAAAlAEAnQEqAgACAAAANCWgAnS6AAOYAP75k2//kB//kB//kB//ID/iF3sgMAA=";
        let ruta = temporal("webp");
        let datos = base64::engine::general_purpose::STANDARD
            .decode(WEBP)
            .expect("fixture WebP en base64");
        std::fs::write(&ruta, datos).expect("se puede crear el WebP temporal");

        let (mut animacion, primero) = Animacion::abrir(&ruta).expect("WebP animado");
        assert_eq!(primero.pixeles, (2, 2));
        let segundo = nuevo(animacion.avanzar());
        let vuelta = nuevo(animacion.avanzar());
        std::fs::remove_file(&ruta).expect("se puede retirar el WebP temporal");
        assert_ne!(&primero.rgba[..], &segundo.rgba[..]);
        assert_eq!(&primero.rgba[..], &vuelta.rgba[..]);
    }

    fn nuevo(paso: Paso) -> Fotograma {
        match paso {
            Paso::Nuevo(fotograma) => fotograma,
            Paso::Mismo | Paso::Fin => panic!("se esperaba un fotograma nuevo"),
        }
    }

    /// Un vídeo girado sale con el ancho y el alto cambiados, sigue en bucle al
    /// acabarse, y al soltarlo no deja ffmpeg vivo. Necesita `ffmpeg` y
    /// `ffprobe` instalados, que es lo mismo que necesita el escritorio.
    #[test]
    fn un_video_girado_se_reproduce_en_bucle() {
        let ruta = temporal("mkv");
        let plano = temporal("plano.mkv");
        let ffmpeg = |args: &[&str], salida: &Path| {
            let ok = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(args)
                .arg(salida)
                .status()
                .expect("ffmpeg instalado");
            assert!(ok.success(), "ffmpeg no pudo crear {salida:?}");
        };
        // Rojo y luego azul, medio segundo cada uno a 4 fps. La fuente `color`
        // de lavfi pasa por YUV y el rojo llega como 253: se compara el canal
        // que domina, no el valor exacto.
        ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=red:s=8x4:r=4:d=0.5",
                "-f",
                "lavfi",
                "-i",
                "color=blue:s=8x4:r=4:d=0.5",
                "-filter_complex",
                "[0][1]concat",
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "bgr0",
            ],
            &plano,
        );
        // El giro va como opción de **entrada** de la copia: es como lo
        // guardan los móviles, en metadatos y no en los píxeles.
        let plano_str = plano.to_str().expect("ruta temporal en UTF-8");
        ffmpeg(
            &["-display_rotation", "90", "-i", plano_str, "-c", "copy"],
            &ruta,
        );
        std::fs::remove_file(&plano).expect("se puede retirar el vídeo plano");

        let (mut animacion, primero) = Animacion::abrir(&ruta).expect("vídeo legible");
        std::fs::remove_file(&ruta).expect("se puede retirar el vídeo temporal");
        assert_eq!(primero.pixeles, (4, 8));
        let es_rojo = |rgba: &[u8]| rgba[0] > 200 && rgba[2] < 50;
        assert!(es_rojo(&primero.rgba));
        let Animacion::Video(video) = &mut animacion else {
            panic!("un .mkv se abre como vídeo");
        };
        let pid = video.proceso.id();
        // Tras el primer rojo: rojo, azul, azul y vuelta al rojo.
        let rojos: Vec<bool> = (0..4)
            .map(|_| {
                let f = video
                    .fotogramas
                    .recv_timeout(Duration::from_secs(5))
                    .expect("el vídeo sigue dando fotogramas");
                es_rojo(&f.rgba)
            })
            .collect();
        assert_eq!(rojos, [true, false, false, true]);
        drop(animacion);
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "ffmpeg sigue vivo tras soltar el fondo"
        );
    }

    /// Un vídeo que no es AV1 se convierte en segundo plano, el resultado se
    /// queda con su nombre definitivo solo al acabar bien, y se reproduce.
    #[test]
    fn un_video_mpeg4_se_convierte_a_av1() {
        let ruta = temporal("avi");
        let destino = temporal("convertido.mkv");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "color=red:s=64x64:r=4:d=1", "-c:v", "mpeg4"])
            .arg(&ruta)
            .status()
            .expect("ffmpeg instalado");
        assert!(ok.success(), "ffmpeg no pudo crear {ruta:?}");

        let sondeo = sondear(&ruta).expect("vídeo legible");
        assert_eq!(sondeo.codec, "mpeg4");
        let (mut video, _) =
            Video::reproducir(&ruta, &sondeo, false).expect("el original se reproduce");
        video.conversion =
            convertir(&ruta, &sondeo, &destino).map(|proceso| Conversion::Convirtiendo {
                proceso,
                destino: destino.clone(),
                sondeo,
            });
        assert!(video.conversion.is_some(), "la conversión arranca");
        let limite = Instant::now() + Duration::from_secs(60);
        let (convertido, primero) = loop {
            if let Some(relevo) = video.relevo() {
                break relevo;
            }
            assert!(video.conversion.is_some(), "la conversión falló");
            assert!(Instant::now() < limite, "la conversión no acaba");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(video.conversion.is_none(), "el relevo cierra la conversión");
        drop((video, convertido));
        std::fs::remove_file(&ruta).expect("se puede retirar el original");

        assert!(!parcial(&destino).exists(), "el parcial se renombra");
        let codec = sondear(&destino).expect("el convertido se lee").codec;
        std::fs::remove_file(&destino).expect("se puede retirar el convertido");
        assert_eq!(codec, "av1");
        assert_eq!(primero.pixeles, (64, 64));
        assert!(
            primero.rgba[0] > 200 && primero.rgba[2] < 50,
            "sigue siendo rojo"
        );
    }

    /// La pareja manda sobre el fondo común, y cambiar de tema cambia la
    /// imagen. Es lo que hacía falta para que pasar a claro no dejara el
    /// escritorio con la foto oscura detrás.
    #[test]
    fn cada_tema_se_lleva_su_fondo() {
        let eleccion = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: Some("/claro.png".into()),
            oscuro: Some("/oscuro.png".into()),
            ..Default::default()
        };
        assert_eq!(eleccion.para(true), Some("/claro.png"));
        assert_eq!(eleccion.para(false), Some("/oscuro.png"));
    }

    /// Con solo uno de los dos puestos, el otro tema cae al común: quien pone
    /// `fondo_oscuro` y deja `fondo` está diciendo «este de noche y el otro el
    /// resto del tiempo».
    #[test]
    fn el_tema_sin_pareja_cae_al_comun() {
        let eleccion = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: None,
            oscuro: Some("/oscuro.png".into()),
            ..Default::default()
        };
        assert_eq!(eleccion.para(true), Some("/comun.png"));
        assert_eq!(eleccion.para(false), Some("/oscuro.png"));
    }

    /// Un `fondo` fijo y sin pareja no depende del tema: recargarlo al cambiar
    /// de claro a oscuro serían veinte megas decodificados para poner lo mismo.
    #[test]
    fn un_fondo_fijo_no_se_recarga_por_el_tema() {
        let fijo = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: None,
            oscuro: None,
            ..Default::default()
        };
        assert!(!fijo.depende_del_tema());

        // Con pareja sí, y sin nada también: ahí manda `buscar`, que elige
        // carpeta y sufijo por el tema.
        let con_pareja = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: Some("/claro.png".into()),
            oscuro: None,
            ..Default::default()
        };
        assert!(con_pareja.depende_del_tema());
        assert!(Eleccion::default().depende_del_tema());
    }

    fn cerca(a: Trozo, b: (f64, f64, f64, f64)) {
        let dif = [a.x - b.0, a.y - b.1, a.w - b.2, a.h - b.3];
        assert!(dif.iter().all(|d| d.abs() < 1e-9), "{a:?} frente a {b:?}");
    }

    /// Cuadrada de 800 en una pantalla de 1600×1000: cada modo recorta o deja
    /// banda en el eje que le toca.
    #[test]
    fn cada_encaje_recorta_o_deja_banda_donde_toca() {
        let con = |e| Encuadre::calcular(e, (800, 800), (1600, 1000));

        let e = con(EncajeFondo::Rellenar);
        cerca(e.imagen, (0.0, 0.1875, 1.0, 0.625));
        cerca(e.pantalla, (0.0, 0.0, 1.0, 1.0));

        let e = con(EncajeFondo::Ajustar);
        cerca(e.imagen, (0.0, 0.0, 1.0, 1.0));
        cerca(e.pantalla, (0.1875, 0.0, 0.625, 1.0));

        // Con esta imagen, a lo ancho coincide con rellenar...
        assert_eq!(con(EncajeFondo::Ancho), con(EncajeFondo::Rellenar));
        // ...y con una panorámica no: deja bandas arriba y abajo.
        let e = Encuadre::calcular(EncajeFondo::Ancho, (1600, 400), (1600, 1000));
        cerca(e.imagen, (0.0, 0.0, 1.0, 1.0));
        cerca(e.pantalla, (0.0, 0.3, 1.0, 0.4));

        let e = con(EncajeFondo::Estirar);
        assert_eq!(e.imagen, Trozo::ENTERO);
        assert_eq!(e.pantalla, Trozo::ENTERO);

        let e = con(EncajeFondo::Centrar);
        cerca(e.imagen, (0.0, 0.0, 1.0, 1.0));
        cerca(e.pantalla, (0.25, 0.1, 0.5, 0.8));
        // Mayor que la pantalla, se recorta y no se reduce.
        let e = Encuadre::calcular(EncajeFondo::Centrar, (2000, 2000), (1600, 1000));
        cerca(e.imagen, (0.1, 0.25, 0.8, 0.5));
        cerca(e.pantalla, (0.0, 0.0, 1.0, 1.0));
    }

    /// El cristal lee lo mismo que se ve, y sobre una banda no se queda sin
    /// imagen: un `uv_tam` a cero dividiría por cero en el shader.
    #[test]
    fn el_cristal_lee_lo_que_hay_detras() {
        let relleno = Encuadre::calcular(EncajeFondo::Rellenar, (800, 800), (1600, 1000));
        let arriba = relleno.detras_de(Trozo {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 0.1,
        });
        cerca(arriba, (0.0, 0.1875, 1.0, 0.0625));

        let bandas = Encuadre::calcular(EncajeFondo::Ajustar, (800, 800), (1600, 1000));
        let en_banda = bandas.detras_de(Trozo {
            x: 0.0,
            y: 0.0,
            w: 0.1,
            h: 1.0,
        });
        assert_eq!(en_banda.x, 0.0);
        assert!(en_banda.w > 0.0);
        assert!(!bandas.tiene_imagen_detras(Trozo {
            x: 0.0,
            y: 0.0,
            w: 0.1,
            h: 1.0,
        }));
        assert!(bandas.tiene_imagen_detras(Trozo {
            x: 0.1,
            y: 0.0,
            w: 0.2,
            h: 1.0,
        }));
    }
}
