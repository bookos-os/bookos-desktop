//! El centro de control.
//!
//! Una sola rejilla de cuatro columnas con celdas cuadradas: un círculo ocupa
//! una celda, una píldora dos y lo que suena un cuadrado de 2×2 junto a Wi-Fi y
//! Bluetooth. Debajo, Pantalla y Sonido en sus tarjetas con título. Se diseñó
//! en la maqueta «Centro de control BookOS 2» (noviembre de 2026) a partir del
//! de macOS Tahoe, pero opaco: sin cristal por control, que en tiny-skia cuesta
//! 17× un relleno opaco.
//!
//! **Círculos y píldoras alternan**: dos círculos y una píldora, y en la fila
//! siguiente al revés. Las píldoras llevan nombre y estado; los círculos son
//! los conmutadores que se reconocen solos.
//!
//! **El encendido es un disco blanco con el glifo en acento** en oscuro, y en
//! claro al revés (el blanco desaparecería sobre la tarjeta). El acento solo
//! dice «encendido».
//!
//! **Todo actúa al soltar.** Apretar hunde el control; soltar fuera lo anula.
//! Mantenerlo más de [`PULSACION_LARGA`] y soltar abre el detalle si lo tiene
//! (lista de Wi-Fi o Bluetooth, o lo que suena en grande) en vez de conmutar.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Length, Point, Rectangle, Shadow, Vector};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono;
use crate::medios::{self, Sonando};
use crate::tema;
use crate::view::PanelElement;

use super::bluetooth::Bluetooth;
use super::control;
use super::lista;
use super::red::Red;
use super::{Ancla, Tecla};

const ANCHO: f32 = 352.0;
const MARGEN: f32 = 16.0;
/// Hueco entre la rejilla y las tarjetas de abajo, y entre esas tarjetas.
const HUECO: f32 = 14.0;
const COLUMNAS: f32 = 4.0;
/// Aire entre columnas, algo mayor que entre filas: la maqueta pedía respirar
/// a lo ancho sin estirar el panel hacia abajo.
const ENTRE_COLUMNAS: f32 = 14.0;
const ENTRE_FILAS: f32 = 12.0;
/// Lado de una celda: (352 − 2·16 − 3·14) / 4 = 69,5. Las filas miden lo mismo,
/// así un círculo es una celda y una píldora encaja al lado sin medidas sueltas.
const CELDA: f32 = (ANCHO - 2.0 * MARGEN - (COLUMNAS - 1.0) * ENTRE_COLUMNAS) / COLUMNAS;
const FILAS: f32 = 4.0;
/// Disco de una píldora: concéntrico, con el mismo margen arriba, abajo y a la
/// izquierda ((69,5 − 40) / 2 ≈ 14,75). Con 44 «No molestar» no cabía.
const DISCO: f32 = 40.0;
/// Un solo tamaño de glifo en todos los conmutadores: lo que cambia entre un
/// círculo y una píldora es la placa, no el icono.
const GLIFO: f32 = 22.0;
/// Cuánto encoge lo apretado: los círculos más que las piezas grandes, que con
/// el mismo porcentaje se verían dar un salto.
const HUNDIDO_CIRCULO: f32 = 0.08;
const HUNDIDO_GRANDE: f32 = 0.04;
/// Mantener apretado más de esto y soltar abre el detalle. 450 ms: menos se
/// dispara con un clic lento; más se siente que no responde.
const PULSACION_LARGA: Duration = Duration::from_millis(450);
/// Radio de las tarjetas de dentro (música, Pantalla, Sonido): el de tarjeta
/// del sistema de diseño.
const R_INTERIOR: f32 = tema::R_TARJETA;

/// Tarjeta de un deslizador: título, y debajo icono, pista e icono.
const DESL_RELLENO_V: f32 = 12.0;
const DESL_RELLENO_H: f32 = 16.0;
const DESL_TITULO: f32 = 16.0;
const DESL_ENTRE: f32 = 6.0;
const DESL_FILA: f32 = 28.0;
const DESLIZADOR: f32 = DESL_RELLENO_V * 2.0 + DESL_TITULO + DESL_ENTRE + DESL_FILA;
/// Grueso de la pista en reposo y agarrada: engorda para confirmar el agarre.
// modified 0.2.1 -- de 6/12 a 10/15: con 6 en reposo costaba verla y apuntarle.
// Es la misma barra que usa la tarjeta de Brillo (`control::barra`).
const PISTA: f32 = control::PISTA;
const PISTA_AGARRADA: f32 = control::PISTA_AGARRADA;
/// La barra de posición de la música es otra cosa y sigue como estaba.
const AVANCE: f32 = 6.0;
const AVANCE_AGARRADO: f32 = 12.0;
const ICONO_DESL: f32 = 28.0;

/// Lo que suena, en la celda de 2×2.
const MUSICA_RELLENO: f32 = 14.0;
const MUSICA_CARATULA: f32 = 40.0;
const MANDO: f32 = 32.0;

/// Lo que suena en grande. Las medidas son las de la maqueta del lienzo
/// («Centro de control — Música»), que la usuaria aprobó 1:1.
const GRANDE_CABECERA: f32 = 36.0;
const GRANDE_CARATULA: f32 = 168.0;
/// Con el detalle de calidad desplegado la carátula cuadrada encoge.
const GRANDE_CARATULA_PEQUEÑA: f32 = 116.0;
/// El hueco de un vídeo: 16:9 a todo el ancho, y su portada cuadrada a la
/// misma altura para que cambiar de vista no mueva nada de lo de debajo.
const GRANDE_VIDEO_ALTO: f32 = 180.0;
const GRANDE_RADIO: f32 = 22.0;
const GRANDE_RADIO_VIDEO: f32 = 14.0;
/// Portada | Vídeo: cápsula de 200 con dos mitades de 32 de alto.
const SELECTOR_ANCHO: f32 = 200.0;
const SELECTOR_ALTO: f32 = 40.0;
const TEXTOS_ALTO: f32 = 44.0;
const GRANDE_AVANCE: f32 = 20.0;
const TIEMPOS: f32 = 20.0;
/// El bloque de calidad: relleno 12, icono 36, 6, nombre 18, dato 15.
const DETALLE_ALTO: f32 = 12.0 + 36.0 + 6.0 + 18.0 + 15.0 + 12.0;
const GRANDE_REPRODUCIR: f32 = 56.0;
/// Los cinco mandos a lo ancho, con el mismo hueco (22) entre ellos:
/// (x desde el margen, lado).
const MANDOS: [(f32, f32); 5] = [
    (0.0, 40.0),
    (62.0, 48.0),
    (132.0, 56.0),
    (210.0, 48.0),
    (280.0, 40.0),
];
/// La lista de salidas, colgada bajo su botón.
const MENU_ANCHO: f32 = 252.0;
const MENU_FILA: f32 = 44.0;
const MENU_RELLENO: f32 = 5.0;

/// Cada pieza de la rejilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Celda {
    Wifi,
    Bluetooth,
    Musica,
    Noche,
    Oscuro,
    NoMolestar,
    Microfono,
    Captura,
    Mantener,
}

/// Dónde va cada pieza: columna, fila, columnas y filas que ocupa. Dos
/// círculos y una píldora, y en la fila siguiente al revés.
const DISPOSICION: [(Celda, f32, f32, f32, f32); 9] = [
    (Celda::Wifi, 0.0, 0.0, 2.0, 1.0),
    (Celda::Musica, 2.0, 0.0, 2.0, 2.0),
    (Celda::Bluetooth, 0.0, 1.0, 2.0, 1.0),
    (Celda::Noche, 0.0, 2.0, 1.0, 1.0),
    (Celda::Oscuro, 1.0, 2.0, 1.0, 1.0),
    (Celda::NoMolestar, 2.0, 2.0, 2.0, 1.0),
    (Celda::Microfono, 0.0, 3.0, 2.0, 1.0),
    (Celda::Captura, 2.0, 3.0, 1.0, 1.0),
    (Celda::Mantener, 3.0, 3.0, 1.0, 1.0),
];

/// El rectángulo de una pieza en la estación.
// añadido en la 0.2.0
fn rect_celda(celda: Celda) -> Rectangle {
    let (_, c, f, w, h) = DISPOSICION
        .iter()
        .copied()
        .find(|d| d.0 == celda)
        .expect("toda celda está en la disposición");
    Rectangle {
        x: MARGEN + c * (CELDA + ENTRE_COLUMNAS),
        y: MARGEN + f * (CELDA + ENTRE_FILAS),
        width: w * CELDA + (w - 1.0) * ENTRE_COLUMNAS,
        height: h * CELDA + (h - 1.0) * ENTRE_FILAS,
    }
}

/// La placa de lo apagado y de las tarjetas: el `--surface` del sistema, que
/// sobre la tarjeta del panel es lo que se despega.
// 0.1.0
fn placa() -> Color {
    tema::superficie_en_tarjeta()
}

/// El disco apagado dentro de una píldora: un paso más claro que la placa.
// añadido en la 0.2.0
fn disco_apagado() -> Color {
    tema::elevado()
}

/// La lista de Wi-Fi o de Bluetooth abierta **dentro** de la estación.
///
/// Es lo que hace la bandeja del sistema de Plasma: pulsar una entrada cambia
/// el contenido del mismo desplegable por la vista completa del applet, con
/// una flecha para volver. Antes se cerraba la estación y se abría la tarjeta
/// suelta colgando de su icono del panel, que está en otro sitio: la mano y la
/// vista tenían que ir a buscarla.
///
/// Son las mismas tarjetas que abre el panel, con su contenido sin la caja. Se
/// dibujan con su ancho (336) centradas en el de la estación (352), así que lo
/// que entra del puntero se corre [`DESVIO`] para que sus zonas coincidan.
enum Detalle {
    Red(Red),
    Bluetooth(Bluetooth),
}

const DESVIO: f32 = (ANCHO - lista::ANCHO) / 2.0;

impl Detalle {
    // 0.1.0
    fn alto(&self) -> f32 {
        match self {
            Self::Red(r) => r.size().1,
            Self::Bluetooth(b) => b.size().1,
        }
    }
    // 0.1.0
    fn contenido(&self) -> PanelElement<'_> {
        match self {
            Self::Red(r) => r.contenido(true),
            Self::Bluetooth(b) => b.contenido(true),
        }
    }
    // 0.1.0
    fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        match self {
            Self::Red(r) => r.pulsar(x - DESVIO, y),
            Self::Bluetooth(b) => b.pulsar(x - DESVIO, y),
        }
    }
    // 0.1.0
    fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let punto = punto.map(|(x, y)| (x - DESVIO, y));
        match self {
            Self::Red(r) => r.puntero(punto),
            Self::Bluetooth(b) => b.puntero(punto),
        }
    }
    // añadido en la 0.2.0
    fn desplazar(&mut self, dy: f32) -> bool {
        match self {
            Self::Red(r) => r.desplazar(dy),
            Self::Bluetooth(b) => b.desplazar(dy),
        }
    }
    // añadido en la 0.2.0
    fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match self {
            Self::Red(r) => r.tecla(tecla),
            Self::Bluetooth(b) => b.tecla(tecla),
        }
    }
    // 0.1.0
    fn animando(&self) -> bool {
        match self {
            Self::Red(r) => r.animando(),
            Self::Bluetooth(b) => b.animando(),
        }
    }
    // 0.1.0
    fn refrescar(&mut self) -> bool {
        match self {
            Self::Red(r) => r.refrescar(),
            Self::Bluetooth(b) => b.refrescar(),
        }
    }
}

/// Qué está agarrado con el ratón.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Agarre {
    Nada,
    Volumen,
    Brillo,
    /// La barra de posición de lo que suena en grande.
    Posicion,
}

/// Lo que se puede apretar: actúa al soltar encima, como un botón de verdad.
// añadido en la 0.2.0
#[derive(Debug, PartialEq, Clone, Copy)]
enum Zona {
    /// Una pieza entera de la rejilla.
    Celda(Celda),
    /// El disco de la píldora de Wi-Fi o Bluetooth: enciende la radio; el
    /// resto de la píldora abre la lista.
    Disco(Celda),
    Medio(medios::Orden),
    /// El icono pequeño del Sonido: silencia.
    Silencio,
    /// Cerrar lo que suena en grande.
    Plegar,
    /// La etiqueta de calidad: despliega o pliega el detalle.
    Calidad,
    Aleatorio,
    Repetir,
    /// El botón redondo de la salida de audio: abre o cierra su lista.
    // añadido en la 0.2.2
    Salida,
    /// Portada (`false`) o vídeo (`true`).
    Vista(bool),
    Favorito,
}

pub struct Centro {
    señalada: tema::Realce,
    wifi: bool,
    /// El servicio puede tardar en aparecer o estar reiniciándose. Se separa
    /// de `wifi` para no pintar «apagado» cuando todavía no hay estado.
    wifi_disponible: bool,
    bluetooth: bool,
    bluetooth_disponible: bool,
    /// La red a la que se está conectado. La radio encendida no basta:
    /// `nmcli radio wifi` dice `enabled` con la antena buscando.
    red: Option<String>,
    dispositivo: Option<String>,
    noche: bool,
    no_molestar: bool,
    microfono_mudo: bool,
    volumen: u8,
    audio_disponible: bool,
    silenciado: bool,
    brillo: u8,
    agarre: Agarre,
    /// Engorde de la pista agarrada, de 0 a 1.
    grosor: tema::Transicion,
    volumen_inicial: u8,
    backlight: Option<(String, u32)>,
    sonando: Option<Sonando>,
    /// Lo que se pidió con reproducir/pausa y cuándo. El hilo de medios puede
    /// traer una lectura salida antes del clic: sin esto pisaba el cambio y
    /// el botón hacía pausa → play → pausa.
    // añadido en la 0.2.2
    alternado: Option<(bool, std::time::Instant)>,
    caratula: Option<iced_widget::image::Handle>,
    /// La carátula grande de la vista ampliada; se descodifica al abrirla.
    caratula_grande: Option<iced_widget::image::Handle>,
    /// El color que resume la carátula: tiñe la tarjeta de lo que suena.
    tinte: Option<Color>,
    /// Las salidas de audio que se pueden elegir; la actual va marcada.
    // añadido en la 0.2.2
    salidas: Vec<OpcionSalida>,
    /// La lista de salidas abierta bajo su botón.
    menu_salida: bool,
    /// Lo desplegada que está la lista, de 0 a 1: crece desde el botón.
    // añadido en la 0.2.0
    menu: tema::Transicion,
    /// La carátula a lo ancho, si es la miniatura de un vídeo.
    caratula_ancha: Option<iced_widget::image::Handle>,
    /// Portada (0) o vídeo (1), con su muelle para el pulgar del selector.
    vista: tema::Transicion,
    /// La portada nueva entrando al cambiar de canción, de 0 a 1, y desde qué
    /// lado (−1 al ir atrás).
    cambio: tema::Transicion,
    sentido: f32,
    /// El `.desktop`, la pista y si es favorita, leídos en el hilo de medios.
    extras: Extras,
    app_icono: Option<crate::icono::Icono>,
    /// La etiqueta de calidad desplegada: archivo, salida y lo que limita.
    calidad_abierta: bool,
    /// Lo abierto que está el bloque de calidad, de 0 a 1. La carátula encoge
    /// y el bloque se descubre al mismo ritmo.
    // añadido en la 0.2.0
    calidad: tema::Transicion,
    /// Desde cuándo corren los tres puntos del bloque de calidad.
    // añadido en la 0.2.0
    onda: Instant,
    /// Aleatorio y repetición del reproductor; se leen al abrir lo que suena
    /// en grande. `None` si no los publica: entonces no se enseñan.
    aleatorio: Option<bool>,
    repetir: Option<medios::Repetir>,
    lectura: Arc<Mutex<Option<Lectura>>>,
    detalle: Option<Box<Detalle>>,
    /// Lo que suena en grande, abierto en el sitio de la estación.
    musica_grande: bool,
    transicion: Option<super::Transicion>,
    origen_detalle: Rectangle,
    apretada: Option<(Zona, f32, f32, Instant)>,
    hundida: Option<Zona>,
    hundido: tema::Transicion,
    puntero_en: Option<(f32, f32)>,
}

// El compositor decide el bloqueo y la atenuación sin consultar logind.
// Este estado dura toda la sesión, aunque se destruya la tarjeta.
static MANTENER_DESPIERTO: AtomicBool = AtomicBool::new(false);

// 0.1.0
pub fn mantener_despierto() -> bool {
    MANTENER_DESPIERTO.load(Ordering::Relaxed)
}

/// Cambia «mantener despierto» y devuelve cómo queda. Lo usan la baldosa del
/// centro y el buscador: [`Accion::MantenerDespierto`] solo le dice al
/// compositor que vuelva a leerlo, no lo cambia.
// añadido en la 0.2.0
pub fn alternar_mantener_despierto() -> bool {
    !MANTENER_DESPIERTO.fetch_xor(true, Ordering::Relaxed)
}

impl Centro {
    // 0.1.0
    // modified 0.2.0 -- rejilla nueva: micrófono, música teñida y salida de audio
    // modified 0.2.2 -- no deja que una lectura vieja deshaga reproducir/pausa
    pub fn refrescar(&mut self) -> bool {
        if let Some(d) = self.detalle.as_mut() {
            return d.refrescar();
        }
        let state = bookos_system::snapshot();
        let wifi_disponible = !state.network.is_null();
        let bluetooth_disponible = !state.bluetooth.is_null();
        let audio_disponible = !state.audio.is_null();
        let wifi = wifi_disponible && state.network["enabled"] == true;
        let bluetooth = bluetooth_disponible && state.bluetooth["enabled"] == true;
        let red = wifi_disponible.then(conectado_a).flatten();
        let dispositivo = bluetooth_disponible.then(emparejado_con).flatten();
        let (volumen_leido, silenciado) =
            bookos_system::volume(false).map_or((self.volumen, self.silenciado), |v| (v.0, v.1));
        let volumen = if self.agarre == Agarre::Volumen {
            self.volumen
        } else {
            volumen_leido
        };
        let microfono_mudo = state.audio["input"]["muted"] == true;
        let salidas = salidas_de_audio(&state.audio);
        let noche = crate::luz_nocturna_encendida();
        let mut changed = self.wifi != wifi
            || self.wifi_disponible != wifi_disponible
            || self.bluetooth != bluetooth
            || self.bluetooth_disponible != bluetooth_disponible
            || self.audio_disponible != audio_disponible
            || self.red != red
            || self.dispositivo != dispositivo
            || self.volumen != volumen
            || self.silenciado != silenciado
            || self.microfono_mudo != microfono_mudo
            || self.salidas != salidas
            || self.noche != noche;
        self.wifi = wifi;
        self.wifi_disponible = wifi_disponible;
        self.bluetooth = bluetooth;
        self.bluetooth_disponible = bluetooth_disponible;
        self.audio_disponible = audio_disponible;
        self.red = red;
        self.dispositivo = dispositivo;
        self.volumen = volumen;
        self.silenciado = silenciado;
        self.microfono_mudo = microfono_mudo;
        self.salidas = salidas;
        self.noche = noche;
        let lectura = self
            .lectura
            .lock()
            .expect("nadie suelta el cerrojo a medias")
            .take();
        if let Some(mut l) = lectura {
            // Una lectura en contra de lo pedido se descarta en lo que toca a
            // reproducir hasta que el reproductor lo confirme. 2 s cubre el
            // segundo del hilo más lo que tarda el móvil detrás de BookOS Link.
            if let Some((quiere, desde)) = self.alternado
                && let Some(leido) = l.sonando.as_mut()
            {
                if leido.reproduciendo == quiere
                    || desde.elapsed() > std::time::Duration::from_secs(2)
                {
                    self.alternado = None;
                } else {
                    leido.reproduciendo = quiere;
                }
            }
            // Mientras se arrastra la barra manda el dedo: la lectura pisaba
            // la posición arrastrada y, al soltar, el destino ya era la
            // posición real y salía `Seek 0`. Solo funcionaba un arrastre de
            // menos de un segundo.
            // modified 0.2.0 -- conserva la posición arrastrada
            if self.agarre == Agarre::Posicion
                && let (Some(antes), Some(leido)) = (&self.sonando, l.sonando.as_mut())
                && antes.bus == leido.bus
            {
                leido.posicion = antes.posicion;
            }
            let otra = self.sonando.as_ref().and_then(|s| s.caratula.as_deref())
                != l.sonando.as_ref().and_then(|s| s.caratula.as_deref());
            changed |= self.sonando != l.sonando || self.caratula != l.caratula;
            self.sonando = l.sonando;
            self.caratula = l.caratula;
            self.tinte = l.tinte;
            if otra && self.musica_grande {
                self.caratula_grande = self.cargar_grande();
                self.caratula_ancha = self.cargar_ancha();
                if !tema::efectos_reducidos() {
                    self.cambio = tema::Transicion::nueva(0.0, tema::M_SUAVE);
                    self.cambio.ir_a(1.0);
                }
            }
            // Los extras solo cuando cambia la pista o el reproductor: el
            // favorito marcado aquí no puede volver atrás por una lectura que
            // el hilo hizo antes del clic.
            if let Some(e) = l.extras
                && (e.pista != self.extras.pista || e.escritorio != self.extras.escritorio)
            {
                if e.escritorio != self.extras.escritorio {
                    self.app_icono = e.escritorio.as_deref().and_then(icono_de_escritorio);
                }
                self.extras = e;
                changed = true;
            }
        }
        if self.agarre != Agarre::Brillo {
            let backlight = crate::backlight();
            let brillo = crate::brillo_actual().unwrap_or(0);
            changed |= self.backlight != backlight || self.brillo != brillo;
            self.backlight = backlight;
            self.brillo = brillo;
        }
        changed
    }

    // 0.1.0
    // modified 0.2.0 -- sin cabecera de usuario; rejilla de cuatro por cuatro
    pub fn new() -> Self {
        let state = bookos_system::snapshot();
        let wifi_disponible = !state.network.is_null();
        let bluetooth_disponible = !state.bluetooth.is_null();
        let audio_disponible = !state.audio.is_null();
        let mut c = Self {
            señalada: tema::Realce::nuevo(),
            wifi: wifi_disponible && state.network["enabled"] == true,
            wifi_disponible,
            bluetooth: bluetooth_disponible && state.bluetooth["enabled"] == true,
            bluetooth_disponible,
            red: wifi_disponible.then(conectado_a).flatten(),
            dispositivo: bluetooth_disponible.then(emparejado_con).flatten(),
            noche: crate::luz_nocturna_encendida(),
            // Lo pone el shell nada más abrir (`poner_no_molestar`): es quien lo sabe.
            no_molestar: false,
            microfono_mudo: state.audio["input"]["muted"] == true,
            volumen: crate::widgets::volumen::consultar("@DEFAULT_AUDIO_SINK@")
                .map(|(n, _)| n)
                .unwrap_or(0),
            audio_disponible,
            silenciado: bookos_system::volume(false).is_some_and(|v| v.1),
            brillo: crate::brillo_actual().unwrap_or(0),
            agarre: Agarre::Nada,
            grosor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            volumen_inicial: 0,
            backlight: crate::backlight(),
            sonando: None,
            caratula: None,
            caratula_grande: None,
            tinte: None,
            salidas: salidas_de_audio(&state.audio),
            menu_salida: false,
            menu: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            caratula_ancha: None,
            vista: tema::Transicion::nueva(1.0, tema::M_RAPIDO),
            cambio: tema::Transicion::nueva(1.0, tema::M_SUAVE),
            sentido: 1.0,
            extras: Extras::default(),
            app_icono: None,
            calidad_abierta: false,
            calidad: tema::Transicion::nueva(0.0, tema::M_SUAVE),
            onda: Instant::now(),
            aleatorio: None,
            repetir: None,
            lectura: Arc::default(),
            alternado: None,
            detalle: None,
            musica_grande: false,
            transicion: None,
            origen_detalle: Rectangle::default(),
            apretada: None,
            hundida: None,
            hundido: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            puntero_en: None,
        };
        // La primera lectura en el sitio: si fuera por el hilo, la tarjeta
        // abriría sin la música y la pintaría un segundo después.
        let primera = Lectura::nueva(Sonando::leer(), None);
        c.sonando = primera.sonando;
        c.caratula = primera.caratula;
        c.tinte = primera.tinte;
        let previa = c
            .sonando
            .as_ref()
            .and_then(|s| s.caratula.clone())
            .zip(c.caratula.clone().zip(c.tinte));
        vigilar_medios(&c.lectura, previa);
        c
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        if let Some(d) = &self.detalle {
            return (ANCHO, d.alto());
        }
        if self.musica_grande {
            return (ANCHO, self.alto_grande());
        }
        (ANCHO, self.alto_estacion())
    }

    /// El alto de la estación. Lo que suena en grande ocupa este mismo alto:
    /// abrirlo no mueve el panel.
    // añadido en la 0.2.0
    fn alto_estacion(&self) -> f32 {
        self.y_deslizador(Agarre::Volumen) + DESLIZADOR + MARGEN
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("control")
    }

    /// Dónde empieza la tarjeta de un deslizador: Pantalla arriba, Sonido abajo.
    // añadido en la 0.2.0
    fn y_deslizador(&self, cual: Agarre) -> f32 {
        let y0 = MARGEN + FILAS * CELDA + (FILAS - 1.0) * ENTRE_FILAS + HUECO;
        match cual {
            Agarre::Volumen => y0 + DESLIZADOR + HUECO,
            _ => y0,
        }
    }

    /// La pista de un deslizador.
    // 0.1.0
    // modified 0.2.0 -- tarjeta con título e iconos a los dos lados
    fn rect_pista(&self, cual: Agarre) -> Rectangle {
        let y = self.y_deslizador(cual) + DESL_RELLENO_V + DESL_TITULO + DESL_ENTRE;
        let x = MARGEN + DESL_RELLENO_H + ICONO_DESL + 8.0;
        Rectangle {
            x,
            y: y + (DESL_FILA - PISTA) / 2.0,
            width: ANCHO - x - MARGEN - DESL_RELLENO_H - ICONO_DESL - 8.0,
            height: PISTA,
        }
    }

    /// El icono pequeño del Sonido, que silencia.
    // añadido en la 0.2.0
    fn rect_silencio(&self) -> Rectangle {
        Rectangle {
            x: MARGEN + DESL_RELLENO_H,
            y: self.y_deslizador(Agarre::Volumen) + DESL_RELLENO_V + DESL_TITULO + DESL_ENTRE,
            width: ICONO_DESL,
            height: DESL_FILA,
        }
    }

    /// Qué pieza de la rejilla cae en un punto.
    // añadido en la 0.2.0
    fn celda_en(&self, x: f32, y: f32) -> Option<Celda> {
        let p = Point::new(x, y);
        DISPOSICION
            .iter()
            .map(|d| d.0)
            .find(|&c| rect_celda(c).contains(p))
    }

    /// Los tres mandos de la tarjeta de lo que suena, en su fila de abajo.
    // 0.1.0
    // modified 0.2.0 -- en la celda de 2×2
    fn medio_en(&self, x: f32, y: f32) -> Option<medios::Orden> {
        self.sonando.as_ref()?;
        let r = rect_celda(Celda::Musica);
        let y0 = r.y + r.height - MUSICA_RELLENO - MANDO;
        if y < y0 || y > y0 + MANDO || x < r.x || x > r.x + r.width {
            return None;
        }
        let tercio = r.width / 3.0;
        Some(match x - r.x {
            d if d < tercio => medios::Orden::Anterior,
            d if d > tercio * 2.0 => medios::Orden::Siguiente,
            _ => medios::Orden::Alternar,
        })
    }

    // --- Lo que suena en grande -----------------------------------------------

    /// Dónde va cada cosa de lo que suena en grande, de arriba abajo. La vista
    /// y las zonas de pulsación salen de aquí.
    // añadido en la 0.2.0
    // modified 0.2.2 -- la maqueta del lienzo: vídeo, selector, ♥ y bloque de calidad
    fn grande(&self) -> Grande {
        let ancho = ANCHO - MARGEN * 2.0;
        let video = self.caratula_ancha.is_some();
        let (caratula_w, caratula_h) = match (video, self.ver_video()) {
            (true, true) => (ancho, GRANDE_VIDEO_ALTO),
            (true, false) => (GRANDE_VIDEO_ALTO, GRANDE_VIDEO_ALTO),
            _ => {
                let lado = GRANDE_CARATULA
                    + (GRANDE_CARATULA_PEQUEÑA - GRANDE_CARATULA) * self.calidad.valor();
                (lado, lado)
            }
        };
        let caratula = MARGEN + GRANDE_CABECERA + 8.0;
        let mut y = caratula + caratula_h;
        let selector = video.then(|| {
            y += 18.0 + SELECTOR_ALTO;
            y - SELECTOR_ALTO
        });
        let textos = y + 16.0;
        let avance = textos + TEXTOS_ALTO + 14.0;
        let tiempos = avance + GRANDE_AVANCE;
        // El hueco del bloque (con su separación de 10) crece con el muelle; el
        // bloque se descubre dentro, recortado, en vez de aparecer de golpe.
        let detalle_alto = (10.0 + DETALLE_ALTO) * self.calidad.valor().clamp(0.0, 1.0);
        let detalle = (detalle_alto > 0.0).then_some(tiempos + TIEMPOS);
        let mandos = tiempos + TIEMPOS + detalle_alto + 10.0;
        Grande {
            caratula,
            caratula_w,
            caratula_h,
            selector,
            textos,
            avance,
            tiempos,
            detalle,
            detalle_alto,
            mandos,
            alto: mandos + GRANDE_REPRODUCIR + MARGEN,
        }
    }

    /// Si se enseña el vídeo y no su portada. El muelle del selector decide:
    /// a medio camino manda hacia dónde va.
    // añadido en la 0.2.2
    fn ver_video(&self) -> bool {
        self.vista.valor() > 0.5
    }

    /// El alto de lo que suena en grande: lo que ocupa y nada más.
    // añadido en la 0.2.2
    fn alto_grande(&self) -> f32 {
        self.grande().alto
    }

    /// La etiqueta de calidad, centrada en la fila de los tiempos.
    // añadido en la 0.2.0
    fn rect_calidad(&self) -> Option<Rectangle> {
        let texto = self.etiqueta_calidad()?;
        // Medida pesimista, como `recortar_texto`: 6,5 px por carácter a 11 px
        // más el destello y el relleno.
        let ancho = texto.chars().count() as f32 * 6.5 + 34.0;
        Some(Rectangle {
            x: (ANCHO - ancho) / 2.0,
            y: self.grande().tiempos,
            width: ancho,
            height: TIEMPOS,
        })
    }

    /// «Hi-Res Lossless», «Lossless» o el códec, si el reproductor lo publica;
    /// «Vídeo» si lo que suena es un vídeo sin más datos.
    // añadido en la 0.2.0
    // modified 0.2.2 -- también los vídeos
    fn etiqueta_calidad(&self) -> Option<String> {
        let s = self.sonando.as_ref()?;
        match &s.calidad {
            Some(c) => Some(match c.nivel() {
                medios::Nivel::HiRes => "Hi-Res Lossless".to_owned(),
                medios::Nivel::SinPerdida => "Lossless".to_owned(),
                medios::Nivel::ConPerdida => c.codec.clone(),
            }),
            None => self.caratula_ancha.is_some().then(|| "Vídeo".to_owned()),
        }
    }

    /// Las dos columnas del bloque de calidad: lo que suena y por dónde llega,
    /// cada una con su nombre y su dato. Sin rótulos: los iconos lo dicen.
    // añadido en la 0.2.2
    fn columnas_calidad(&self) -> [(&'static str, String, String); 2] {
        let s = self.sonando.as_ref();
        let origen = match s.and_then(|s| s.calidad.as_ref()) {
            Some(c) => {
                let mut dato = Vec::new();
                if c.nivel() != medios::Nivel::ConPerdida && c.bits > 0 {
                    dato.push(format!("{} bits", c.bits));
                }
                if c.frecuencia > 0 {
                    dato.push(medios::khz(c.frecuencia));
                }
                ("musica", c.codec.clone(), dato.join(" · "))
            }
            None => (
                if self.caratula_ancha.is_some() {
                    "reproducir"
                } else {
                    "musica"
                },
                s.map(|s| s.aplicacion.clone()).unwrap_or_default(),
                if self.caratula_ancha.is_some() {
                    "Vídeo".to_owned()
                } else {
                    String::new()
                },
            ),
        };
        let destino = match self.salida_actual() {
            Some(o) => (
                o.glifo,
                o.titulo.clone(),
                o.via
                    .strip_prefix("Bluetooth · ")
                    .unwrap_or(&o.via)
                    .to_owned(),
            ),
            None => ("volumen-alto", "Salida".to_owned(), String::new()),
        };
        [origen, destino]
    }

    // añadido en la 0.2.2
    fn salida_actual(&self) -> Option<&OpcionSalida> {
        self.salidas.iter().find(|o| o.actual)
    }

    /// La lista de salidas: dónde cae cada fila.
    // añadido en la 0.2.2
    fn rect_menu(&self) -> Rectangle {
        Rectangle {
            x: ANCHO - MARGEN - MENU_ANCHO,
            y: MARGEN + GRANDE_CABECERA + 6.0,
            width: MENU_ANCHO,
            height: MENU_RELLENO * 2.0 + MENU_FILA * self.salidas.len() as f32,
        }
    }

    // añadido en la 0.2.2
    fn opcion_en(&self, x: f32, y: f32) -> Option<usize> {
        let r = self.rect_menu();
        if !r.contains(Point::new(x, y)) {
            return None;
        }
        let i = ((y - r.y - MENU_RELLENO) / MENU_FILA).floor();
        (i >= 0.0 && (i as usize) < self.salidas.len()).then_some(i as usize)
    }

    // añadido en la 0.2.0
    // modified 0.2.2 -- salida, selector, ♥ y los cinco mandos a lo ancho
    fn grande_en(&self, x: f32, y: f32) -> Option<Zona> {
        let p = Point::new(x, y);
        let g = self.grande();
        let cabecera = (MARGEN..=MARGEN + GRANDE_CABECERA).contains(&y);
        if cabecera && (MARGEN..=MARGEN + GRANDE_CABECERA).contains(&x) {
            return Some(Zona::Plegar);
        }
        if cabecera && (ANCHO - MARGEN - GRANDE_CABECERA..=ANCHO - MARGEN).contains(&x) {
            return Some(Zona::Salida);
        }
        if let Some(sy) = g.selector {
            let x0 = (ANCHO - SELECTOR_ANCHO) / 2.0;
            if (sy..=sy + SELECTOR_ALTO).contains(&y) && (x0..=x0 + SELECTOR_ANCHO).contains(&x) {
                return Some(Zona::Vista(x > ANCHO / 2.0));
            }
        }
        if self.extras.favorito.is_some()
            && (g.textos + 4.0..=g.textos + 40.0).contains(&y)
            && (ANCHO - MARGEN - 36.0..=ANCHO - MARGEN).contains(&x)
        {
            return Some(Zona::Favorito);
        }
        if let Some(mut r) = self.rect_calidad() {
            r.y -= 4.0;
            r.height += 8.0;
            if r.contains(p) {
                return Some(Zona::Calidad);
            }
        }
        if !(g.mandos..=g.mandos + GRANDE_REPRODUCIR).contains(&y) {
            return None;
        }
        let i = MANDOS
            .iter()
            .position(|(x0, lado)| (MARGEN + x0..=MARGEN + x0 + lado).contains(&x))?;
        Some(match i {
            0 if self.aleatorio.is_some() => Zona::Aleatorio,
            1 => Zona::Medio(medios::Orden::Anterior),
            2 => Zona::Medio(medios::Orden::Alternar),
            3 => Zona::Medio(medios::Orden::Siguiente),
            4 if self.repetir.is_some() => Zona::Repetir,
            _ => return None,
        })
    }

    /// La barra de posición, de margen a margen: alineada con el título y los
    /// tiempos.
    // añadido en la 0.2.0
    // modified 0.2.2 -- a todo el ancho
    fn rect_avance(&self) -> Rectangle {
        Rectangle {
            x: MARGEN,
            y: self.grande().avance,
            width: ANCHO - MARGEN * 2.0,
            height: GRANDE_AVANCE,
        }
    }

    // añadido en la 0.2.0
    fn cargar_grande(&self) -> Option<iced_widget::image::Handle> {
        let fuente = self.sonando.as_ref()?.caratula.as_deref()?;
        // Tres píxeles por punto, como la pequeña: nítida hasta escala 3. A
        // 180, el lado de la portada de un vídeo, la mayor de las dos.
        medios::cargar_caratula(fuente, (GRANDE_VIDEO_ALTO * 3.0) as u32, GRANDE_RADIO * 3.0)
            .map(|(h, _)| h)
    }

    // añadido en la 0.2.2
    fn cargar_ancha(&self) -> Option<iced_widget::image::Handle> {
        let fuente = self.sonando.as_ref()?.caratula.as_deref()?;
        let ancho = ANCHO - MARGEN * 2.0;
        medios::cargar_caratula_ancha(
            fuente,
            (ancho * 3.0) as u32,
            (GRANDE_VIDEO_ALTO * 3.0) as u32,
            GRANDE_RADIO_VIDEO * 3.0,
        )
    }

    // añadido en la 0.2.0
    // modified 0.2.2 -- carátula ancha, vista de vídeo y lista de salidas cerrada
    fn abrir_musica(&mut self) {
        self.caratula_grande = self.cargar_grande();
        self.caratula_ancha = self.cargar_ancha();
        self.vista = tema::Transicion::nueva(1.0, tema::M_RAPIDO);
        self.cambio = tema::Transicion::nueva(1.0, tema::M_SUAVE);
        self.menu_salida = false;
        self.menu.fijar(0.0);
        (self.aleatorio, self.repetir) = self
            .sonando
            .as_ref()
            .map_or((None, None), |s| medios::modos(&s.bus));
        self.calidad_abierta = false;
        self.calidad.fijar(0.0);
        self.musica_grande = true;
        self.señalada.señalar(None);
        self.origen_detalle = rect_celda(Celda::Musica);
        self.animar(super::Transicion::AbrirDetalle(self.origen_detalle));
    }

    // --- Entrada --------------------------------------------------------------

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.puntero_en = punto;
        if let Some((zona, ..)) = self.apretada {
            let encima = punto.and_then(|(x, y)| self.zona_en(x, y)) == Some(zona);
            self.hundido.ir_a(encima as u8 as f32);
        }
        if let Some(d) = self.detalle.as_mut() {
            return d.puntero(punto);
        }
        if let Some((x, _)) = punto {
            match self.agarre {
                Agarre::Volumen => return self.poner_volumen(self.nivel_en(x)),
                Agarre::Brillo => return self.poner_brillo(self.nivel_en(x)),
                Agarre::Posicion => return self.mover_posicion(x),
                Agarre::Nada => {}
            }
        }
        if self.musica_grande {
            return false;
        }
        let señalada = punto
            .and_then(|(x, y)| self.celda_en(x, y))
            .and_then(|c| DISPOSICION.iter().position(|d| d.0 == c));
        self.señalada.señalar(señalada)
    }

    /// De una `x` a un nivel de 0 a 100 en la pista de los deslizadores.
    // añadido en la 0.2.0
    fn nivel_en(&self, x: f32) -> u8 {
        let r = self.rect_pista(Agarre::Brillo);
        (((x - r.x) / r.width).clamp(0.0, 1.0) * 100.0).round() as u8
    }

    // añadido en la 0.2.0
    fn mover_posicion(&mut self, x: f32) -> bool {
        let r = self.rect_avance();
        let fraccion = ((x - r.x) / r.width).clamp(0.0, 1.0);
        let Some(s) = self.sonando.as_mut() else {
            return false;
        };
        let Some(d) = s.duracion else { return false };
        let nueva = Some((fraccion * d as f32).round() as u64);
        let cambia = s.posicion != nueva;
        s.posicion = nueva;
        cambia
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando()
            || self.hundido.animando()
            || self.grosor.animando()
            || self.calidad.animando()
            || self.menu.animando()
            || self.puntos_corriendo()
            || self.vista.animando()
            || self.cambio.animando()
            || self.detalle.as_ref().is_some_and(|d| d.animando())
    }

    // 0.1.0
    pub fn agarrado(&self) -> bool {
        self.agarre != Agarre::Nada
    }

    // 0.1.0
    // modified 0.2.0 -- mantener y soltar abre el detalle
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        if let Some((zona, x, y, desde)) = self.apretada.take() {
            self.hundido.ir_a(0.0);
            let encima = self.puntero_en.and_then(|(px, py)| self.zona_en(px, py)) == Some(zona);
            if !encima {
                return (true, None);
            }
            if desde.elapsed() >= PULSACION_LARGA && self.abrir_detalle_de(zona) {
                return (true, None);
            }
            return (true, self.ejecutar(x, y));
        }
        (self.soltar_agarre(), None)
    }

    /// Lo que abre una pulsación larga: Wi-Fi y Bluetooth su lista, la música
    /// su vista grande. El resto no tiene detalle y se queda en un toque.
    // añadido en la 0.2.0
    fn abrir_detalle_de(&mut self, zona: Zona) -> bool {
        match zona {
            Zona::Celda(Celda::Wifi) | Zona::Disco(Celda::Wifi) => self.abrir_lista(false),
            Zona::Celda(Celda::Bluetooth) | Zona::Disco(Celda::Bluetooth) => self.abrir_lista(true),
            Zona::Celda(Celda::Musica) if self.sonando.is_some() => {
                self.abrir_musica();
                true
            }
            _ => false,
        }
    }

    // 0.1.0
    fn soltar_agarre(&mut self) -> bool {
        let agarre = std::mem::replace(&mut self.agarre, Agarre::Nada);
        self.grosor.ir_a(0.0);
        match agarre {
            Agarre::Volumen if self.volumen != self.volumen_inicial => {
                bookos_system::request(bookos_system::Operation::Volume {
                    target: "output".into(),
                    value: self.volumen as u32,
                });
            }
            Agarre::Posicion => {
                if let Some(s) = &self.sonando
                    && let Some(destino) = s.posicion
                {
                    // Al soltar y no en cada paso: el reproductor del móvil
                    // tarda en contestar y saltaría a saltos.
                    let leida = Sonando::leer()
                        .filter(|l| l.bus == s.bus)
                        .and_then(|l| l.posicion)
                        .unwrap_or(destino);
                    medios::saltar_en(&s.bus, destino as i64 - leida as i64);
                }
            }
            _ => {}
        }
        agarre != Agarre::Nada
    }

    // añadido en la 0.2.0
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        if self.detalle.is_some() {
            return false;
        }
        let p = Point::new(x, y);
        let holgada = |mut r: Rectangle| {
            r.y -= 10.0;
            r.height += 20.0;
            r
        };
        if self.musica_grande {
            return holgada(self.rect_avance()).contains(p);
        }
        [Agarre::Volumen, Agarre::Brillo]
            .into_iter()
            .any(|cual| holgada(self.rect_pista(cual)).contains(p))
    }

    /// Qué botón cae en un punto. Las pistas no: se arrastran desde el primer
    /// momento.
    // añadido en la 0.2.0
    fn zona_en(&self, x: f32, y: f32) -> Option<Zona> {
        if self.musica_grande {
            return self.grande_en(x, y);
        }
        if self.rect_silencio().contains(Point::new(x, y)) {
            return Some(Zona::Silencio);
        }
        if let Some(orden) = self.medio_en(x, y) {
            return Some(Zona::Medio(orden));
        }
        let celda = self.celda_en(x, y)?;
        if matches!(celda, Celda::Wifi | Celda::Bluetooth) {
            let r = rect_celda(celda);
            if x < r.x + (r.height - DISCO) / 2.0 + DISCO {
                return Some(Zona::Disco(celda));
            }
        }
        Some(Zona::Celda(celda))
    }

    /// Cuánto se dibuja hundido algo, de 0 a 1.
    // añadido en la 0.2.0
    fn hundimiento(&self, zona: Zona) -> f32 {
        if self.hundida == Some(zona) {
            self.hundido.valor()
        } else {
            0.0
        }
    }

    /// Hundimiento de una pieza entera, venga de ella o de su disco.
    // añadido en la 0.2.0
    fn hundimiento_celda(&self, celda: Celda) -> f32 {
        self.hundimiento(Zona::Celda(celda))
            .max(self.hundimiento(Zona::Disco(celda)))
    }

    // añadido en la 0.2.0
    pub fn desplazar(&mut self, dy: f32) -> bool {
        self.detalle.as_mut().is_some_and(|d| d.desplazar(dy))
    }

    // 0.1.0
    // modified 0.2.0 -- los botones esperan a soltar; aquí solo se hunden
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.detalle.is_none()
            && let Some(zona) = self.zona_en(x, y)
        {
            self.apretada = Some((zona, x, y, Instant::now()));
            self.hundida = Some(zona);
            self.puntero_en = Some((x, y));
            self.hundido.ir_a(1.0);
            return None;
        }
        self.ejecutar(x, y)
    }

    /// Abre la lista de Wi-Fi (`false`) o Bluetooth (`true`) en el sitio. Sin
    /// servicio no hay lista: la tarjeta diría «Cargando…» para siempre.
    // añadido en la 0.2.0
    fn abrir_lista(&mut self, bluetooth: bool) -> bool {
        let disponible = if bluetooth {
            self.bluetooth_disponible
        } else {
            self.wifi_disponible
        };
        if !disponible {
            return false;
        }
        self.detalle = Some(Box::new(if bluetooth {
            Detalle::Bluetooth(Bluetooth::new())
        } else {
            Detalle::Red(Red::new())
        }));
        self.señalada.señalar(None);
        self.origen_detalle = rect_celda(if bluetooth {
            Celda::Bluetooth
        } else {
            Celda::Wifi
        });
        self.animar(super::Transicion::AbrirDetalle(self.origen_detalle));
        true
    }

    // 0.1.0
    // modified 0.2.0 -- rejilla nueva y lo que suena en grande
    fn ejecutar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(d) = self.detalle.as_mut() {
            if lista::volver_en(x - DESVIO, y) {
                self.volver();
                return None;
            }
            return d.pulsar(x, y);
        }
        let p = Point::new(x, y);
        let holgada = |mut r: Rectangle| {
            r.y -= 10.0;
            r.height += 20.0;
            r
        };
        if self.musica_grande {
            // Con la lista abierta, cualquier clic la cierra: dentro elige, fuera
            // solo cierra. Sin pasar al botón de debajo.
            if self.menu_salida {
                self.poner_menu(false);
                if let Some(o) = self.opcion_en(x, y).map(|i| self.salidas[i].clone())
                    && !o.actual
                {
                    bookos_system::request(bookos_system::Operation::AudioDefault {
                        target: o.sink.clone(),
                        input: false,
                    });
                    // Marcada ya: la lectura de PipeWire llega en el siguiente
                    // refresco y no debe verse el salto atrás.
                    for s in &mut self.salidas {
                        s.actual = s.sink == o.sink;
                    }
                }
                return None;
            }
            // La etiqueta antes que la barra: la holgura de 10 px de la barra
            // tapaba la mitad de arriba de la píldora, y pulsarla ahí
            // agarraba el avance en vez de desplegar la calidad.
            // modified 0.2.0 -- la calidad gana a la holgura de la barra
            let calidad = self.grande_en(x, y) == Some(Zona::Calidad);
            if !calidad && holgada(self.rect_avance()).contains(p) {
                self.agarre = Agarre::Posicion;
                self.grosor.ir_a(1.0);
                self.mover_posicion(x);
                return None;
            }
            return match self.grande_en(x, y) {
                Some(Zona::Plegar) => {
                    self.volver();
                    None
                }
                Some(Zona::Salida) => {
                    self.poner_menu(!self.salidas.is_empty());
                    None
                }
                Some(Zona::Vista(video)) => {
                    self.vista.ir_a(if video { 1.0 } else { 0.0 });
                    if tema::efectos_reducidos() {
                        self.vista.fijar(if video { 1.0 } else { 0.0 });
                    }
                    None
                }
                Some(Zona::Favorito) => {
                    if let (Some(f), Some(pista), Some(s)) =
                        (self.extras.favorito, &self.extras.pista, &self.sonando)
                        && medios::poner_favorito(&s.bus, pista, !f)
                    {
                        self.extras.favorito = Some(!f);
                    }
                    None
                }
                Some(Zona::Medio(orden)) => {
                    self.sentido = if orden == medios::Orden::Anterior {
                        -1.0
                    } else {
                        1.0
                    };
                    self.mandar(orden);
                    None
                }
                Some(Zona::Calidad) => {
                    self.calidad_abierta = !self.calidad_abierta;
                    self.calidad.ir_a(self.calidad_abierta as u8 as f32);
                    if self.calidad_abierta {
                        self.onda = Instant::now();
                    }
                    None
                }
                Some(Zona::Aleatorio) => {
                    if let (Some(a), Some(s)) = (self.aleatorio, &self.sonando)
                        && medios::poner_aleatorio(&s.bus, !a)
                    {
                        // Sin esperar a releerlo, como el play.
                        self.aleatorio = Some(!a);
                    }
                    None
                }
                Some(Zona::Repetir) => {
                    if let (Some(r), Some(s)) = (self.repetir, &self.sonando)
                        && medios::poner_repetir(&s.bus, r.siguiente())
                    {
                        self.repetir = Some(r.siguiente());
                    }
                    None
                }
                _ => None,
            };
        }
        if self.rect_silencio().contains(p) {
            if self.audio_disponible {
                self.silenciado = !self.silenciado;
                bookos_system::request(bookos_system::Operation::Mute {
                    target: "output".into(),
                    muted: Some(self.silenciado),
                });
            }
            return None;
        }
        for cual in [Agarre::Volumen, Agarre::Brillo] {
            if holgada(self.rect_pista(cual)).contains(p) {
                self.agarre = cual;
                self.grosor.ir_a(1.0);
                if cual == Agarre::Volumen {
                    self.volumen_inicial = self.volumen;
                    self.poner_volumen(self.nivel_en(x));
                } else {
                    self.poner_brillo(self.nivel_en(x));
                }
                return None;
            }
        }
        if let Some(orden) = self.medio_en(x, y) {
            self.mandar(orden);
            return None;
        }
        match self.zona_en(x, y)? {
            Zona::Disco(celda) => {
                let bluetooth = celda == Celda::Bluetooth;
                let (disponible, encendido) = if bluetooth {
                    (self.bluetooth_disponible, self.bluetooth)
                } else {
                    (self.wifi_disponible, self.wifi)
                };
                if disponible {
                    bookos_system::request(if bluetooth {
                        bookos_system::Operation::BluetoothPower {
                            enabled: !encendido,
                        }
                    } else {
                        bookos_system::Operation::WifiPower {
                            enabled: !encendido,
                        }
                    });
                }
                None
            }
            Zona::Celda(Celda::Wifi) => {
                self.abrir_lista(false);
                None
            }
            Zona::Celda(Celda::Bluetooth) => {
                self.abrir_lista(true);
                None
            }
            Zona::Celda(Celda::Musica) => {
                if self.sonando.is_some() {
                    self.abrir_musica();
                    None
                } else {
                    // Nada sonando: la tarjeta lleva al reproductor de BookOS.
                    Some(Accion::Lanzar("bookos-player".into()))
                }
            }
            Zona::Celda(Celda::Noche) => {
                self.noche = !self.noche;
                Some(Accion::LuzNocturna(self.noche))
            }
            Zona::Celda(Celda::Oscuro) => Some(Accion::Apariencia {
                // Elegir uno fijo: con «automático» puesto, pulsar fija el
                // contrario del que se ve ahora, que es lo que se espera.
                modo: if tema::es_claro() {
                    tema::ModoTema::Oscuro
                } else {
                    tema::ModoTema::Claro
                },
                acento: tema::acento_actual(),
            }),
            Zona::Celda(Celda::NoMolestar) => {
                self.no_molestar = !self.no_molestar;
                Some(Accion::NoMolestar(self.no_molestar))
            }
            Zona::Celda(Celda::Microfono) => {
                self.microfono_mudo = !self.microfono_mudo;
                bookos_system::request(bookos_system::Operation::Mute {
                    target: "input".into(),
                    muted: Some(self.microfono_mudo),
                });
                None
            }
            Zona::Celda(Celda::Captura) => Some(Accion::AbrirCaptura),
            Zona::Celda(Celda::Mantener) => {
                alternar_mantener_despierto();
                Some(Accion::MantenerDespierto)
            }
            Zona::Medio(_)
            | Zona::Silencio
            | Zona::Plegar
            | Zona::Calidad
            | Zona::Aleatorio
            | Zona::Repetir
            | Zona::Salida
            | Zona::Vista(_)
            | Zona::Favorito => None,
        }
    }

    /// Una orden al reproductor, sin `Accion`: `Lanzar` cerraría la tarjeta y
    /// aquí se quiere seguir viendo lo que suena.
    // añadido en la 0.2.0
    // modified 0.2.2 -- recuerda lo pedido para que una lectura vieja no lo deshaga
    fn mandar(&mut self, orden: medios::Orden) {
        if let Some(sonando) = self.sonando.as_mut()
            && sonando.ejecutar(orden)
            && orden == medios::Orden::Alternar
        {
            // Sin esperar a la lectura de vuelta: con el móvil detrás de
            // BookOS Link tarda y el botón parecería muerto.
            sonando.reproduciendo = !sonando.reproduciendo;
            self.alternado = Some((sonando.reproduciendo, std::time::Instant::now()));
        }
    }

    /// «No molestar» lo guarda el shell; la tarjeta solo lo enseña.
    // 0.1.0
    pub fn poner_no_molestar(&mut self, activo: bool) {
        self.no_molestar = activo;
    }

    // 0.1.0
    fn poner_volumen(&mut self, nivel: u8) -> bool {
        if nivel == self.volumen {
            return false;
        }
        self.volumen = nivel;
        bookos_system::request(bookos_system::Operation::VolumePreview {
            target: "output".into(),
            value: nivel as u32,
        });
        true
    }

    // 0.1.0
    fn poner_brillo(&mut self, nivel: u8) -> bool {
        // Mismo suelo que el emergente del brillo: a cero no se ve ni el
        // deslizador con el que volver a subirlo.
        if self.backlight.is_none() {
            return false;
        }
        let nivel = nivel.clamp(5, 100);
        if nivel == self.brillo {
            return false;
        }
        self.brillo = nivel;
        if let Some((dispositivo, maximo)) = &self.backlight {
            let crudo = (*maximo as f64 * nivel as f64 / 100.0).round() as u32;
            crate::retroiluminacion::solicitar("backlight", dispositivo, crudo);
        }
        true
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            // Esc cierra primero la lista de salidas, después la vista.
            crate::TeclaPulsada::Escape if self.menu_salida => {
                self.poner_menu(false);
                Tecla::Consumida
            }
            crate::TeclaPulsada::Escape if self.detalle.is_some() || self.musica_grande => {
                self.volver();
                Tecla::Consumida
            }
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            tecla => match self.detalle.as_mut() {
                Some(d) => d.tecla(tecla),
                None => Tecla::Ignorada,
            },
        }
    }

    // 0.1.0
    fn animar(&mut self, transicion: super::Transicion) {
        if !tema::efectos_reducidos() {
            self.transicion = Some(transicion);
        }
    }

    // añadido en la 0.2.0
    fn poner_menu(&mut self, abierto: bool) {
        self.menu_salida = abierto;
        self.menu.ir_a(abierto as u8 as f32);
    }

    /// Los puntos solo corren con el bloque a la vista y la música sonando:
    /// parados no dicen nada que no diga ya el botón de pausa, y cada vuelta
    /// repinta la tarjeta entera en CPU.
    // añadido en la 0.2.0
    fn puntos_corriendo(&self) -> bool {
        self.musica_grande
            && self.calidad_abierta
            && self.sonando.as_ref().is_some_and(|s| s.reproduciendo)
    }

    // 0.1.0
    pub fn tomar_transicion(&mut self) -> Option<super::Transicion> {
        self.transicion.take()
    }

    /// De la lista o de la música a la estación, de vuelta a la pieza que la
    /// abrió. Se relee al volver: dentro no se refrescaba.
    // 0.1.0
    fn volver(&mut self) {
        self.detalle = None;
        self.musica_grande = false;
        self.menu_salida = false;
        self.menu.fijar(0.0);
        self.caratula_grande = None;
        self.caratula_ancha = None;
        self.calidad_abierta = false;
        self.calidad.fijar(0.0);
        self.animar(super::Transicion::CerrarDetalle(self.origen_detalle));
        self.red = self.wifi_disponible.then(conectado_a).flatten();
        self.dispositivo = self.bluetooth_disponible.then(emparejado_con).flatten();
        self.refrescar();
    }

    // --- Dibujo ---------------------------------------------------------------

    /// Un glifo sólido teñido, o su hueco si no existe.
    // añadido en la 0.2.0
    fn glifo(nombre: &str, px: f32, color: Color) -> PanelElement<'static> {
        match icono::propio_solido(nombre) {
            Some(ic) => icono::ver_teñido_propio(&ic, px, color),
            None => Space::new().width(Length::Fixed(px)).into(),
        }
    }

    /// Un círculo con un glifo dentro.
    // 0.1.0
    fn circulo(
        nombre: &str,
        lado: f32,
        px: f32,
        fondo: Color,
        tinta: Color,
    ) -> PanelElement<'static> {
        container(Self::glifo(nombre, px, tinta))
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: (lado / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Una caja de fondo y radio dados, centrada en su celda y encogida por el
    /// hundimiento: los vecinos no se mueven.
    // añadido en la 0.2.0
    fn en_celda<'a>(
        celda: Celda,
        h: f32,
        encoge: f32,
        fondo: Color,
        radio: f32,
        dentro: PanelElement<'a>,
    ) -> PanelElement<'a> {
        let r = rect_celda(celda);
        let k = 1.0 - encoge * h;
        let fondo = tema::mezclar(fondo, Color::BLACK, 0.10 * h);
        container(
            container(dentro)
                .width(Length::Fixed(r.width * k))
                .height(Length::Fixed(r.height * k))
                .clip(true)
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: radio.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(r.width))
        .height(Length::Fixed(r.height))
        .center_x(Length::Fixed(r.width))
        .center_y(Length::Fixed(r.height))
        .into()
    }

    /// Un conmutador redondo de una celda.
    // añadido en la 0.2.0
    fn circulo_celda(&self, celda: Celda, nombre: &'static str, activo: bool) -> PanelElement<'_> {
        let i = DISPOSICION.iter().position(|d| d.0 == celda).unwrap_or(0);
        let (fondo, tinta) = if activo {
            control::encendido()
        } else {
            (
                tema::mezclar(placa(), tema::tinta(), 0.06 * self.señalada.intensidad(i)),
                tema::texto(),
            )
        };
        Self::en_celda(
            celda,
            self.hundimiento_celda(celda),
            HUNDIDO_CIRCULO,
            fondo,
            CELDA / 2.0,
            container(Self::glifo(nombre, GLIFO, tinta))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into(),
        )
    }

    /// Una píldora: disco con el glifo, nombre y estado.
    // añadido en la 0.2.0
    fn pildora(
        &self,
        celda: Celda,
        nombre: &'static str,
        glifo: &'static str,
        estado: String,
        activo: bool,
    ) -> PanelElement<'_> {
        let i = DISPOSICION.iter().position(|d| d.0 == celda).unwrap_or(0);
        let r = rect_celda(celda);
        let margen = (r.height - DISCO) / 2.0;
        let (disco, tinta) = if activo {
            control::encendido()
        } else {
            (disco_apagado(), tema::texto())
        };
        let hd = self.hundimiento(Zona::Disco(celda));
        let disco = tema::mezclar(disco, Color::BLACK, 0.12 * hd);
        let ancho_texto = r.width - margen - DISCO - 8.0 - 6.0;
        let fondo = tema::mezclar(placa(), tema::tinta(), 0.06 * self.señalada.intensidad(i));
        Self::en_celda(
            celda,
            self.hundimiento(Zona::Celda(celda)),
            HUNDIDO_GRANDE,
            fondo,
            r.height / 2.0,
            row![
                Space::new().width(Length::Fixed(margen)),
                Self::circulo(
                    glifo,
                    DISCO * (1.0 - HUNDIDO_CIRCULO * hd),
                    20.0,
                    disco,
                    tinta
                ),
                Space::new().width(Length::Fixed(8.0)),
                column![
                    text(recortar_px(nombre, ancho_texto, 13.0))
                        .size(13.0)
                        .font(control::peso(Weight::Semibold))
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
                    text(recortar_px(&estado, ancho_texto, 11.0))
                        .size(11.0)
                        .color(tema::TEXTO2)
                        .wrapping(iced_core::text::Wrapping::None),
                ]
                .spacing(1.0),
            ]
            .align_y(Vertical::Center)
            .height(Length::Fill)
            .into(),
        )
    }

    /// Wi-Fi o Bluetooth. El disco se enciende con la radio y el estado dice si
    /// hay conexión: con la antena encendida y sin red, «No conectado».
    // 0.1.0
    // modified 0.2.0 -- píldora de la rejilla nueva
    fn conexion(&self, bluetooth: bool) -> PanelElement<'_> {
        let (celda, nombre, disponible, encendida, conectado) = if bluetooth {
            (
                Celda::Bluetooth,
                "Bluetooth",
                self.bluetooth_disponible,
                self.bluetooth,
                self.dispositivo.as_deref(),
            )
        } else {
            (
                Celda::Wifi,
                "Wi-Fi",
                self.wifi_disponible,
                self.wifi,
                self.red.as_deref(),
            )
        };
        let glifo = match (bluetooth, encendida && disponible) {
            (true, true) => "bluetooth",
            (true, false) => "bluetooth-apagado",
            (false, true) => "wifi",
            (false, false) => "sin-red",
        };
        let estado = match (disponible, encendida, conectado) {
            (false, ..) => "No disponible".to_owned(),
            (true, _, Some(donde)) => donde.to_owned(),
            (true, true, None) => "No conectado".to_owned(),
            (true, false, None) => "Desactivado".to_owned(),
        };
        self.pildora(celda, nombre, glifo, estado, encendida && disponible)
    }

    /// Lo que suena, en la celda de 2×2, teñido con el color de la carátula.
    // 0.1.0
    // modified 0.2.0 -- cuadrado junto a Wi-Fi y Bluetooth, mandos sólidos sin placa
    fn musica(&self) -> PanelElement<'_> {
        let r = rect_celda(Celda::Musica);
        let ancho = r.width - MUSICA_RELLENO * 2.0;
        // El tinte se mezcla en opaco con la placa —más en oscuro, menos en
        // claro— para que el texto siga leyéndose sobre cualquier carátula.
        let fondo = match self.tinte {
            Some(t) => tema::mezclar(placa(), t, if tema::es_claro() { 0.22 } else { 0.42 }),
            None => placa(),
        };
        let tinta = tema::tinta_sobre(fondo);
        let tinta2 = tema::mezclar(tinta, fondo, 0.4);
        let caratula: PanelElement<'_> = match &self.caratula {
            Some(foto) => iced_widget::image(foto.clone())
                .width(Length::Fixed(MUSICA_CARATULA))
                .height(Length::Fixed(MUSICA_CARATULA))
                .into(),
            None => Self::circulo(
                "musica",
                MUSICA_CARATULA,
                20.0,
                disco_apagado(),
                tema::TEXTO2,
            ),
        };
        let (titulo, artista) = match &self.sonando {
            Some(s) => (
                s.titulo.as_str(),
                if s.es_de_movil() {
                    s.aplicacion.as_str()
                } else {
                    s.artista.as_str()
                },
            ),
            None => ("No se reproduce nada", "Player"),
        };
        let mando = |nombre: &'static str, px: f32| -> PanelElement<'static> {
            container(Self::glifo(nombre, px, tinta))
                .width(Length::Fixed(ancho / 3.0))
                .height(Length::Fixed(MANDO))
                .center_x(Length::Fixed(ancho / 3.0))
                .center_y(Length::Fixed(MANDO))
                .into()
        };
        let mut contenido = column![
            caratula,
            Space::new().height(Length::Fixed(8.0)),
            text(recortar_px(titulo, ancho, 13.0))
                .size(13.0)
                .font(control::peso(Weight::Semibold))
                .color(tinta)
                .wrapping(iced_core::text::Wrapping::None),
            text(recortar_px(artista, ancho, 11.0))
                .size(11.0)
                .color(tinta2)
                .wrapping(iced_core::text::Wrapping::None),
            Space::new().height(Length::Fill),
        ];
        if let Some(s) = &self.sonando {
            contenido = contenido.push(row![
                mando("anterior", 20.0),
                mando(
                    if s.reproduciendo {
                        "pausa"
                    } else {
                        "reproducir"
                    },
                    24.0
                ),
                mando("siguiente", 20.0),
            ]);
        }
        Self::en_celda(
            Celda::Musica,
            self.hundimiento(Zona::Celda(Celda::Musica)),
            HUNDIDO_GRANDE,
            fondo,
            R_INTERIOR,
            container(contenido)
                .padding(MUSICA_RELLENO)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        )
    }

    /// Pantalla o Sonido: título, y debajo icono pequeño, pista e icono grande.
    // 0.1.0
    // modified 0.2.0 -- como Tahoe, sin cifra; la pista engorda al agarrarla
    fn deslizador(&self, cual: Agarre) -> PanelElement<'_> {
        let r = self.rect_pista(cual);
        let (titulo, nivel, pequeño, grande, apagado) = match cual {
            Agarre::Brillo => (
                "Pantalla",
                self.brillo,
                "brillo",
                "brillo",
                self.backlight.is_none(),
            ),
            _ => (
                "Sonido",
                self.volumen,
                if self.silenciado || self.volumen == 0 {
                    "volumen-silencio"
                } else {
                    "volumen-bajo"
                },
                "volumen-alto",
                self.silenciado,
            ),
        };
        let grueso = if self.agarre == cual {
            PISTA + (PISTA_AGARRADA - PISTA) * self.grosor.valor()
        } else {
            PISTA
        };
        let icono = |nombre: &'static str, px: f32| -> PanelElement<'static> {
            container(Self::glifo(nombre, px, tema::texto()))
                .width(Length::Fixed(ICONO_DESL))
                .height(Length::Fixed(DESL_FILA))
                .center_x(Length::Fixed(ICONO_DESL))
                .center_y(Length::Fixed(DESL_FILA))
                .into()
        };
        let color = if apagado { tema::TEXTO2 } else { tema::acento() };
        let pista = control::barra(
            r.width,
            grueso,
            nivel.min(100) as f32 / 100.0,
            color,
            disco_apagado(),
            &[],
            None,
        );
        container(column![
            text(titulo)
                .size(12.5)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto())
                .height(Length::Fixed(DESL_TITULO)),
            Space::new().height(Length::Fixed(DESL_ENTRE)),
            row![
                icono(pequeño, 15.0),
                Space::new().width(Length::Fixed(8.0)),
                container(pista)
                    .height(Length::Fixed(DESL_FILA))
                    .center_y(Length::Fixed(DESL_FILA)),
                Space::new().width(Length::Fixed(8.0)),
                icono(grande, 20.0),
            ]
            .align_y(Vertical::Center)
            .height(Length::Fixed(DESL_FILA)),
        ])
        .padding(iced_core::Padding {
            top: DESL_RELLENO_V,
            bottom: DESL_RELLENO_V,
            left: DESL_RELLENO_H,
            right: DESL_RELLENO_H,
        })
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(DESLIZADOR))
        .style(|_: &iced_widget::Theme| container::Style {
            background: Some(placa().into()),
            border: Border {
                radius: R_INTERIOR.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Los colores de lo que suena en grande, todos opacos (BookOS no usa
    /// superficies translúcidas, y en tiny-skia un relleno con alfa cuesta
    /// ~17× uno opaco). El fondo es un degradado del color de la carátula
    /// mezclado con casi negro o con blanco: mezclarlo con el gris de la
    /// tarjeta daba marrones barro.
    // añadido en la 0.2.2
    fn paleta_grande(&self) -> Paleta {
        let claro = tema::es_claro();
        let negro = Color::from_rgb8(8, 8, 10);
        let (arriba, abajo) = match self.tinte {
            Some(c) if claro => (
                tema::mezclar(Color::WHITE, c, 0.16),
                tema::mezclar(Color::WHITE, c, 0.03),
            ),
            Some(c) => (tema::mezclar(negro, c, 0.46), tema::mezclar(negro, c, 0.22)),
            None => (tema::card(), tema::card()),
        };
        let medio = tema::mezclar(arriba, abajo, 0.5);
        let tinta = tema::tinta_sobre(medio);
        Paleta {
            arriba,
            abajo,
            medio,
            tinta,
            tinta2: tema::mezclar(tinta, medio, 0.4),
            pista: tema::mezclar(medio, tinta, 0.18),
            placa: if claro {
                tema::mezclar(medio, Color::WHITE, 0.72)
            } else {
                tema::mezclar(medio, Color::WHITE, 0.11)
            },
            apagado: tema::mezclar(tinta, medio, 0.7),
            menu: if claro {
                Color::WHITE
            } else {
                tema::mezclar(medio, Color::WHITE, 0.1)
            },
        }
    }

    /// Una caja de color con su radio.
    // añadido en la 0.2.2
    fn caja<'a>(
        contenido: PanelElement<'a>,
        w: f32,
        h: f32,
        fondo: Color,
        radio: f32,
    ) -> PanelElement<'a> {
        container(contenido)
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .center_x(Length::Fixed(w))
            .center_y(Length::Fixed(h))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: radio.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Lo que suena en grande, como la maqueta del lienzo: cabecera con la app
    /// y la salida, carátula (o vídeo), título con ♥, posición, calidad, mandos
    /// y, encima, la lista de salidas si está abierta.
    // añadido en la 0.2.0
    // modified 0.2.2 -- la maqueta del lienzo, 1:1
    fn musica_grande(&self) -> PanelElement<'_> {
        let ancho = ANCHO - MARGEN * 2.0;
        let g = self.grande();
        let p = self.paleta_grande();
        let s = self.sonando.as_ref();

        // Cabecera: volver · icono y nombre de la app · salida.
        let origen = s.map(|s| s.aplicacion.clone()).unwrap_or_default();
        let mut app = row![].spacing(6.0).align_y(Vertical::Center);
        if let Some(ic) = &self.app_icono {
            app = app.push(crate::icono::ver(ic, 18.0, 18.0));
        }
        app = app.push(
            text(recortar_px(
                &origen,
                ancho - GRANDE_CABECERA * 2.0 - 40.0,
                12.0,
            ))
            .size(12.0)
            .font(control::peso(Weight::Semibold))
            .color(p.tinta2)
            .wrapping(iced_core::text::Wrapping::None),
        );
        let glifo_salida = self.salida_actual().map_or("volumen-alto", |o| o.glifo);
        // Fundido con el mismo muelle que despliega la lista: el botón y la
        // lista llegan a la vez, al abrir y al cerrar.
        let m = self.menu.valor().clamp(0.0, 1.0);
        let (fondo_salida, tinta_salida) = (
            tema::mezclar(p.placa, p.tinta, m),
            tema::mezclar(p.tinta, p.medio, m),
        );
        let cabecera = row![
            Self::circulo("chevron-abajo", GRANDE_CABECERA, 18.0, p.placa, p.tinta),
            container(app).width(Length::Fill).center_x(Length::Fill),
            Self::circulo(
                glifo_salida,
                GRANDE_CABECERA,
                18.0,
                fondo_salida,
                tinta_salida
            ),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fixed(GRANDE_CABECERA));

        // Carátula: entra desde el lado al cambiar de canción; pausar no la mueve.
        let entra = self.cambio.valor();
        let (radio, foto) = match (&self.caratula_ancha, self.ver_video()) {
            (Some(ancha), true) => (GRANDE_RADIO_VIDEO, Some(ancha)),
            _ => (GRANDE_RADIO, self.caratula_grande.as_ref()),
        };
        let caratula: PanelElement<'_> = match foto {
            Some(h) => iced_widget::image(h.clone())
                .width(Length::Fixed(g.caratula_w))
                .height(Length::Fixed(g.caratula_h))
                .opacity(entra)
                .into(),
            // Sin carátula, el mismo cuadrado redondeado que ocupa la foto.
            None => Self::caja(
                Self::glifo("musica", 44.0, p.tinta2),
                g.caratula_w,
                g.caratula_h,
                p.placa,
                radio,
            ),
        };
        let dx = 24.0 * (1.0 - entra) * self.sentido;
        let caratula = container(caratula)
            .padding(iced_core::Padding {
                left: dx.max(0.0),
                right: (-dx).max(0.0),
                ..Default::default()
            })
            .width(Length::Fill)
            .center_x(Length::Fill)
            .height(Length::Fixed(g.caratula_h));

        let mut cuerpo = column![
            cabecera,
            Space::new().height(Length::Fixed(g.caratula - MARGEN - GRANDE_CABECERA)),
            caratula,
        ];

        // Portada | Vídeo: el mismo dibujo que «Modo de energía», a su tamaño.
        if let Some(sy) = g.selector {
            let mitad = (SELECTOR_ANCHO - 8.0) / 2.0;
            let v = self.vista.valor();
            let opcion = |nombre: &'static str, elegida: bool| -> PanelElement<'static> {
                container(
                    text(nombre)
                        .size(12.5)
                        .font(control::peso(if elegida {
                            Weight::Semibold
                        } else {
                            Weight::Medium
                        }))
                        .color(if elegida { p.medio } else { p.tinta2 }),
                )
                .width(Length::Fixed(mitad))
                .height(Length::Fixed(SELECTOR_ALTO - 8.0))
                .center_x(Length::Fixed(mitad))
                .center_y(Length::Fixed(SELECTOR_ALTO - 8.0))
                .into()
            };
            let pulgar = row![
                Space::new().width(Length::Fixed(mitad * v)),
                Self::caja(
                    Space::new().into(),
                    mitad,
                    SELECTOR_ALTO - 8.0,
                    p.tinta,
                    16.0
                ),
            ];
            let selector = Self::caja(
                container(iced_widget::stack![
                    pulgar,
                    row![opcion("Portada", v <= 0.5), opcion("Vídeo", v > 0.5)],
                ])
                .padding(4.0)
                .into(),
                SELECTOR_ANCHO,
                SELECTOR_ALTO,
                p.placa,
                SELECTOR_ALTO / 2.0,
            );
            cuerpo = cuerpo
                .push(Space::new().height(Length::Fixed(sy - g.caratula - g.caratula_h)))
                .push(
                    container(selector)
                        .width(Length::Fill)
                        .center_x(Length::Fill),
                );
        }

        // Título y artista a la izquierda; ♥ a la derecha si el reproductor lo da.
        let ancho_textos = if self.extras.favorito.is_some() {
            ancho - 44.0
        } else {
            ancho
        };
        let mut fila_textos = row![
            column![
                text(recortar_px(
                    s.map_or("No se reproduce nada", |s| s.titulo.as_str()),
                    ancho_textos,
                    18.0
                ))
                .size(18.0)
                .line_height(iced_core::text::LineHeight::Absolute(24.0.into()))
                .font(control::peso(Weight::Bold))
                .color(p.tinta)
                .wrapping(iced_core::text::Wrapping::None),
                text(recortar_px(
                    s.map_or("", |s| s.artista.as_str()),
                    ancho_textos,
                    14.0
                ))
                .size(14.0)
                .line_height(iced_core::text::LineHeight::Absolute(20.0.into()))
                .color(p.tinta2)
                .wrapping(iced_core::text::Wrapping::None),
            ]
            .width(Length::Fixed(ancho_textos)),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fixed(TEXTOS_ALTO));
        if let Some(fav) = self.extras.favorito {
            fila_textos = fila_textos.push(Space::new().width(Length::Fill)).push(
                container(Self::glifo(
                    if fav { "corazon-lleno" } else { "corazon" },
                    20.0,
                    if fav { p.tinta } else { p.tinta2 },
                ))
                .width(Length::Fixed(36.0))
                .center_x(Length::Fixed(36.0)),
            );
        }

        // Posición: engorda de 6 a 12 al agarrarla, solo en alto.
        let r = self.rect_avance();
        let avance = s.and_then(|s| s.avance()).unwrap_or(0.0);
        let grueso = if self.agarre == Agarre::Posicion {
            AVANCE + (AVANCE_AGARRADO - AVANCE) * self.grosor.valor()
        } else {
            AVANCE
        };
        let pista = Self::caja(
            row![Self::caja(
                Space::new().into(),
                (r.width * avance).max(grueso),
                grueso,
                p.tinta,
                grueso / 2.0
            )]
            .width(Length::Fill)
            .into(),
            r.width,
            grueso,
            p.pista,
            grueso / 2.0,
        );
        // Con un navegador detrás a veces llega sin posición o sin duración
        // (visto con Firefox y YouTube; en pausa sí publica las dos). Se
        // enseña lo que haya y sin nada el hueco queda vacío, no «--:--».
        let (ya, resta) = match s.map(|s| (s.posicion, s.duracion)) {
            Some((Some(pos), Some(d))) => (
                medios::reloj(pos),
                format!("-{}", medios::reloj(d.saturating_sub(pos))),
            ),
            Some((Some(pos), None)) => (medios::reloj(pos), String::new()),
            _ => (String::new(), String::new()),
        };
        let etiqueta: PanelElement<'_> = match self.etiqueta_calidad() {
            Some(nombre) => {
                let ancho_e = self.rect_calidad().map_or(0.0, |r| r.width);
                Self::caja(
                    row![
                        Self::glifo("destello", 11.0, p.tinta),
                        text(nombre)
                            .size(10.5)
                            .font(control::peso(Weight::Semibold))
                            .color(p.tinta),
                    ]
                    .spacing(4.0)
                    .align_y(Vertical::Center)
                    .into(),
                    ancho_e,
                    TIEMPOS,
                    p.placa,
                    TIEMPOS / 2.0,
                )
            }
            None => Space::new().into(),
        };
        cuerpo = cuerpo
            .push(Space::new().height(Length::Fixed(16.0)))
            .push(fila_textos)
            .push(Space::new().height(Length::Fixed(14.0)))
            .push(
                container(pista)
                    .height(Length::Fixed(GRANDE_AVANCE))
                    .center_y(Length::Fixed(GRANDE_AVANCE)),
            )
            .push(iced_widget::stack![
                row![
                    text(ya).size(11.0).color(p.tinta2),
                    Space::new().width(Length::Fill),
                    text(resta).size(11.0).color(p.tinta2),
                ]
                .align_y(Vertical::Center)
                .height(Length::Fixed(TIEMPOS)),
                container(etiqueta)
                    .width(Length::Fill)
                    .height(Length::Fixed(TIEMPOS))
                    .center_x(Length::Fill)
                    .center_y(Length::Fixed(TIEMPOS)),
            ]);

        // Bloque de calidad: lo que suena → por dónde llega, sin rótulos.
        if g.detalle.is_some() {
            let mitad = (ancho - 24.0 - 34.0) / 2.0;
            let columna =
                |(glifo, nombre, dato): (&'static str, String, String)| -> PanelElement<'static> {
                    column![
                        Self::caja(Self::glifo(glifo, 18.0, p.tinta), 36.0, 36.0, p.pista, 11.0),
                        Space::new().height(Length::Fixed(6.0)),
                        text(recortar_px(&nombre, mitad, 13.5))
                            .size(13.5)
                            .line_height(iced_core::text::LineHeight::Absolute(18.0.into()))
                            .font(control::peso(Weight::Bold))
                            .color(p.tinta)
                            .wrapping(iced_core::text::Wrapping::None),
                        text(recortar_px(&dato, mitad, 11.0))
                            .size(11.0)
                            .line_height(iced_core::text::LineHeight::Absolute(15.0.into()))
                            .color(p.tinta2)
                            .wrapping(iced_core::text::Wrapping::None),
                    ]
                    .align_x(Horizontal::Center)
                    .width(Length::Fixed(mitad))
                    .into()
                };
            // Una onda que va de lo que suena a por dónde llega: cada punto se
            // enciende 0,2 s después del anterior en un ciclo de 1,2 s.
            let fase = self.onda.elapsed().as_secs_f32();
            let punto = |i: usize| {
                let t = if self.puntos_corriendo() {
                    let x = (fase - i as f32 * 0.2).rem_euclid(1.2) / 1.2;
                    (x * std::f32::consts::TAU).sin().max(0.0)
                } else {
                    0.0
                };
                Self::caja(
                    Space::new().into(),
                    4.0,
                    4.0,
                    tema::mezclar(p.tinta2, p.tinta, t),
                    2.0,
                )
            };
            let [origen, destino] = self.columnas_calidad();
            let bloque = Self::caja(
                row![
                    columna(origen),
                    container(row![punto(0), punto(1), punto(2)].spacing(4.0))
                        .width(Length::Fixed(34.0))
                        .height(Length::Fixed(36.0))
                        .center_x(Length::Fixed(34.0))
                        .center_y(Length::Fixed(36.0)),
                    columna(destino),
                ]
                .padding(12.0)
                .into(),
                ancho,
                DETALLE_ALTO,
                p.placa,
                tema::R_CONTROL,
            );
            cuerpo = cuerpo.push(crate::recorte::Recorte::new(
                column![Space::new().height(Length::Fixed(10.0)), bloque],
                ancho,
                g.detalle_alto,
                0.0,
            ));
        }

        // Mandos: cinco a lo ancho; aleatorio y repetir en gris si el
        // reproductor no los ofrece, sobre placa si están puestos.
        let reproduciendo = s.is_some_and(|s| s.reproduciendo);
        let lateral =
            |nombre: &'static str, estado: Option<bool>, uno: bool| -> PanelElement<'static> {
                let (fondo, color) = match estado {
                    Some(true) => (Some(p.placa), p.tinta),
                    Some(false) => (None, p.tinta2),
                    None => (None, p.apagado),
                };
                let mut capa = iced_widget::stack![Self::caja(
                    Self::glifo(nombre, 20.0, color),
                    40.0,
                    40.0,
                    fondo.unwrap_or(Color::TRANSPARENT),
                    20.0,
                )];
                if uno {
                    capa = capa.push(
                        container(Self::caja(
                            text("1")
                                .size(8.0)
                                .font(control::peso(Weight::Bold))
                                .color(p.medio)
                                .into(),
                            11.0,
                            11.0,
                            p.tinta,
                            5.5,
                        ))
                        .padding(iced_core::Padding {
                            top: 6.0,
                            right: 6.0,
                            ..Default::default()
                        })
                        .width(Length::Fixed(40.0))
                        .align_x(Horizontal::Right),
                    );
                }
                capa.into()
            };
        let mando = |nombre: &'static str, lado: f32, px: f32| -> PanelElement<'static> {
            Self::caja(
                Self::glifo(nombre, px, p.tinta),
                lado,
                lado,
                Color::TRANSPARENT,
                lado / 2.0,
            )
        };
        let hueco = |w: f32| Space::new().width(Length::Fixed(w));
        let repetir = self.repetir.map(|r| r != medios::Repetir::No);
        let uno = self.repetir == Some(medios::Repetir::Cancion);
        let mandos = row![
            lateral("aleatorio", self.aleatorio, false),
            hueco(22.0),
            mando("anterior", 48.0, 30.0),
            hueco(22.0),
            mando(
                if reproduciendo { "pausa" } else { "reproducir" },
                GRANDE_REPRODUCIR,
                38.0
            ),
            hueco(22.0),
            mando("siguiente", 48.0, 30.0),
            hueco(22.0),
            lateral("repetir", repetir, uno),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fixed(GRANDE_REPRODUCIR));
        let ocupado = g.tiempos + TIEMPOS + g.detalle_alto;
        cuerpo = cuerpo
            .push(Space::new().height(Length::Fixed(g.mandos - ocupado)))
            .push(mandos);

        // La tarjeta: degradado opaco de arriba abajo, sin borde.
        let degradado = iced_core::gradient::Linear::new(iced_core::Radians(std::f32::consts::PI))
            .add_stop(0.0, p.arriba)
            .add_stop(1.0, p.abajo);
        let tarjeta = container(cuerpo)
            .padding(MARGEN)
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(g.alto))
            .clip(true);
        let mut capas = iced_widget::stack![tarjeta];
        if self.menu.valor() > 0.01 {
            capas = capas.push(self.menu_salidas(&p));
        }
        container(capas)
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(g.alto))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(degradado.into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// La lista de salidas, colgada bajo su botón. Popover del HIG: radio 18,
    /// sombra de nivel 2 y el material de la tarjeta, un punto más claro.
    // añadido en la 0.2.2
    fn menu_salidas(&self, p: &Paleta) -> PanelElement<'static> {
        let r = self.rect_menu();
        let mut filas = column![];
        for o in &self.salidas {
            let (fondo, tinta) = if o.actual {
                (p.tinta, p.medio)
            } else {
                (p.placa, p.tinta)
            };
            let mut fila = row![
                Self::caja(Self::glifo(o.glifo, 15.0, tinta), 30.0, 30.0, fondo, 15.0),
                column![
                    text(recortar_px(&o.titulo, MENU_ANCHO - 90.0, 13.0))
                        .size(13.0)
                        .font(control::peso(Weight::Semibold))
                        .color(p.tinta)
                        .wrapping(iced_core::text::Wrapping::None),
                    text(recortar_px(&o.via, MENU_ANCHO - 90.0, 11.0))
                        .size(11.0)
                        .color(p.tinta2)
                        .wrapping(iced_core::text::Wrapping::None),
                ]
                .width(Length::Fill),
            ]
            .spacing(10.0)
            .align_y(Vertical::Center)
            .padding([7.0, 8.0])
            .height(Length::Fixed(MENU_FILA));
            if o.actual {
                fila = fila.push(Self::glifo("comprobado", 14.0, p.tinta));
            }
            filas = filas.push(fila);
        }
        // Crece desde el botón: la placa y su sombra miden lo desplegado y las
        // filas se descubren recortadas, sin encogerse.
        let alto = r.height * self.menu.valor().clamp(0.0, 1.0);
        let menu = p.menu;
        let menu = container(crate::recorte::Recorte::new(
            container(filas).padding(MENU_RELLENO),
            r.width,
            alto,
            0.0,
        ))
        .width(Length::Fixed(r.width))
        .height(Length::Fixed(alto))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(menu.into()),
            border: Border {
                radius: tema::R_POPOVER.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                offset: Vector::new(0.0, 6.0),
                blur_radius: 22.0,
            },
            ..Default::default()
        });
        column![
            Space::new().height(Length::Fixed(r.y)),
            row![Space::new().width(Length::Fixed(r.x)), menu],
        ]
        .into()
    }

    // 0.1.0
    // modified 0.2.0 -- rejilla de 4×4 y tarjetas de Pantalla y Sonido
    pub fn view(&self) -> PanelElement<'_> {
        if let Some(d) = &self.detalle {
            return control::tarjeta(
                container(d.contenido()).padding([0.0, DESVIO]).into(),
                ANCHO,
                MARGEN,
            );
        }
        if self.musica_grande {
            return self.musica_grande();
        }
        let hueco_col = || Space::new().width(Length::Fixed(ENTRE_COLUMNAS));
        let hueco_fila = || Space::new().height(Length::Fixed(ENTRE_FILAS));
        let izquierda = column![self.conexion(false), hueco_fila(), self.conexion(true)];
        let rejilla = column![
            row![izquierda, hueco_col(), self.musica()],
            hueco_fila(),
            row![
                self.circulo_celda(Celda::Noche, "noche", self.noche),
                hueco_col(),
                self.circulo_celda(Celda::Oscuro, "oscuro", !tema::es_claro()),
                hueco_col(),
                self.pildora(
                    Celda::NoMolestar,
                    "No molestar",
                    "notificaciones",
                    if self.no_molestar {
                        "Activado"
                    } else {
                        "Desactivado"
                    }
                    .to_owned(),
                    self.no_molestar,
                ),
            ],
            hueco_fila(),
            row![
                self.pildora(
                    Celda::Microfono,
                    "Micrófono",
                    if self.microfono_mudo {
                        "micro-silencio"
                    } else {
                        "micro"
                    },
                    if self.microfono_mudo {
                        "Silenciado"
                    } else {
                        "Activo"
                    }
                    .to_owned(),
                    self.microfono_mudo,
                ),
                hueco_col(),
                self.circulo_celda(Celda::Captura, "captura", false),
                hueco_col(),
                self.circulo_celda(Celda::Mantener, "mantener", mantener_despierto()),
            ],
        ];
        let contenido = column![
            rejilla,
            Space::new().height(Length::Fixed(HUECO)),
            self.deslizador(Agarre::Brillo),
            Space::new().height(Length::Fixed(HUECO)),
            self.deslizador(Agarre::Volumen),
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Recorta al ancho en píxeles, con la misma medida que las listas.
// 0.1.0
fn recortar_px(texto: &str, ancho: f32, tamaño: f32) -> String {
    super::lista::recortar(texto, ancho, tamaño)
}

// 0.1.0
fn conectado_a() -> Option<String> {
    let s = bookos_system::snapshot();
    s.network["ssid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
// 0.1.0
fn emparejado_con() -> Option<String> {
    let s = bookos_system::snapshot();
    s.bluetooth["devices"]
        .as_array()?
        .iter()
        .find(|d| d["connected"] == true)?["name"]
        .as_str()
        .map(str::to_owned)
}

/// Las salidas de audio para la lista de lo que suena en grande: cada sumidero
/// de PipeWire con un nombre para personas, cómo llega y su glifo.
///
/// Fuera los HDMI sin pantalla (todos sus puertos «not available») y, cuando
/// dos sumideros son lo mismo para quien escucha —los altavoces y su cadena
/// de efectos de BookOS—, se queda el que está en uso.
// añadido en la 0.2.2
fn salidas_de_audio(audio: &serde_json::Value) -> Vec<OpcionSalida> {
    let Some(sinks) = audio["sinks"].as_array() else {
        return Vec::new();
    };
    let mut lista: Vec<OpcionSalida> = Vec::new();
    for s in sinks {
        let nombre = s["name"].as_str().unwrap_or_default();
        let actual = s["isDefault"] == true;
        let sin_puerto = s["ports"].as_array().is_some_and(|p| {
            !p.is_empty() && p.iter().all(|p| p["availability"] == "not available")
        });
        if sin_puerto && !actual {
            continue;
        }
        let bajo = nombre.to_lowercase();
        let puerto = s["activePort"].as_str().unwrap_or_default().to_lowercase();
        let descripcion = s["description"].as_str().unwrap_or(nombre);
        let (titulo, via, glifo) = if let Some(codec) = s["codec"].as_str() {
            // BASC se anuncia a 24 bits por decisión de producto: es la
            // precisión de su codificador de rango, no del audio (ver basc/docs).
            let bits = if codec.eq_ignore_ascii_case("basc") {
                Some(24)
            } else {
                bits_de(s["spec"].as_str().unwrap_or_default())
            };
            let codec = codec.to_uppercase();
            let via = match bits {
                Some(b) => format!("Bluetooth · {codec} · {b} bits"),
                None => format!("Bluetooth · {codec}"),
            };
            let glifo = if descripcion.to_lowercase().contains("buds") {
                "buds"
            } else {
                "casco"
            };
            (descripcion.to_owned(), via, glifo)
        } else if bajo.contains("headphone") || puerto.contains("headphone") {
            (
                "Auriculares con cable".to_owned(),
                "Jack de 3,5 mm".to_owned(),
                "casco",
            )
        } else if bajo.contains("hdmi") {
            (
                "HDMI / DisplayPort".to_owned(),
                "Pantalla externa".to_owned(),
                "volumen-alto",
            )
        } else {
            (
                "Altavoces".to_owned(),
                "Del portátil".to_owned(),
                "volumen-alto",
            )
        };
        match lista.iter_mut().find(|o| o.titulo == titulo) {
            Some(o) if actual => {
                o.sink = nombre.to_owned();
                o.actual = true;
            }
            Some(_) => {}
            None => lista.push(OpcionSalida {
                sink: nombre.to_owned(),
                titulo,
                via,
                glifo,
                actual,
            }),
        }
    }
    lista
}

/// Los bits del formato de un sumidero (`s16le 2ch 48000Hz` → 16).
// añadido en la 0.2.2
fn bits_de(spec: &str) -> Option<u32> {
    let formato = spec.split_whitespace().next()?;
    let cifras: String = formato
        .trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    cifras.parse().ok()
}

/// Una salida de audio que se puede elegir.
// añadido en la 0.2.2
#[derive(Debug, Clone, PartialEq)]
struct OpcionSalida {
    /// El sumidero de PipeWire, para `pactl set-default-sink`.
    sink: String,
    titulo: String,
    /// Cómo llega: «Bluetooth · BASC · 24 bits», «Del portátil»…
    via: String,
    glifo: &'static str,
    actual: bool,
}

/// Los colores de lo que suena en grande; ver [`Centro::paleta_grande`].
// añadido en la 0.2.2
#[derive(Clone, Copy)]
struct Paleta {
    arriba: Color,
    abajo: Color,
    medio: Color,
    tinta: Color,
    tinta2: Color,
    pista: Color,
    placa: Color,
    apagado: Color,
    menu: Color,
}

/// Lo que se lee del reproductor solo al cambiar de pista: su `.desktop` (para
/// el icono), el `mpris:trackid` y si es favorita (`None` sin la interfaz de
/// BookOS Player).
// añadido en la 0.2.2
#[derive(Debug, Clone, Default, PartialEq)]
struct Extras {
    escritorio: Option<String>,
    pista: Option<String>,
    favorito: Option<bool>,
}

impl Extras {
    // añadido en la 0.2.2
    fn leer(bus: &str) -> Self {
        let pista = medios::pista(bus);
        Self {
            escritorio: medios::escritorio(bus),
            favorito: pista.as_deref().and_then(|p| medios::favorito(bus, p)),
            pista,
        }
    }
}

/// El icono de la app por su `.desktop`; si no hay `.desktop`, por el mismo
/// nombre en el tema.
// añadido en la 0.2.2
fn icono_de_escritorio(escritorio: &str) -> Option<crate::icono::Icono> {
    let nombre =
        crate::apps::por_app_id(escritorio).map_or_else(|| escritorio.to_owned(), |a| a.icono);
    crate::icono::cargar(&nombre)
}

/// Las alturas de lo que suena en grande; ver [`Centro::grande`].
struct Grande {
    caratula: f32,
    caratula_w: f32,
    caratula_h: f32,
    /// Portada | Vídeo, solo si la carátula es la de un vídeo.
    selector: Option<f32>,
    textos: f32,
    avance: f32,
    tiempos: f32,
    /// Dónde empieza el hueco del bloque de calidad, si asoma algo.
    detalle: Option<f32>,
    /// Lo que mide ahora ese hueco, separación incluida.
    detalle_alto: f32,
    mandos: f32,
    alto: f32,
}

/// Lo que suena, su carátula ya lista para pintar y el color que la resume.
struct Lectura {
    sonando: Option<Sonando>,
    caratula: Option<iced_widget::image::Handle>,
    tinte: Option<Color>,
    /// Solo la pone el hilo, y solo al cambiar de pista.
    extras: Option<Extras>,
}

type Previa = Option<(String, (iced_widget::image::Handle, Color))>;

impl Lectura {
    /// Descodifica la carátula solo si ha cambiado de fichero: un JPEG por
    /// segundo para la misma canción sería trabajo tirado.
    // 0.1.0
    fn nueva(sonando: Option<Sonando>, previa: Previa) -> Self {
        let ruta = sonando.as_ref().and_then(|s| s.caratula.as_deref());
        let (caratula, tinte) = match (ruta, previa) {
            (Some(r), Some((p, (h, c)))) if r == p => (Some(h), Some(c)),
            (Some(r), _) => match caratula(r) {
                Some((h, c)) => (Some(h), Some(c)),
                None => (None, None),
            },
            (None, _) => (None, None),
        };
        Self {
            sonando,
            caratula,
            tinte,
            extras: None,
        }
    }
}

/// Relee lo que suena cada segundo mientras la tarjeta exista.
///
/// En un hilo porque cada lectura son varios `busctl`, y con el móvil detrás
/// de BookOS Link la respuesta puede tardar: en el hilo del compositor serían
/// tirones. El hilo solo guarda un `Weak`: al cerrarse la tarjeta el `Arc`
/// muere con ella y el hilo termina en la vuelta siguiente, sin avisarle.
/// El compositor recoge la lectura con su propio temporizador de un segundo
/// (`multimedia::vigilar_centro`).
///
/// `previa` es la carátula ya descodificada con su ruta, para reutilizarla.
// 0.1.0
fn vigilar_medios(lectura: &Arc<Mutex<Option<Lectura>>>, mut previa: Previa) {
    let destino = Arc::downgrade(lectura);
    let mut clave_extras: Option<(String, String)> = None;
    let resultado = std::thread::Builder::new()
        .name("bookos-centro-medios".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if destino.strong_count() == 0 {
                    return;
                }
                let sonando = match Sonando::consultar() {
                    Ok(sonando) => sonando,
                    Err(error) => {
                        tracing::debug!("se conserva la tarjeta multimedia: {error}");
                        continue;
                    }
                };
                let mut l = Lectura::nueva(sonando, previa.take());
                // El `.desktop`, la pista y el favorito: tres `busctl` más, así
                // que solo cuando cambian el reproductor o la canción.
                let clave = l
                    .sonando
                    .as_ref()
                    .map(|s| (s.bus.clone(), s.titulo.clone()));
                if clave != clave_extras {
                    l.extras = Some(
                        l.sonando
                            .as_ref()
                            .map(|s| Extras::leer(&s.bus))
                            .unwrap_or_default(),
                    );
                    clave_extras = clave;
                }
                previa = l
                    .sonando
                    .as_ref()
                    .and_then(|s| s.caratula.clone())
                    .zip(l.caratula.clone().zip(l.tinte));
                let Some(destino) = destino.upgrade() else {
                    return;
                };
                *destino.lock().expect("nadie suelta el cerrojo a medias") = Some(l);
            }
        });
    if let Err(err) = resultado {
        tracing::warn!("sin hilo para seguir lo que suena: {err}");
    }
}

// 0.1.0
fn caratula(ruta: &str) -> Option<(iced_widget::image::Handle, Color)> {
    // Tres píxeles por punto: nítida hasta escala 3.
    medios::cargar_caratula(
        ruta,
        (MUSICA_CARATULA * 3.0) as u32,
        tema::R_BOTON_PEQUENO * 3.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apretar y soltar en el mismo sitio, que es lo que hace un clic.
    fn clic(c: &mut Centro, x: f32, y: f32) -> Option<Accion> {
        c.pulsar(x, y).or_else(|| c.soltar().1)
    }

    fn centro_de(celda: Celda) -> (f32, f32) {
        let r = rect_celda(celda);
        (r.center_x(), r.center_y())
    }

    #[test]
    fn la_disposicion_no_se_pisa_y_cabe_en_el_ancho() {
        for (i, a) in DISPOSICION.iter().enumerate() {
            let ra = rect_celda(a.0);
            assert!(
                ra.x + ra.width <= ANCHO - MARGEN + 0.01,
                "{:?} se sale",
                a.0
            );
            for b in &DISPOSICION[i + 1..] {
                assert!(
                    ra.intersection(&rect_celda(b.0)).is_none(),
                    "{:?} pisa a {:?}",
                    a.0,
                    b.0
                );
            }
        }
    }

    #[test]
    fn cada_punto_de_la_rejilla_es_de_su_pieza() {
        let c = Centro::new();
        for d in DISPOSICION {
            let (x, y) = centro_de(d.0);
            assert_eq!(c.celda_en(x, y), Some(d.0));
        }
        assert_eq!(c.celda_en(2.0, 2.0), None, "el margen no es de nadie");
    }

    #[test]
    fn apretar_hunde_y_salirse_antes_de_soltar_lo_anula() {
        let mut c = Centro::new();
        let (x, y) = centro_de(Celda::Noche);
        let antes = c.noche;
        assert_eq!(c.pulsar(x, y), None, "apretar todavía no hace nada");
        assert!(c.animando(), "la pieza empieza a hundirse");
        c.puntero(Some((2.0, 2.0)));
        assert_eq!(c.soltar().1, None, "soltar fuera no hace nada");
        assert_eq!(c.noche, antes);
    }

    #[test]
    fn los_deslizadores_no_se_pisan() {
        let mut c = Centro::new();
        let r = c.rect_pista(Agarre::Brillo);
        c.pulsar(r.center_x(), r.center_y());
        assert_eq!(c.agarre, Agarre::Brillo);
        c.soltar();
        let r = c.rect_pista(Agarre::Volumen);
        c.pulsar(r.center_x(), r.center_y());
        assert_eq!(c.agarre, Agarre::Volumen);
    }

    #[test]
    fn el_texto_de_la_pildora_abre_su_lista_y_el_disco_no() {
        let mut c = Centro::new();
        c.wifi_disponible = true;
        c.bluetooth_disponible = true;
        let r = rect_celda(Celda::Wifi);
        assert_eq!(clic(&mut c, r.x + r.width - 10.0, r.center_y()), None);
        assert!(matches!(c.detalle.as_deref(), Some(Detalle::Red(_))));
        c.tecla(crate::TeclaPulsada::Escape);
        assert!(c.detalle.is_none(), "Esc vuelve a la estación");
        let antes = c.wifi;
        assert_eq!(clic(&mut c, r.x + 20.0, r.center_y()), None);
        assert!(c.detalle.is_none(), "el disco no abre la lista");
        assert_eq!(
            c.wifi, antes,
            "el estado solo cambia al confirmarlo el servicio"
        );
    }

    #[test]
    fn mantener_el_disco_y_soltar_abre_la_lista_en_vez_de_conmutar() {
        let mut c = Centro::new();
        c.wifi_disponible = true;
        let r = rect_celda(Celda::Wifi);
        let (x, y) = (r.x + 20.0, r.center_y());
        c.pulsar(x, y);
        let (zona, px, py, _) = c.apretada.expect("apretado");
        c.apretada = Some((zona, px, py, Instant::now() - PULSACION_LARGA));
        c.soltar();
        assert!(matches!(c.detalle.as_deref(), Some(Detalle::Red(_))));
    }

    #[test]
    fn las_listas_nacen_de_su_pieza_y_vuelven_a_ella() {
        let mut c = Centro::new();
        for celda in [Celda::Wifi, Celda::Bluetooth] {
            c.wifi_disponible = true;
            c.bluetooth_disponible = true;
            let r = rect_celda(celda);
            clic(&mut c, r.x + r.width - 10.0, r.center_y());
            assert_eq!(
                c.tomar_transicion(),
                Some(super::super::Transicion::AbrirDetalle(r))
            );
            c.tecla(crate::TeclaPulsada::Escape);
            assert_eq!(
                c.tomar_transicion(),
                Some(super::super::Transicion::CerrarDetalle(r))
            );
        }
    }

    #[test]
    fn los_conmutadores_piden_su_accion() {
        let mut c = Centro::new();
        let (x, y) = centro_de(Celda::NoMolestar);
        c.poner_no_molestar(false);
        assert_eq!(clic(&mut c, x, y), Some(Accion::NoMolestar(true)));
        assert_eq!(clic(&mut c, x, y), Some(Accion::NoMolestar(false)));
        let (x, y) = centro_de(Celda::Captura);
        assert_eq!(clic(&mut c, x, y), Some(Accion::AbrirCaptura));
        let (x, y) = centro_de(Celda::Noche);
        let antes = c.noche;
        assert_eq!(clic(&mut c, x, y), Some(Accion::LuzNocturna(!antes)));
    }

    #[test]
    fn mantener_despierto_persiste_al_cerrar_la_tarjeta() {
        let mut c = Centro::new();
        let antes = mantener_despierto();
        let (x, y) = centro_de(Celda::Mantener);
        assert_eq!(clic(&mut c, x, y), Some(Accion::MantenerDespierto));
        assert_eq!(mantener_despierto(), !antes);
        drop(c);
        let mut c = Centro::new();
        clic(&mut c, x, y);
        assert_eq!(mantener_despierto(), antes);
    }

    #[test]
    fn sin_musica_la_tarjeta_abre_el_reproductor() {
        let mut c = Centro::new();
        c.sonando = None;
        let (x, y) = centro_de(Celda::Musica);
        assert_eq!(
            clic(&mut c, x, y),
            Some(Accion::Lanzar("bookos-player".into()))
        );
        assert!(!c.musica_grande);
    }

    fn sonando_flac() -> Sonando {
        Sonando {
            bus: "org.mpris.MediaPlayer2.bookosplayer".into(),
            aplicacion: "BookOS Player".into(),
            caratula: None,
            titulo: "Nocturne in E-flat".into(),
            artista: "Chopin".into(),
            reproduciendo: true,
            posicion: Some(84),
            duracion: Some(270),
            calidad: Some(medios::Calidad {
                codec: "FLAC".into(),
                frecuencia: 96_000,
                bits: 24,
                tasa: 0,
            }),
        }
    }

    /// Pausar y que el hilo traiga una lectura de antes del clic: la tarjeta
    /// no puede volver a «sonando» y luego otra vez a pausa.
    #[test]
    fn una_lectura_vieja_no_deshace_la_pausa() {
        let mut c = Centro::new();
        c.sonando = Some(sonando_flac());
        c.alternado = Some((false, std::time::Instant::now()));
        c.sonando.as_mut().expect("puesto arriba").reproduciendo = false;
        let entregar = |c: &mut Centro, suena: bool| {
            let mut s = sonando_flac();
            s.reproduciendo = suena;
            *c.lectura.lock().expect("sin hilos en el test") = Some(Lectura::nueva(Some(s), None));
            c.refrescar();
            c.sonando.as_ref().expect("hay música").reproduciendo
        };
        assert!(!entregar(&mut c, true), "la lectura vieja pisó la pausa");
        assert!(!entregar(&mut c, false));
        assert!(c.alternado.is_none(), "confirmada, ya no se fuerza");
        assert!(
            entregar(&mut c, true),
            "sin orden pendiente manda el reproductor"
        );
    }

    /// Arrastrar la barra hacia atrás más de un segundo: la lectura del hilo
    /// no puede devolverla a donde iba la canción.
    #[test]
    fn la_lectura_no_pisa_la_barra_arrastrada() {
        let mut c = Centro::new();
        c.sonando = Some(sonando_flac());
        c.agarre = Agarre::Posicion;
        c.sonando.as_mut().expect("puesto arriba").posicion = Some(10);
        *c.lectura.lock().expect("sin hilos en el test") =
            Some(Lectura::nueva(Some(sonando_flac()), None));
        c.refrescar();
        assert_eq!(c.sonando.as_ref().expect("hay música").posicion, Some(10));
    }

    fn salidas_buds_basc() -> Vec<OpcionSalida> {
        vec![
            OpcionSalida {
                sink: "bluez_output.buds".into(),
                titulo: "Buds3 Pro".into(),
                via: "Bluetooth · BASC · 24 bits".into(),
                glifo: "buds",
                actual: true,
            },
            OpcionSalida {
                sink: "alsa_output.speaker".into(),
                titulo: "Altavoces".into(),
                via: "Del portátil".into(),
                glifo: "volumen-alto",
                actual: false,
            },
        ]
    }

    /// Vuelca lo que suena en grande a un PNG si se pide por entorno, para
    /// mirarlo sin sesión: `BOOKOS_CENTRO_MUSICA_PNG=/tmp/x.png`.
    fn volcar(c: Centro, sufijo: &str) {
        let Ok(ruta) = std::env::var("BOOKOS_CENTRO_MUSICA_PNG") else {
            return;
        };
        let ruta = ruta.replace(".png", &format!("-{sufijo}.png"));
        // El shell aplica el tema de su configuración: se le pasa el que haya.
        let config = crate::Config {
            tema: tema::actual(),
            ..Default::default()
        };
        let mut shell = crate::Shell::con_config(1646, 1.0, config);
        shell.abrir(super::super::Emergente::Centro(c));
        let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        for p in buf.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
        }
        image::save_buffer(ruta, &buf, w, h, image::ExtendedColorType::Rgba8)
            .expect("se escribe el PNG");
    }

    fn grande_con_flac() -> Centro {
        let mut c = Centro::new();
        c.sonando = Some(sonando_flac());
        c.salidas = salidas_buds_basc();
        c.aleatorio = Some(false);
        c.repetir = Some(medios::Repetir::No);
        c.extras.favorito = Some(false);
        c.extras.pista = Some("/org/bookos/player/track/1".into());
        c.tinte = Some(Color::from_rgb8(0x3b, 0x6f, 0xd8));
        c.musica_grande = true;
        c
    }

    /// El bloque de calidad: el archivo a un lado y la salida al otro, sin
    /// rótulos ni el «Bluetooth ·» que ya dice el icono.
    #[test]
    fn la_calidad_dice_archivo_y_salida() {
        let c = grande_con_flac();
        assert_eq!(c.etiqueta_calidad().as_deref(), Some("Hi-Res Lossless"));
        let [origen, destino] = c.columnas_calidad();
        assert_eq!(origen, ("musica", "FLAC".into(), "24 bits · 96 kHz".into()));
        assert_eq!(
            destino,
            ("buds", "Buds3 Pro".into(), "BASC · 24 bits".into())
        );
    }

    /// Pulsar la etiqueta despliega el bloque, nada se pisa y el panel mide
    /// lo que ocupa.
    #[test]
    fn la_etiqueta_despliega_y_cabe() {
        let mut c = grande_con_flac();
        let r = c.rect_calidad().expect("hay etiqueta");
        assert_eq!(clic(&mut c, r.center_x(), r.center_y()), None);
        assert!(c.calidad_abierta);
        // Al final del muelle: lo de en medio lo mira el test de abajo.
        c.calidad.fijar(1.0);
        let g = c.grande();
        let detalle = g.detalle.expect("desplegado");
        assert!(detalle >= g.tiempos + TIEMPOS, "el bloque pisa los tiempos");
        assert!(
            g.mandos >= detalle + DETALLE_ALTO,
            "los mandos pisan el bloque"
        );
        assert_eq!(c.size().1, g.alto);
        assert!(
            g.mandos + GRANDE_REPRODUCIR + MARGEN <= g.alto + 0.5,
            "los mandos se salen"
        );
        volcar(c, "calidad");
    }

    /// La mitad de arriba de la píldora también despliega: la holgura de la
    /// barra de avance se la quedaba y el clic agarraba la barra.
    #[test]
    fn la_pildora_gana_a_la_holgura_de_la_barra() {
        let mut c = grande_con_flac();
        let r = c.rect_calidad().expect("hay etiqueta");
        assert_eq!(clic(&mut c, r.center_x(), r.y + 2.0), None);
        assert!(c.calidad_abierta, "el clic agarró la barra");
    }

    /// A mitad del muelle el bloque asoma recortado, la carátula va entre sus
    /// dos tamaños y la lista de salidas crece desde el botón.
    #[test]
    fn calidad_y_salidas_se_animan() {
        let mut c = grande_con_flac();
        let r = c.rect_calidad().expect("hay etiqueta");
        clic(&mut c, r.center_x(), r.center_y());
        assert!(c.animando(), "desplegar no anima");
        c.calidad.fijar(0.5);
        let g = c.grande();
        assert!(g.caratula_w < GRANDE_CARATULA && g.caratula_w > GRANDE_CARATULA_PEQUEÑA);
        assert!(g.detalle_alto > 0.0 && g.detalle_alto < 10.0 + DETALLE_ALTO);
        assert_eq!(c.size().1, g.alto, "el lienzo no sigue al alto animado");
        c.poner_menu(true);
        assert!(c.animando(), "la lista no anima");
        c.menu.fijar(0.5);
        volcar(c, "a-medias");
    }

    /// Cada uno de los cinco mandos responde en su sitio; aleatorio y repetir
    /// no hacen nada si el reproductor no los ofrece.
    #[test]
    fn los_mandos_caen_en_su_sitio() {
        let mut c = grande_con_flac();
        let y = c.grande().mandos + GRANDE_REPRODUCIR / 2.0;
        let centro = |i: usize| MARGEN + MANDOS[i].0 + MANDOS[i].1 / 2.0;
        assert_eq!(c.grande_en(centro(0), y), Some(Zona::Aleatorio));
        assert_eq!(
            c.grande_en(centro(1), y),
            Some(Zona::Medio(medios::Orden::Anterior))
        );
        assert_eq!(
            c.grande_en(centro(2), y),
            Some(Zona::Medio(medios::Orden::Alternar))
        );
        assert_eq!(
            c.grande_en(centro(3), y),
            Some(Zona::Medio(medios::Orden::Siguiente))
        );
        assert_eq!(c.grande_en(centro(4), y), Some(Zona::Repetir));
        assert!((MARGEN + MANDOS[4].0 + MANDOS[4].1 - (ANCHO - MARGEN)).abs() < 0.5);
        c.aleatorio = None;
        assert_eq!(c.grande_en(centro(0), y), None);
    }

    /// La lista de salidas: el botón la abre, un clic fuera la cierra sin
    /// cambiar nada, Esc también, y elegir una la marca al momento.
    #[test]
    fn la_lista_de_salidas_se_abre_y_se_cierra() {
        let mut c = grande_con_flac();
        let boton = (
            ANCHO - MARGEN - GRANDE_CABECERA / 2.0,
            MARGEN + GRANDE_CABECERA / 2.0,
        );
        clic(&mut c, boton.0, boton.1);
        assert!(c.menu_salida);
        // Fuera: sobre el título, que debajo no debe recibir el clic.
        let g = c.grande();
        clic(&mut c, MARGEN + 10.0, g.textos + 10.0);
        assert!(!c.menu_salida);
        assert!(c.salidas[0].actual, "cerrar fuera no cambia la salida");
        clic(&mut c, boton.0, boton.1);
        assert!(matches!(
            c.tecla(crate::TeclaPulsada::Escape),
            Tecla::Consumida
        ));
        assert!(
            !c.menu_salida && c.musica_grande,
            "Esc cierra la lista, no la vista"
        );
        clic(&mut c, boton.0, boton.1);
        volcar(grande_con_flac_abierta(), "menu");
        let r = c.rect_menu();
        clic(&mut c, r.center_x(), r.y + MENU_RELLENO + MENU_FILA * 1.5);
        assert!(!c.menu_salida);
        assert!(c.salidas[1].actual && !c.salidas[0].actual);
    }

    fn grande_con_flac_abierta() -> Centro {
        let mut c = grande_con_flac();
        c.poner_menu(true);
        c.menu.fijar(1.0);
        c
    }

    /// El ♥ solo con la interfaz de favoritos, y pulsarlo lo marca sin esperar.
    #[test]
    fn el_corazon_solo_si_el_reproductor_lo_da() {
        let mut c = grande_con_flac();
        let g = c.grande();
        let (x, y) = (ANCHO - MARGEN - 18.0, g.textos + TEXTOS_ALTO / 2.0);
        assert_eq!(c.grande_en(x, y), Some(Zona::Favorito));
        c.extras.favorito = None;
        assert_eq!(c.grande_en(x, y), None);
        volcar(grande_con_flac(), "normal");
    }

    #[test]
    fn las_salidas_salen_con_nombre_y_bits() {
        let audio = serde_json::json!({
            "sinks": [
                {"name": "effect_input.bookos_audio", "description": "Altavoces (efectos de sonido)", "isDefault": false},
                {"name": "alsa_output.x.HiFi__HDMI1__sink", "description": "HDMI 1", "isDefault": false,
                 "ports": [{"availability": "not available"}]},
                {"name": "alsa_output.x.HiFi__Speaker__sink", "description": "Speaker", "isDefault": true,
                 "ports": [{"availability": "availability unknown"}]},
                {"name": "bluez_output.AA.1", "description": "Buds3 Pro de Evelyn", "codec": "basc",
                 "spec": "float32le 2ch 48000Hz", "isDefault": false},
                {"name": "bluez_output.BB.1", "description": "WH-1000XM5", "codec": "aac",
                 "spec": "s16le 2ch 48000Hz", "isDefault": false}
            ]
        });
        let lista = salidas_de_audio(&audio);
        let resumen: Vec<_> = lista
            .iter()
            .map(|o| (o.titulo.as_str(), o.via.as_str(), o.glifo, o.actual))
            .collect();
        assert_eq!(
            resumen,
            [
                ("Altavoces", "Del portátil", "volumen-alto", true),
                (
                    "Buds3 Pro de Evelyn",
                    "Bluetooth · BASC · 24 bits",
                    "buds",
                    false
                ),
                ("WH-1000XM5", "Bluetooth · AAC · 16 bits", "casco", false),
            ]
        );
        assert_eq!(
            lista[0].sink, "alsa_output.x.HiFi__Speaker__sink",
            "se queda el que está en uso"
        );
    }
}
