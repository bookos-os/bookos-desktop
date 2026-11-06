//! El centro de control, con el aspecto del plasmoide `bookos-controlcenter`.
//!
//! Cinco bloques, de arriba abajo: quién ha iniciado sesión con dos botones al
//! lado, las dos píldoras de conectividad, la rejilla de ocho conmutadores, los
//! deslizadores de volumen y brillo, y lo que esté sonando. Las medidas son las
//! del lienzo «Widgets de BookOS»: 352 de ancho, 16 de margen y 12 entre
//! bloques, con los tres de abajo en grupos grises.
//!
//! **El acento significa «encendido» y nada más.** Un conmutador apagado va en
//! el color de la tarjeta, que dentro de su grupo gris es lo que se despega, con
//! el glifo en el color del texto. Antes iba en acento aclarado, y el resultado se ve en cuanto se
//! mira la rejilla entera: ocho círculos azules de los que ninguno dice si está
//! activo, porque el único color que quedaba libre para decirlo era ese mismo
//! azul. La regla es del sistema de diseño —el acento es lo único que significa
//! «esto está activo»— y aquí no se cumplía.
//!
//! **Cada baldosa lleva su nombre debajo.** Ocho círculos mudos obligan a
//! aprenderse cuál es el ojo y cuál el rombo.
//!
//! **Todo lo que se dibuja hace algo.** Los estados que se pueden leer se leen
//! desde la caché compartida para red, Bluetooth y audio; los que no tienen
//! forma de consultarse son acciones, no interruptores. Mantener la pantalla
//! encendida lo guarda el shell para que dure aunque se cierre la tarjeta.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::bloqueo::Usuario;
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
/// Hueco entre bloques.
const HUECO: f32 = 12.0;
/// Alto de la cabecera y de sus botones redondos.
const CABECERA: f32 = 52.0;
const BOTON_CABECERA: f32 = 40.0;
/// Lado del avatar.
const AVATAR: f32 = 40.0;
/// Alto de las dos píldoras de conectividad.
const CONEXION: f32 = 52.0;
/// Lado del círculo de una píldora de conectividad.
const ICONO_CONEXION: f32 = 40.0;
/// Relleno de la píldora hasta su círculo.
const RELLENO_CONEXION: f32 = 6.0;
/// Rejilla: cuatro columnas por dos filas de botones redondos.
const COLUMNAS: usize = 4;
const BOTON: f32 = 48.0;
/// Relleno del grupo de la rejilla.
const REJILLA_MARGEN: f32 = 14.0;
/// Aire entre las dos filas de la rejilla.
const ENTRE_FILAS: f32 = 14.0;
/// Alto de una celda de la rejilla: solo el círculo, sin rótulo debajo. Con
/// iconos macizos se reconocen solos, y la tarjeta gana 40 px de alto.
const CELDA: f32 = BOTON;
/// Cuánto encoge un botón apretado: lo justo para notarse bajo el dedo sin
/// que el icono parezca cambiar de tamaño.
// añadido en la 0.2.0
const HUNDIDO: f32 = 0.08;
/// Relleno de los grupos de los deslizadores y de lo que suena.
const GRUPO_MARGEN: f32 = 12.0;
/// Aire entre los dos deslizadores.
const ENTRE_DESLIZADORES: f32 = 10.0;
/// Lado de la carátula de lo que suena.
const CARATULA: f32 = 48.0;
/// Alto de la franja de la barra de avance: lo que mide el tirador. La barra
/// era la píldora de 10 px del volumen y se leía como otro deslizador de
/// volumen; ahora es un surco fino con su tirador, que dice «posición».
const AVANCE: f32 = 12.0;
/// Grueso del surco de la barra de avance.
const SURCO_AVANCE: f32 = 4.0;
/// Alto del renglón de los tiempos.
const TIEMPOS: f32 = 14.0;
/// Lado del botón de reproducir.
const REPRODUCIR: f32 = 44.0;
/// Hueco entre los botones de reproducción.
const ENTRE_MANDOS: f32 = 28.0;
/// Alto de lo que suena: carátula y textos, avance, tiempos y mandos, con el
/// relleno del grupo.
const MEDIOS: f32 =
    GRUPO_MARGEN * 2.0 + CARATULA + 10.0 + AVANCE + 6.0 + TIEMPOS + 10.0 + REPRODUCIR;

/// El encendido de un conmutador de la estación: el acento del sistema.
///
/// Era el `#4184FF` del plasmoide, escrito a pelo. Deja de valer desde que el
/// acento se elige: con la constante, poner el escritorio en verde dejaba esta
/// tarjeta —la más azul de todas— en azul, y el apagado `#AECAFF` de al lado,
/// que sí es un azul deliberado, se leía como el activo de otro tema.
// 0.1.0
fn acento_centro() -> Color {
    tema::acento()
}
/// La tinta del icono dentro de un círculo apagado: el texto secundario.
///
/// El mismo gris en los dos temas —lo fija así el sistema de diseño—, y por eso
/// no se calcula contra el fondo como cuando el círculo era de acento: sobre
/// `--surface`, que es casi el fondo, sale igual en claro y en oscuro.
// 0.1.0
fn tinta_apagado() -> Color {
    tema::TEXTO2
}

/// Fondo de las tarjetas interiores: el `--surface` del sistema de diseño.
///
/// Antes era `alfa(card(), 0.80)` en oscuro, o sea el mismo token que pinta el
/// panel que las contiene: `#1c1c1e` al 80 % **sobre** `#1c1c1e` da otra vez
/// `#1c1c1e`. Las cuatro subdivisiones existían en el árbol de widgets y no se
/// veía ninguna. En claro pasaba lo mismo de forma más suave: `bg` al 80 %
/// sobre la tarjeta blanca deja `#f5f5f9`, tres puntos por canal.
// 0.1.0
fn contenedor() -> Color {
    tema::superficie_en_tarjeta()
}

/// Un conmutador o una acción de la rejilla.
struct Baldosa {
    icono: &'static str,
    /// Encendida: círculo en acento sólido. Apagada: color de la tarjeta.
    activa: bool,
    /// Qué ejecuta. `None` = lo resuelve el propio shell.
    accion: Option<Accion>,
    /// Es un conmutador del shell y no una orden suelta.
    interna: Option<Interna>,
}

/// Los conmutadores cuyo estado vive aquí dentro.
#[derive(Clone, Copy, PartialEq)]
enum Interna {
    /// Mantener la pantalla encendida.
    Mantener,
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
}

/// Lo que se puede apretar en la estación: actúa al soltar encima, como un
/// botón de verdad. Al apretar solo se hunde, y salirse antes de soltar lo
/// anula sin que haya pasado nada.
// añadido en la 0.2.0
#[derive(Debug, PartialEq, Clone, Copy)]
enum Zona {
    Baldosa(usize),
    Conexion(bool),
    Cabecera(bool),
    Boton(Agarre),
    Medio(medios::Orden),
}

pub struct Centro {
    usuario: Usuario,
    baldosas: Vec<Baldosa>,
    señalada: tema::Realce,
    wifi: bool,
    /// El servicio puede tardar en aparecer o estar reiniciándose. Se separa
    /// de `wifi` para no pintar «apagado» cuando todavía no hay estado.
    wifi_disponible: bool,
    bluetooth: bool,
    bluetooth_disponible: bool,
    /// La red a la que se está conectado, si hay alguna. La radio encendida no
    /// basta: `nmcli radio wifi` dice `enabled` con la antena buscando y sin
    /// haber llegado a ninguna parte.
    red: Option<String>,
    /// El dispositivo Bluetooth conectado, si hay alguno.
    dispositivo: Option<String>,
    volumen: u8,
    audio_disponible: bool,
    /// El altavoz silenciado. Lo dice el botón de al lado del volumen.
    silenciado: bool,
    brillo: u8,
    /// Si hay sensor de luz en esta máquina. Se mira una vez, como en el
    /// icono del panel: no aparece ni desaparece con la sesión abierta.
    sensor: bool,
    /// Si el brillo de pantalla sigue al sensor ahora mismo. Solo importa con
    /// sensor: sin él, el botón se queda apagado y no hace nada al pulsarlo.
    automatico: bool,
    agarre: Agarre,
    volumen_inicial: u8,
    backlight: Option<(String, u32)>,
    sonando: Option<Sonando>,
    /// La carátula de `sonando`, ya descodificada.
    caratula: Option<iced_widget::image::Handle>,
    /// Donde deja su última lectura el hilo que vigila lo que suena.
    lectura: Arc<Mutex<Option<Lectura>>>,
    /// La lista de Wi-Fi o Bluetooth abierta en el sitio, si la hay. En caja:
    /// una tarjeta de red entera dentro hacía de la estación la variante más
    /// grande de `Emergente`, y todas pagaban ese tamaño.
    detalle: Option<Box<Detalle>>,
    transicion: Option<super::Transicion>,
    origen_detalle: iced_core::Rectangle,
    // añadido en la 0.2.0 -- dónde se apretó, esperando a que se suelte.
    apretada: Option<(Zona, f32, f32)>,
    /// La que se dibuja hundida. Sobrevive a `apretada` para que la vuelta
    /// del muelle se vea después de soltar.
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
        let volumen = if self.agarre == Agarre::Volumen {
            self.volumen
        } else {
            bookos_system::volume(false)
                .map(|v| v.0)
                .unwrap_or(self.volumen)
        };
        let silenciado = bookos_system::volume(false).map_or(self.silenciado, |v| v.1);
        let changed = self.wifi != wifi
            || self.wifi_disponible != wifi_disponible
            || self.bluetooth != bluetooth
            || self.bluetooth_disponible != bluetooth_disponible
            || self.audio_disponible != audio_disponible
            || self.red != red
            || self.dispositivo != dispositivo
            || self.volumen != volumen
            || self.silenciado != silenciado;
        self.wifi = wifi;
        self.wifi_disponible = wifi_disponible;
        self.bluetooth = bluetooth;
        self.bluetooth_disponible = bluetooth_disponible;
        self.audio_disponible = audio_disponible;
        self.red = red;
        self.dispositivo = dispositivo;
        self.volumen = volumen;
        self.silenciado = silenciado;
        let mut changed = changed;
        let lectura = self
            .lectura
            .lock()
            .expect("nadie suelta el cerrojo a medias")
            .take();
        if let Some(l) = lectura {
            changed |= self.sonando != l.sonando || self.caratula != l.caratula;
            self.sonando = l.sonando;
            self.caratula = l.caratula;
        }
        if self.agarre != Agarre::Brillo {
            let backlight = crate::backlight();
            let brillo = crate::brillo_actual().unwrap_or(0);
            // El sensor no se vuelve a mirar: no aparece ni desaparece con la
            // sesión abierta, y es un recorrido de `/sys/bus/iio/devices` que
            // no hace falta pagar en cada refresco del panel.
            let automatico = self.sensor && crate::retroiluminacion::automatico().pantalla;
            changed |= self.backlight != backlight
                || self.brillo != brillo
                || self.automatico != automatico;
            self.backlight = backlight;
            self.brillo = brillo;
            self.automatico = automatico;
        }
        if let Some(tile) = self.baldosas.iter_mut().find(|b| b.icono == "avion") {
            let active = wifi_disponible
                && bluetooth_disponible
                && state.network["enabled"] == false
                && state.bluetooth["enabled"] == false;
            changed |= tile.activa != active;
            tile.activa = active;
        }
        changed
    }

    // 0.1.0
    pub fn new() -> Self {
        let state = bookos_system::snapshot();
        let wifi_disponible = !state.network.is_null();
        let bluetooth_disponible = !state.bluetooth.is_null();
        let audio_disponible = !state.audio.is_null();
        let wifi = wifi_disponible && state.network["enabled"] == true;
        let bluetooth = bluetooth_disponible && state.bluetooth["enabled"] == true;
        let red = wifi_disponible.then(conectado_a).flatten();
        let dispositivo = bluetooth_disponible.then(emparejado_con).flatten();
        let sensor = crate::retroiluminacion::hay_sensor_luz();
        let mut c = Self {
            // El centro de control no ve la configuración: la foto de `panel.conf`
            // la resuelve el bloqueo, y aquí basta con las de siempre.
            usuario: Usuario::leer(None),
            baldosas: Vec::new(),
            señalada: tema::Realce::nuevo(),
            wifi,
            wifi_disponible,
            bluetooth,
            bluetooth_disponible,
            red,
            dispositivo,
            // El nivel de partida viene de la caché compartida.
            // medidos, pero esto ocurre al abrir la tarjeta y no en el refresco
            // del panel.
            volumen: crate::widgets::volumen::consultar("@DEFAULT_AUDIO_SINK@")
                .map(|(n, _)| n)
                .unwrap_or(0),
            audio_disponible,
            silenciado: bookos_system::volume(false).is_some_and(|v| v.1),
            brillo: crate::brillo_actual().unwrap_or(0),
            sensor,
            automatico: sensor && crate::retroiluminacion::automatico().pantalla,
            agarre: Agarre::Nada,
            volumen_inicial: 0,
            backlight: crate::backlight(),
            sonando: None,
            caratula: None,
            lectura: Arc::default(),
            detalle: None,
            transicion: None,
            origen_detalle: iced_core::Rectangle::default(),
            apretada: None,
            hundida: None,
            hundido: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            puntero_en: None,
        };
        // La primera lectura en el sitio: si fuera por el hilo, la tarjeta
        // abriría sin la parte de la música y crecería un segundo después.
        let primera = Lectura::nueva(Sonando::leer(), None);
        c.sonando = primera.sonando;
        c.caratula = primera.caratula;
        let previa = c
            .sonando
            .as_ref()
            .and_then(|s| s.caratula.clone())
            .zip(c.caratula.clone());
        vigilar_medios(&c.lectura, previa);
        c.baldosas = baldosas();
        c
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        if let Some(d) = &self.detalle {
            return (ANCHO, d.alto());
        }
        let mut alto = MARGEN * 2.0
            + CABECERA
            + HUECO
            + CONEXION
            + HUECO
            + self.alto_rejilla()
            + HUECO
            + self.alto_deslizadores();
        if self.sonando.is_some() {
            alto += HUECO + MEDIOS;
        }
        (ANCHO, alto)
    }

    // 0.1.0
    fn alto_rejilla(&self) -> f32 {
        let filas = self.baldosas.len().div_ceil(COLUMNAS) as f32;
        REJILLA_MARGEN * 2.0 + filas * CELDA + (filas - 1.0).max(0.0) * ENTRE_FILAS
    }

    // 0.1.0
    fn alto_deslizadores(&self) -> f32 {
        GRUPO_MARGEN * 2.0 + control::FILA_PILDORA * 2.0 + ENTRE_DESLIZADORES
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("control")
    }

    // 0.1.0
    fn y_conexiones(&self) -> f32 {
        MARGEN + CABECERA + HUECO
    }

    // 0.1.0
    fn y_rejilla(&self) -> f32 {
        self.y_conexiones() + CONEXION + HUECO
    }

    // 0.1.0
    fn y_deslizadores(&self) -> f32 {
        self.y_rejilla() + self.alto_rejilla() + HUECO
    }

    // 0.1.0
    fn y_medios(&self) -> f32 {
        self.y_deslizadores() + self.alto_deslizadores() + HUECO
    }

    /// Qué baldosa cae en un punto.
    // 0.1.0
    fn baldosa_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = MARGEN + REJILLA_MARGEN;
        let y0 = self.y_rejilla() + REJILLA_MARGEN;
        let ancho_celda = (ANCHO - x0 * 2.0) / COLUMNAS as f32;
        if x < x0 || y < y0 {
            return None;
        }
        let columna = ((x - x0) / ancho_celda) as usize;
        // El aire entre filas va con la de arriba: al apuntar con prisa se cae
        // por debajo del círculo, y no hay nada ahí que pudiera querer el clic.
        let fila = ((y - y0) / (CELDA + ENTRE_FILAS)) as usize;
        if columna >= COLUMNAS {
            return None;
        }
        let i = fila * COLUMNAS + columna;
        (i < self.baldosas.len()).then_some(i)
    }

    /// Cuál de las dos píldoras de conectividad cae en un punto: `false` la del
    /// Wi-Fi, `true` la del Bluetooth.
    // 0.1.0
    fn conexion_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_conexiones();
        if y < y0 || y > y0 + CONEXION || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        Some(x > ANCHO / 2.0)
    }

    /// Los dos botones redondos de la cabecera: `false` Preferencias, `true`
    /// apagar.
    // 0.1.0
    fn cabecera_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = MARGEN + (CABECERA - BOTON_CABECERA) / 2.0;
        if y < y0 || y > y0 + BOTON_CABECERA {
            return None;
        }
        let apagar = ANCHO - MARGEN - BOTON_CABECERA;
        let preferencias = apagar - 8.0 - BOTON_CABECERA;
        match x {
            x if x >= apagar && x <= apagar + BOTON_CABECERA => Some(true),
            x if x >= preferencias && x <= preferencias + BOTON_CABECERA => Some(false),
            _ => None,
        }
    }

    /// El ancho de la fila de un deslizador, dentro de su grupo.
    // 0.1.0
    fn ancho_fila(&self) -> f32 {
        ANCHO - (MARGEN + GRUPO_MARGEN) * 2.0
    }

    // 0.1.0
    fn ancho_pildora(&self) -> f32 {
        control::ancho_pildora(self.ancho_fila(), true)
    }

    // 0.1.0
    fn rect_pildora(&self, cual: Agarre) -> iced_core::Rectangle {
        let y0 = self.y_deslizadores() + GRUPO_MARGEN;
        let y = match cual {
            Agarre::Brillo => y0 + control::FILA_PILDORA + ENTRE_DESLIZADORES,
            _ => y0,
        };
        iced_core::Rectangle {
            x: MARGEN + GRUPO_MARGEN + control::ICONO + control::HUECO,
            y: y + (control::FILA_PILDORA - control::PILDORA) / 2.0,
            width: self.ancho_pildora(),
            height: control::PILDORA,
        }
    }

    /// El botón redondo a la derecha de una píldora.
    // 0.1.0
    fn rect_boton(&self, cual: Agarre) -> iced_core::Rectangle {
        let p = self.rect_pildora(cual);
        iced_core::Rectangle {
            x: p.x + p.width + control::HUECO,
            y: p.y - (control::FILA_PILDORA - control::PILDORA) / 2.0,
            width: control::BOTON,
            height: control::BOTON,
        }
    }

    /// Dónde está la barra de avance, con la holgura vertical que hace falta
    /// para acertarle a un surco de 4 px.
    // 0.1.1
    fn avance_en(&self, x: f32, y: f32) -> Option<f32> {
        let y0 = self.y_medios() + GRUPO_MARGEN + CARATULA + 10.0;
        let x0 = MARGEN + GRUPO_MARGEN;
        let ancho = ANCHO - x0 * 2.0;
        (y >= y0 - 6.0 && y <= y0 + AVANCE + 6.0 && x >= x0 - 4.0 && x <= x0 + ancho + 4.0)
            .then(|| ((x - x0) / ancho).clamp(0.0, 1.0))
    }

    /// Los tres mandos de lo que suena, si está.
    // 0.1.0
    fn medio_en(&self, x: f32, y: f32) -> Option<medios::Orden> {
        self.sonando.as_ref()?;
        let y0 = self.y_medios() + MEDIOS - GRUPO_MARGEN - REPRODUCIR;
        if y < y0 || y > y0 + REPRODUCIR {
            return None;
        }
        let centro = ANCHO / 2.0;
        let mitad = REPRODUCIR / 2.0;
        let alcance = mitad + ENTRE_MANDOS + 22.0;
        Some(match x {
            x if x < centro - alcance || x > centro + alcance => return None,
            x if x < centro - mitad => medios::Orden::Anterior,
            x if x > centro + mitad => medios::Orden::Siguiente,
            _ => medios::Orden::Alternar,
        })
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.puntero_en = punto;
        // Salirse con el botón apretado devuelve la baldosa a su sitio, y
        // volver a entrar la hunde otra vez: así se ve si soltar hará algo.
        if let Some((zona, ..)) = self.apretada {
            let encima = punto.and_then(|(x, y)| self.zona_en(x, y)) == Some(zona);
            self.hundido.ir_a(encima as u8 as f32);
        }
        if let Some(d) = self.detalle.as_mut() {
            return d.puntero(punto);
        }
        if let Some((x, _)) = punto {
            let r = self.rect_pildora(Agarre::Volumen);
            let nivel = control::nivel_en(x, r.x, r.width);
            match self.agarre {
                Agarre::Volumen => return self.poner_volumen(nivel),
                Agarre::Brillo => return self.poner_brillo(nivel),
                Agarre::Nada => {}
            }
        }
        let señalada = punto.and_then(|(x, y)| self.baldosa_en(x, y));
        self.señalada.señalar(señalada)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando()
            || self.hundido.animando()
            || self.detalle.as_ref().is_some_and(|d| d.animando())
    }

    // 0.1.0
    pub fn agarrado(&self) -> bool {
        self.agarre != Agarre::Nada
    }

    // 0.1.0
    // modified 0.2.0 -- lo apretado actúa aquí, si se suelta encima
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        if let Some((zona, x, y)) = self.apretada.take() {
            self.hundido.ir_a(0.0);
            let encima = self.puntero_en.and_then(|(px, py)| self.zona_en(px, py)) == Some(zona);
            return (true, if encima { self.ejecutar(x, y) } else { None });
        }
        (self.soltar_agarre(), None)
    }

    // 0.1.0
    fn soltar_agarre(&mut self) -> bool {
        let agarre = std::mem::replace(&mut self.agarre, Agarre::Nada);
        if agarre == Agarre::Volumen && self.volumen != self.volumen_inicial {
            bookos_system::request(bookos_system::Operation::Volume {
                target: "output".into(),
                value: self.volumen as u32,
            });
        }
        agarre != Agarre::Nada
    }

    // añadido en la 0.2.0 -- las baldosas se confirman al soltar; solo las píldoras admiten arrastre.
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        self.detalle.is_none()
            && [Agarre::Volumen, Agarre::Brillo].into_iter().any(|cual| {
                let mut zona = self.rect_pildora(cual);
                zona.y -= control::MARGEN_AGARRE;
                zona.height += control::MARGEN_AGARRE * 2.0;
                zona.contains(iced_core::Point::new(x, y))
            })
    }

    /// Qué botón cae en un punto, en el mismo orden en que `ejecutar` los
    /// mira. Las píldoras y la barra de avance no: se arrastran y responden
    /// desde el primer momento.
    // añadido en la 0.2.0
    fn zona_en(&self, x: f32, y: f32) -> Option<Zona> {
        let punto = iced_core::Point::new(x, y);
        if self.rect_boton(Agarre::Volumen).contains(punto) {
            return Some(Zona::Boton(Agarre::Volumen));
        }
        if self.sensor && self.rect_boton(Agarre::Brillo).contains(punto) {
            return Some(Zona::Boton(Agarre::Brillo));
        }
        if let Some(apagar) = self.cabecera_en(x, y) {
            return Some(Zona::Cabecera(apagar));
        }
        if self.avance_en(x, y).is_some() {
            return None;
        }
        if let Some(orden) = self.medio_en(x, y) {
            return Some(Zona::Medio(orden));
        }
        if let Some(bluetooth) = self.conexion_en(x, y) {
            return Some(Zona::Conexion(bluetooth));
        }
        self.baldosa_en(x, y).map(Zona::Baldosa)
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

    /// La rueda solo mueve algo con una lista de Wi-Fi o Bluetooth abierta.
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
            self.apretada = Some((zona, x, y));
            self.hundida = Some(zona);
            self.puntero_en = Some((x, y));
            self.hundido.ir_a(1.0);
            return None;
        }
        self.ejecutar(x, y)
    }

    // 0.1.0
    // modified 0.1.1 -- pulsar la barra de avance salta en la canción
    fn ejecutar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(d) = self.detalle.as_mut() {
            if lista::volver_en(x - DESVIO, y) {
                self.volver();
                return None;
            }
            return d.pulsar(x, y);
        }
        let punto = iced_core::Point::new(x, y);
        let holgada = |mut r: iced_core::Rectangle| {
            r.y -= control::MARGEN_AGARRE;
            r.height += control::MARGEN_AGARRE * 2.0;
            r
        };
        let r = self.rect_pildora(Agarre::Volumen);
        let nivel = control::nivel_en(x, r.x, r.width);
        if self.rect_boton(Agarre::Volumen).contains(punto) {
            if self.audio_disponible {
                self.silenciado = !self.silenciado;
                bookos_system::request(bookos_system::Operation::Mute {
                    target: "output".into(),
                    muted: Some(self.silenciado),
                });
            }
            return None;
        }
        // El botón del brillo pone o quita el automático, igual que el del
        // volumen silencia: los dos actúan en el sitio, sin abrir nada más. El
        // teclado se queda sin sitio aquí, pero sigue a un clic del icono del
        // panel, que abre la tarjeta entera; la luz nocturna tiene su baldosa.
        if self.sensor && self.rect_boton(Agarre::Brillo).contains(punto) {
            self.automatico = !self.automatico;
            let mut elegido = crate::retroiluminacion::automatico();
            elegido.pantalla = self.automatico;
            return Some(Accion::BrilloAutomatico(elegido));
        }
        if let Some(apagar) = self.cabecera_en(x, y) {
            return Some(if apagar {
                Accion::Emergente("apagar")
            } else {
                Accion::Lanzar("bookos-settings".into())
            });
        }
        if holgada(self.rect_pildora(Agarre::Volumen)).contains(punto) {
            self.volumen_inicial = self.volumen;
            self.agarre = Agarre::Volumen;
            self.poner_volumen(nivel);
            return None;
        }
        if holgada(self.rect_pildora(Agarre::Brillo)).contains(punto) {
            self.agarre = Agarre::Brillo;
            self.poner_brillo(nivel);
            return None;
        }
        if let Some(fraccion) = self.avance_en(x, y)
            && let Some(sonando) = self.sonando.as_mut()
            && let (Some(posicion), Some(duracion)) = (sonando.posicion, sonando.duracion)
        {
            let destino = (fraccion * duracion as f32).round() as u64;
            if medios::saltar_en(&sonando.bus, destino as i64 - posicion as i64) {
                // Como el play: sin esperar la lectura de vuelta, que con el
                // móvil tarda hasta un segundo y el tirador parecería no moverse.
                sonando.posicion = Some(destino);
            }
            return None;
        }
        if let Some(orden) = self.medio_en(x, y) {
            // Directo al reproductor y sin `Accion`: `Lanzar` es para abrir
            // aplicaciones y cierra la tarjeta, y aquí el usuario quiere seguir
            // viendo lo que suena.
            if let Some(sonando) = self.sonando.as_mut()
                && sonando.ejecutar(orden)
                && orden == medios::Orden::Alternar
            {
                // Sin esperar a leer el estado de vuelta: con el móvil detrás
                // de BookOS Link la respuesta tarda y el botón parecería muerto.
                sonando.reproduciendo = !sonando.reproduciendo;
            }
            return None;
        }
        if let Some(bluetooth) = self.conexion_en(x, y) {
            // El círculo enciende y apaga la radio; el resto de la tarjeta abre
            // la lista. Es lo que hace el plasmoide, y resuelve las dos cosas
            // que se piden de un vistazo: apagar el Wi-Fi rápido, o elegir a
            // qué red conectarse.
            let inicio = if bluetooth { ANCHO / 2.0 + 4.0 } else { MARGEN };
            let disponible = if bluetooth {
                self.bluetooth_disponible
            } else {
                self.wifi_disponible
            };
            if x < inicio + RELLENO_CONEXION + ICONO_CONEXION {
                if !disponible {
                    return None;
                }
                let encendido = if bluetooth { self.bluetooth } else { self.wifi };
                bookos_system::request(if bluetooth {
                    bookos_system::Operation::BluetoothPower {
                        enabled: !encendido,
                    }
                } else {
                    bookos_system::Operation::WifiPower {
                        enabled: !encendido,
                    }
                });
                return None;
            }
            // Sin servicio no hay lista que enseñar: la tarjeta diría
            // «Cargando…» para siempre.
            if disponible {
                self.detalle = Some(Box::new(if bluetooth {
                    Detalle::Bluetooth(Bluetooth::new())
                } else {
                    Detalle::Red(Red::new())
                }));
                self.señalada.señalar(None);
                self.origen_detalle = iced_core::Rectangle {
                    x: inicio,
                    y: self.y_conexiones(),
                    width: (ANCHO - MARGEN * 2.0 - 8.0) / 2.0,
                    height: CONEXION,
                };
                self.animar(super::Transicion::AbrirDetalle(self.origen_detalle));
            }
            return None;
        }
        let i = self.baldosa_en(x, y)?;
        if let Some(interna) = self.baldosas[i].interna {
            return Some(self.alternar(interna, i));
        }
        if self.baldosas[i].icono == "notificaciones" {
            // «No molestar» vive en su propia tarjeta, que es donde están las
            // duraciones; desde aquí solo se abre.
            return Some(Accion::Emergente("notificaciones"));
        }
        if self.baldosas[i].icono == "noche" {
            let encender = !self.baldosas[i].activa;
            self.baldosas[i].activa = encender;
            return Some(Accion::LuzNocturna(encender));
        }
        if self.baldosas[i].icono == "avion" {
            bookos_system::request(bookos_system::Operation::Airplane {
                enabled: !self.baldosas[i].activa,
            });
            return None;
        }
        if self.baldosas[i].icono == "ahorro" {
            self.baldosas[i].activa = !self.baldosas[i].activa;
            return Some(Accion::PerfilEnergia(if self.baldosas[i].activa {
                "power-saver"
            } else {
                "balanced"
            }));
        }
        self.baldosas[i].accion.clone()
    }

    /// «No molestar» lo guarda el shell, que es quien lo sabe; la tarjeta solo
    /// lo enseña. Ver `Shell::abrir`.
    // 0.1.0
    pub fn poner_no_molestar(&mut self, activo: bool) {
        if let Some(b) = self
            .baldosas
            .iter_mut()
            .find(|b| b.icono == "notificaciones")
        {
            b.activa = activo;
        }
    }

    // 0.1.0
    fn alternar(&mut self, cual: Interna, i: usize) -> Accion {
        match cual {
            Interna::Mantener => {
                let activo = alternar_mantener_despierto();
                self.baldosas[i].activa = activo;
                Accion::MantenerDespierto
            }
        }
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
            // Dentro de una lista, Esc vuelve a la estación, como la flecha.
            crate::TeclaPulsada::Escape if self.detalle.is_some() => {
                self.volver();
                Tecla::Consumida
            }
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            // Dentro de una lista, el resto de teclas son de la lista.
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

    // 0.1.0
    pub fn tomar_transicion(&mut self) -> Option<super::Transicion> {
        self.transicion.take()
    }

    /// De la lista a la estación. Se relee al volver: mientras estaba en la
    /// lista no se refrescaba, y la píldora diría la red de antes.
    // 0.1.0
    fn volver(&mut self) {
        self.detalle = None;
        self.animar(super::Transicion::CerrarDetalle(self.origen_detalle));
        self.red = self.wifi_disponible.then(conectado_a).flatten();
        self.dispositivo = self.bluetooth_disponible.then(emparejado_con).flatten();
        self.refrescar();
    }

    // --- Dibujo ------------------------------------------------------------

    /// Un círculo con un icono dentro, que es la forma de casi todo aquí.
    // 0.1.0
    // modified 0.1.1 -- iconos sólidos
    fn circulo(
        nombre: &str,
        lado: f32,
        icono_px: f32,
        fondo: Color,
        tinta: Color,
    ) -> PanelElement<'static> {
        let dibujo: PanelElement<'static> = match icono::propio_solido(nombre) {
            Some(ic) => icono::ver_teñido_propio(&ic, icono_px, tinta),
            None => Space::new().width(Length::Fixed(icono_px)).into(),
        };
        container(dibujo)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: (lado / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    // 0.1.0
    fn cabecera(&self) -> PanelElement<'_> {
        let avatar: PanelElement<'_> = match &self.usuario.foto {
            Some(handle) => iced_widget::image(handle.clone())
                .width(Length::Fixed(AVATAR))
                .height(Length::Fixed(AVATAR))
                .content_fit(iced_core::ContentFit::Cover)
                .into(),
            None => text(
                self.usuario
                    .nombre
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
            )
            .size(16.0)
            .font(control::peso(Weight::Semibold))
            .color(tema::sobre_acento())
            .into(),
        };
        let redondo = container(avatar)
            .width(Length::Fixed(AVATAR))
            .height(Length::Fixed(AVATAR))
            .center_x(Length::Fixed(AVATAR))
            .center_y(Length::Fixed(AVATAR))
            .clip(true)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(acento_centro().into()),
                border: Border {
                    radius: (AVATAR / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let pildora = container(
            row![
                column![
                    text(self.usuario.nombre.clone())
                        .size(17.0)
                        .font(control::peso(Weight::Bold))
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
                    text(format!("@{}", self.usuario.cuenta))
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                redondo,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(
            ANCHO - MARGEN * 2.0 - (BOTON_CABECERA + 8.0) * 2.0,
        ))
        .height(Length::Fixed(CABECERA))
        .padding(iced_core::Padding {
            top: 6.0,
            right: 6.0,
            bottom: 6.0,
            left: 16.0,
        })
        .center_y(Length::Fixed(CABECERA))
        .clip(true)
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(contenedor().into()),
            border: Border {
                radius: (CABECERA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        // Apagar va el último y con el glifo en rojo. Es la única acción de la
        // tarjeta que se lleva la sesión por delante: la esquina y el color son
        // lo que impide darle queriendo abrir Preferencias.
        row![
            pildora,
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo(
                "preferencias",
                BOTON_CABECERA,
                18.0,
                contenedor(),
                tema::texto()
            ),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("apagar", BOTON_CABECERA, 18.0, contenedor(), tema::rojo()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Una de las dos píldoras de conectividad.
    // 0.1.0
    // modified 0.1.1 -- fondo elevado en vez de card
    fn conexion(&self, bluetooth: bool) -> PanelElement<'_> {
        // El círculo va en acento cuando hay **conexión**, no cuando la radio
        // está encendida. Es la misma regla que la rejilla: el acento dice que
        // algo está funcionando. Con la antena encendida y sin red, la píldora
        // se queda apagada y el subtítulo dice «No conectado», que es lo que
        // hay que ver cuando algo no va.
        let disponible = if bluetooth {
            self.bluetooth_disponible
        } else {
            self.wifi_disponible
        };
        let (nombre, encendido, conectado, icono) = if bluetooth {
            (
                "Bluetooth",
                self.bluetooth,
                self.dispositivo.as_deref(),
                if self.bluetooth && disponible {
                    "bluetooth"
                } else {
                    "bluetooth-apagado"
                },
            )
        } else {
            (
                "Wi-Fi",
                self.wifi,
                self.red.as_deref(),
                if self.wifi && disponible {
                    "wifi"
                } else {
                    "sin-red"
                },
            )
        };
        // El subtítulo se corta a lo que cabe en una línea: la píldora tiene el
        // alto fijo y una segunda línea se saldría por abajo.
        let estado = if !disponible {
            "No disponible".to_owned()
        } else {
            match (encendido, conectado) {
                (_, Some(donde)) => recortar(donde, 14),
                (true, None) => "No conectado".to_owned(),
                (false, None) => "Desactivado".to_owned(),
            }
        };
        let (fondo, tinta) = if conectado.is_some() {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::elevado(), tinta_apagado())
        };
        let h = self.hundimiento(Zona::Conexion(bluetooth));
        let fondo = tema::mezclar(fondo, Color::BLACK, 0.12 * h);
        let lado = ICONO_CONEXION * (1.0 - HUNDIDO * h);
        let caja = tema::mezclar(contenedor(), Color::BLACK, 0.08 * h);
        container(
            row![
                container(Self::circulo(icono, lado, 18.0, fondo, tinta))
                    .width(Length::Fixed(ICONO_CONEXION))
                    .center_x(Length::Fixed(ICONO_CONEXION)),
                Space::new().width(Length::Fixed(10.0)),
                column![
                    text(nombre)
                        .size(14.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
                    // Sin envolver: el SSID recortado sigue teniendo espacios
                    // y iced parte por ellos antes que desbordar.
                    text(estado)
                        .size(11.0)
                        .color(tema::TEXTO2)
                        .wrapping(iced_core::text::Wrapping::None),
                ],
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed((ANCHO - MARGEN * 2.0 - 8.0) / 2.0))
        .height(Length::Fixed(CONEXION))
        .padding(iced_core::Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: RELLENO_CONEXION,
        })
        .center_y(Length::Fixed(CONEXION))
        .clip(true)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(caja.into()),
            border: Border {
                radius: (CONEXION / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    // 0.1.0
    // modified 0.1.1 -- baldosas solo con el círculo, sin rótulo, sobre fondo elevado
    fn baldosa(&self, i: usize) -> PanelElement<'_> {
        let baldosa = &self.baldosas[i];
        // Lo que cambia entre encendida y apagada es el círculo de detrás, no
        // el icono: acento contra el color de la tarjeta, que dentro del grupo
        // gris es lo que se despega. Señalada, se mezcla un poco hacia la
        // tinta, interpolado y no conmutado: la rejilla tiene ocho baldosas
        // juntas y un salto se lee como un parpadeo al cruzarla con el ratón.
        let (base, tinta) = if baldosa.activa {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::elevado(), tema::texto())
        };
        let fondo = tema::mezclar(base, tema::tinta(), 0.08 * self.señalada.intensidad(i));
        // Hundida encoge y se oscurece un poco, como un botón que cede bajo
        // el dedo. El hueco de la celda no cambia: los vecinos no se mueven.
        let h = self.hundimiento(Zona::Baldosa(i));
        let fondo = tema::mezclar(fondo, Color::BLACK, 0.12 * h);
        let lado = BOTON * (1.0 - HUNDIDO * h);
        let celda = (ANCHO - (MARGEN + REJILLA_MARGEN) * 2.0) / COLUMNAS as f32;
        container(Self::circulo(
            baldosa.icono,
            lado,
            20.0 * (1.0 - HUNDIDO * h),
            fondo,
            tinta,
        ))
        .width(Length::Fixed(celda))
        .height(Length::Fixed(BOTON))
        .center_x(Length::Fixed(celda))
        .center_y(Length::Fixed(BOTON))
        .into()
    }

    // 0.1.0
    // modified 0.1.1 -- iconos sólidos y fondo elevado
    fn deslizador(&self, cual: Agarre) -> PanelElement<'_> {
        let (nivel, icono, boton, encendido, apagado) = match cual {
            Agarre::Brillo => (
                self.brillo,
                "brillo",
                // El icono del botón dice si el automático está puesto, igual
                // que el del panel: un sol con «A» en vez del sol pelado.
                if self.automatico {
                    "brillo-automatico"
                } else {
                    "brillo"
                },
                self.sensor && self.automatico,
                self.backlight.is_none(),
            ),
            _ => (
                self.volumen,
                "volumen-alto",
                if self.silenciado || self.volumen == 0 {
                    "volumen-silencio"
                } else {
                    match self.volumen {
                        1..=33 => "volumen-bajo",
                        34..=66 => "volumen-medio",
                        _ => "volumen-alto",
                    }
                },
                !self.silenciado && self.audio_disponible,
                self.silenciado,
            ),
        };
        let dibujo = |nombre: &str, px: f32, color: Color| -> PanelElement<'static> {
            match icono::propio_solido(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, px, color),
                None => Space::new().width(Length::Fixed(px)).into(),
            }
        };
        // El botón del volumen silencia y va en acento mientras suena; el del
        // brillo pone o quita el automático y va en acento mientras está
        // puesto. Los dos dicen «esto está activo» con el mismo lenguaje.
        let (fondo, tinta) = if encendido {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::elevado(), tema::TEXTO2)
        };
        row![
            dibujo(icono, control::ICONO, tema::texto()),
            Space::new().width(Length::Fixed(control::HUECO)),
            control::pildora(
                self.ancho_pildora(),
                control::PILDORA,
                nivel,
                apagado,
                tema::elevado()
            ),
            Space::new().width(Length::Fixed(control::HUECO)),
            Self::circulo(boton, control::BOTON, 16.0, fondo, tinta),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fixed(control::FILA_PILDORA))
        .into()
    }

    // 0.1.0
    // modified 0.1.1 -- iconos sólidos y barra de avance que se puede pulsar
    fn medios(&self, sonando: &Sonando) -> PanelElement<'_> {
        let ancho = ANCHO - (MARGEN + GRUPO_MARGEN) * 2.0;
        let ancho_texto = ancho - CARATULA - 12.0;
        let caratula = container(match (&self.caratula, icono::propio_solido("musica")) {
            (Some(foto), _) => iced_widget::image(foto.clone())
                .width(Length::Fixed(CARATULA))
                .height(Length::Fixed(CARATULA))
                .into(),
            (None, Some(ic)) => icono::ver_teñido_propio(&ic, 22.0, tema::TEXTO2),
            (None, None) => Space::new().width(Length::Fixed(22.0)).into(),
        })
        .width(Length::Fixed(CARATULA))
        .height(Length::Fixed(CARATULA))
        .center_x(Length::Fixed(CARATULA))
        .center_y(Length::Fixed(CARATULA))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let textos = column![
            // Sin «Reproduciendo en»: con BookOS Link el nombre ya es largo
            // («BitChord - Galaxy S22 Ultra») y el prefijo cortaba el móvil.
            text(recortar_px(&sonando.aplicacion, ancho_texto, 11.0))
                .size(11.0)
                .color(tema::TEXTO2),
            text(recortar_px(&sonando.titulo, ancho_texto, 14.0))
                .size(14.0)
                .font(control::peso(Weight::Medium))
                .color(tema::texto()),
            text(recortar_px(&sonando.artista, ancho_texto, 11.0))
                .size(11.0)
                .color(tema::TEXTO2),
        ];
        let mando = |nombre: &str| -> PanelElement<'static> {
            match icono::propio_solido(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, 22.0, tema::texto()),
                None => Space::new().width(Length::Fixed(22.0)).into(),
            }
        };
        let contenido = column![
            row![caratula, Space::new().width(Length::Fixed(12.0)), textos]
                .align_y(Vertical::Center)
                .height(Length::Fixed(CARATULA)),
            Space::new().height(Length::Fixed(10.0)),
            barra_avance(ancho, sonando.avance()),
            Space::new().height(Length::Fixed(6.0)),
            row![
                text(medios::reloj(sonando.posicion.unwrap_or(0)))
                    .size(11.0)
                    .color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                text(
                    sonando
                        .duracion
                        .map(medios::reloj)
                        .unwrap_or_else(|| "--:--".into())
                )
                .size(11.0)
                .color(tema::TEXTO2),
            ]
            .height(Length::Fixed(TIEMPOS)),
            Space::new().height(Length::Fixed(10.0)),
            container(
                row![
                    mando("anterior"),
                    Space::new().width(Length::Fixed(ENTRE_MANDOS)),
                    Self::circulo(
                        if sonando.reproduciendo {
                            "pausa"
                        } else {
                            "reproducir"
                        },
                        REPRODUCIR,
                        20.0,
                        acento_centro(),
                        tema::sobre_acento(),
                    ),
                    Space::new().width(Length::Fixed(ENTRE_MANDOS)),
                    mando("siguiente"),
                ]
                .align_y(Vertical::Center)
            )
            .width(Length::Fill)
            .height(Length::Fixed(REPRODUCIR))
            .align_x(Horizontal::Center),
        ];
        Self::grupo(contenido.into(), MEDIOS, GRUPO_MARGEN)
    }

    /// Un grupo gris con su alto fijo: el alto es lo que usan las zonas de
    /// clic, así que se fija en vez de dejarlo al contenido.
    // 0.1.0
    fn grupo<'a>(contenido: PanelElement<'a>, alto: f32, relleno: f32) -> PanelElement<'a> {
        container(control::grupo(contenido, ANCHO - MARGEN * 2.0, relleno))
            .height(Length::Fixed(alto))
            .into()
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        if let Some(d) = &self.detalle {
            // El margen de la estación más el desvío: la lista queda centrada.
            return control::tarjeta(
                container(d.contenido()).padding([0.0, DESVIO]).into(),
                ANCHO,
                MARGEN,
            );
        }
        let mut rejilla = column![].spacing(ENTRE_FILAS);
        for fila in 0..self.baldosas.len().div_ceil(COLUMNAS) {
            let mut linea = row![];
            for columna in 0..COLUMNAS {
                let i = fila * COLUMNAS + columna;
                if i < self.baldosas.len() {
                    linea = linea.push(self.baldosa(i));
                }
            }
            rejilla = rejilla.push(linea);
        }

        let mut contenido = column![
            self.cabecera(),
            Space::new().height(Length::Fixed(HUECO)),
            row![
                self.conexion(false),
                Space::new().width(Length::Fixed(8.0)),
                self.conexion(true),
            ],
            Space::new().height(Length::Fixed(HUECO)),
            Self::grupo(rejilla.into(), self.alto_rejilla(), REJILLA_MARGEN),
            Space::new().height(Length::Fixed(HUECO)),
            Self::grupo(
                column![
                    self.deslizador(Agarre::Volumen),
                    Space::new().height(Length::Fixed(ENTRE_DESLIZADORES)),
                    self.deslizador(Agarre::Brillo),
                ]
                .into(),
                self.alto_deslizadores(),
                GRUPO_MARGEN,
            ),
        ];
        if let Some(sonando) = &self.sonando {
            contenido = contenido
                .push(Space::new().height(Length::Fixed(HUECO)))
                .push(self.medios(sonando));
        }
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Las ocho baldosas de la rejilla, en el orden del plasmoide.
// 0.1.0
// modified 0.1.1 -- sin rótulos en las baldosas
fn baldosas() -> Vec<Baldosa> {
    // `rfkill list` dice si hay algo bloqueado por software; el modo avión es
    // exactamente eso.
    let state = bookos_system::snapshot();
    let avion = !state.network.is_null()
        && !state.bluetooth.is_null()
        && state.network["enabled"] == false
        && state.bluetooth["enabled"] == false;
    let ahorro = super::energia::perfil_activo().as_deref() == Some("power-saver");
    vec![
        Baldosa {
            icono: "avion",
            activa: avion,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "notificaciones",
            activa: false,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "ahorro",
            activa: ahorro,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "mantener",
            activa: mantener_despierto(),
            accion: None,
            interna: Some(Interna::Mantener),
        },
        Baldosa {
            icono: "noche",
            activa: crate::luz_nocturna_encendida(),
            // La acción se arma al pulsar: lleva el estado al que se va, y
            // uno calculado aquí se quedaría viejo al segundo toque.
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "compartir",
            activa: false,
            accion: Some(Accion::Lanzar("bookos-share || kdeconnect-app".into())),
            interna: None,
        },
        Baldosa {
            icono: "captura",
            activa: false,
            accion: Some(Accion::AbrirCaptura),
            interna: None,
        },
        Baldosa {
            icono: "bloquear",
            activa: false,
            accion: Some(Accion::Bloquear),
            interna: None,
        },
    ]
}

/// Recorta al ancho en píxeles, con la misma medida que las listas.
// 0.1.0
fn recortar_px(texto: &str, ancho: f32, tamaño: f32) -> String {
    super::lista::recortar(texto, ancho, tamaño)
}

/// Corta un nombre a `max` caracteres y le pone puntos suspensivos.
///
/// Por caracteres y no por píxeles: medir el texto pide el `Renderer`, que aquí
/// no está, y el error de contar caracteres solo se nota con un SSID lleno de
/// emes. El corte es por `char` y no por byte para no partir una letra acentuada
/// en dos.
// 0.1.0
fn recortar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_owned();
    }
    texto.chars().take(max - 1).collect::<String>() + "…"
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

/// Descodifica la carátula, recortada al cuadrado central y con las esquinas
/// del hueco ya redondeadas.
///
/// El redondeo va en los píxeles porque el renderizador de CPU de iced ignora
/// el radio de las imágenes. Solo al cambiar de carátula, no por frame.
/// Lo que suena y su carátula ya lista para pintar.
struct Lectura {
    sonando: Option<Sonando>,
    caratula: Option<iced_widget::image::Handle>,
}

impl Lectura {
    /// Descodifica la carátula solo si ha cambiado de fichero. `previa` es la
    /// de la lectura anterior, que se reutiliza tal cual: un JPEG por segundo
    /// para la misma canción sería trabajo tirado.
    // 0.1.0
    fn nueva(
        sonando: Option<Sonando>,
        previa: Option<(String, iced_widget::image::Handle)>,
    ) -> Self {
        let ruta = sonando.as_ref().and_then(|s| s.caratula.as_deref());
        let caratula = match (ruta, previa) {
            (Some(r), Some((p, h))) if r == p => Some(h),
            (Some(r), _) => caratula(r),
            (None, _) => None,
        };
        Self { sonando, caratula }
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
fn vigilar_medios(
    lectura: &Arc<Mutex<Option<Lectura>>>,
    mut previa: Option<(String, iced_widget::image::Handle)>,
) {
    let destino = Arc::downgrade(lectura);
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
                let l = Lectura::nueva(sonando, previa.take());
                previa = l
                    .sonando
                    .as_ref()
                    .and_then(|s| s.caratula.clone())
                    .zip(l.caratula.clone());
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

/// El surco fino con lo reproducido en acento y un tirador al final.
///
/// Sin duración —el móvil a veces no la manda, y en una radio no la hay— queda
/// el surco vacío y sin tirador: un tirador clavado al principio dice «0:00»,
/// y eso es mentira.
// 0.1.1
fn barra_avance<'a>(ancho: f32, avance: Option<f32>) -> PanelElement<'a> {
    let pildora = |ancho: f32, alto: f32, color: Color| {
        container(Space::new())
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: (alto / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
    };
    let surco = container(pildora(ancho, SURCO_AVANCE, tema::elevado()))
        .height(Length::Fixed(AVANCE))
        .center_y(Length::Fixed(AVANCE));
    let Some(avance) = avance else {
        return surco.into();
    };
    // El tirador se mueve entre sus dos medios lados y no entre los bordes:
    // así a 0 % y a 100 % no se sale del grupo.
    let x = (ancho - AVANCE) * avance;
    let lleno = container(pildora(x + AVANCE / 2.0, SURCO_AVANCE, tema::acento()))
        .height(Length::Fixed(AVANCE))
        .center_y(Length::Fixed(AVANCE));
    let tirador = row![
        Space::new().width(Length::Fixed(x)),
        pildora(AVANCE, AVANCE, tema::acento()),
    ];
    iced_widget::stack![surco, lleno, tirador].into()
}

// 0.1.0
fn caratula(ruta: &str) -> Option<iced_widget::image::Handle> {
    // Tres píxeles por punto: nítida hasta escala 3 y aun así 144×144.
    medios::cargar_caratula(ruta, (CARATULA * 3.0) as u32, tema::R_BOTON_PEQUENO * 3.0)
        .map(|(h, _)| h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apretar y soltar en el mismo sitio, que es lo que hace un clic.
    fn clic(c: &mut Centro, x: f32, y: f32) -> Option<Accion> {
        c.pulsar(x, y).or_else(|| c.soltar().1)
    }

    #[test]
    fn apretar_hunde_y_salirse_antes_de_soltar_lo_anula() {
        let mut c = Centro::new();
        let i = c
            .baldosas
            .iter()
            .position(|b| b.icono == "ahorro")
            .expect("la estación trae ahorro");
        let celda = (ANCHO - (MARGEN + REJILLA_MARGEN) * 2.0) / COLUMNAS as f32;
        let x = MARGEN + REJILLA_MARGEN + celda * ((i % COLUMNAS) as f32 + 0.5);
        let y = c.y_rejilla()
            + REJILLA_MARGEN
            + (CELDA + ENTRE_FILAS) * (i / COLUMNAS) as f32
            + CELDA / 2.0;
        let antes = c.baldosas[i].activa;
        assert_eq!(c.pulsar(x, y), None, "apretar todavía no hace nada");
        assert_eq!(c.baldosas[i].activa, antes);
        assert!(c.animando(), "la baldosa empieza a hundirse");
        c.puntero(Some((2.0, 2.0)));
        assert_eq!(c.soltar().1, None, "soltar fuera no hace nada");
        assert_eq!(c.baldosas[i].activa, antes);
    }

    /// La rejilla reparte los clics por celdas, cuatro por dos.
    #[test]
    fn cada_celda_de_la_rejilla_es_de_su_baldosa() {
        let c = Centro::new();
        let x0 = MARGEN + REJILLA_MARGEN;
        let y0 = c.y_rejilla() + REJILLA_MARGEN;
        let ancho_celda = (ANCHO - x0 * 2.0) / COLUMNAS as f32;
        assert_eq!(c.baldosa_en(x0 + 5.0, y0 + 5.0), Some(0));
        assert_eq!(
            c.baldosa_en(x0 + ancho_celda * 3.0 + 5.0, y0 + 5.0),
            Some(3)
        );
        assert_eq!(
            c.baldosa_en(x0 + 5.0, y0 + CELDA + ENTRE_FILAS + 5.0),
            Some(4),
            "la segunda fila empieza tras la celda entera, rótulo incluido"
        );
        assert_eq!(
            c.baldosa_en(x0 + 5.0, y0 + BOTON + 12.0),
            Some(0),
            "el rótulo de la primera fila sigue siendo su baldosa"
        );
        assert_eq!(
            c.baldosa_en(2.0, y0 + 5.0),
            None,
            "el margen no es de nadie"
        );
    }

    /// Los dos deslizadores no se pisan: agarrar el de abajo no mueve el de
    /// arriba, que es el fallo que ya tuvo el emergente del brillo.
    #[test]
    fn los_deslizadores_no_se_pisan() {
        let mut c = Centro::new();
        let r = c.rect_pildora(Agarre::Brillo);
        c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(c.agarre, Agarre::Brillo);
        c.soltar();
        let r = c.rect_pildora(Agarre::Volumen);
        c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(c.agarre, Agarre::Volumen);
    }

    /// El botón del brillo pone y quita el automático en el sitio, igual que
    /// el del volumen silencia: sin abrir ninguna otra tarjeta.
    #[test]
    fn el_boton_del_brillo_alterna_el_automatico_en_el_sitio() {
        let mut c = Centro::new();
        c.sensor = true;
        c.automatico = false;
        let r = c.rect_boton(Agarre::Brillo);
        let accion = clic(&mut c, r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(c.automatico, "el botón no puso el automático");
        match accion {
            Some(Accion::BrilloAutomatico(elegido)) => assert!(elegido.pantalla),
            otra => panic!("se esperaba BrilloAutomatico con pantalla=true, llegó {otra:?}"),
        }

        let accion = clic(&mut c, r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(!c.automatico, "el segundo toque no lo quitó");
        match accion {
            Some(Accion::BrilloAutomatico(elegido)) => assert!(!elegido.pantalla),
            otra => panic!("se esperaba BrilloAutomatico con pantalla=false, llegó {otra:?}"),
        }
    }

    /// Sin sensor no hay automático que alternar, así que el botón no toca
    /// nada al pulsarlo.
    #[test]
    fn sin_sensor_el_boton_del_brillo_no_hace_nada() {
        let mut c = Centro::new();
        c.sensor = false;
        c.automatico = false;
        let r = c.rect_boton(Agarre::Brillo);
        clic(&mut c, r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(!c.automatico, "sin sensor no debería haber cambiado nada");
    }

    /// Las dos tarjetas de conectividad abren su lista, cada una la suya, en
    /// el sitio: sin acción, que cerraría la estación.
    #[test]
    fn las_tarjetas_de_conexion_abren_su_lista() {
        let mut c = Centro::new();
        c.wifi_disponible = true;
        c.bluetooth_disponible = true;
        let y = c.y_conexiones() + CONEXION / 2.0;
        // Fuera del círculo: la mitad derecha de cada tarjeta.
        assert_eq!(clic(&mut c, 140.0, y), None);
        assert!(matches!(c.detalle.as_deref(), Some(Detalle::Red(_))));
        c.tecla(crate::TeclaPulsada::Escape);
        assert!(c.detalle.is_none(), "Esc vuelve a la estación");
        // Volver relee el estado real, y en una máquina sin Bluetooth eso
        // deshace lo forzado arriba.
        c.bluetooth_disponible = true;
        assert_eq!(clic(&mut c, ANCHO - 40.0, y), None);
        assert!(matches!(c.detalle.as_deref(), Some(Detalle::Bluetooth(_))));
    }

    /// Y el círculo de esas tarjetas enciende y apaga la radio, sin abrir nada.
    #[test]
    fn el_circulo_de_la_tarjeta_alterna_la_radio() {
        let mut c = Centro::new();
        let y = c.y_conexiones() + CONEXION / 2.0;
        let antes = c.wifi;
        let accion = clic(&mut c, MARGEN + ICONO_CONEXION / 2.0, y);
        assert_eq!(
            c.wifi, antes,
            "el estado solo cambia al confirmarlo el servicio"
        );
        assert_eq!(accion, None);
    }

    #[test]
    fn mantener_despierto_persiste_al_cerrar_la_tarjeta() {
        let mut c = Centro::new();
        let i = c
            .baldosas
            .iter()
            .position(|b| b.interna == Some(Interna::Mantener))
            .expect("la rejilla contiene mantener despierto");
        assert_eq!(c.alternar(Interna::Mantener, i), Accion::MantenerDespierto);
        assert!(mantener_despierto());
        drop(c);
        let mut c = Centro::new();
        assert!(c.baldosas[i].activa);
        c.alternar(Interna::Mantener, i);
        assert!(!mantener_despierto());
        assert!(!c.baldosas[i].activa);
    }
    #[test]
    fn ahorro_alterna_el_perfil_en_cada_pulsacion() {
        let mut c = Centro::new();
        let i = c
            .baldosas
            .iter()
            .position(|b| b.icono == "ahorro")
            .expect("la rejilla contiene ahorro");
        c.baldosas[i].activa = false;
        let x = MARGEN
            + REJILLA_MARGEN
            + 2.5 * (ANCHO - 2.0 * (MARGEN + REJILLA_MARGEN)) / COLUMNAS as f32;
        let y = c.y_rejilla() + REJILLA_MARGEN + BOTON / 2.0;
        for perfil in ["power-saver", "balanced", "power-saver"] {
            assert_eq!(clic(&mut c, x, y), Some(Accion::PerfilEnergia(perfil)));
        }
    }

    #[test]
    fn las_acciones_no_se_quedan_encendidas() {
        let mut c = Centro::new();
        let ancho = (ANCHO - 2.0 * (MARGEN + REJILLA_MARGEN)) / COLUMNAS as f32;
        for (columna, accion) in [
            (1, Accion::Lanzar("bookos-share || kdeconnect-app".into())),
            (2, Accion::AbrirCaptura),
            (3, Accion::Bloquear),
        ] {
            let x = MARGEN + REJILLA_MARGEN + (columna as f32 + 0.5) * ancho;
            let y = c.y_rejilla() + REJILLA_MARGEN + CELDA + ENTRE_FILAS + BOTON / 2.0;
            assert_eq!(clic(&mut c, x, y), Some(accion));
            assert!(!c.baldosas[COLUMNAS + columna].activa);
        }
    }

    #[test]
    fn las_listas_nacen_de_su_tarjeta_y_vuelven_a_ella() {
        let mut c = Centro::new();
        for (x, inicio) in [(140.0, MARGEN), (ANCHO - 40.0, ANCHO / 2.0 + 4.0)] {
            c.wifi_disponible = true;
            c.bluetooth_disponible = true;
            let y = c.y_conexiones() + CONEXION / 2.0;
            clic(&mut c, x, y);
            let Some(super::super::Transicion::AbrirDetalle(origen)) = c.tomar_transicion() else {
                panic!("la lista no pidió la animación desde su tarjeta");
            };
            assert_eq!(origen.x, inicio);
            assert_eq!(origen.y, c.y_conexiones());
            assert!(origen.contains(iced_core::Point::new(x, origen.center_y())));
            c.tecla(crate::TeclaPulsada::Escape);
            assert_eq!(
                c.tomar_transicion(),
                Some(super::super::Transicion::CerrarDetalle(origen))
            );
            assert!(c.tomar_transicion().is_none());
        }
    }
}
