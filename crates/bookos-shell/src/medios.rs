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
}

impl Sonando {
    /// Lo que debe salir en la tarjeta. Prefiere lo que esté reproduciendo y
    /// descarta visores de imágenes que publican MPRIS para una presentación.
    pub fn leer() -> Option<Self> {
        Self::consultar().unwrap_or_else(|error| {
            tracing::debug!("no se pudo consultar el reproductor: {error}");
            None
        })
    }

    /// Una lectura fallida no confirma que se haya cerrado el reproductor.
    /// El centro conserva su última tarjeta en ese caso.
    pub fn consultar() -> Result<Option<Self>, String> {
        let mut pausado = None;
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
            let posicion = propiedad(&bus, "Position")
                .and_then(|s| numero(&s))
                .map(|us| us / 1_000_000);
            let Some(actual) = Self::desde_mpris(bus, &estado, &metadatos, posicion) else {
                continue;
            };
            if actual.reproduciendo {
                return Ok(Some(actual));
            }
            pausado.get_or_insert(actual);
        }
        match (pausado, fallo) {
            (Some(actual), _) => Ok(Some(actual)),
            (None, Some(error)) => Err(error),
            (None, None) => Ok(None),
        }
    }

    fn desde_mpris(
        bus: String,
        estado: &str,
        metadatos: &str,
        posicion: Option<u64>,
    ) -> Option<Self> {
        if es_imagen(&bus, metadatos) {
            return None;
        }
        let titulo = campo(metadatos, "xesam:title")?;
        let reproduciendo = estado.contains("Playing");
        let duracion = campo_numero(metadatos, "mpris:length").map(|us| us / 1_000_000);
        // KDE Connect extrapola la posición; un desfase no prueba que se haya
        // cerrado la sesión ni autoriza a ocultar sus controles.
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
            titulo,
            bus,
        })
    }

    /// La orden que hay que ejecutar para un botón de la tarjeta.
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
    pub fn ejecutar(&self, que: Orden) -> bool {
        ejecutar_en(&self.bus, que)
    }

    /// El móvil emparejado con BookOS Link. Su reproductor MPRIS sigue en el
    /// bus, pausado y sin `mpris:length`, mientras Android conserve la sesión
    /// multimedia —medido el 2026-10-26: `Paused`, `Position` 6,6 s congelada,
    /// `CanSeek` falso—, así que «pausado» ahí no quiere decir «lo acabo de
    /// pausar».
    pub fn es_de_movil(&self) -> bool {
        self.bus.contains(".kdeconnect.")
    }

    /// De 0 a 1, para la barra de progreso. `None` si el reproductor no publica
    /// las dos cosas —los navegadores a menudo no dan `Position`—.
    pub fn avance(&self) -> Option<f32> {
        let (p, d) = (self.posicion?, self.duracion?);
        (d > 0).then(|| (p as f32 / d as f32).clamp(0.0, 1.0))
    }
}

/// La carátula cuadrada, reducida a `lado` píxeles con las esquinas de `radio`,
/// y el color que la resume. `fuente` es lo que guarda [`Sonando::caratula`].
///
/// La comparten el centro de control y el bloqueo; cada uno la llama solo al
/// cambiar de fuente, no por lectura.
pub fn cargar_caratula(fuente: &str, lado: u32, radio: f32) -> Option<(Handle, Color)> {
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
    let imagen = image::load_from_memory(&bytes)
        .inspect_err(|err| tracing::debug!("carátula ilegible: {err}"))
        .ok()?;
    let (ancho, alto) = (imagen.width(), imagen.height());
    let corte = ancho.min(alto);
    let cuadrada = image::imageops::crop_imm(
        &imagen,
        (ancho - corte) / 2,
        (alto - corte) / 2,
        corte,
        corte,
    )
    .to_image();
    let escalada =
        image::imageops::resize(&cuadrada, lado, lado, image::imageops::FilterType::Triangle);
    let mut rgba = escalada.into_raw();
    let color = crate::actividad::color_dominante(&crate::actividad::Portada {
        rgba: rgba.clone(),
        width: lado,
        height: lado,
    });
    crate::redondear_esquinas(&mut rgba, lado, lado, radio, crate::Alfa::Recto);
    Some((Handle::from_rgba(lado, lado, rgba), color))
}

/// La variante para una vista que ya guardó solo el nombre del bus.
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
pub fn saltar_en(bus: &str, desfase: i64) -> bool {
    llamar(bus, &["Seek", "x", &(desfase * 1_000_000).to_string()])
}

/// Un método de `org.mpris.MediaPlayer2.Player` sin esperar la respuesta.
fn llamar(bus: &str, metodo: &[&str]) -> bool {
    let Ok(mut hijo) = Command::new("busctl")
        .args([
            "--user",
            "call",
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orden {
    Anterior,
    Alternar,
    Siguiente,
}

/// Formatea unos segundos como `m:ss`.
pub fn reloj(segundos: u64) -> String {
    format!("{}:{:02}", segundos / 60, segundos % 60)
}

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

fn propiedad(bus: &str, nombre: &str) -> Option<String> {
    propiedad_de(bus, "org.mpris.MediaPlayer2.Player", nombre)
}

fn identidad(bus: &str) -> Option<String> {
    // `busctl` la imprime como `s "Nombre"`.
    let volcado = propiedad_de(bus, "org.mpris.MediaPlayer2", "Identity")?;
    let nombre = cadena(&volcado)?;
    let nombre = nombre.trim();
    (!nombre.is_empty()).then(|| nombre.to_string())
}

fn propiedad_de(bus: &str, interfaz: &str, nombre: &str) -> Option<String> {
    consultar_propiedad_de(bus, interfaz, nombre).ok()
}

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
fn cadena(texto: &str) -> Option<String> {
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
fn campo_numero(volcado: &str, clave: &str) -> Option<u64> {
    let resto = volcado.split_once(&format!("\"{clave}\""))?.1;
    numero(resto)
}

/// El primer número entero que aparece en un trozo de salida de `busctl`.
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
            Some(6411),
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
            Some(6411),
        )
        .expect("la pausa sigue siendo una sesión");
        assert!(!pausa.reproduciendo);
        assert_eq!(pausa.posicion, Some(253));
        let directo = Sonando::desde_mpris(
            "org.mpris.MediaPlayer2.prueba".into(),
            "Playing",
            r#""xesam:title" s "Directo""#,
            Some(6411),
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
