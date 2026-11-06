//! Los tokens visuales de BookOS: colores, radios y movimiento.
//!
//! No son inventados aquí. La fuente es `BookOS-HIG/AI-DESIGN-SYSTEM.md`, el
//! mismo documento del que salen los widgets de Plasma y el resto de apps del
//! ecosistema, para que el escritorio propio y los plasmoides se vean como la
//! misma cosa. **Si algo no está en ese documento, no se escribe aquí**: la
//! forma de estropear esto no es cambiar un token, es poner un `radius: 4.0`
//! suelto en un widget porque en ese sitio quedaba bien.
//!
//! Las dos secciones que este fichero refleja literalmente son la tabla de
//! radios (§2.3) y la de movimiento (§2.7), las dos cerradas: cada duración va
//! con **su** curva y no son intercambiables.
//!
//! ## Los dos temas
//!
//! Los colores que cambian entre claro y oscuro son **funciones**, no
//! constantes: `tema::texto()`, `tema::card()`. Lo que no cambia —los radios,
//! las curvas, los tamaños de letra y el gris secundario, que el sistema de
//! diseño fija igual en los dos— sigue siendo `const`, y así el compilador
//! separa una cosa de la otra sin tener que recordarlo.
//!
//! Cuál está puesto lo dice un entero global. Es un dato del proceso y no un
//! parámetro que se arrastre por las cincuenta funciones de dibujo: el shell
//! entero se pinta con el mismo tema y cambiarlo repinta todo, así que pasarlo
//! de mano en mano sería ceremonia sin nadie a quien servir.
//!
//! ## El acento se elige
//!
//! El sistema de diseño fija un acento —el azul— y aquí hay diez, uno a la vez,
//! el que diga `acento` en `panel.conf` o la tarjeta de Apariencia. Sigue
//! siendo un solo color de acción, que es lo que el documento pide; lo que
//! cambia es que ya no está clavado en el código. La tabla y el criterio con
//! el que se eligió cada par claro/oscuro están en [`Acento`].
//!
//! Lo que **no** se deriva del acento son los colores de estado: el verde de
//! «conectado» y el rojo de «batería baja» significan eso y no el gusto de
//! nadie.
//!
//! Desviaciones conscientes:
//!
//! - **El acento azul del tema claro es el `#007AFF` del sistema de diseño**,
//!   no el `#5C95FF` de la paleta de BookOS: ese está elegido para vibrar sobre
//!   negro y sobre una tarjeta blanca se queda en un azul lavado que no llega
//!   al contraste de un texto de acción.
//! - **Los colores de estado sí cambian**, aunque la paleta de BookOS no traiga
//!   variante clara: los suyos están elegidos para brillar sobre negro y como
//!   texto sobre una tarjeta blanca no se leen. En claro se usan los de la
//!   sección 2.1 del sistema de diseño.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use iced_core::Color;

/// Cuál de los dos temas se está pintando.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tema {
    #[default]
    Oscuro,
    Claro,
}

/// Lo que el usuario ha **elegido**, que no siempre es un tema concreto.
///
/// `Automatico` es un tercer estado y no un booleano suelto por lo de siempre:
/// con `Tema` + una bandera «sigue la hora» se pueden escribir combinaciones que
/// no significan nada, y alguien acabaría preguntando por la bandera sin mirar
/// el tema. Aquí solo hay tres casos y el `match` los cubre.
///
/// El tema **efectivo** —el que se pinta— sale de [`ModoTema::resolver`], y en
/// automático cambia solo a lo largo del día.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModoTema {
    Claro,
    #[default]
    Oscuro,
    Automatico,
}

/// Una hora del reloj de pared: `(hora, minuto)`.
pub type HoraDelDia = (u8, u8);

impl ModoTema {
    /// Qué tema toca a esa hora.
    ///
    /// Es pura y toma la hora en vez de leer el reloj para poder probarla: con
    /// `SystemTime::now()` dentro, un test tendría que esperar a las ocho de la
    /// tarde para comprobar el caso interesante.
    ///
    /// El tramo claro va de `claro_desde` (incluida) a `oscuro_desde`
    /// (excluida). Se admite que el claro empiece **después** del oscuro —quien
    /// trabaja de noche y quiere el tema claro de madrugada—, y entonces el
    /// tramo cruza la medianoche; de ahí que la comparación no sea un simple
    /// `a <= x && x < b`.
    // 0.1.0
    pub fn resolver(
        self,
        claro_desde: HoraDelDia,
        oscuro_desde: HoraDelDia,
        ahora: HoraDelDia,
    ) -> Tema {
        match self {
            Self::Claro => Tema::Claro,
            Self::Oscuro => Tema::Oscuro,
            Self::Automatico => {
                if en_tramo(claro_desde, oscuro_desde, ahora) {
                    Tema::Claro
                } else {
                    Tema::Oscuro
                }
            }
        }
    }

    /// Cuántos minutos faltan para el próximo cambio de tema.
    ///
    /// `None` cuando no va a haber ninguno: los modos fijos, y el automático con
    /// los dos extremos iguales. Sirve para programar **un** despertar en el
    /// instante justo en vez de sondear el reloj, que es lo que este proyecto no
    /// hace: un temporizador cada minuto para mirar si ya son las ocho son mil
    /// cuatrocientos despertares al día para dos cambios.
    ///
    /// Nunca devuelve cero: si ahora mismo es la hora del cambio, el siguiente
    /// es el otro, no este otra vez.
    // 0.1.0
    pub fn minutos_al_cambio(
        self,
        claro_desde: HoraDelDia,
        oscuro_desde: HoraDelDia,
        ahora: HoraDelDia,
    ) -> Option<u32> {
        if !matches!(self, Self::Automatico) {
            return None;
        }
        minutos_al_borde(claro_desde, oscuro_desde, ahora)
    }
}

/// ¿Cae `ahora` en el tramo que va de `desde` (incluida) a `hasta` (excluida)?
///
/// El tramo puede cruzar la medianoche —`desde` después de `hasta`—, y de ahí
/// que la comparación no sea un simple `a <= x && x < b`. Con los dos extremos
/// iguales no hay tramo: sería un día entero de cada cosa a la vez.
///
/// Lo comparten el tema automático y el horario de la luz nocturna.
// 0.1.0
pub fn en_tramo(desde: HoraDelDia, hasta: HoraDelDia, ahora: HoraDelDia) -> bool {
    let m = |(h, min): HoraDelDia| h as u32 * 60 + min as u32;
    let (a, b, x) = (m(desde), m(hasta), m(ahora));
    if a == b {
        false
    } else if a < b {
        x >= a && x < b
    } else {
        x >= a || x < b
    }
}

/// Minutos hasta el próximo de los dos extremos del tramo. `None` si son
/// iguales, que es no tener tramo. Nunca cero: si ahora es uno de los bordes,
/// el siguiente es el otro.
// 0.1.0
pub fn minutos_al_borde(desde: HoraDelDia, hasta: HoraDelDia, ahora: HoraDelDia) -> Option<u32> {
    let m = |(h, min): HoraDelDia| h as u32 * 60 + min as u32;
    let (a, b, x) = (m(desde), m(hasta), m(ahora));
    if a == b {
        return None;
    }
    // El primero de los dos que caiga por delante, dando la vuelta al día.
    let falta = |objetivo: u32| match objetivo.checked_sub(x) {
        Some(0) | None => objetivo + 24 * 60 - x,
        Some(d) => d,
    };
    Some(falta(a).min(falta(b)))
}

static ACTUAL: AtomicU8 = AtomicU8::new(0);

/// Lo elegido, global del proceso por lo mismo que el tema y el acento: lo
/// pregunta la tarjeta de Apariencia, que se construye desde una tabla de
/// punteros a función y no recibe la configuración.
static MODO: AtomicU8 = AtomicU8::new(1);

/// Guarda lo que el usuario eligió. **No** aplica ningún tema: el que toca sale
/// de [`ModoTema::resolver`] y lo pone [`aplicar`].
// 0.1.0
pub fn aplicar_modo(modo: ModoTema) {
    MODO.store(
        match modo {
            ModoTema::Claro => 0,
            ModoTema::Oscuro => 1,
            ModoTema::Automatico => 2,
        },
        Ordering::Relaxed,
    );
}

// 0.1.0
pub fn modo_actual() -> ModoTema {
    match MODO.load(Ordering::Relaxed) {
        0 => ModoTema::Claro,
        2 => ModoTema::Automatico,
        _ => ModoTema::Oscuro,
    }
}

/// Pone el tema del proceso. Quien lo llame tiene que repintar: los colores ya
/// dibujados no se enteran.
// 0.1.0
pub fn aplicar(tema: Tema) {
    ACTUAL.store(matches!(tema, Tema::Claro) as u8, Ordering::Relaxed);
}

// 0.1.0
pub fn actual() -> Tema {
    if ACTUAL.load(Ordering::Relaxed) == 1 {
        Tema::Claro
    } else {
        Tema::Oscuro
    }
}

// 0.1.0
pub fn es_claro() -> bool {
    actual() == Tema::Claro
}

/// Efectos reducidos: sin desenfoque y sin animaciones de ventana.
///
/// Global del proceso como el tema, y por lo mismo: lo pregunta código que se
/// ejecuta en medio de un dibujo, donde no llega ninguna configuración. Lo
/// enciende la configuración del usuario y también, solo, la batería baja —una
/// animación de ventana son 260 ms de GPU y de repintados que en el 15 % de
/// batería no compensan.
static REDUCIDOS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ALTO_CONTRASTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Devuelve si el valor **cambió**, que es lo que decide si hay que repintar.
// 0.1.0
pub fn aplicar_efectos_reducidos(reducidos: bool) -> bool {
    REDUCIDOS.swap(reducidos, Ordering::Relaxed) != reducidos
}

// 0.1.0
pub fn efectos_reducidos() -> bool {
    REDUCIDOS.load(Ordering::Relaxed)
}

/// Activa la paleta de accesibilidad. Devuelve si el valor cambió para que el
/// compositor pueda invalidar todas las superficies en caliente.
// 0.1.0
pub fn aplicar_alto_contraste(activo: bool) -> bool {
    ALTO_CONTRASTE.swap(activo, Ordering::Relaxed) != activo
}

// 0.1.0
pub fn alto_contraste() -> bool {
    ALTO_CONTRASTE.load(Ordering::Relaxed)
}

// 0.1.0
fn token(nombre: &str, claro: Color, oscuro: Color) -> Color {
    let normal = if es_claro() { claro } else { oscuro };
    if !alto_contraste() {
        return normal;
    }
    match nombre {
        "bg" => {
            if es_claro() {
                Color::WHITE
            } else {
                Color::BLACK
            }
        }
        "card" => {
            if es_claro() {
                Color::WHITE
            } else {
                hex(0x101010)
            }
        }
        "texto" => {
            if es_claro() {
                Color::BLACK
            } else {
                Color::WHITE
            }
        }
        "divisor" => hexa(if es_claro() { 0x000000 } else { 0xffffff }, 0.42),
        "hover" => hexa(if es_claro() { 0x000000 } else { 0xffffff }, 0.16),
        "surco" => hexa(if es_claro() { 0x000000 } else { 0xffffff }, 0.30),
        "control_apagado" => {
            if es_claro() {
                hex(0xb8b8b8)
            } else {
                hex(0x5a5a5a)
            }
        }
        "borde" => hexa(if es_claro() { 0x000000 } else { 0xffffff }, 0.55),
        "panel" => hex(if es_claro() { 0xffffff } else { 0x000000 }),
        _ => normal,
    }
}

/// Declara un color del tema como función, con su valor en cada uno.
///
/// Existe para que la paleta se lea como una tabla de dos columnas —que es como
/// viene en el sistema de diseño— en vez de como veinte `if` iguales.
macro_rules! tokens {
    ($($(#[$att:meta])* $nombre:ident: claro $claro:expr, oscuro $oscuro:expr;)*) => {
        $(
            $(#[$att])*
            pub fn $nombre() -> Color {
                token(stringify!($nombre), $claro, $oscuro)
            }
        )*
    };
}

/// Un color de `palette.css`/`palette.json`: `#rrggbb` o `rgba(r, g, b, a)`.
// 0.1.0
// modificado en la 0.2.0 -- sale del buscador: la isla también lee la paleta
pub fn color_css(valor: &str) -> Option<Color> {
    if let Some(hex) = valor.strip_prefix('#') {
        return (hex.len() == 6)
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .map(self::hex);
    }
    let rgba = valor.strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut canales = rgba.split(',').map(str::trim);
    let mut canal = || canales.next()?.parse::<u8>().ok().map(|n| n as f32 / 255.0);
    let (r, g, b) = (canal()?, canal()?, canal()?);
    let a = canales.next()?.parse::<f32>().ok()?;
    if canales.next().is_some() || !(0.0..=1.0).contains(&a) {
        return None;
    }
    Some(Color::from_rgba(r, g, b, a))
}

/// Roles de la paleta que Ajustes deriva del fondo de pantalla, del tema que se
/// está pintando ahora (`palette.json`, claves `light`/`dark`).
///
/// Se lee del disco en cada llamada: Ajustes la reescribe al cambiar de fondo y
/// nadie avisa. Quien la llame lo hace al recibir estado, no al dibujar cada
/// fotograma. `None` si no hay paleta, falta un rol o va en alto contraste:
/// quien llama decide su respaldo.
// añadido en la 0.2.0
pub fn roles_paleta<const N: usize>(nombres: [&str; N]) -> Option<[Color; N]> {
    if alto_contraste() {
        return None;
    }
    let json = std::fs::read_to_string(
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config/bookos/palette.json"),
    )
    .ok()?;
    let paleta: serde_json::Value = serde_json::from_str(&json).ok()?;
    let roles = paleta.get(if es_claro() { "light" } else { "dark" })?;
    let mut colores = [Color::BLACK; N];
    for (color, nombre) in colores.iter_mut().zip(nombres) {
        *color = color_css(roles.get(nombre)?.as_str()?)?;
    }
    Some(colores)
}

/// De `#rrggbb` a color. En tiempo de compilación, para poder escribir los
/// tokens con el mismo hex que el sistema de diseño y no con decimales que ya
/// nadie sabe de dónde salen.
///
/// Es pública para los pocos colores que **no** son tokens y viven fuera de
/// aquí con su motivo escrito —el surco del OSD, el campo del bloqueo—: en
/// decimales nadie los reconoce y el comentario acaba diciendo otro valor que
/// el código. Pasó: el surco decía `#c7c7cc` y era `#c6c6cc`.
// 0.1.0
pub const fn hex(v: u32) -> Color {
    Color::from_rgb(
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
    )
}

// 0.1.0
pub const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

// --- Colores ---------------------------------------------------------------

tokens! {
    /// Fondo del escritorio y de las superficies a pantalla completa.
    bg: claro hex(0xf2f2f7), oscuro hex(0x000000);
    /// Tarjetas y popups.
    card: claro hex(0xffffff), oscuro hex(0x1c1c1e);
    texto: claro hex(0x000000), oscuro hex(0xffffff);
}

// --- El acento -------------------------------------------------------------
//
// El sistema de diseño fija **un** acento, el azul, y dice además que es el
// único color que significa «esto se toca». Los diez de aquí no lo contradicen:
// siguen siendo un solo acento a la vez, el que el usuario elija en Apariencia.
// La tabla es una extensión declarada en este fichero —el documento no la
// trae— y por eso cada par se elige con el mismo criterio que ya se le aplicó
// al azul: el de claro tiene que leerse sobre la tarjeta blanca, el de oscuro
// tiene que no vibrar sobre el negro.
//
// Los que ya existían como token de estado —rojo, naranja, amarillo, verde— son
// literalmente los mismos valores: dos amarillos casi iguales en el mismo
// escritorio es lo que hace que se note cosido a mano.

/// El color elegido: una muestra fija o el dominante del fondo, cuyo contraste
/// se ajusta al tema antes de usarlo en los controles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Acento {
    #[default]
    Azul,
    Indigo,
    Morado,
    Rosa,
    Rojo,
    Naranja,
    Amarillo,
    Arena,
    Verde,
    Turquesa,
    Grafito,
    Automatico,
}

impl Acento {
    pub const FIJOS: [Acento; 11] = [
        Acento::Azul,
        Acento::Indigo,
        Acento::Morado,
        Acento::Rosa,
        Acento::Rojo,
        Acento::Naranja,
        Acento::Amarillo,
        Acento::Arena,
        Acento::Verde,
        Acento::Turquesa,
        Acento::Grafito,
    ];

    /// Todos, en el orden en que se pintan en la rejilla de Apariencia: el
    /// círculo cromático, que es el orden en que se buscan con el ojo.
    pub const TODOS: [Acento; 12] = [
        Acento::Azul,
        Acento::Indigo,
        Acento::Morado,
        Acento::Rosa,
        Acento::Rojo,
        Acento::Naranja,
        Acento::Amarillo,
        Acento::Arena,
        Acento::Verde,
        Acento::Turquesa,
        Acento::Grafito,
        Acento::Automatico,
    ];

    /// Su nombre en la configuración y en la interfaz. Sin tildes ni mayúsculas
    /// para que sea también lo que se escribe en `panel.conf`.
    // 0.1.0
    // modified 0.1.1 -- acento automático
    pub fn nombre(self) -> &'static str {
        match self {
            Acento::Azul => "azul",
            Acento::Indigo => "indigo",
            Acento::Morado => "morado",
            Acento::Rosa => "rosa",
            Acento::Rojo => "rojo",
            Acento::Naranja => "naranja",
            Acento::Amarillo => "amarillo",
            Acento::Arena => "arena",
            Acento::Verde => "verde",
            Acento::Turquesa => "turquesa",
            Acento::Grafito => "grafito",
            Acento::Automatico => "automatico",
        }
    }

    // 0.1.0
    pub fn desde_nombre(nombre: &str) -> Option<Acento> {
        Acento::TODOS
            .into_iter()
            .find(|a| a.nombre() == nombre.trim())
    }

    /// Su color en el tema que esté puesto.
    // 0.1.0
    // modified 0.1.1 -- el acento automático sale del fondo
    pub fn color(self) -> Color {
        let (claro, oscuro) = match self {
            // El par calibrado del que salen todos los demás: ver el histórico
            // de este fichero.
            Acento::Azul => (0x007aff, 0x5c95ff),
            Acento::Indigo => (0x5856d6, 0x7d7aff),
            Acento::Morado => (0xaf52de, 0xc979f0),
            Acento::Rosa => (0xff2d55, 0xff6482),
            // Los cuatro siguientes son los tokens de estado, sin retocar.
            Acento::Rojo => (0xff3b30, 0xff4a4a),
            Acento::Naranja => (0xff9500, 0xf8a13a),
            Acento::Amarillo => (0xffcc00, 0xf8db36),
            Acento::Verde => (0x34c759, 0x65ff8c),
            Acento::Turquesa => (0x009ba8, 0x40cbd4),
            // El mismo en claro y en oscuro, elegido a mano. Es muy claro: el
            // texto encima sale oscuro (`sobre_acento` lo decide por
            // luminancia), pero usado como texto sobre la tarjeta blanca casi
            // no se lee.
            Acento::Arena => (0xffeec0, 0xffeec0),
            // El acento de quien no quiere acento. No es `TEXTO2`: ese es el
            // gris del texto secundario y usarlo aquí haría que un botón
            // primario se leyera como deshabilitado.
            Acento::Grafito => (0x6e6e73, 0xa1a1a6),
            Acento::Automatico => {
                return color_automatico(hex(PALETA_FONDO[0].load(Ordering::Relaxed)), es_claro());
            }
        };
        hex(if es_claro() { claro } else { oscuro })
    }
}

static ACENTO: AtomicU8 = AtomicU8::new(0);
/// Los colores del fondo, el dominante primero. El acento automático es el
/// primero; los otros dos solo se enseñan en su casilla de Apariencia.
static PALETA_FONDO: [AtomicU32; 3] = [const { AtomicU32::new(0x5c95ff) }; 3];

/// Se calcula al cargar la imagen, también con un acento fijo, para que elegir
/// el automático no tenga que volver a decodificarla. En animaciones se
/// conserva el primer fotograma: cambiar la paleta en cada frame haría
/// parpadear los controles.
// 0.1.1
pub fn actualizar_acento_del_fondo(rgba: &[u8]) -> bool {
    PALETA_FONDO
        .iter()
        .zip(paleta_del_fondo(rgba))
        // El `swap` delante del `||`: detrás, el cortocircuito dejaría sin
        // guardar los colores que siguen al primero que cambie.
        .fold(false, |cambio, (guardado, color)| {
            guardado.swap(color, Ordering::Relaxed) != color || cambio
        })
}

/// La paleta ya corregida para el tema puesto, como se vería cada color si
/// fuera el acento. El primero **es** el acento automático.
// 0.1.1
pub fn paleta_automatica() -> [Color; 3] {
    let claro = es_claro();
    PALETA_FONDO
        .each_ref()
        .map(|c| color_automatico(hex(c.load(Ordering::Relaxed)), claro))
}

// 0.1.1
fn paleta_del_fondo(rgba: &[u8]) -> [u32; 3] {
    let mut grupos = [(0.0_f32, [0_u32; 3], 0_u32); 512];
    let paso = (rgba.len() / 4 / 4096).max(1);
    for p in rgba.as_chunks::<4>().0.iter().step_by(paso) {
        if p[3] < 128 {
            continue;
        }
        let max = p[0].max(p[1]).max(p[2]);
        let min = p[0].min(p[1]).min(p[2]);
        let i = ((p[0] as usize >> 5) << 6) | ((p[1] as usize >> 5) << 3) | (p[2] as usize >> 5);
        let grupo = &mut grupos[i];
        // Las franjas negras no deben ganar al paisaje que enmarcan. El croma
        // pesa más, pero una foto gris también debe aportar su propio acento.
        grupo.0 += (max - min) as f32 + max as f32 * 0.05;
        for (suma, canal) in grupo.1.iter_mut().zip(p) {
            *suma += *canal as u32;
        }
        grupo.2 += 1;
    }
    let mut vivos: Vec<(f32, [u32; 3])> = grupos
        .iter()
        .filter(|g| g.2 > 0)
        .map(|(peso, suma, cantidad)| (*peso, suma.map(|canal| canal / cantidad)))
        .collect();
    vivos.sort_by(|a, b| b.0.total_cmp(&a.0));
    let primero = vivos.first().map_or([0x5c, 0x95, 0xff], |g| g.1);
    let mut paleta = [primero; 3];
    let mut elegidos = 1;
    for (_, color) in vivos.iter().skip(1) {
        if elegidos == paleta.len() {
            break;
        }
        // Dos cubos vecinos son el mismo color partido por la rejilla de 32
        // niveles: sin esta distancia las tres franjas salían casi iguales.
        let distinto = paleta[..elegidos].iter().all(|ya| {
            ya.iter()
                .zip(color)
                .map(|(a, b)| a.abs_diff(*b))
                .sum::<u32>()
                >= 96
        });
        if distinto {
            paleta[elegidos] = *color;
            elegidos += 1;
        }
    }
    // Un fondo de un solo color deja las tres iguales, y la casilla lisa.
    paleta.map(|c| (c[0] << 16) | (c[1] << 8) | c[2])
}

/// Sube la saturación y lleva el color a su brillo máximo, sin mover el tono.
///
/// El color de una foto es la media de un cubo y sale apagado; aclararlo luego
/// hacia blanco para que contraste lo dejaba en gris. Así llega a ese paso con
/// croma de sobra. La saturación se multiplica y se topa en 0,6 en vez de
/// fijarse: un fondo gris se queda gris y no se inventa un color del ruido.
// 0.1.1
fn avivar(c: Color) -> Color {
    let max = c.r.max(c.g).max(c.b);
    let croma = max - c.r.min(c.g).min(c.b);
    if croma <= f32::EPSILON {
        return c;
    }
    let saturacion = croma / max;
    let objetivo = (saturacion * 2.5).min(0.6).max(saturacion);
    let canal = |x: f32| (max - (max - x) * objetivo / saturacion) / max;
    Color {
        r: canal(c.r),
        g: canal(c.g),
        b: canal(c.b),
        a: c.a,
    }
}

// 0.1.1
fn color_automatico(color: Color, claro: bool) -> Color {
    let color = avivar(color);
    let superficie = if claro { Color::WHITE } else { hex(0x1c1c1e) };
    let destino = if claro { Color::BLACK } else { Color::WHITE };
    // 4,5:1 permite usar el acento también como texto de acción en la tarjeta.
    for paso in 0..=100 {
        let candidato = mezclar(color, destino, paso as f32 / 100.0);
        let a = luminancia(candidato);
        let b = luminancia(superficie);
        if (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5 {
            return candidato;
        }
    }
    destino
}

/// Pone el acento del proceso. Como con el tema, quien lo llame tiene que
/// repintar: lo ya dibujado no se entera.
// 0.1.0
pub fn aplicar_acento(acento: Acento) {
    ACENTO.store(
        Acento::TODOS.iter().position(|a| *a == acento).unwrap_or(0) as u8,
        Ordering::Relaxed,
    );
}

// 0.1.0
pub fn acento_actual() -> Acento {
    let i = ACENTO.load(Ordering::Relaxed) as usize;
    Acento::TODOS.get(i).copied().unwrap_or_default()
}

/// El acento del sistema, el único color que significa «esto se toca».
// 0.1.0
pub fn acento() -> Color {
    acento_actual().color()
}

/// El acento apagado: el relleno de un conmutador que está, pero no encendido.
///
/// Se deriva y no se escribe a mano porque hay diez acentos y diez parejas
/// escritas a ojo se separan a la primera corrección. Las fracciones son las
/// que más se acercan a los dos valores que ya estaban calibrados para el
/// azul: `#b8d3ff` en claro y `#c8daff` en oscuro. No los clavan —los de antes
/// se eligieron a ojo y no son una mezcla lineal: en claro el verde queda 4
/// niveles de 255 por encima— pero por debajo de esa distancia no hay color
/// que se vea distinto.
// 0.1.0
pub fn acento_suave() -> Color {
    mezclar(acento(), Color::WHITE, if es_claro() { 0.70 } else { 0.66 })
}

/// El color del texto y de los enlaces: el acento corrido hacia donde se lee.
///
/// En claro se oscurece un 13 % —un texto fino en `#007aff` sobre blanco se
/// queda corto de contraste—; en oscuro se aclara un 17 %. Igual que arriba,
/// son las fracciones que devuelven los `#0a6ede` y `#77a2ff` que estaban
/// escritos a mano cuando el único acento era el azul.
// 0.1.0
pub fn enlace() -> Color {
    if es_claro() {
        mezclar(acento(), Color::BLACK, 0.13)
    } else {
        mezclar(acento(), Color::WHITE, 0.17)
    }
}

/// La tinta que se lee encima del acento.
///
/// Con el acento azul esto es blanco siempre y por eso el shell lo tenía
/// escrito así. Con la paleta abierta deja de serlo: el amarillo `#ffcc00`
/// tiene luminancia 0,66 y el texto blanco encima desaparece. Donde el fondo es
/// el acento se pregunta aquí, no se asume.
// 0.1.0
// modified 0.1.1 -- tinta propia sobre el acento automático
pub fn sobre_acento() -> Color {
    if acento_actual() == Acento::Automatico {
        tinta_automatica(acento())
    } else {
        tinta_sobre(acento())
    }
}

// 0.1.1
pub fn tinta_automatica(fondo: Color) -> Color {
    if luminancia(fondo) > 0.179 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// Interpola dos colores. `t = 0` da `a`, `t = 1` da `b`.
///
/// En sRGB directo, sin linealizar: es lo que hace `color-mix` de CSS por
/// defecto y lo que esperan los valores calibrados a ojo de los que sale la
/// tabla de arriba. Linealizar aquí daría mezclas más claras que las que el
/// sistema de diseño tiene escritas.
// 0.1.0
pub fn mezclar(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let m = |x: f32, y: f32| x + (y - x) * t;
    Color {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    }
}

/// El mismo color con otro alfa. Se escribía a mano —`Color { a: 0.12, ..c }`—
/// en cincuenta sitios.
// 0.1.0
pub fn alfa(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Texto secundario. Es el mismo en claro y en oscuro —lo fija así el sistema
/// de diseño— y por eso sigue siendo constante.
pub const TEXTO2: Color = hex(0x8e8e93);

/// Una superficie **por encima** de la que hay debajo: el `--surface` del
/// sistema de diseño, un velo del color del texto (blanco al 8 % en oscuro,
/// negro al 6 % en claro).
///
/// Es un alfa sobre lo que haya detrás y no un color fijo a propósito: sirve
/// igual dentro de un popover que sobre el escritorio, y es la única forma de
/// que un contenedor se vea **dentro** de otro. Dar el mismo token del padre
/// con alfa —lo que hacía el centro de control con `alfa(card(), 0.80)`— no
/// pinta nada: `#1c1c1e` al 80 % sobre `#1c1c1e` es `#1c1c1e`, y por eso las
/// cuatro tarjetas interiores no se distinguían del fondo del panel.
// 0.1.0
pub fn superficie() -> Color {
    alfa(texto(), if es_claro() { 0.06 } else { 0.08 })
}

/// [`superficie`] ya compuesta sobre [`card`]: el mismo color en pantalla,
/// pero **opaco**. Para los fondos grandes dentro de una tarjeta, que siempre
/// va sobre `card()`.
///
/// Opaco porque tiny-skia copia un relleno opaco y mezcla uno translúcido
/// píxel a píxel. Medido en release con 704×896 px, la tarjeta del centro de
/// control a escala 2: **0,21 ms opaco contra 3,57 translúcido**
/// (`examples/rasterizado.rs`). Los grupos grises eran casi toda la tarjeta y
/// se pagaban en cada repintado de un hover.
// 0.1.0
pub fn superficie_en_tarjeta() -> Color {
    let s = superficie();
    mezclar(card(), Color { a: 1.0, ..s }, s.a)
}

/// Lo que va **en relieve** dentro de un grupo: baldosas, botones redondos,
/// el surco de un deslizador.
///
/// En claro es la tarjeta blanca sobre el grupo gris y ya se despega. En oscuro
/// la tarjeta es más oscura que el grupo y esas piezas se veían como agujeros:
/// todo hundido y apagado. Por eso aquí sube un escalón por encima del grupo
/// (8 % de tinta): al 16 % queda en `#3d3d3f`, casi el `#3a3a3c` de iOS.
// 0.1.1
pub fn elevado() -> Color {
    if es_claro() {
        card()
    } else {
        mezclar(card(), tinta(), 0.16)
    }
}

/// El color de la tinta, para rellenos y bordes con muy poco alfa.
///
/// Es lo que había escrito como `Color { a: 0.12, ..Color::WHITE }` por todo el
/// shell: un velo del color del texto sobre la superficie. En claro ese blanco
/// desaparece contra la tarjeta, y por eso el alfa se aplica sobre esto y no
/// sobre un blanco fijo. Cuando lo que hay debajo es el acento —un botón azul,
/// una fila señalada— la tinta sí es blanca siempre, y ahí se escribe
/// `Color::WHITE` a propósito.
// 0.1.0
pub fn tinta() -> Color {
    texto()
}

// Estado, de la sección `Status / Feedback` de la paleta.
//
// Los de BookOS están elegidos para brillar sobre negro y en claro se caen: el
// amarillo `#F8DB36` como texto sobre una tarjeta blanca no se lee —medido en
// el «78 %» de la tarjeta de batería—, así que en claro se usan los de la
// sección 2.1 del sistema de diseño, que sí tienen variante para fondo claro.
tokens! {
    verde: claro hex(0x34c759), oscuro hex(0x65ff8c);
    rojo: claro hex(0xff3b30), oscuro hex(0xff4a4a);
    /// El ámbar de aviso. En claro es el `warning` naranja del sistema y no un
    /// amarillo más oscuro: el amarillo puro no llega al contraste ni bajándolo.
    amarillo: claro hex(0xff9500), oscuro hex(0xf8db36);
}

/// Los tres perfiles de energía, con el color que les da el diseño. Se usan en
/// el widget de la batería y en su tarjeta, y tienen que ser los mismos en los
/// dos sitios: es lo único que dice de un vistazo en qué perfil va el equipo.
///
/// Son fijos, iguales en claro y en oscuro, y no salen de `amarillo()`,
/// `verde()` ni `acento()`: esos cambian de tono con el tema —el amarillo pasa
/// a naranja en claro— o con el acento elegido, y el perfil cambiaba de color
/// al cambiar de tema. Aquí nunca son texto: rellenan un círculo o la pila del
/// panel, y lo que va encima lo elige [`tinta_sobre`]. Son los del HIG.
// 0.1.0
pub fn perfil_ahorro() -> Color {
    hex(0xffcc00)
}
// 0.1.0
pub fn perfil_equilibrado() -> Color {
    hex(0x34c759)
}
// 0.1.0
pub fn perfil_rendimiento() -> Color {
    hex(0x0a84ff)
}

/// Blanco o negro, el que se lea sobre `fondo`.
///
/// Hace falta donde el fondo lo pone un dato y no el diseño: la fila del perfil
/// de energía se pinta del color del perfil, y el mismo blanco que va bien sobre
/// el azul `#5C95FF` es ilegible sobre el amarillo `#F8DB36`. Medido con la
/// luminancia relativa de la WCAG: azul 0,31 —blanco—, verde 0,76 y amarillo
/// 0,71 —negro—.
// 0.1.0
// modified 0.1.1 -- la luminancia pasa a luminancia()
pub fn tinta_sobre(fondo: Color) -> Color {
    let l = luminancia(fondo);
    if l > 0.45 { Color::BLACK } else { Color::WHITE }
}

// 0.1.1
fn luminancia(fondo: Color) -> f32 {
    // Los canales de iced ya están en 0..1 pero con la curva de sRGB; hay que
    // linealizarlos antes de pesarlos o el verde sale muy por debajo de lo que
    // el ojo ve.
    let lin = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(fondo.r) + 0.7152 * lin(fondo.g) + 0.0722 * lin(fondo.b)
}

// Los cuatro rellenos de encima de una superficie son el **mismo color de la
// tinta** con muy poco alfa: blanco sobre oscuro, negro sobre claro. Por eso no
// hay un valor claro inventado para cada uno, solo cambia de qué lado tira.
tokens! {
    /// Línea de separación de 1 px.
    divisor: claro hexa(0x000000, 0.08), oscuro hexa(0xffffff, 0.08);
    /// Relleno de lo que está bajo el puntero.
    hover: claro hexa(0x000000, 0.04), oscuro hexa(0xffffff, 0.06);
    /// El canal vacío de un deslizador. Es el `trough` de los plasmoides, que se
    /// ve sin competir con la parte llena.
    surco: claro hexa(0x000000, 0.12), oscuro hexa(0xffffff, 0.14);
    /// El relleno de un control que está pero no está encendido: el
    /// `--toggle-off` del sistema de diseño (§2.1).
    ///
    /// Es **opaco y neutro**, no el acento aclarado: sobre una tarjeta blanca
    /// un acento suave se confunde con el encendido de al lado. Donde sí se
    /// quiere el acento aclarado —los conmutadores del centro de control en
    /// tema oscuro— se pide [`acento_suave`] a propósito.
    control_apagado: claro hex(0xe0e0e0), oscuro hex(0x48484a);
    /// Borde de un popup.
    borde: claro hexa(0x000000, 0.10), oscuro hexa(0xffffff, 0.09);

    /// Fondo totalmente opaco del panel. En claro es blanco y en oscuro es
    /// negro, para que los widgets no mezclen su fondo con el escritorio.
    panel: claro hex(0xffffff), oscuro hex(0x000000);
}

// --- Sombras ---------------------------------------------------------------
//
// Los cuatro niveles del sistema de diseño y ninguno más. `reposo` no está: en
// oscuro no lleva sombra y en claro ninguna superficie del shell la usa todavía.

// 0.1.0
fn sombra(y: f32, desenfoque: f32, a: f32) -> iced_core::Shadow {
    iced_core::Shadow {
        color: Color { a, ..Color::BLACK },
        offset: iced_core::Vector::new(0.0, y),
        blur_radius: desenfoque,
    }
}

/// Avisos que flotan solos: el OSD.
// 0.1.0
pub fn sombra_toast() -> iced_core::Shadow {
    sombra(4.0, 20.0, 0.15)
}

/// Tarjetas flotantes: la isla de actividades y la de medios del bloqueo.
// 0.1.0
pub fn sombra_popover() -> iced_core::Shadow {
    sombra(6.0, 22.0, 0.18)
}

/// Diálogos. Pide 56 px de margen alrededor; quien pinte en un búfer justo
/// tiene que usar [`sombra_popover`].
// 0.1.0
pub fn sombra_modal() -> iced_core::Shadow {
    sombra(20.0, 56.0, 0.30)
}

// --- Radios ----------------------------------------------------------------
//
// Tabla cerrada: no se inventan radios nuevos. Un radio suelto en un widget es
// lo que hace que un escritorio parezca cosido a mano. La tabla completa del
// sistema de diseño tiene además dialog 26, control 14, popItem 13, button 12 y
// smallBtn 10; aquí solo están los que se usan, y los demás se añaden cuando
// haya un widget que los pida.

/// Tarjeta agrupadora. Es el radio del dock, que es una tarjeta flotante.
pub const R_TARJETA: f32 = 22.0;
/// Diálogos y superficies modales centradas.
///
/// El buscador es un diálogo de teclado, no una tarjeta anclada al panel: el
/// HIG le da el radio mayor para que se lea como una superficie temporal.
pub const R_DIALOGO: f32 = 26.0;
pub const R_POPOVER: f32 = 18.0;
/// Lo que va dentro de un popover: los grupos grises de las tarjetas que
/// cuelgan del panel.
pub const R_ITEM_POPOVER: f32 = 13.0;
/// Controles: conmutadores, campos, botones de diálogo.
pub const R_CONTROL: f32 = 14.0;
/// Botones estándar y selects.
pub const R_BOTON: f32 = 12.0;
/// Botones pequeños. También la celda del calendario, que es uno.
pub const R_BOTON_PEQUENO: f32 = 10.0;
pub const R_CHIP: f32 = 6.0;
/// Círculos, avatares y píldoras.
///
/// Existe como token y no como "la mitad del alto" porque escribir `2.5` para
/// un punto de 5 px deja la intención en una cuenta que hay que rehacer cada
/// vez que cambia el tamaño — y que se olvida.
pub const R_PILL: f32 = 999.0;

// --- Tipografía ------------------------------------------------------------

/// Lado de un icono en el panel. El panel mide 32 px lógicos; 20 deja 6 de aire
/// arriba y abajo. Con 18 los iconos se quedaban por debajo de los del panel de
/// Plasma puesto al lado —comparado en capturas a la misma escala— y la barra
/// se leía apagada.
pub const ICONO_PANEL: f32 = 20.0;

/// Cuerpo del panel y de las listas.
/// Título de una superficie emergente. El "Sonido" del plasmoide de volumen
/// va a 18 con peso fuerte; aquí solo se fija el cuerpo.
pub const T_TITULO: f32 = 18.0;
pub const T_CUERPO: f32 = 13.0;
/// Texto secundario y etiquetas.
pub const T_PEQUENO: f32 = 12.0;

// --- Movimiento ------------------------------------------------------------
//
// Tabla cerrada, igual que la de radios: §2.7 del sistema de diseño empareja
// cada duración con **su** curva, y no son intercambiables. Lo que aparece de
// golpe (un popover, un modal) sale con muelle y se pasa un poco de su tamaño
// antes de asentarse; lo que se desliza o cambia de página va con una curva de
// entrada normal, sin rebote. Antes aquí se animaba con potencias —`1-(1-t)³`,
// `1-(1-t)⁵`— que se acercan a la de entrada pero no llegan a ninguna de las
// dos, y sobre todo no saben hacer el rebote: sin él, una emergente se lee como
// que se despliega en vez de aparecer.

/// Una curva `cubic-bezier` de CSS, con los extremos fijos en (0,0) y (1,1).
///
/// El eje Y puede salirse de [0,1] —es lo que hace el muelle— pero el X no: los
/// dos puntos de control tienen que quedar dentro para que la curva sea una
/// función del tiempo y se pueda invertir.
#[derive(Debug, Clone, Copy)]
pub struct Curva {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl Curva {
    // 0.1.0
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Qué fracción del recorrido toca, dada la fracción de tiempo `x`.
    ///
    /// Una bézier es paramétrica, así que no se puede evaluar en `x`
    /// directamente: primero hay que encontrar el parámetro `t` cuyo `X(t)` es
    /// la `x` que nos dan, y solo entonces leer `Y(t)`. Es lo mismo que hacen
    /// los navegadores, y por eso se hace igual: Newton desde `t = x`, que
    /// converge en un par de vueltas porque X es casi la identidad.
    // 0.1.0
    pub fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        // Los extremos son exactos por definición. Devolverlos sin iterar
        // importa: de que el final valga 1,0 **exacto** depende que quien
        // dibuja pueda comparar con 1,0 y saltarse la capa de escalado.
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }

        let mut t = x;
        for _ in 0..8 {
            let error = curva(self.x1, self.x2, t) - x;
            if error.abs() < 1e-5 {
                break;
            }
            let pendiente = derivada(self.x1, self.x2, t);
            // Con la pendiente casi plana Newton se dispara fuera del intervalo.
            // Pasa en el arranque de las curvas con `x1` muy pequeño, y ahí el
            // paso siguiente ya sale del [0,1]: se corta y se acepta el `t` que
            // haya, que está a menos de un píxel.
            if pendiente.abs() < 1e-6 {
                break;
            }
            t = (t - error / pendiente).clamp(0.0, 1.0);
        }
        curva(self.y1, self.y2, t)
    }
}

/// Una coordenada de la bézier cúbica con extremos en 0 y 1.
// 0.1.0
fn curva(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

// 0.1.0
fn derivada(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Hover, pulsación y cambios de fondo.
pub const C_SUAVE: Curva = Curva::new(0.4, 0.0, 0.2, 1.0);
/// Aparición de un popover. Muelle: se pasa y vuelve.
pub const C_MUELLE_POPOVER: Curva = Curva::new(0.32, 1.3, 0.5, 1.0);
/// Aparición de una tarjeta y transición de página. Sin rebote.
///
/// Arranca despacio, acelera en el centro y frena con una cola larga, como un
/// objeto con masa. La anterior, `(.25, .46, .45, .94)`, salía ya al 19 % del
/// recorrido en el primer 10 % del tiempo: se veía como un tirón. Esta va al
/// 3 % en el primer 10 % y al 78 % a mitad de camino. La simétrica
/// `(.45, 0, .55, 1)` arranca todavía más suave, pero al cuarto de tiempo
/// solo ha recorrido un 12 %, y en 220-280 ms eso se nota como retraso al
/// pulsar.
pub const C_ENTRADA: Curva = Curva::new(0.4, 0.0, 0.2, 1.0);
/// Geometría de ventanas: maximizar, restaurar, encajar y resize diferido.
/// La misma forma que [`C_ENTRADA`], sin rebote, porque un muelle aquí haría
/// oscilar texto y bordes ya rasterizados. La anterior, `(.22, 1, .36, 1)`,
/// recorría el 40 % en el primer 10 % del tiempo y la ventana saltaba.
pub const C_VENTANA: Curva = Curva::new(0.4, 0.0, 0.2, 1.0);
/// Conmutador y aparición de un modal. El muelle más marcado de los dos.
pub const C_MUELLE: Curva = Curva::new(0.34, 1.4, 0.64, 1.0);
/// Cambio de tema claro/oscuro. Simétrica y lenta al empezar: con
/// [`C_SUAVE`] la foto del tema viejo perdía la mitad de su opacidad en el
/// primer tercio y el blanco entraba como un fogonazo.
pub const C_TEMA: Curva = Curva::new(0.45, 0.0, 0.55, 1.0);

use std::time::Duration;

/// Hover, pulsación, cambios de fondo. Va con [`C_SUAVE`].
pub const D_HOVER: Duration = Duration::from_millis(120);
/// Aparición de un popover. Va con [`C_MUELLE_POPOVER`].
pub const D_POPOVER: Duration = Duration::from_millis(180);
/// Aparición de una tarjeta. Va con [`C_ENTRADA`].
pub const D_TARJETA: Duration = Duration::from_millis(220);
/// Maximizar, restaurar y encajar. Va con [`C_VENTANA`].
pub const D_VENTANA: Duration = Duration::from_millis(260);
/// Conmutador y aparición de un modal. Va con [`C_MUELLE`].
pub const D_MODAL: Duration = Duration::from_millis(250);
/// Transición de página completa. Va con [`C_ENTRADA`].
pub const D_PAGINA: Duration = Duration::from_millis(280);
/// Cambio de tema. Va con [`C_TEMA`]. Más largo que una página porque cambia
/// el brillo de **toda** la pantalla: de negro a blanco en 280 ms el ojo no
/// llega a adaptarse y se ve un parpadeo, no un fundido.
pub const D_TEMA: Duration = Duration::from_millis(450);
/// Salida del bloqueo. Son dos movimientos escalonados (el bloqueo se va y
/// luego entran las ventanas), y en 280 ms a las ventanas les quedaban menos
/// de 200 para entrar: llegaban de golpe.
pub const D_DESBLOQUEO: Duration = Duration::from_millis(400);

/// Cuánto se lleva recorrido de una duración, entre 0 y 1.
// 0.1.0
pub fn fraccion(pasado: Duration, total: Duration) -> f32 {
    (pasado.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
}

/// Como [`fraccion`], pero saltando al final cuando los efectos están
/// reducidos.
///
/// Va aparte y no dentro de `fraccion` a propósito: `fraccion` también mide
/// desvanecidos que **no** son adorno —el aviso de contraseña incorrecta, la
/// salida del OSD— y saltárselos escondería información en vez de ahorrar
/// trabajo. Aquí se marca en cada sitio que de verdad es una animación
/// prescindible.
// 0.1.0
pub fn avance(pasado: Duration, total: Duration) -> f32 {
    if efectos_reducidos() {
        return 1.0;
    }
    fraccion(pasado, total)
}

// añadido en la 0.2.0 -- masa unitaria: los tokens comparten unidades de valor por segundo.
#[derive(Debug, Clone, Copy)]
pub struct ParametrosMuelle {
    rigidez: f32,
    amortiguacion: f32,
}

impl ParametrosMuelle {
    // añadido en la 0.2.0 -- respuesta es el periodo natural, no una duración que bloquee la interacción.
    pub const fn nuevo(amortiguacion: f32, respuesta: f32) -> Self {
        let frecuencia = std::f32::consts::TAU / respuesta;
        Self {
            rigidez: frecuencia * frecuencia,
            amortiguacion: 2.0 * amortiguacion * frecuencia,
        }
    }
}

// modificado en la 0.2.0 -- el movimiento ordinario es crítico; el rebote queda reservado al impulso.
pub const M_RAPIDO: ParametrosMuelle = ParametrosMuelle::nuevo(1.0, 0.21);
pub const M_SUAVE: ParametrosMuelle = ParametrosMuelle::nuevo(1.0, 0.315);
pub const M_REBOTE: ParametrosMuelle = ParametrosMuelle::nuevo(0.8, 0.28);

// añadido en la 0.2.0 -- integral de una velocidad con decaimiento exponencial de 200 ms; no espera al usuario.
pub fn proyectar_impulso(posicion: f32, velocidad: f32) -> f32 {
    posicion + velocidad * 0.2
}

// añadido en la 0.2.0 -- redirigir conserva el impulso; no rearranca una curva desde velocidad cero.
#[derive(Debug, Clone, Copy)]
pub struct Muelle {
    valor: f32,
    velocidad: f32,
    destino: f32,
    ultimo: std::time::Instant,
    parametros: ParametrosMuelle,
}

impl Muelle {
    // añadido en la 0.2.0
    pub fn nuevo(valor: f32, parametros: ParametrosMuelle) -> Self {
        Self {
            valor,
            velocidad: 0.0,
            destino: valor,
            ultimo: std::time::Instant::now(),
            parametros,
        }
    }

    // añadido en la 0.2.0
    pub fn valor(&self) -> f32 {
        self.valor
    }

    // añadido en la 0.2.0
    pub fn velocidad(&self) -> f32 {
        self.velocidad
    }

    // añadido en la 0.2.0
    pub fn destino(&self) -> f32 {
        self.destino
    }

    // añadido en la 0.2.0
    pub fn quieto(&self) -> bool {
        (self.valor - self.destino).abs() < 0.0005 && self.velocidad.abs() < 0.005
    }

    // añadido en la 0.2.0
    pub fn ir_a(&mut self, destino: f32) {
        // añadido en la 0.2.0 -- el tiempo en reposo no pertenece a la próxima animación.
        if self.quieto() {
            self.ultimo = std::time::Instant::now();
        }
        // modificado en la 0.2.0 -- una orden posterior no hereda el rebote reservado al lanzamiento anterior.
        self.parametros.amortiguacion = 2.0 * self.parametros.rigidez.sqrt();
        self.destino = destino;
    }

    // añadido en la 0.2.0
    pub fn soltar(&mut self, destino: f32, velocidad: f32) {
        self.destino = destino;
        self.velocidad = velocidad;
        self.ultimo = std::time::Instant::now();
    }

    // añadido en la 0.2.0 -- solo cede al llegar si el impulso ya alcanzaría el destino por sí mismo.
    pub fn soltar_gesto(&mut self, destino: f32, velocidad: f32) {
        let distancia = destino - self.valor;
        let impulso = proyectar_impulso(0.0, velocidad);
        let rebota = impulso * distancia > 0.0 && impulso.abs() >= distancia.abs();
        self.parametros.amortiguacion =
            2.0 * if rebota { 0.8 } else { 1.0 } * self.parametros.rigidez.sqrt();
        self.soltar(destino, velocidad);
    }

    // añadido en la 0.2.0 -- mientras manda el dedo no se integra tiempo acumulado.
    pub fn fijar(&mut self, valor: f32) {
        self.valor = valor;
        self.destino = valor;
        self.velocidad = 0.0;
        self.ultimo = std::time::Instant::now();
    }

    // añadido en la 0.2.0
    pub fn avanzar(&mut self, ahora: std::time::Instant) -> bool {
        if efectos_reducidos() {
            let cambio = self.valor != self.destino;
            self.fijar(self.destino);
            self.ultimo = ahora;
            return cambio;
        }
        self.integrar(ahora)
    }

    // modificado en la 0.2.0 -- pasos de 2 ms: con amortiguación 80 %, 4 ms recortaban el rebote al 0,92 %; ahora 1,21 %.
    fn integrar(&mut self, ahora: std::time::Instant) -> bool {
        let pasado = ahora.saturating_duration_since(self.ultimo);
        self.ultimo = ahora;
        let anterior = self.valor;
        let mut restante = pasado.as_secs_f32();
        while restante > 0.0 {
            if self.quieto() {
                self.valor = self.destino;
                self.velocidad = 0.0;
                break;
            }
            let dt = restante.min(0.002);
            let aceleracion = self.parametros.rigidez * (self.destino - self.valor)
                - self.parametros.amortiguacion * self.velocidad;
            self.velocidad += aceleracion * dt;
            self.valor += self.velocidad * dt;
            restante -= dt;
        }
        self.valor != anterior
    }
}

/// Un número que va hacia otro con un muelle crítico.
///
/// Es lo mínimo que hace falta para animar dentro de un buffer del shell: el
/// que dibuja pregunta [`Transicion::valor`] en cada frame y quien manda el
/// bucle pregunta [`Transicion::animando`] para saber si tiene que pedir otro.
/// No hay reloj propio ni hilo: el tiempo lo pone el `Instant` de la última
/// vez que cambió el destino, así que una transición parada no cuesta nada.
///
/// **Al redirigirla a medio camino arranca del valor y la velocidad de
/// ahora.** Sin el valor, sacar el ratón de un botón antes de que acabe de
/// encenderse lo hacía saltar al principio; sin la velocidad, pulsar dos veces
/// un interruptor frenaba en seco antes de volver.
///
/// A diferencia de [`Muelle`], se evalúa en forma cerrada y no necesita
/// `&mut` para avanzar: las vistas de las tarjetas solo tienen `&self`. Por
/// eso es siempre crítico (la amortiguación de los parámetros se ignora): aquí
/// la mueven clics, que no traen impulso que justifique un rebote.
// modificado en la 0.2.0 -- muelle en vez de curva con duración fija
#[derive(Debug, Clone, Copy)]
pub struct Transicion {
    origen: f32,
    velocidad: f32,
    destino: f32,
    inicio: std::time::Instant,
    frecuencia: f32,
    /// Lo que se considera «llegado», proporcional al salto: con un umbral
    /// fijo de 0,0005 un desplazamiento de 500 px seguía pidiendo fotogramas
    /// de movimiento subpíxel casi el doble de tiempo que uno de 0 a 1.
    tolerancia: f32,
}

impl Transicion {
    /// Quieta en `valor`, sin animación pendiente.
    // modificado en la 0.2.0 -- parámetros de muelle en vez de duración y curva
    pub fn nueva(valor: f32, parametros: ParametrosMuelle) -> Self {
        Self {
            origen: valor,
            velocidad: 0.0,
            destino: valor,
            inicio: std::time::Instant::now(),
            frecuencia: parametros.rigidez.sqrt(),
            tolerancia: 0.0005,
        }
    }

    /// Posición y velocidad a `t` segundos del último cambio de destino.
    // añadido en la 0.2.0
    fn estado(&self, t: f32) -> (f32, f32) {
        let w = self.frecuencia;
        let c1 = self.origen - self.destino;
        let c2 = self.velocidad + w * c1;
        let e = (-w * t).exp();
        (
            self.destino + (c1 + c2 * t) * e,
            (c2 - w * (c1 + c2 * t)) * e,
        )
    }

    // añadido en la 0.2.0
    fn ahora(&self) -> (f32, f32) {
        if efectos_reducidos() {
            return (self.destino, 0.0);
        }
        self.estado(self.inicio.elapsed().as_secs_f32())
    }

    /// El valor de ahora mismo.
    // 0.1.0
    pub fn valor(&self) -> f32 {
        self.ahora().0
    }

    /// Le pone otro destino. `true` si eso cambia algo y hay que repintar.
    // modificado en la 0.2.0 -- conserva la velocidad
    pub fn ir_a(&mut self, destino: f32) -> bool {
        if self.destino == destino {
            return false;
        }
        let (valor, velocidad) = self.ahora();
        self.origen = valor;
        self.velocidad = velocidad;
        self.destino = destino;
        self.inicio = std::time::Instant::now();
        self.tolerancia = 0.0005 * (destino - valor).abs().max(1.0);
        true
    }

    /// La planta en `valor` sin animar. Para cuando el cambio no es del
    /// usuario: al abrirse una tarjeta, sus conmutadores no tienen que hacer el
    /// recorrido desde cero delante de él.
    // 0.1.0
    pub fn fijar(&mut self, valor: f32) {
        self.origen = valor;
        self.velocidad = 0.0;
        self.destino = valor;
    }

    /// ¿Se sigue moviendo? Mientras sí, hay que pedir otro fotograma.
    // modificado en la 0.2.0 -- termina por distancia y velocidad, no por reloj
    pub fn animando(&self) -> bool {
        let (valor, velocidad) = self.ahora();
        (valor - self.destino).abs() >= self.tolerancia || velocidad.abs() >= self.tolerancia * 10.0
    }

    // 0.1.0
    pub fn destino(&self) -> f32 {
        self.destino
    }
}

/// El realce que se pasea por una lista: qué fila está señalada y cuánto le
/// queda a la que se acaba de dejar.
///
/// Una [`Transicion`] por fila sería lo obvio y es justo lo que no hace falta:
/// solo hay dos filas moviéndose a la vez —la que entra y la que sale— y las
/// listas del shell se reconstruyen enteras en cada `view`, así que guardar
/// estado por fila obligaría a mantenerlo alineado con una lista que cambia
/// (las redes a la vista aparecen y desaparecen solas). Con dos índices y un
/// instante, el estado no depende de cuántas filas haya.
#[derive(Debug, Clone, Copy)]
pub struct Realce {
    actual: Option<usize>,
    previa: Option<usize>,
    desde: std::time::Instant,
}

impl Default for Realce {
    // 0.1.0
    fn default() -> Self {
        Self::nuevo()
    }
}

impl Realce {
    // 0.1.0
    pub fn nuevo() -> Self {
        Self {
            actual: None,
            previa: None,
            desde: std::time::Instant::now() - D_HOVER,
        }
    }

    /// Señala otra fila. `true` si hay que repintar.
    // 0.1.0
    pub fn señalar(&mut self, i: Option<usize>) -> bool {
        if self.actual == i {
            return false;
        }
        // La que se va **es la que estaba**, no la que hubiera antes: pasar el
        // ratón deprisa por tres filas tiene que apagar la de en medio, no
        // dejarla encendida hasta que se acabe su curva.
        self.previa = self.actual;
        self.actual = i;
        self.desde = std::time::Instant::now();
        true
    }

    /// Cuánto realce le toca a la fila `i`, entre 0 y 1.
    // 0.1.0
    pub fn intensidad(&self, i: usize) -> f32 {
        let t = C_SUAVE.eval(avance(self.desde.elapsed(), D_HOVER));
        if self.actual == Some(i) {
            t
        } else if self.previa == Some(i) {
            1.0 - t
        } else {
            0.0
        }
    }

    /// Da la animación por terminada, dejando la fila señalada ya al máximo.
    ///
    /// Es para el estado inicial: una tarjeta que se abre con un color ya
    /// elegido tiene que enseñarlo marcado, no marcándose.
    // 0.1.0
    pub fn terminar(&mut self) {
        self.previa = self.actual;
        self.desde = std::time::Instant::now() - D_HOVER;
    }

    /// Qué fila está señalada ahora. Para el hit-test, que no anima.
    // 0.1.0
    pub fn actual(&self) -> Option<usize> {
        self.actual
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        !efectos_reducidos() && self.actual != self.previa && self.desde.elapsed() < D_HOVER
    }
}

#[cfg(test)]
mod tema_automatico {
    use super::*;

    const AMANECE: HoraDelDia = (7, 0);
    const ANOCHECE: HoraDelDia = (20, 0);

    /// Los modos fijos no miran el reloj: es lo que separa «quiero oscuro» de
    /// «quiero lo que toque».
    #[test]
    fn los_modos_fijos_ignoran_la_hora() {
        for hora in [(3, 0), (12, 0), (23, 59)] {
            assert_eq!(
                ModoTema::Claro.resolver(AMANECE, ANOCHECE, hora),
                Tema::Claro
            );
            assert_eq!(
                ModoTema::Oscuro.resolver(AMANECE, ANOCHECE, hora),
                Tema::Oscuro
            );
        }
        assert_eq!(
            ModoTema::Claro.minutos_al_cambio(AMANECE, ANOCHECE, (12, 0)),
            None
        );
    }

    /// El día es claro y la noche oscura, con los bordes en su sitio: la hora
    /// del amanecer ya es clara y la del anochecer ya es oscura.
    #[test]
    fn el_automatico_sigue_al_sol() {
        let claro = |h, m| ModoTema::Automatico.resolver(AMANECE, ANOCHECE, (h, m)) == Tema::Claro;
        assert!(
            !claro(6, 59),
            "un minuto antes de amanecer todavía es de noche"
        );
        assert!(claro(7, 0), "la hora del amanecer ya es de día");
        assert!(claro(19, 59));
        assert!(!claro(20, 0), "la hora del anochecer ya es de noche");
        assert!(!claro(3, 0));
    }

    /// Quien trabaja de noche puede querer el tramo claro cruzando la
    /// medianoche: de las 22:00 a las 06:00. El tramo da la vuelta al día.
    #[test]
    fn el_tramo_claro_puede_cruzar_la_medianoche() {
        let (a, b) = ((22, 0), (6, 0));
        let claro = |h, m| ModoTema::Automatico.resolver(a, b, (h, m)) == Tema::Claro;
        assert!(claro(23, 30), "antes de medianoche, dentro del tramo");
        assert!(claro(2, 0), "después de medianoche, sigue dentro");
        assert!(!claro(6, 0), "y a las seis se acaba");
        assert!(!claro(12, 0));
    }

    /// Los dos extremos iguales no describen ningún tramo: sería un día entero
    /// de cada cosa a la vez. Se queda oscuro y no se programa ningún cambio,
    /// que es mejor que despertar cada día para no hacer nada.
    #[test]
    fn con_los_dos_extremos_iguales_no_hay_tramo() {
        let h = (9, 0);
        assert_eq!(ModoTema::Automatico.resolver(h, h, (9, 0)), Tema::Oscuro);
        assert_eq!(ModoTema::Automatico.resolver(h, h, (21, 0)), Tema::Oscuro);
        assert_eq!(ModoTema::Automatico.minutos_al_cambio(h, h, (9, 0)), None);
    }

    /// El despertar se programa para el primero de los dos cambios que caiga
    /// por delante, dando la vuelta al día si hace falta.
    #[test]
    fn el_despertar_va_al_proximo_cambio() {
        let falta = |h, m| ModoTema::Automatico.minutos_al_cambio(AMANECE, ANOCHECE, (h, m));
        assert_eq!(falta(6, 0), Some(60), "una hora para amanecer");
        assert_eq!(falta(12, 0), Some(8 * 60), "ocho horas para anochecer");
        assert_eq!(
            falta(21, 0),
            Some(10 * 60),
            "diez horas hasta el amanecer de mañana"
        );
        // Justo en la hora del cambio, el siguiente es el **otro**: si no,
        // el temporizador saltaría cada cero minutos y giraría en vacío.
        assert_eq!(
            falta(7, 0),
            Some(13 * 60),
            "en el amanecer, toca esperar al anochecer"
        );
        assert_eq!(falta(20, 0), Some(11 * 60));
    }

    /// Y nunca es cero, a ninguna hora del día: un temporizador de cero
    /// segundos es un bucle cerrado.
    #[test]
    fn el_despertar_nunca_es_ya() {
        for h in 0..24u8 {
            for m in [0u8, 30, 59] {
                let falta = ModoTema::Automatico
                    .minutos_al_cambio(AMANECE, ANOCHECE, (h, m))
                    .expect("en automático siempre hay un próximo cambio");
                assert!(
                    falta > 0,
                    "a las {h}:{m} el próximo cambio salía en {falta}"
                );
                assert!(falta <= 24 * 60, "a las {h}:{m} salía en {falta} minutos");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn el_dominante_descarta_las_franjas_negras_y_admite_grises() {
        let mut rgba = vec![0; 400];
        for p in rgba.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        rgba.extend_from_slice(&[20, 100, 200, 255].repeat(10));
        assert_eq!(super::paleta_del_fondo(&rgba)[0], 0x1464c8);
        assert_eq!(super::paleta_del_fondo(&[80, 80, 80, 255])[0], 0x505050);
        assert_eq!(super::paleta_del_fondo(&[255, 0, 0, 0])[0], 0x5c95ff);
    }

    #[test]
    fn la_paleta_no_repite_un_color_partido_en_dos_cubos() {
        // 95 y 97 caen en cubos distintos de la rejilla y son el mismo azul.
        let mut rgba = [20, 95, 200, 255].repeat(10);
        rgba.extend_from_slice(&[20, 97, 200, 255].repeat(8));
        rgba.extend_from_slice(&[200, 60, 40, 255].repeat(4));
        let paleta = super::paleta_del_fondo(&rgba);
        assert_eq!(paleta[0], 0x145fc8);
        assert_eq!(
            paleta[1], 0xc83c28,
            "el segundo es el rojo, no el otro azul"
        );
        assert_eq!(
            paleta[2], paleta[0],
            "sin tercer color se repite el primero"
        );
    }

    #[test]
    fn avivar_conserva_el_tono_y_deja_el_gris_como_estaba() {
        use super::*;
        assert_eq!(avivar(hex(0x505050)), hex(0x505050));
        let apagado = hex(0x8a7a60);
        let vivo = avivar(apagado);
        assert!(
            vivo.r > vivo.g && vivo.g > vivo.b,
            "el orden de canales es el tono"
        );
        assert!((vivo.r - 1.0).abs() < 1e-6);
        assert!(1.0 - vivo.b > 0.59, "saturación {}", 1.0 - vivo.b);
    }

    #[test]
    fn el_acento_automatico_y_su_letra_tienen_contraste() {
        use super::*;
        let contraste = |a, b| {
            let (a, b) = (luminancia(a), luminancia(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for color in [0x000000, 0xffffff, 0x505050, 0xff0000, 0x00ff00, 0x0000ff] {
            for claro in [false, true] {
                let acento = color_automatico(hex(color), claro);
                let tarjeta = if claro { Color::WHITE } else { hex(0x1c1c1e) };
                assert!(contraste(acento, tarjeta) >= 4.5, "acento sobre tarjeta");
                assert!(
                    contraste(acento, tinta_automatica(acento)) >= 4.5,
                    "letra sobre acento"
                );
            }
        }
    }

    use super::*;

    /// El tema conmuta de verdad y la tinta lo sigue.
    ///
    /// Deja el tema **como estaba** al terminar: es un global del proceso y los
    /// tests del mismo binario corren en hilos a la vez.
    #[test]
    fn el_tema_claro_invierte_la_tinta() {
        let antes = actual();
        aplicar(Tema::Claro);
        assert!(es_claro());
        assert_eq!(texto(), hex(0x000000), "en claro la tinta es negra");
        assert_eq!(card(), hex(0xffffff));
        aplicar(Tema::Oscuro);
        assert_eq!(texto(), hex(0xffffff));
        assert_eq!(bg(), hex(0x000000));
        // Lo que el sistema de diseño fija igual en los dos no se mueve.
        aplicar(Tema::Claro);
        assert_eq!(TEXTO2, hex(0x8e8e93));
        aplicar(antes);
    }

    /// Las fórmulas de `acento_suave` y `enlace` tienen que devolver, con el
    /// azul, los mismos colores que estaban escritos a mano antes de que
    /// hubiera diez acentos. Si esto se va, la paleta entera se desplaza.
    #[test]
    fn las_derivaciones_del_acento_reproducen_el_azul_calibrado() {
        let antes = (actual(), acento_actual());
        aplicar_acento(Acento::Azul);

        let cerca = |a: Color, b: Color, que: &str| {
            for (x, y) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
                // 12/255. Los valores de antes se eligieron a ojo y no son
                // mezclas lineales: `#0a6ede` tiene 10 de rojo donde el azul
                // del que sale tiene 0, y oscurecer no sube un canal. Lo que
                // este test protege es que la fórmula no se desvíe de lo
                // calibrado, no un redondeo; 10 niveles en el rojo de un azul
                // no se distinguen.
                assert!((x - y).abs() < 12.0 / 255.0, "{que}: {a:?} vs {b:?}");
            }
        };

        aplicar(Tema::Claro);
        assert_eq!(acento(), hex(0x007aff));
        cerca(acento_suave(), hex(0xb8d3ff), "acento_suave claro");
        cerca(enlace(), hex(0x0a6ede), "enlace claro");
        assert_eq!(sobre_acento(), Color::WHITE, "el azul lleva tinta blanca");

        aplicar(Tema::Oscuro);
        assert_eq!(acento(), hex(0x5c95ff));
        cerca(acento_suave(), hex(0xc8daff), "acento_suave oscuro");
        cerca(enlace(), hex(0x77a2ff), "enlace oscuro");

        aplicar(antes.0);
        aplicar_acento(antes.1);
    }

    /// El motivo de que `sobre_acento` exista: con el amarillo puesto, el
    /// blanco de siempre deja de leerse.
    #[test]
    fn el_amarillo_pide_tinta_negra() {
        let antes = (actual(), acento_actual());
        aplicar(Tema::Oscuro);
        aplicar_acento(Acento::Amarillo);
        assert_eq!(sobre_acento(), Color::BLACK);
        aplicar_acento(Acento::Azul);
        assert_eq!(sobre_acento(), Color::WHITE);
        aplicar(antes.0);
        aplicar_acento(antes.1);
    }

    #[test]
    fn los_nombres_de_los_acentos_van_y_vuelven() {
        for a in Acento::TODOS {
            assert_eq!(Acento::desde_nombre(a.nombre()), Some(a));
        }
        assert_eq!(Acento::desde_nombre("fucsia"), None);
    }

    /// Redirigir a medio camino no puede dar un salto: es lo que se ve al
    /// sacar el ratón de un botón antes de que acabe de encenderse.
    #[test]
    fn una_transicion_redirigida_arranca_donde_estaba() {
        let mut t = Transicion::nueva(0.0, M_RAPIDO);
        assert_eq!(t.valor(), 0.0);
        assert!(!t.animando(), "recién creada no anima");

        assert!(t.ir_a(1.0));
        assert!(t.animando());
        assert!(!t.ir_a(1.0), "el mismo destino no reinicia nada");
        std::thread::sleep(Duration::from_millis(50));
        let a_medias = t.valor();
        assert!(a_medias > 0.0 && a_medias < 1.0, "{a_medias}");

        t.ir_a(0.0);
        // El primer valor tras redirigir es el que había, no el 1,0 del destino
        // anterior ni el 0,0 del nuevo.
        assert!(
            (t.valor() - a_medias).abs() < 0.05,
            "{} vs {a_medias}",
            t.valor()
        );

        t.fijar(1.0);
        assert_eq!(t.valor(), 1.0);
        assert!(!t.animando());
    }

    /// Pulsar dos veces un interruptor no frena en seco: tras redirigir, sigue
    /// un instante hacia el destino viejo antes de volver.
    #[test]
    fn una_transicion_redirigida_conserva_la_velocidad() {
        let mut t = Transicion::nueva(0.0, M_RAPIDO);
        t.ir_a(1.0);
        std::thread::sleep(Duration::from_millis(30));
        t.ir_a(0.0);
        let al_redirigir = t.valor();
        std::thread::sleep(Duration::from_millis(5));
        assert!(t.valor() > al_redirigir, "{} vs {al_redirigir}", t.valor());
        assert!(t.animando());
    }

    /// Pasar el ratón deprisa por tres filas apaga la de en medio; si no, se
    /// quedan dos encendidas a la vez.
    #[test]
    fn el_realce_solo_deja_dos_filas_a_la_vez() {
        let mut r = Realce::nuevo();
        assert!(r.señalar(Some(0)));
        assert!(!r.señalar(Some(0)));
        r.señalar(Some(1));
        r.señalar(Some(2));
        assert_eq!(r.actual(), Some(2));
        assert_eq!(
            r.intensidad(0),
            0.0,
            "la de hace dos saltos ya está apagada"
        );
        assert!(r.intensidad(1) > 0.0, "la anterior se está yendo");
        assert!(r.animando());

        std::thread::sleep(D_HOVER);
        assert_eq!(r.intensidad(2), 1.0);
        assert_eq!(r.intensidad(1), 0.0);
        assert!(!r.animando());
    }

    #[test]
    fn los_radios_estan_en_la_tabla_cerrada() {
        // La tabla del sistema de diseño (§2.3), entera. Este test existe para
        // que añadir un radio nuevo obligue a mirarla: el modo de estropear
        // esto no es cambiar un token, es escribir un `radius: 4.0` suelto en
        // un widget porque en ese sitio quedaba bien.
        const TABLA: [f32; 12] = [
            26.0, 25.0, 22.0, 20.0, 18.0, 14.0, 13.0, 12.0, 10.0, 8.0, 6.0, 999.0,
        ];
        for r in [
            R_TARJETA,
            R_POPOVER,
            R_CONTROL,
            R_BOTON,
            R_BOTON_PEQUENO,
            R_CHIP,
            R_PILL,
        ] {
            assert!(TABLA.contains(&r), "radio fuera de la tabla: {r}");
        }
    }

    #[test]
    fn las_curvas_empiezan_en_cero_y_acaban_en_uno_exacto() {
        // Lo de "exacto" no es purismo: el compositor compara el resultado con
        // 1,0 para decidir si puede dibujar sin la capa de escalado, y un
        // 0,999999 dejaría a las ventanas envueltas el resto de su vida.
        for curva in [C_SUAVE, C_MUELLE_POPOVER, C_ENTRADA, C_VENTANA, C_MUELLE] {
            assert_eq!(curva.eval(0.0), 0.0);
            assert_eq!(curva.eval(1.0), 1.0);
        }
    }

    #[test]
    fn el_muelle_se_pasa_y_la_entrada_no() {
        // Es la diferencia que separa "aparece" de "se despliega", y la razón
        // de que la tabla empareje cada duración con su curva.
        let maximo = |c: Curva| {
            (0..=100)
                .map(|i| c.eval(i as f32 / 100.0))
                .fold(0.0f32, f32::max)
        };
        assert!(maximo(C_MUELLE) > 1.0, "el muelle tiene que pasarse de 1");
        assert!(maximo(C_MUELLE_POPOVER) > 1.0);
        assert!(maximo(C_ENTRADA) <= 1.0, "la entrada no debe rebotar");
        assert!(maximo(C_VENTANA) <= 1.0, "el resize no debe rebotar");
        assert!(maximo(C_SUAVE) <= 1.0);
    }

    #[test]
    fn las_curvas_avanzan_con_el_tiempo() {
        // X tiene que ser monótona o la inversión no tendría sentido; se
        // comprueba sobre Y de la curva sin rebote, que sí debe serlo.
        let mut anterior = 0.0;
        for i in 1..=50 {
            let v = C_ENTRADA.eval(i as f32 / 50.0);
            assert!(v >= anterior, "retrocede en {i}: {v} < {anterior}");
            anterior = v;
        }
    }

    #[test]
    fn lo_que_se_mueve_arranca_despacio_y_frena_al_final() {
        // Arranca más lento que la recta, la adelanta en el centro y llega
        // frenando: lo contrario de un movimiento a velocidad constante.
        for curva in [C_ENTRADA, C_VENTANA] {
            assert!(
                curva.eval(0.1) < 0.05,
                "arranca de golpe: {}",
                curva.eval(0.1)
            );
            assert!(curva.eval(0.5) > 0.5);
            let ultimo_tramo = curva.eval(1.0) - curva.eval(0.9);
            assert!(ultimo_tramo < 0.02, "no frena al llegar: {ultimo_tramo}");
        }
    }

    #[test]
    fn la_curva_suave_va_por_delante_de_la_recta() {
        // `cubic-bezier(.4, 0, .2, 1)` es la de material/One UI: arranca lenta y
        // adelanta a la mitad. Si esto falla, los ejes están cambiados.
        assert!(C_SUAVE.eval(0.5) > 0.45);
    }

    // añadido en la 0.2.0
    #[test]
    fn el_reposo_no_se_integra_en_el_siguiente_movimiento() {
        let mut muelle = Muelle::nuevo(0.0, M_SUAVE);
        muelle.ultimo -= Duration::from_secs(5);
        let reposo = muelle.ultimo;
        muelle.ir_a(1.0);
        assert!(muelle.ultimo.duration_since(reposo) >= Duration::from_secs(5));
        muelle.integrar(muelle.ultimo + Duration::from_millis(16));
        assert!(muelle.valor() > 0.0 && muelle.valor() < 0.5);
    }

    // añadido en la 0.2.0
    #[test]
    fn el_muelle_llega_y_deja_de_pedir_fotogramas() {
        let mut m = Muelle::nuevo(0.0, M_SUAVE);
        let inicio = m.ultimo;
        m.ir_a(1.0);
        m.integrar(inicio + Duration::from_secs(2));
        assert_eq!(m.valor(), 1.0);
        assert_eq!(m.velocidad(), 0.0);
        assert!(m.quieto());
    }

    // añadido en la 0.2.0
    #[test]
    fn redirigir_el_muelle_conserva_valor_y_velocidad() {
        let mut m = Muelle::nuevo(0.0, M_RAPIDO);
        m.ir_a(1.0);
        m.integrar(m.ultimo + Duration::from_millis(40));
        let estado = (m.valor(), m.velocidad());
        m.ir_a(0.0);
        assert_eq!((m.valor(), m.velocidad()), estado);
        m.integrar(m.ultimo + Duration::from_secs(2));
        assert_eq!(m.valor(), 0.0);
    }

    // añadido en la 0.2.0
    #[test]
    fn el_rebote_hereda_el_impulso_y_el_critico_frena() {
        let maximo = |parametros| {
            let mut m = Muelle::nuevo(0.0, parametros);
            m.soltar(1.0, 1.0);
            let inicio = m.ultimo;
            let mut maximo = 0.0_f32;
            for paso in 1..=500 {
                m.integrar(inicio + Duration::from_millis(paso * 4));
                maximo = maximo.max(m.valor());
            }
            assert!(m.quieto());
            maximo
        };
        assert!(maximo(M_REBOTE) > 1.01);
        assert!(maximo(M_SUAVE) <= 1.0);
    }

    // añadido en la 0.2.0
    #[test]
    fn solo_el_impulso_que_alcanza_el_destino_reduce_la_amortiguacion() {
        let mut m = Muelle::nuevo(0.3, M_SUAVE);
        m.soltar_gesto(1.0, 1.0);
        assert_eq!(
            m.parametros.amortiguacion,
            2.0 * m.parametros.rigidez.sqrt()
        );
        m.soltar_gesto(1.0, 4.0);
        assert_eq!(
            m.parametros.amortiguacion,
            1.6 * m.parametros.rigidez.sqrt()
        );
        let estado = (m.valor(), m.velocidad());
        m.ir_a(0.0);
        assert_eq!((m.valor(), m.velocidad()), estado);
        assert_eq!(
            m.parametros.amortiguacion,
            2.0 * m.parametros.rigidez.sqrt()
        );
    }

    // añadido en la 0.2.0
    #[test]
    fn el_muelle_recorre_lo_mismo_a_distintos_refrescos() {
        let recorrido = |fps: u32| {
            let mut m = Muelle::nuevo(0.0, M_SUAVE);
            m.soltar(1.0, 2.0);
            let inicio = m.ultimo;
            for fotograma in 1..=fps / 5 {
                m.integrar(inicio + Duration::from_secs_f64(fotograma as f64 / fps as f64));
            }
            m.valor()
        };
        let referencia = recorrido(240);
        for fps in [30, 60, 120] {
            assert!(
                (recorrido(fps) - referencia).abs() < 0.015,
                "refresco {fps}"
            );
        }
    }

    /// Los efectos reducidos saltan al final de la animación, pero **no**
    /// tocan `fraccion`: los desvanecidos que llevan información —el aviso de
    /// contraseña incorrecta— siguen su curso.
    #[test]
    fn los_efectos_reducidos_solo_saltan_lo_prescindible() {
        let mitad = D_VENTANA / 2;
        assert!(!efectos_reducidos(), "de serie están completos");
        assert!((avance(mitad, D_VENTANA) - 0.5).abs() < 0.01);

        assert!(aplicar_efectos_reducidos(true), "cambia la primera vez");
        assert!(!aplicar_efectos_reducidos(true), "y no la segunda");
        assert_eq!(avance(mitad, D_VENTANA), 1.0);
        // añadido en la 0.2.0 -- accesibilidad también cancela la física y su impulso.
        let mut m = Muelle::nuevo(0.0, M_REBOTE);
        m.soltar(1.0, 12.0);
        m.avanzar(std::time::Instant::now());
        assert_eq!(m.valor(), 1.0);
        assert_eq!(m.velocidad(), 0.0);
        assert!(m.quieto());
        assert!((fraccion(mitad, D_VENTANA) - 0.5).abs() < 0.01);

        aplicar_efectos_reducidos(false);
    }
}
