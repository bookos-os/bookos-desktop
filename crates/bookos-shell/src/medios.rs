//! Qué se está reproduciendo, para la tarjeta del centro de control.
//!
//! **MPRIS con `busctl`, no con un cliente de D-Bus.** La especificación
//! (`org.mpris.MediaPlayer2.Player`) es el estándar que implementan todos los
//! reproductores; hablarla desde aquí exigiría una dependencia nueva y un hilo
//! dentro del compositor. `busctl` viene con systemd y es el mismo camino que
//! ya se usa para el brillo y para los perfiles de energía.
//!
//! **Solo se consulta con la tarjeta abierta.** Son tres procesos por
//! consulta —listar, estado y metadatos—, así que en reposo esto no corre: el
//! centro de control cerrado no existe.
//!
//! La carátula se lee si `mpris:artUrl` es un fichero local —lo que dan
//! kdeconnect (BookOS Link) y los reproductores de escritorio— o una `data:`
//! URL, que es como la publica BookOS Player. Las URL `https` de los
//! navegadores no se descargan desde aquí.

use std::process::{Command, Stdio};

use iced_core::Color;
use iced_widget::image::Handle;

/// Lo que está sonando ahora mismo.
#[derive(Debug, Clone, PartialEq)]
pub struct Sonando {
    /// El nombre del bus, para mandarle las órdenes: `org.mpris.MediaPlayer2.vlc`.
    pub bus: String,
    /// Cómo se llama el reproductor, ya legible: `VLC`, o con BookOS Link
    /// `BitChord - Galaxy S22 Ultra`, que dice también de qué móvil viene.
    pub aplicacion: String,
    /// La carátula como ruta local o como `data:` URL, si el reproductor
    /// publica una. Ver [`cargar_caratula`].
    pub caratula: Option<String>,
    pub titulo: String,
    pub artista: String,
    pub reproduciendo: bool,
    /// Posición y duración en segundos, si el reproductor las publica.
    pub posicion: Option<u64>,
    pub duracion: Option<u64>,
    /// Posición y duración en milisegundos, para avanzar la barra de progreso
    /// con suavidad entre lecturas de MPRIS.
    pub posicion_ms: Option<u64>,
    pub duracion_ms: Option<u64>,
    /// Códec, frecuencia y bits del fichero. Solo lo publica BookOS Player, en
    /// claves `bookos:*`: MPRIS no tiene ninguna para esto.
    // añadido en la 0.2.0
    pub calidad: Option<Calidad>,
}

/// La calidad del fichero que suena, tal como la publica BookOS Player.
// añadido en la 0.2.0
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Calidad {
    pub codec: String,
    /// Hz; 0 si no se sabe.
    pub frecuencia: u32,
    /// 0 en los códecs con pérdida, que no tienen profundidad de bits.
    pub bits: u32,
    /// kbps.
    pub tasa: u32,
}

/// Qué nivel enseña la etiqueta, como Apple Music: «Hi-Res Lossless» desde 24
/// bits y más de 48 kHz, «Lossless» con un códec sin pérdida, y el nombre del
/// códec sin más en los demás.
// añadido en la 0.2.0
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Nivel {
    HiRes,
    SinPerdida,
    ConPerdida,
}

impl Calidad {
    // añadido en la 0.2.0
    pub fn nivel(&self) -> Nivel {
        let sin_perdida = matches!(
            self.codec.as_str(),
            "FLAC" | "ALAC" | "PCM" | "PCM Float" | "WavPack" | "Monkey’s Audio"
        );
        match (sin_perdida, self.bits, self.frecuencia) {
            (true, b, f) if b >= 24 && f > 48_000 => Nivel::HiRes,
            (true, ..) => Nivel::SinPerdida,
            (false, ..) => Nivel::ConPerdida,
        }
    }

    /// El detalle técnico: «FLAC · 24 bits · 96 kHz» o «AAC · 256 kbps».
    // añadido en la 0.2.0
    pub fn detalle(&self) -> String {
        let mut partes = vec![self.codec.clone()];
        if self.nivel() == Nivel::ConPerdida {
            if self.tasa > 0 {
                partes.push(format!("{} kbps", self.tasa));
            }
        } else {
            if self.bits > 0 {
                partes.push(format!("{} bits", self.bits));
            }
            if self.frecuencia > 0 {
                partes.push(khz(self.frecuencia));
            }
        }
        partes.join(" · ")
    }
}

/// 44100 → «44,1 kHz»; 48000 → «48 kHz».
// añadido en la 0.2.0
pub fn khz(hz: u32) -> String {
    let k = hz as f32 / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{} kHz", k.round() as u32)
    } else {
        format!("{:.1} kHz", k).replace('.', ",")
    }
}

/// Cómo repite el reproductor, de `LoopStatus`.
// añadido en la 0.2.0
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Repetir {
    No,
    Cancion,
    Lista,
}

impl Repetir {
    /// El siguiente al pulsar: no → lista → canción → no, como Apple Music.
    // añadido en la 0.2.0
    pub fn siguiente(self) -> Self {
        match self {
            Self::No => Self::Lista,
            Self::Lista => Self::Cancion,
            Self::Cancion => Self::No,
        }
    }
    fn mpris(self) -> &'static str {
        match self {
            Self::No => "None",
            Self::Cancion => "Track",
            Self::Lista => "Playlist",
        }
    }
}

/// Aleatorio y repetición del reproductor, si los publica. Se leen al abrir lo
/// que suena en grande y no cada segundo: son dos `busctl` más por lectura.
// añadido en la 0.2.0
pub fn modos(bus: &str) -> (Option<bool>, Option<Repetir>) {
    let aleatorio = propiedad(bus, "Shuffle").map(|v| v.contains("true"));
    let repetir = propiedad(bus, "LoopStatus").and_then(|v| {
        if v.contains("Playlist") {
            Some(Repetir::Lista)
        } else if v.contains("Track") {
            Some(Repetir::Cancion)
        } else if v.contains("None") {
            Some(Repetir::No)
        } else {
            None
        }
    });
    (aleatorio, repetir)
}

/// Pone el aleatorio, sin esperar respuesta.
// añadido en la 0.2.0
pub fn poner_aleatorio(bus: &str, si: bool) -> bool {
    fijar(bus, "Shuffle", "b", if si { "true" } else { "false" })
}

/// Pone la repetición, sin esperar respuesta.
// añadido en la 0.2.0
pub fn poner_repetir(bus: &str, modo: Repetir) -> bool {
    fijar(bus, "LoopStatus", "s", modo.mpris())
}

/// Escribe una propiedad de `org.mpris.MediaPlayer2.Player`, como [`llamar`].
// añadido en la 0.2.0
fn fijar(bus: &str, nombre: &str, tipo: &str, valor: &str) -> bool {
    let Ok(mut hijo) = Command::new("busctl")
        .args([
            "--user",
            "set-property",
            bus,
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            nombre,
            tipo,
            valor,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    std::thread::spawn(move || {
        let _ = hijo.wait();
    });
    true
}

impl Sonando {
    /// Lo que debe salir en la tarjeta. Prefiere lo que esté reproduciendo y
    /// descarta visores de imágenes que publican MPRIS para una presentación.
    // 0.1.0
    pub fn leer() -> Option<Self> {
        Self::consultar().unwrap_or_else(|error| {
            tracing::debug!("no se pudo consultar el reproductor: {error}");
            None
        })
    }

    /// Todas las fuentes multimedia activas, primero las que están
    /// reproduciendo y después las pausadas.
    // añadido en la 0.2.4
    pub fn leer_todos() -> Vec<Self> {
        Self::consultar_todos().unwrap_or_else(|error| {
            tracing::debug!("no se pudieron consultar los reproductores: {error}");
            Vec::new()
        })
    }

    /// Una lectura fallida no confirma que se haya cerrado el reproductor.
    /// El centro conserva su última tarjeta en ese caso.
    // 0.1.0
    pub fn consultar() -> Result<Option<Self>, String> {
        Self::consultar_todos().map(|lista| lista.into_iter().next())
    }

    /// Consulta todos los reproductores MPRIS válidos: los que están
    /// reproduciendo primero y los pausados después.
    // añadido en la 0.2.4
    pub fn consultar_todos() -> Result<Vec<Self>, String> {
        let mut reproduciendo = Vec::new();
        let mut pausados = Vec::new();
        let mut fallo = None;
        for bus in reproductores()? {
            let lectura =
                consultar_propiedad_de(&bus, "org.mpris.MediaPlayer2.Player", "PlaybackStatus")
                    .and_then(|estado| {
                        consultar_propiedad_de(&bus, "org.mpris.MediaPlayer2.Player", "Metadata")
                            .map(|metadatos| (estado, metadatos))
                    });
            let (estado, metadatos) = match lectura {
                Ok(lectura) => lectura,
                Err(error) => {
                    fallo = Some(error);
                    continue;
                }
            };
            let posicion_us = propiedad(&bus, "Position").and_then(|s| numero(&s));
            let Some(actual) = Self::desde_mpris(bus, &estado, &metadatos, posicion_us) else {
                continue;
            };
            // El `<audio>` del Player hace que WebKit publique su propia sesión
            // con el título de la ventana («BookOS Player») y sin carátula. Lo
            // de verdad va por `bookosplayer`; si ese falta, mejor sin tarjeta
            // que una tarjeta vacía. Visto en el bloqueo el 2026-11-18.
            if actual.bus.contains(".org.webkit.app-") && actual.aplicacion == "bookos-player" {
                continue;
            }
            if actual.reproduciendo {
                reproduciendo.push(actual);
            } else {
                pausados.push(actual);
            }
        }
        reproduciendo.extend(pausados);
        if reproduciendo.is_empty()
            && let Some(error) = fallo
        {
            return Err(error);
        }
        Ok(reproduciendo)
    }

    // 0.1.0
    fn desde_mpris(
        bus: String,
        estado: &str,
        metadatos: &str,
        posicion_us: Option<u64>,
    ) -> Option<Self> {
        if es_imagen(&bus, metadatos) {
            return None;
        }
        let titulo = campo(metadatos, "xesam:title")?;
        let reproduciendo = estado.contains("Playing");
        let duracion_us = campo_numero(metadatos, "mpris:length");
        let duracion_ms = duracion_us.map(|us| us / 1_000);
        let duracion = duracion_us.map(|us| us / 1_000_000);
        let posicion_ms = match (posicion_us.map(|us| us / 1_000), duracion_ms) {
            (Some(p), Some(d)) if d > 0 => Some(p.min(d)),
            (p, _) => p,
        };
        let posicion = posicion_ms.map(|ms| ms / 1_000);
        // KDE Connect extrapola la posición; un desfase no prueba que se haya
        // cerrado la sesión ni autoriza a ocultar sus controles.
        if posicion.is_none() || duracion.is_none() {
            tracing::debug!(
                bus,
                ?posicion,
                ?duracion,
                "el reproductor no publica posición o duración"
            );
        }
        let posicion = match (posicion, duracion) {
            (Some(p), Some(d)) if d > 0 => Some(p.min(d)),
            (p, _) => p,
        };
        Some(Self {
            // `Identity` es el nombre que el propio reproductor quiere enseñar.
            // El sufijo del bus solo vale de reserva: el de kdeconnect es
            // `mpris_d4b8b016…`, un identificador de dispositivo ilegible.
            aplicacion: identidad(&bus).unwrap_or_else(|| {
                bus.rsplit('.')
                    .next()
                    .unwrap_or(&bus)
                    .replace("instance", "")
            }),
            caratula: campo(metadatos, "mpris:artUrl").and_then(|url| {
                if url.starts_with("data:image/") {
                    Some(url)
                } else {
                    url.strip_prefix("file://").map(str::to_string)
                }
            }),
            artista: campo(metadatos, "xesam:artist").unwrap_or_default(),
            reproduciendo,
            posicion,
            duracion,
            posicion_ms,
            duracion_ms,
            calidad: campo(metadatos, "bookos:codec").map(|codec| Calidad {
                codec,
                frecuencia: campo_numero(metadatos, "bookos:samplerate").unwrap_or(0) as u32,
                bits: campo_numero(metadatos, "bookos:bitdepth").unwrap_or(0) as u32,
                tasa: campo_numero(metadatos, "bookos:bitrate").unwrap_or(0) as u32,
            }),
            titulo,
            bus,
        })
    }

    /// La orden que hay que ejecutar para un botón de la tarjeta.
    // 0.1.0
    pub fn orden(&self, que: Orden) -> String {
        let metodo = match que {
            Orden::Anterior => "Previous",
            Orden::Siguiente => "Next",
            Orden::Alternar => "PlayPause",
        };
        format!(
            "busctl --user call {} /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player {metodo}",
            self.bus
        )
    }

    /// Ejecuta una orden sin esperar a que el reproductor responda.
    ///
    /// El lockscreen usa este camino: el clic debe volver al compositor de
    /// inmediato y `busctl` termina por su cuenta. El nombre del bus ya se
    /// resolvió al leer la tarjeta, así que aquí no se lanza una consulta extra.
    // 0.1.0
    pub fn ejecutar(&self, que: Orden) -> bool {
        ejecutar_en(&self.bus, que)
    }

    /// El móvil emparejado con BookOS Link. Su reproductor MPRIS sigue en el
    /// bus, pausado y sin `mpris:length`, mientras Android conserve la sesión
    /// multimedia —medido el 2026-10-26: `Paused`, `Position` 6,6 s congelada,
    /// `CanSeek` falso—, así que «pausado» ahí no quiere decir «lo acabo de
    /// pausar».
    // 0.1.1
    pub fn es_de_movil(&self) -> bool {
        self.bus.contains(".kdeconnect.")
    }

    /// Posición en segundos con precisión de milisegundos si está disponible.
    pub fn posicion_seg(&self) -> Option<f64> {
        self.posicion_ms
            .map(|ms| ms as f64 / 1000.0)
            .or_else(|| self.posicion.map(|s| s as f64))
    }

    /// Duración en segundos con precisión de milisegundos si está disponible.
    pub fn duracion_seg(&self) -> Option<f64> {
        self.duracion_ms
            .map(|ms| ms as f64 / 1000.0)
            .or_else(|| self.duracion.map(|s| s as f64))
    }

    /// De 0 a 1, para la barra de progreso. `None` si el reproductor no publica
    /// las dos cosas —los navegadores a menudo no dan `Position`—.
    // 0.1.0
    pub fn avance(&self) -> Option<f32> {
        let (p, d) = (self.posicion_seg()?, self.duracion_seg()?);
        (d > 0.0).then(|| (p / d).clamp(0.0, 1.0) as f32)
    }
}

/// El `.desktop` del reproductor (`DesktopEntry` de MPRIS): `bookos-player`,
/// `firefox`… Con él se busca su icono. Se lee al abrir lo que suena en
/// grande, no en cada lectura.
// añadido en la 0.2.2
pub fn escritorio(bus: &str) -> Option<String> {
    let nombre = cadena(&propiedad_de(
        bus,
        "org.mpris.MediaPlayer2",
        "DesktopEntry",
    )?)?;
    (!nombre.trim().is_empty()).then(|| nombre.trim().to_string())
}

/// El `mpris:trackid` de lo que suena, que pide el favorito de BookOS Player.
// añadido en la 0.2.2
pub fn pista(bus: &str) -> Option<String> {
    campo(&propiedad(bus, "Metadata")?, "mpris:trackid")
}

/// Si la canción es favorita, por `org.bookos.MediaExtras1` (solo BookOS
/// Player). `None` si el reproductor no tiene la interfaz: entonces no se
/// enseña el corazón.
// añadido en la 0.2.2
pub fn favorito(bus: &str, pista: &str) -> Option<bool> {
    let salida = Command::new("busctl")
        .args([
            "--user",
            "call",
            bus,
            "/org/mpris/MediaPlayer2",
            "org.bookos.MediaExtras1",
            "IsFavorite",
            "o",
            pista,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !salida.status.success() {
        return None;
    }
    // `busctl call` contesta `b true` o `b false`.
    Some(String::from_utf8_lossy(&salida.stdout).contains("true"))
}

/// Marca o desmarca la favorita, sin esperar respuesta, como [`poner_aleatorio`].
// añadido en la 0.2.2
pub fn poner_favorito(bus: &str, pista: &str, si: bool) -> bool {
    let Ok(mut hijo) = Command::new("busctl")
        .args([
            "--user",
            "call",
            bus,
            "/org/mpris/MediaPlayer2",
            "org.bookos.MediaExtras1",
            "SetFavorite",
            "ob",
            pista,
            if si { "true" } else { "false" },
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    std::thread::spawn(move || {
        let _ = hijo.wait();
    });
    true
}

/// La carátula cuadrada, reducida a `lado` píxeles con las esquinas de `radio`,
/// y el color que la resume. `fuente` es lo que guarda [`Sonando::caratula`].
///
/// La comparten el centro de control y el bloqueo; cada uno la llama solo al
/// cambiar de fuente, no por lectura.
// 0.1.0
pub fn cargar_caratula(fuente: &str, lado: u32, radio: f32) -> Option<(Handle, Color)> {
    let imagen = abrir_caratula(fuente)?;
    let (handle, rgba) = recortar(&imagen, lado, lado, radio);
    let color = crate::actividad::color_dominante(&crate::actividad::Portada {
        rgba,
        width: lado,
        height: lado,
    });
    Some((handle, color))
}

/// La versión a lo ancho para un vídeo: `None` si la carátula no es apaisada
/// (más de 4:3), que es como se distingue la miniatura de un vídeo de una
/// portada de disco. El hueco de la vista grande es 16:9.
// añadido en la 0.2.2
pub fn cargar_caratula_ancha(fuente: &str, ancho: u32, alto: u32, radio: f32) -> Option<Handle> {
    let imagen = abrir_caratula(fuente)?;
    (imagen.width() as f32 > imagen.height() as f32 * 1.34)
        .then(|| recortar(&imagen, ancho, alto, radio).0)
}

// añadido en la 0.2.2 -- sale de cargar_caratula para servir también a la ancha
fn abrir_caratula(fuente: &str) -> Option<image::DynamicImage> {
    use base64::Engine;
    let bytes = match fuente.strip_prefix("data:image/") {
        Some(resto) => base64::engine::general_purpose::STANDARD
            .decode(resto.split_once(',')?.1)
            .inspect_err(|err| tracing::debug!("carátula data: ilegible: {err}"))
            .ok()?,
        None => std::fs::read(fuente)
            .inspect_err(|err| tracing::debug!(fuente, "carátula ilegible: {err}"))
            .ok()?,
    };
    image::load_from_memory(&bytes)
        .inspect_err(|err| tracing::debug!("carátula ilegible: {err}"))
        .ok()
}

/// Recorta al centro con la proporción `ancho`×`alto`, escala y redondea.
/// Devuelve también los píxeles sin redondear, para sacar el color.
// añadido en la 0.2.2
fn recortar(imagen: &image::DynamicImage, ancho: u32, alto: u32, radio: f32) -> (Handle, Vec<u8>) {
    let (w, h) = (imagen.width(), imagen.height());
    // El corte más grande con la proporción pedida que cabe en la imagen.
    let (cw, ch) = if w as u64 * alto as u64 > h as u64 * ancho as u64 {
        ((h as u64 * ancho as u64 / alto as u64) as u32, h)
    } else {
        (w, (w as u64 * alto as u64 / ancho as u64) as u32)
    };
    let corte = image::imageops::crop_imm(imagen, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image();
    let escalada =
        image::imageops::resize(&corte, ancho, alto, image::imageops::FilterType::Triangle);
    let mut rgba = escalada.into_raw();
    let sin_esquinas = rgba.clone();
    crate::redondear_esquinas(&mut rgba, ancho, alto, radio, crate::Alfa::Recto);
    (Handle::from_rgba(ancho, alto, rgba), sin_esquinas)
}

/// La variante para una vista que ya guardó solo el nombre del bus.
// 0.1.0
// modified 0.1.1 -- la llamada a busctl pasa por llamar()
pub fn ejecutar_en(bus: &str, que: Orden) -> bool {
    let metodo = match que {
        Orden::Anterior => "Previous",
        Orden::Siguiente => "Next",
        Orden::Alternar => "PlayPause",
    };
    llamar(bus, &[metodo])
}

/// Mueve la reproducción `desfase` segundos, hacia delante o hacia atrás.
///
/// `Seek` relativo y no `SetPosition`: este último pide el `mpris:trackid`, que
/// `busctl` da como ruta de objeto y no todos los reproductores publican; el
/// relativo lo implementan igual kdeconnect (`mprisremote`) y BookOS Player.
// 0.1.1
pub fn saltar_en(bus: &str, desfase: i64) -> bool {
    llamar(bus, &["Seek", "x", &(desfase * 1_000_000).to_string()])
}

/// Un método de `org.mpris.MediaPlayer2.Player` sin esperar la respuesta.
// 0.1.1
fn llamar(bus: &str, metodo: &[&str]) -> bool {
    let Ok(mut hijo) = Command::new("busctl")
        .args([
            "--user",
            "call",
            // Sin esto un `Seek` hacia atrás no llegaba nunca: busctl leía
            // «-5000000» como una opción («invalid option -- '5'») y el error
            // se perdía en el stderr nulo.
            "--",
            bus,
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        ])
        .args(metodo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Sin `wait`, cada pulsación dejaba un `busctl` zombi; en un hilo para no
    // parar el bucle lo que tarde en contestar el reproductor.
    std::thread::spawn(move || {
        let _ = hijo.wait();
    });
    true
}

/// Los tres botones de la tarjeta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orden {
    Anterior,
    Alternar,
    Siguiente,
}

/// Formatea unos segundos como `m:ss`.
// 0.1.0
pub fn reloj(segundos: u64) -> String {
    format!("{}:{:02}", segundos / 60, segundos % 60)
}

// 0.1.0
fn reproductores() -> Result<Vec<String>, String> {
    let salida = Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("No se pudo listar MPRIS: {error}"))?;
    if !salida.status.success() {
        return Err(format!(
            "La consulta de reproductores terminó con {}",
            salida.status
        ));
    }
    let texto = String::from_utf8_lossy(&salida.stdout);
    let mut nombres: Vec<&str> = texto
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    nombres.sort();
    Ok(nombres.into_iter().map(str::to_string).collect())
}

/// MPRIS también lo implementan algunos visores para avanzar diapositivas.
/// Eso no convierte una foto en música: se reconoce por el reproductor o por
/// la URL original, nunca por `mpris:artUrl` porque esa sí es una carátula.
// 0.1.0
fn es_imagen(bus: &str, metadatos: &str) -> bool {
    let bus = bus.to_ascii_lowercase();
    let visor = [
        "gwenview",
        "loupe",
        "eog",
        "ristretto",
        "nomacs",
        "qimgv",
        ".imv",
        ".feh",
    ]
    .iter()
    .any(|nombre| bus.contains(nombre));
    if visor {
        return true;
    }
    let Some(url) = campo(metadatos, "xesam:url") else {
        return false;
    };
    let ruta = url
        .split(['?', '#'])
        .next()
        .unwrap_or(&url)
        .to_ascii_lowercase();
    [
        "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "bmp", "tif", "tiff", "svg",
    ]
    .iter()
    .any(|extension| ruta.ends_with(&format!(".{extension}")))
}

// 0.1.0
fn propiedad(bus: &str, nombre: &str) -> Option<String> {
    propiedad_de(bus, "org.mpris.MediaPlayer2.Player", nombre)
}

// 0.1.0
fn identidad(bus: &str) -> Option<String> {
    // `busctl` la imprime como `s "Nombre"`.
    let volcado = propiedad_de(bus, "org.mpris.MediaPlayer2", "Identity")?;
    let nombre = cadena(&volcado)?;
    let nombre = nombre.trim();
    (!nombre.is_empty()).then(|| nombre.to_string())
}

// 0.1.0
fn propiedad_de(bus: &str, interfaz: &str, nombre: &str) -> Option<String> {
    consultar_propiedad_de(bus, interfaz, nombre).ok()
}

// 0.1.0
fn consultar_propiedad_de(bus: &str, interfaz: &str, nombre: &str) -> Result<String, String> {
    let salida = Command::new("busctl")
        .args([
            "--user",
            "get-property",
            bus,
            "/org/mpris/MediaPlayer2",
            interfaz,
            nombre,
        ])
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("No se pudo consultar {nombre} de {bus}: {error}"))?;
    if !salida.status.success() {
        return Err(format!(
            "No se pudo leer {nombre} de {bus}: {}",
            salida.status
        ));
    }
    Ok(String::from_utf8_lossy(&salida.stdout).into_owned())
}

/// Saca un campo de texto del volcado de `Metadata`.
///
/// `busctl` imprime el diccionario en una línea: `... "xesam:title" s "Canción"
/// "xesam:artist" as 1 "Artista" ...`. Se busca la clave y se coge la siguiente
/// cadena entrecomillada, saltándose el tipo. En vez de analizar el formato
/// entero —que cambia entre versiones— solo se leen los tres campos que se
/// enseñan.
// 0.1.0
fn campo(volcado: &str, clave: &str) -> Option<String> {
    let resto = volcado.split_once(&format!("\"{clave}\""))?.1;
    let valor = cadena(resto)?;
    let valor = valor.trim();
    (!valor.is_empty()).then(|| valor.to_string())
}

/// La primera cadena entrecomillada de un trozo de salida de `busctl`, con sus
/// escapes deshechos.
///
/// `busctl` escapa en octal todo byte que no sea ASCII imprimible: «Más» sale
/// como `M\303\241s` (visto con Firefox), y las comillas de dentro como `\"`.
/// Copiar el texto tal cual enseñaba los códigos en el bloqueo, y cortar en
/// cualquier comilla partía los títulos que llevan comillas.
// 0.1.0
// modified 0.2.0 -- pub(crate): la tarjeta de Bluetooth lee el JSON de budsd por busctl
pub(crate) fn cadena(texto: &str) -> Option<String> {
    let mut bytes = texto.split_once('"')?.1.bytes();
    let mut valor = Vec::new();
    while let Some(b) = bytes.next() {
        match b {
            b'"' => return Some(String::from_utf8_lossy(&valor).into_owned()),
            b'\\' => match bytes.next()? {
                d @ b'0'..=b'7' => {
                    let mut n = u32::from(d - b'0');
                    for _ in 0..2 {
                        n = n * 8 + u32::from(bytes.next()?.checked_sub(b'0')?);
                    }
                    valor.push(u8::try_from(n).ok()?);
                }
                b'n' => valor.push(b'\n'),
                b't' => valor.push(b'\t'),
                otro => valor.push(otro),
            },
            _ => valor.push(b),
        }
    }
    None
}

/// Igual, pero para un campo numérico: `"mpris:length" x 253000000`.
// 0.1.0
fn campo_numero(volcado: &str, clave: &str) -> Option<u64> {
    let resto = volcado.split_once(&format!("\"{clave}\""))?.1;
    numero(resto)
}

/// El primer número entero que aparece en un trozo de salida de `busctl`.
// 0.1.0
fn numero(texto: &str) -> Option<u64> {
    let mut digitos = String::new();
    for c in texto.chars() {
        if c.is_ascii_digit() {
            digitos.push(c);
        } else if !digitos.is_empty() {
            break;
        }
    }
    digitos.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El volcado real de `busctl` para una canción, con el artista dentro de
    /// un array —que es como lo publica MPRIS.
    const VOLCADO: &str = r#"a{sv} 5 "mpris:trackid" o "/org/mpris/MediaPlayer2/track/3" "mpris:length" x 253000000 "mpris:artUrl" s "file:///tmp/caratula.jpg" "xesam:title" s "Nombre de la canción" "xesam:artist" as 1 "Nombre Artista""#;

    #[test]
    fn saca_titulo_y_artista() {
        assert_eq!(
            campo(VOLCADO, "xesam:title").as_deref(),
            Some("Nombre de la canción")
        );
        assert_eq!(
            campo(VOLCADO, "xesam:artist").as_deref(),
            Some("Nombre Artista")
        );
    }

    /// Lo que imprime `busctl` de verdad para un título con tildes, copiado de
    /// Firefox, y unas comillas escapadas dentro.
    #[test]
    fn deshace_los_escapes_de_busctl() {
        let volcado = r#""xesam:title" s "El Contenido M\303\241s AL L\303\215MITE" "xesam:artist" as 1 "Dijo \"hola\"""#;
        assert_eq!(
            campo(volcado, "xesam:title").as_deref(),
            Some("El Contenido Más AL LÍMITE")
        );
        assert_eq!(
            campo(volcado, "xesam:artist").as_deref(),
            Some("Dijo \"hola\"")
        );
    }

    #[test]
    fn saca_la_duracion_en_microsegundos() {
        assert_eq!(campo_numero(VOLCADO, "mpris:length"), Some(253_000_000));
    }

    #[test]
    fn lee_la_calidad_de_bookos_player() {
        let volcado = r#"a{sv} 6 "xesam:title" s "Nocturne" "bookos:codec" s "FLAC" "bookos:samplerate" u 96000 "bookos:bitdepth" u 24 "bookos:bitrate" u 2304"#;
        let s = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.bookosplayer".into(),
            "s \"Playing\"",
            volcado,
            None,
        )
        .expect("hay canción");
        let c = s.calidad.expect("publica calidad");
        assert_eq!(c.nivel(), Nivel::HiRes);
        assert_eq!(c.detalle(), "FLAC · 24 bits · 96 kHz");
        let aac = Calidad {
            codec: "AAC".into(),
            frecuencia: 44_100,
            bits: 0,
            tasa: 256,
        };
        assert_eq!(aac.nivel(), Nivel::ConPerdida);
        assert_eq!(aac.detalle(), "AAC · 256 kbps");
        let cd = Calidad {
            codec: "ALAC".into(),
            frecuencia: 44_100,
            bits: 16,
            tasa: 0,
        };
        assert_eq!(cd.nivel(), Nivel::SinPerdida);
        assert_eq!(cd.detalle(), "ALAC · 16 bits · 44,1 kHz");
    }

    /// Un campo que no está no puede devolver el del vecino.
    #[test]
    fn un_campo_ausente_es_none() {
        assert_eq!(campo(VOLCADO, "xesam:album"), None);
        assert_eq!(campo_numero("", "mpris:length"), None);
    }

    #[test]
    fn pasarse_del_final_conserva_la_tarjeta_y_acota_el_progreso() {
        let actual = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.prueba".into(),
            "Playing",
            VOLCADO,
            Some(6_411_000_000),
        )
        .expect("el desfase no elimina una sesión con título");
        assert_eq!(actual.posicion, Some(253));
        assert!(
            actual.reproduciendo,
            "el progreso no decide si está en pausa"
        );
        assert_eq!(actual.titulo, "Nombre de la canción");
    }

    #[test]
    fn una_pausa_y_un_directo_conservan_sus_controles() {
        let pausa = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.prueba".into(),
            "Paused",
            VOLCADO,
            Some(6_411_000_000),
        )
        .expect("la pausa sigue siendo una sesión");
        assert!(!pausa.reproduciendo);
        assert_eq!(pausa.posicion, Some(253));
        let directo = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.prueba".into(),
            "Playing",
            r#""xesam:title" s "Directo""#,
            Some(6_411_000_000),
        )
        .expect("el directo no necesita duración");
        assert_eq!(directo.posicion, Some(6411));
        assert_eq!(directo.duracion, None);
    }

    #[test]
    fn el_reloj_lleva_dos_cifras_en_los_segundos() {
        assert_eq!(reloj(43), "0:43");
        assert_eq!(reloj(300), "5:00");
        assert_eq!(reloj(3), "0:03");
    }

    #[test]
    fn una_imagen_mpris_no_es_una_cancion() {
        let imagen = r#"a{sv} 2 "xesam:title" s "Foto" "xesam:url" s "file:///tmp/foto.JPG""#;
        assert!(es_imagen("org.mpris.MediaPlayer2.Gwenview", VOLCADO));
        assert!(es_imagen("org.mpris.MediaPlayer2.otro", imagen));
        assert!(!es_imagen("org.mpris.MediaPlayer2.vlc", VOLCADO));
    }

    /// BookOS Player publica la carátula como `data:`; antes se descartaba
    /// por no ser `file://` y ni el bloqueo ni el centro de control la veían.
    #[test]
    fn la_caratula_en_data_url_se_carga_con_su_color() {
        use base64::Engine;
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(40, 24, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("PNG en memoria");
        let url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        );
        let volcado = format!(r#"a{{sv}} 2 "mpris:artUrl" s "{url}" "xesam:title" s "T""#);
        let s = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.bookos".into(),
            "Playing",
            &volcado,
            None,
        )
        .expect("hay canción");
        assert_eq!(s.caratula.as_deref(), Some(url.as_str()));
        let (_, color) = cargar_caratula(&url, 30, 6.0).expect("se descodifica");
        assert!(
            color.r > color.g && color.r > color.b,
            "el color sale del rojo"
        );
    }
}
