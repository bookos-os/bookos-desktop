//! La pantalla de bloqueo.
//!
//! Sigue el frame "Bloqueo De Pantalla" del diseño, con la disposición del
//! bloqueo de macOS: el reloj y la fecha grandes arriba, y **abajo** el bloque
//! de acceso —avatar, nombre y campo de contraseña— con la tarjeta de lo que
//! esté sonando debajo. El botón de apagado va abajo a la izquierda y despliega
//! Apagar / Reiniciar / Suspender.
//!
//! La composición no lleva coordenadas físicas: posiciones, reloj y avatar
//! salen de [`crate::config::Bloqueo`]. BookOS Settings puede moverlos u
//! ocultarlos escribiendo la configuración compartida, y HiDPI solo cambia la
//! cantidad de píxeles con la que se rasteriza esa misma composición lógica.
//!
//! **El fondo no se pinta aquí.** Es una imagen de 2880×1800 y rasterizarla en
//! CPU costaría más que todo lo demás junto; la sube el compositor como textura
//! una sola vez y esta superficie va encima, transparente.
//!
//! **La contraseña no se guarda como texto ni se enseña.** Aquí solo se sabe
//! cuántos caracteres lleva, para dibujar los puntos. Quien la guarda y la
//! valida es el compositor, que es quien puede hablar con PAM.

use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::config::FuenteReloj;
use crate::emergente::t;
use crate::tema;
use crate::view::PanelElement;

/// De los 100…900 de la configuración a los nueve grosores de iced. Lo que no
/// es un múltiplo de 100 no llega aquí: lo filtra la lectura de `panel.conf`.
// 0.1.0
// modified 0.1.1 -- pública para el greeter
pub fn grosor(valor: u16) -> iced_core::font::Weight {
    use iced_core::font::Weight;
    match valor {
        ..=100 => Weight::Thin,
        101..=200 => Weight::ExtraLight,
        201..=300 => Weight::Light,
        301..=400 => Weight::Normal,
        401..=500 => Weight::Medium,
        501..=600 => Weight::Semibold,
        601..=700 => Weight::Bold,
        701..=800 => Weight::ExtraBold,
        _ => Weight::Black,
    }
}

/// Los estilos de reloj cuya fuente está instalada.
///
/// Se pregunta a la misma base de fuentes con la que se dibuja el bloqueo y no
/// a fontconfig: fontdb no lee sus alias, y lo que importa es si *esto* la
/// encuentra. Sin la fuente, cosmic-text dibujaría otra cualquiera, y el estilo
/// elegido en Ajustes no se parecería a lo que sale en pantalla.
// 0.1.0
pub fn estilos_instalados() -> Vec<FuenteReloj> {
    // Un candado envenenado solo dice que otro hilo cayó con él cogido; la
    // base de fuentes se carga una vez y no queda a medias por eso.
    let mut sistema = iced_graphics::text::font_system()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let db = sistema.raw().db();
    FuenteReloj::TODAS
        .into_iter()
        .filter(|f| {
            db.faces().any(|cara| {
                cara.families
                    .iter()
                    .any(|(nombre, _)| nombre == f.familia())
            })
        })
        .collect()
}

/// Ancho de la píldora de contraseña, con su candado y su botón dentro. Es lo
/// que ocupaban el campo de 260 y el botón suelto de al lado, así que el
/// bloque no cambia de ancho.
pub const CAMPO: f32 = 312.0;
/// Lado del botón redondo de dentro de la píldora.
pub const BOTON: f32 = 36.0;
/// La ola de los puntos mientras PAM comprueba: dos vueltas y se para. No dura
/// lo que tarde PAM a propósito, para no repintar la pantalla entera todo ese
/// rato; si tarda más, los puntos se quedan quietos y el texto dice
/// «Comprobando…».
pub const D_OLA: Duration = Duration::from_millis(1040);
/// Alto del campo y lado del botón redondo que lleva al lado.
pub const ALTO_CAMPO: f32 = 44.0;
/// Cuerpo de la fecha, bajo el reloj.
const FECHA: f32 = 22.0;
/// Diámetro de los puntos de la contraseña.
pub const PUNTO: f32 = 10.0;
pub const ESTADO_TOTAL: Duration = Duration::from_millis(520);
/// Pulso del icono de huella al activarse o pedir reintento.
pub const PULSO_HUELLA: Duration = Duration::from_millis(620);
/// Duración del destello verde de confirmación al reconocer la huella.
pub const D_RECONOCIDA: Duration = Duration::from_millis(320);
// modificado en la 0.2.3 -- la tarjeta del Player: carátula grande, barra a
// todo el ancho con los tiempos a los lados y mandos sin placa. En vertical,
// como el Player, no cabe: a 1280×800 lógicos el bloque de acceso acaba hacia
// los 775 px y quedan ~240 debajo.
const MEDIOS_ANCHO: f32 = 372.0;
const MEDIOS_PAD: f32 = 16.0;
const MEDIOS_ARTE: f32 = 96.0;
/// Donde empieza el texto, a la derecha de la carátula.
const MEDIOS_INFO_X: f32 = MEDIOS_PAD + MEDIOS_ARTE + 14.0;
const MEDIOS_INFO: f32 = MEDIOS_ANCHO - MEDIOS_INFO_X - MEDIOS_PAD;
/// La fila de la barra, a 12 de la carátula.
const MEDIOS_BARRA_Y: f32 = MEDIOS_PAD + MEDIOS_ARTE + 12.0;
const MEDIOS_BARRA_FILA: f32 = 16.0;
/// Lo que ocupa cada tiempo a un lado de la barra.
const MEDIOS_TIEMPO: f32 = 40.0;
const MEDIOS_BARRA_X: f32 = MEDIOS_PAD + MEDIOS_TIEMPO + 8.0;
const MEDIOS_BARRA_W: f32 = MEDIOS_ANCHO - 2.0 * MEDIOS_BARRA_X;
const MEDIOS_CONTROLES_Y: f32 = MEDIOS_BARRA_Y + MEDIOS_BARRA_FILA + 8.0;
/// Alto de la fila de mandos y de la zona que atiende cada uno.
const MEDIOS_CONTROL: f32 = 40.0;
/// De centro a centro entre dos mandos.
const MEDIOS_PASO: f32 = 64.0;
const MEDIOS_ALTO: f32 = MEDIOS_CONTROLES_Y + MEDIOS_CONTROL + MEDIOS_PAD;

/// Alto de la línea del nombre de usuario: tamaño 20 con el interlineado
/// habitual de 1,2.
const ALTO_NOMBRE: f32 = 24.0;
/// Alto del renglón de estado bajo el campo: la píldora de [`aviso`].
// añadido en la 0.2.3
pub const ALTO_MENSAJE: f32 = 26.0;
/// Alto del aviso de la huella: otra píldora y 4 de aire sobre ella. Está
/// vacío la mayor parte del tiempo, pero reserva su hueco igual —la tarjeta de
/// medios no puede depender de si hoy hay algo que decir sobre la huella.
// modificado en la 0.2.3 -- píldora en vez de texto suelto
pub const ALTO_HUELLA: f32 = 4.0 + 30.0;
/// Aire mínimo entre el bloque de acceso y la tarjeta de medios.
///
/// Sin este mínimo, la tarjeta se posicionaba a una fracción fija de la
/// pantalla (`medios_y`) sin saber lo que ocupaba el bloque de encima, que es
/// de tamaño fijo en píxeles y no escala con la pantalla. En una resolución
/// lógica baja —1280×800, la de este portátil a escala 1,75— el bloque de
/// acceso llegaba más abajo que donde empezaba la tarjeta y se comían: el
/// aviso "Introduce tu contraseña para desbloquear" quedaba tapado a medias
/// por la carátula. En una pantalla más alta este mínimo no hace nada, porque
/// `medios_y` ya deja de sobra.
const MARGEN_MEDIOS: f32 = 24.0;
/// Aire mínimo bajo la tarjeta de medios hasta el borde de la pantalla.
// added 0.2.3
const MARGEN_INFERIOR: f32 = 40.0;

/// Fondo del campo y del botón: casi negro y translúcido, para que se vea el
/// fondo por debajo sin perder el contraste de los puntos.
pub const FONDO_CAMPO: Color = tema::hexa(0x16191e, 0.82);

/// El cristal del campo, el menú y la tarjeta según el tema. En claro es la
/// tarjeta del tema con el mismo alfa que el oscuro, para que el fondo se
/// siga intuyendo igual en los dos.
// añadido en la 0.2.3
// modificado en la 0.2.4 -- compartido también con el greeter
pub fn fondo_campo() -> Color {
    if tema::es_claro() {
        tema::alfa(tema::card(), FONDO_CAMPO.a)
    } else {
        FONDO_CAMPO
    }
}

/// La tinta que va sobre [`fondo_campo`]: el blanco de siempre en oscuro y el
/// texto del tema en claro.
// añadido en la 0.2.3
// modificado en la 0.2.4 -- compartido también con el greeter
pub fn tinta_campo() -> Color {
    if tema::es_claro() {
        tema::texto()
    } else {
        Color::WHITE
    }
}

/// El fondo de la tarjeta de medios: el del campo teñido con la carátula,
/// como la tarjeta grande del centro de control. Sin carátula, el del campo:
/// el acento que guarda `Medio::color` en ese caso no es de la canción.
// added 0.2.3
fn fondo_medio(medio: &Medio) -> Color {
    let base = fondo_campo();
    if medio.caratula.is_none() {
        return base;
    }
    // 22 % en claro, el de la celda del centro; en oscuro hace falta más
    // porque sobre casi negro el mismo porcentaje apenas se ve.
    let cuanto = if tema::es_claro() { 0.22 } else { 0.32 };
    Color {
        a: base.a,
        ..tema::mezclar(base, medio.color, cuanto)
    }
}

/// Lo recorrido de la barra: el color de la carátula llevado hacia la tinta
/// hasta separarse 4,5:1 del fondo de la tarjeta. Sin carátula, la tinta.
// added 0.2.3
fn realce_medio(medio: &Medio, fondo: Color) -> Color {
    let tinta = tinta_campo();
    if medio.caratula.is_none() {
        return tinta;
    }
    (0..=10)
        .map(|i| tema::mezclar(medio.color, tinta, i as f32 / 10.0))
        .find(|r| tema::contraste(*r, fondo) >= 4.5)
        .unwrap_or(tinta)
}

/// Separación del botón de apagado respecto al borde de la pantalla.
pub const MARGEN_ENERGIA: f32 = 22.0;
/// Lado del botón de apagado.
pub const LADO_ENERGIA: f32 = 32.0;
/// Ancho del menú que despliega. Da para el icono, su hueco y la palabra más
/// larga de las tres a 15 px.
pub const ANCHO_MENU: f32 = 196.0;
/// Alto de cada entrada del menú. Fijo y no derivado del padding porque el
/// hit-test tiene que dar exactamente lo mismo que el dibujo.
pub const ALTO_ENTRADA: f32 = 46.0;
/// Entre el botón y el menú que cuelga de él.
const HUECO_MENU: f32 = 10.0;
/// Lado del icono de una entrada.
// modificado en la 0.2.0 -- glifo blanco suelto, no insignia: las de color
// son iconos de aplicación y en un menú de acciones sobre el fondo chocaban.
// modified 0.2.0 -- 18 → 20: el mismo lado que los glifos de la fila de
// estado de la que cuelga el menú (wifi y encendido van a 20).
const ICONO_ENTRADA: f32 = 20.0;

/// Lo que ofrece el menú de apagado, en el orden del diseño. Hibernar solo si
/// el equipo puede: ver [`crate::confirmacion::hibernar_disponible`]. Cambiar
/// de usuario solo si hay otra cuenta de persona: con una sola, la ventana de
/// inicio enseñaría a la misma que ya está delante.
// 0.1.0
// modified 0.2.0 -- «Cambiar usuario», como el botón del bloqueo de macOS
pub fn entradas() -> &'static [(&'static str, &'static str, Peticion)] {
    static ENTRADAS: std::sync::OnceLock<Vec<(&str, &str, Peticion)>> = std::sync::OnceLock::new();
    ENTRADAS.get_or_init(|| {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let varias = listables(&passwd).len() > 1;
        [
            Some(("Apagar", "apagar", Peticion::Apagar)),
            Some(("Reiniciar", "reiniciar", Peticion::Reiniciar)),
            varias.then_some(("Cambiar usuario", "usuarios", Peticion::CambiarUsuario)),
            Some(("Cerrar sesión", "salir", Peticion::CerrarSesion)),
            Some(("Suspender", "dormir", Peticion::Suspender)),
            crate::confirmacion::hibernar_disponible().then_some((
                "Hibernar",
                "hibernar",
                Peticion::Hibernar,
            )),
        ]
        .into_iter()
        .flatten()
        .collect()
    })
}

/// Lo que el bloqueo puede pedirle al compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peticion {
    Apagar,
    Reiniciar,
    CerrarSesion,
    Suspender,
    Hibernar,
    /// Lleva a la ventana de inicio sin cerrar nada: no pide confirmación.
    CambiarUsuario,
}

impl Peticion {
    // 0.1.0
    // modified 0.2.0 -- `None` para lo que no se confirma (cambiar usuario)
    pub fn energia(self) -> Option<crate::confirmacion::Energia> {
        use crate::confirmacion::Energia;
        Some(match self {
            Self::Apagar => Energia::Apagar,
            Self::Reiniciar => Energia::Reiniciar,
            Self::CerrarSesion => Energia::CerrarSesion,
            Self::Suspender => Energia::Suspender,
            Self::Hibernar => Energia::Hibernar,
            Self::CambiarUsuario => return None,
        })
    }
}

/// Qué está pasando con lo que se ha escrito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    Escribiendo,
    /// Se ha mandado a validar y aún no hay respuesta.
    Comprobando,
    /// La contraseña no era. El campo se pinta en rojo.
    Fallo,
    /// Demora local tras varios fallos consecutivos.
    Espera(u32),
}

/// Una tarjeta de "esto está sonando", tal cual la enseña el diseño.
#[derive(Clone, PartialEq)]
pub struct Medio {
    pub bus: String,
    pub titulo: String,
    pub detalle: String,
    /// Color de la tarjeta: el de la carátula, o el acento si no hay.
    pub color: Color,
    /// De dónde salió la carátula, para no volver a descodificarla en cada
    /// lectura de la misma canción.
    pub fuente: Option<String>,
    pub caratula: Option<iced_widget::image::Handle>,
    pub progreso: Option<f32>,
    pub posicion: Option<u64>,
    pub duracion: Option<u64>,
    pub reproduciendo: bool,
    // añadido en la 0.2.3
    pub calidad: Option<crate::medios::Calidad>,
}

impl Medio {
    /// `previo` es la tarjeta anterior: si la carátula viene de la misma
    /// fuente se reutiliza con su color.
    // 0.1.0
    fn desde_sonando(sonando: crate::medios::Sonando, previo: Option<&Medio>) -> Self {
        let progreso = sonando.avance();
        let (caratula, color) = match (sonando.caratula.as_deref(), previo) {
            (Some(f), Some(p)) if p.fuente.as_deref() == Some(f) => (p.caratula.clone(), p.color),
            // Tres píxeles por punto, como el centro de control.
            (Some(f), _) => {
                crate::medios::cargar_caratula(f, (MEDIOS_ARTE * 3.0) as u32, tema::R_CONTROL * 3.0)
                    .map_or((None, tema::acento()), |(h, c)| (Some(h), c))
            }
            (None, _) => (None, tema::acento()),
        };
        Self {
            fuente: sonando.caratula,
            caratula,
            bus: sonando.bus,
            titulo: sonando.titulo,
            detalle: if sonando.artista.is_empty() {
                sonando.aplicacion
            } else {
                sonando.artista
            },
            color,
            progreso,
            posicion: sonando.posicion,
            duracion: sonando.duracion,
            reproduciendo: sonando.reproduciendo,
            calidad: sonando.calidad,
        }
    }
}

/// La foto del usuario y cómo se llama.
///
/// La foto se busca donde la dejan los escritorios de siempre: `~/.face` es lo
/// que usan SDDM y GDM, y `/var/lib/AccountsService/icons/<usuario>` es donde
/// la guarda AccountsService cuando se cambia desde los ajustes de Plasma. Si
/// no hay ninguna, quedan las iniciales sobre el acento, que es preferible al
/// círculo gris de antes: dice de quién es la sesión.
pub struct Usuario {
    pub nombre: String,
    /// El nombre de la cuenta: lo que va detrás de la arroba en el centro de
    /// control. No siempre coincide con `nombre`, que es el completo.
    pub cuenta: String,
    pub foto: Option<iced_widget::image::Handle>,
}

impl Usuario {
    /// `avatar` es la ruta que diga `panel.conf`, y va **primera**: quien la
    /// escribe está eligiendo a mano, y eso pesa más que lo que haya dejado
    /// puesto una instalación anterior en `~/.face`.
    // 0.1.0
    pub fn leer(avatar: Option<&str>) -> Self {
        let login = std::env::var("USER").unwrap_or_else(|_| "usuario".into());
        let casa = std::env::var("HOME").unwrap_or_default();
        let candidatas = [
            // `~` a mano: el fichero de configuración se escribe con la tilde y
            // nadie espera tener que poner la ruta entera.
            avatar
                .map(|r| r.replacen('~', &casa, 1))
                .unwrap_or_default(),
            format!("{casa}/.face"),
            format!("{casa}/.face.icon"),
            format!("/var/lib/AccountsService/icons/{login}"),
        ];
        let foto = candidatas
            .iter()
            .find(|r| std::path::Path::new(r).is_file())
            .and_then(|r| foto_circular(r));
        Self {
            // El nombre completo sale del quinto campo de `/etc/passwd`, que es
            // donde lo pone `useradd`. Vacío —lo habitual en una instalación
            // rápida— se queda el de la cuenta.
            nombre: nombre_completo(&login).unwrap_or_else(|| login.clone()),
            cuenta: login,
            foto,
        }
    }
}

/// Recorta la foto en un círculo y la deja al tamaño del avatar.
///
/// **El recorte se hace aquí y no con el contenedor.** `container(...).clip(true)`
/// con un radio redondea el fondo, pero no la imagen que lleva dentro: se probó
/// y la foto salía cuadrada, sobresaliendo del círculo. Como además hay que
/// recortarla al cuadrado central —una foto apaisada metida en un círculo se
/// deforma—, sale más barato hacer las dos cosas de una pasada al cargarla, que
/// ocurre **una vez** por bloqueo y no por frame.
// 0.1.0
// modified 0.1.1 -- pública para el greeter
pub fn foto_circular(ruta: &str) -> Option<iced_widget::image::Handle> {
    // La configuración admite hasta 220 lógicos. Se guarda al doble para que
    // incluso a escala 2 la GPU reduzca una imagen nítida en vez de ampliar una
    // miniatura; sigue siendo menos de 1 MiB y se genera una sola vez.
    foto_circular_de(ruta, 440)
}

/// Recorta y escala una imagen de perfil a un círculo de `lado × lado` píxeles.
pub(crate) fn foto_circular_de(ruta: &str, lado: u32) -> Option<iced_widget::image::Handle> {
    let lado = lado.max(1);
    // `image::open` deduce el formato de la extensión, y `~/.face` y los iconos
    // de AccountsService no la tienen: fallaba con «format could not be
    // determined» (visto en bookos-session.log). Se mira el contenido.
    let imagen = image::ImageReader::open(ruta)
        .and_then(|lector| lector.with_guessed_format())
        .map_err(image::ImageError::from)
        .and_then(|lector| lector.decode())
        .inspect_err(|err| tracing::warn!(ruta, "no se pudo leer la foto de perfil: {err}"))
        .ok()?;
    // El cuadrado central: es lo que hace `ContentFit::Cover`, pero de verdad.
    let (ancho, alto) = (imagen.width(), imagen.height());
    let lado_origen = ancho.min(alto);
    let recorte = image::imageops::crop_imm(
        &imagen,
        (ancho - lado_origen) / 2,
        (alto - lado_origen) / 2,
        lado_origen,
        lado_origen,
    )
    .to_image();
    let mut escalada =
        image::imageops::resize(&recorte, lado, lado, image::imageops::FilterType::Lanczos3);

    // La máscara, con el borde suavizado en un píxel: sin eso el círculo sale
    // con escalones y se nota mucho más que en una forma recta.
    let radio = lado as f32 / 2.0;
    for (x, y, px) in escalada.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - radio;
        let dy = y as f32 + 0.5 - radio;
        let d = (dx * dx + dy * dy).sqrt();
        let dentro = ((radio - d) / 1.5).clamp(0.0, 1.0);
        px.0[3] = (px.0[3] as f32 * dentro) as u8;
    }
    Some(iced_widget::image::Handle::from_rgba(
        lado,
        lado,
        escalada.into_raw(),
    ))
}

/// El nombre completo de `/etc/passwd`. Sin `libc`: el fichero es de texto y
/// leerlo entero son cuatro líneas en un equipo de escritorio.
// 0.1.0
// modified 0.1.1 -- pública para el greeter
pub fn nombre_completo(login: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/etc/passwd").ok()?;
    for linea in texto.lines() {
        let mut campos = linea.split(':');
        if campos.next()? != login {
            continue;
        }
        // usuario:x:uid:gid:GECOS:casa:shell — el GECOS es el quinto, y puede
        // traer más datos separados por comas: el nombre es el primero.
        let gecos = campos.nth(3)?;
        let nombre = gecos.split(',').next()?.trim();
        return (!nombre.is_empty()).then(|| nombre.to_string());
    }
    None
}

/// Las cuentas de persona: el rango de `UID_MIN`…`UID_MAX` de `login.defs` y un
/// shell con el que se pueda entrar. La comparten el greeter y el menú del
/// logo (cambio rápido de usuario).
// 0.1.1
pub fn listables(passwd: &str) -> Vec<(String, u32, String)> {
    passwd
        .lines()
        .filter_map(|linea| {
            let campos: Vec<&str> = linea.split(':').collect();
            let [cuenta, _, uid, _, gecos, _, shell] = campos[..] else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            let entra = !(shell.ends_with("/nologin") || shell.ends_with("/false"));
            let nombre = gecos.split(',').next().unwrap_or_default().trim();
            ((1000..60000).contains(&uid) && entra).then(|| {
                let nombre = if nombre.is_empty() { cuenta } else { nombre };
                (cuenta.to_string(), uid, nombre.to_string())
            })
        })
        .collect()
}

/// Lo que dice la huella debajo del campo.
///
/// Un estado y no solo un texto: sin él, un dedo que no casaba y uno que sí se
/// veían igual —el mismo renglón gris— hasta que PAM agotaba los tres intentos
/// o la pantalla desaparecía de golpe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Huella {
    /// Nada que decir: sin lector, o con la huella desactivada.
    Callada,
    /// Un aviso neutro: «coloca el dedo», «no disponible»…
    Aviso(&'static str),
    /// El lector ha leído un dedo que no es. PAM sigue esperando otro.
    NoCoincide,
    /// Ha casado; lo que queda es la salida del bloqueo.
    Reconocida,
}

// añadido en la 0.2.0 -- cada capa se rasteriza en su rectángulo y se mueve en la GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capa {
    Reloj,
    Acceso,
    Medios,
    Energia,
}

impl Capa {
    pub const TODAS: [Self; 4] = [Self::Reloj, Self::Acceso, Self::Medios, Self::Energia];
}

pub struct Bloqueo {
    // añadido en la 0.2.0 -- el estado visual vive aquí aunque el compositor lo dibuje en la GPU.
    entrada: [tema::Muelle; 4],
    entrada_desde: Instant,
    /// La hora, ya formateada. La pone el compositor para no tener dos relojes
    /// distintos en el mismo escritorio.
    pub hora: String,
    /// La fecha larga, debajo del reloj.
    pub fecha: String,
    /// Cuántos caracteres lleva la contraseña. El texto no se guarda aquí.
    pub escritos: usize,
    pub estado: Estado,
    huella: Huella,
    /// Desde cuándo dice lo que dice, para la sacudida del «no coincide».
    huella_desde: Instant,
    confirmacion: Option<(Peticion, crate::confirmacion::Confirmacion)>,
    /// Estado de Bloq Mayús. No forma parte de la contraseña, pero explicarlo
    /// evita intentos fallidos que parecen una clave incorrecta.
    pub caps_lock: bool,
    /// Si el menú de apagado está desplegado.
    pub menu: bool,
    /// Cuándo empezó a abrirse o a cerrarse, para animarlo.
    ///
    /// Guarda el instante y hacia dónde va. Se suelta al terminar: dejarlo
    /// puesto haría que cada frame del resto de la sesión volviera a interpolar
    /// algo que ya acabó.
    menu_desde: Option<(std::time::Instant, bool)>,
    pub medios: Vec<Medio>,
    /// Dónde está el dedo en la barra mientras se arrastra, de 0 a 1. Manda
    /// sobre lo que diga MPRIS hasta soltar: la lectura de cada segundo la
    /// devolvería atrás a mitad del gesto.
    // añadido en la 0.2.3
    arrastre: Option<f32>,
    /// El cursor está sobre la barra de la música.
    // añadido en la 0.2.3
    sobre_barra: bool,
    /// Cuánto ha engordado la barra (0 fina, 1 gruesa), con el muelle de la
    /// del centro de control: antes saltaba de 5 a 10 px de un fotograma a otro.
    // added 0.2.3
    grosor_barra: tema::Transicion,
    /// El mando apretado y dónde empezó el toque. La orden sale al soltar
    /// (HIG §8), y alejarse más de [`CANCELA_TOQUE`] la cancela.
    // added 0.2.3
    apretado: Option<(Mando, (f32, f32))>,
    /// Cuánto está hundido `hundido_cual` (0 a 1). Se queda con el mando al
    /// soltar para que vuelva a su tamaño animado y no de golpe.
    // added 0.2.3
    hundido: tema::Transicion,
    // added 0.2.3
    hundido_cual: Option<Mando>,
    /// Cuándo empezó a hundirse y, si se soltó antes de [`HUNDIDO_MINIMO`],
    /// que la vuelta está pendiente.
    // added 0.2.3
    hundido_desde: Instant,
    // added 0.2.3
    vuelta_pendiente: bool,
    /// Aleatorio y repetición, solo con BookOS Player: los demás reproductores
    /// van con los tres mandos de siempre. `None` en los dos, sin ellos.
    // añadido en la 0.2.3
    aleatorio: Option<bool>,
    repetir: Option<crate::medios::Repetir>,
    usuario: Usuario,
    config: crate::config::Bloqueo,
    /// Entrada de reloj, identidad y tarjetas, escalonada desde este instante.
    /// Último cambio del campo. Da vida a comprobar y al error sin guardar la
    /// contraseña ni ningún dato extra sobre ella.
    estado_desde: Instant,
    medio_desde: Option<Instant>,
    /// Pictogramas del campo, hechos una vez: un `Handle` nuevo por fotograma
    /// obligaría a iced a volver a rasterizar el SVG en cada repintado.
    candado: crate::icono::Icono,
    flecha: crate::icono::Icono,
    icono_huella: crate::icono::Icono,
    wifi: crate::icono::Icono,
    encendido: crate::icono::Icono,
    /// Lo que enseña la fila de arriba. Solo informa: abrir la lista de redes
    /// sin haber entrado sería dejar la configuración a quien pase por delante.
    bateria: Option<(crate::icono::Icono, (f32, f32), String, Color)>,
    red: Option<crate::Network>,
}

/// Candado y flecha de Heroicons, macizos: el teñido sustituye el relleno.
/// Compartidos con el greeter: es la misma fila de estado.
pub const WIFI: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M1.371 8.143c5.858-5.857 15.356-5.857 21.213 0a.75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.06 0c-4.98-4.979-13.053-4.979-18.032 0a.75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.182 3.182c4.1-4.1 10.749-4.1 14.85 0a.75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.062 0 8.25 8.25 0 0 0-11.667 0 .75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.204 3.182a6 6 0 0 1 8.486 0 .75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.061 0 3.75 3.75 0 0 0-5.304 0 .75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.182 3.182a1.5 1.5 0 0 1 2.122 0 .75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.061 0l-.53-.53a.75.75 0 0 1 0-1.06Z" clip-rule="evenodd"/></svg>"##;
pub const ENCENDIDO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M12 2.25a.75.75 0 0 1 .75.75v9a.75.75 0 0 1-1.5 0V3a.75.75 0 0 1 .75-.75ZM6.166 5.106a.75.75 0 0 1 0 1.06 8.25 8.25 0 1 0 11.668 0 .75.75 0 1 1 1.06-1.06c3.808 3.807 3.808 9.98 0 13.788-3.807 3.808-9.98 3.808-13.788 0-3.808-3.807-3.808-9.98 0-13.788a.75.75 0 0 1 1.06 0Z" clip-rule="evenodd"/></svg>"##;
/// Candado y flecha.
pub const CANDADO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" clip-rule="evenodd" d="M12 1.5a5.25 5.25 0 0 0-5.25 5.25v3a3 3 0 0 0-3 3v6.75a3 3 0 0 0 3 3h10.5a3 3 0 0 0 3-3v-6.75a3 3 0 0 0-3-3v-3c0-2.9-2.35-5.25-5.25-5.25Zm3.75 8.25v-3a3.75 3.75 0 1 0-7.5 0v3h7.5Z"/></svg>"##;
pub const FLECHA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" clip-rule="evenodd" d="M12.97 3.97a.75.75 0 0 1 1.06 0l7.5 7.5a.75.75 0 0 1 0 1.06l-7.5 7.5a.75.75 0 1 1-1.06-1.06l6.22-6.22H3a.75.75 0 0 1 0-1.5h16.19l-6.22-6.22a.75.75 0 0 1 0-1.06Z"/></svg>"##;
/// Huella digital para el aviso biométrico bajo el campo.
pub const HUELLA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" clip-rule="evenodd" d="M12 2.25a9.75 9.75 0 0 0-7.79 3.88.75.75 0 0 0 1.2.9 8.25 8.25 0 0 1 13.18 0 .75.75 0 1 0 1.2-.9A9.75 9.75 0 0 0 12 2.25Zm-5.4 6.2a6.75 6.75 0 0 1 10.8 0 .75.75 0 0 0 1.2-.9 8.25 8.25 0 0 0-13.2 0 .75.75 0 0 0 1.2.9Zm1.9 2.38a3.75 3.75 0 0 1 7 0 .75.75 0 1 0 1.38-.6 5.25 5.25 0 0 0-9.76 0 .75.75 0 0 0 1.38.6ZM12 9.75a2.25 2.25 0 0 0-2.25 2.25c0 2.5-.62 4.65-1.93 6.37a.75.75 0 1 0 1.2.9c1.49-1.96 2.23-4.4 2.23-7.27a.75.75 0 0 1 1.5 0c0 3.33-.86 6.26-2.55 8.53a.75.75 0 1 0 1.2.9c1.89-2.52 2.85-5.76 2.85-9.43A2.25 2.25 0 0 0 12 9.75Zm-4.5 2.25a4.5 4.5 0 0 1 9 0c0 2.94-.67 5.65-1.96 7.98a.75.75 0 1 0 1.32.72C17.27 18.14 18 15.17 18 12a6 6 0 0 0-12 0c0 1.98-.46 3.7-1.45 5.14a.75.75 0 1 0 1.24.84C6.95 16.27 7.5 14.27 7.5 12Z"/></svg>"##;

impl Bloqueo {
    // 0.1.0
    pub fn new(
        hora: String,
        fecha: String,
        avatar: Option<&str>,
        config: crate::config::Bloqueo,
    ) -> Self {
        let ahora = Instant::now();
        Self {
            entrada: [{
                let mut m = tema::Muelle::nuevo(0.0, tema::M_SUAVE);
                m.ir_a(1.0);
                m
            }; 4],
            entrada_desde: ahora,
            hora,
            fecha,
            escritos: 0,
            estado: Estado::Escribiendo,
            huella: Huella::Callada,
            huella_desde: ahora - ESTADO_TOTAL,
            confirmacion: None,
            caps_lock: false,
            menu: false,
            menu_desde: None,
            medios: Vec::new(),
            arrastre: None,
            sobre_barra: false,
            grosor_barra: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            apretado: None,
            hundido: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            hundido_cual: None,
            hundido_desde: Instant::now(),
            vuelta_pendiente: false,
            aleatorio: None,
            repetir: None,
            usuario: Usuario::leer(avatar),
            config,
            estado_desde: ahora - ESTADO_TOTAL,
            medio_desde: None,
            candado: crate::icono::desde_svg(CANDADO),
            flecha: crate::icono::desde_svg(FLECHA),
            icono_huella: crate::icono::desde_svg(HUELLA),
            wifi: crate::icono::desde_svg(WIFI),
            encendido: crate::icono::desde_svg(ENCENDIDO),
            bateria: crate::Battery::read(false).map(|b| crate::bateria_sobre_fondo(&b)),
            red: crate::Network::read(),
        }
    }

    /// Aplica una composición nueva manteniendo el estado sensible y los
    /// medios actuales. Cambiar un deslizador no reinicia la contraseña ni la
    /// animación de entrada del bloqueo que ya está puesto.
    // 0.1.0
    pub fn aplicar_config(&mut self, config: crate::config::Bloqueo) {
        self.config = config;
    }

    // 0.1.0
    // modificado en la 0.2.0 -- las previsualizaciones conservan la composición final completa.
    pub fn view(&self, pantalla: (f32, f32)) -> PanelElement<'_> {
        self.componer(pantalla, None)
    }

    // añadido en la 0.2.0
    pub fn view_capa(&self, pantalla: (f32, f32), capa: Capa) -> PanelElement<'_> {
        self.componer(pantalla, Some(capa))
    }

    // añadido en la 0.2.0 -- el margen del acceso deja sitio a la sacudida sin recortar texto ni avatar.
    pub fn geometria_capa(&self, capa: Capa, pantalla: (f32, f32)) -> Option<(f32, f32, f32, f32)> {
        let (w, h) = pantalla;
        let (x, y, ancho, alto) = match capa {
            Capa::Reloj => (
                0.0,
                h * self.config.reloj_y,
                w,
                self.config.reloj_tamano * 1.3 + if self.config.fecha { FECHA * 1.3 } else { 0.0 },
            ),
            Capa::Acceso => {
                let ancho = (CAMPO + 100.0).max(self.avatar_tamano(h) + 40.0).min(w);
                let alto = self.avatar_tamano(h)
                    + if self.config.nombre {
                        12.0 + ALTO_NOMBRE
                    } else {
                        0.0
                    }
                    + 16.0
                    + ALTO_CAMPO
                    + 10.0
                    + 40.0
                    + ALTO_HUELLA;
                ((w - ancho) / 2.0, self.acceso_y(h), ancho, alto)
            }
            Capa::Medios if self.config.medios && !self.medios.is_empty() => {
                let ancho = MEDIOS_ANCHO.min(w);
                (
                    (w - ancho) / 2.0,
                    self.medios_y_asentada(h),
                    ancho,
                    self.medios.len().min(2) as f32 * (MEDIOS_ALTO + 10.0),
                )
            }
            Capa::Medios => return None,
            Capa::Energia if self.confirmacion.is_some() => (0.0, 0.0, w, h),
            // modificado en la 0.2.0 -- arriba a la derecha, como en el greeter.
            Capa::Energia if self.config.energia => (
                (w - ANCHO_MENU - 2.0 * MARGEN_ENERGIA).max(0.0),
                0.0,
                (ANCHO_MENU + 2.0 * MARGEN_ENERGIA).min(w),
                320.0_f32.min(h),
            ),
            Capa::Energia => return None,
        };
        Some((x, y, ancho, alto.min((h - y).max(1.0))))
    }

    /// La hora, la batería y la red de ahora. `true` si hay que repintar.
    ///
    /// Sin esto la hora se quedaba en la del momento de bloquear: se fijaba al
    /// crear el bloqueo y nadie volvía a tocarla.
    // añadido en la 0.2.0
    pub fn refrescar(&mut self, hora: String, fecha: String) -> bool {
        let bateria = crate::Battery::read(false).map(|b| crate::bateria_sobre_fondo(&b));
        let red = crate::Network::read();
        let cifra = |b: &Option<(crate::icono::Icono, (f32, f32), String, Color)>| {
            b.as_ref().map(|(_, _, c, t)| (c.clone(), *t))
        };
        let cambio = self.hora != hora
            || self.fecha != fecha
            || cifra(&self.bateria) != cifra(&bateria)
            || self.red != red;
        self.hora = hora;
        self.fecha = fecha;
        self.bateria = bateria;
        self.red = red;
        cambio
    }

    // añadido en la 0.2.0
    pub fn entradas(&self) -> [f32; 4] {
        self.entrada.map(|m| m.valor().clamp(0.0, 1.0))
    }

    // añadido en la 0.2.0
    pub fn avanzar_entrada(&mut self, ahora: Instant) -> bool {
        if self.vuelta_pendiente {
            self.devolver_hundido(ahora);
        }
        let animada = self.entrada_animada();
        let mut cambio = false;
        for (indice, muelle) in self.entrada.iter_mut().enumerate() {
            let retraso = Duration::from_millis([0, 90, 180, 240][indice]);
            if !animada {
                cambio |= muelle.valor() != 1.0;
                muelle.fijar(1.0);
            } else if ahora.duration_since(self.entrada_desde) < retraso {
                muelle.fijar(0.0);
                muelle.ir_a(1.0);
            } else {
                cambio |= muelle.avanzar(ahora);
            }
        }
        cambio
    }

    // añadido en la 0.2.0
    pub fn entrada_animada(&self) -> bool {
        self.config.animaciones && !tema::efectos_reducidos()
    }

    fn componer(&self, pantalla: (f32, f32), capa: Option<Capa>) -> PanelElement<'_> {
        let (origen_y, ancho, alto) = capa
            .and_then(|c| self.geometria_capa(c, pantalla))
            .map_or((0.0, pantalla.0, pantalla.1), |(_, y, w, h)| (y, w, h));
        // modificado en la 0.2.0 -- solo la previsualización completa hornea la entrada en píxeles.
        let reloj_avance = if capa.is_none() {
            self.entradas()[0]
        } else {
            1.0
        };
        let acceso_avance = if capa.is_none() {
            self.entradas()[1]
        } else {
            1.0
        };
        let medios_avance = if capa.is_none() {
            self.medio_avance()
        } else {
            1.0
        };
        let energia_avance = if capa.is_none() {
            self.entradas()[3]
        } else {
            1.0
        };

        let familia = iced_core::font::Family::Name(self.config.reloj_fuente.familia());
        let mut reloj = column![
            text(self.hora.clone())
                .size(self.config.reloj_tamano)
                .font(iced_core::Font {
                    family: familia,
                    weight: grosor(self.config.reloj_grosor),
                    ..iced_core::Font::DEFAULT
                })
                .color(Color {
                    a: reloj_avance,
                    ..Color::WHITE
                })
                .align_x(Horizontal::Center)
                .width(Length::Fill)
        ]
        .align_x(Horizontal::Center);
        if self.config.fecha {
            reloj = reloj.push(
                text(self.fecha.clone())
                    .size(FECHA)
                    .font(iced_core::Font {
                        family: familia,
                        weight: iced_core::font::Weight::Medium,
                        ..iced_core::Font::DEFAULT
                    })
                    .color(Color {
                        a: 0.75 * reloj_avance,
                        ..Color::WHITE
                    })
                    .align_x(Horizontal::Center)
                    .width(Length::Fill),
            );
        }
        let reloj_y = pantalla.1 * self.config.reloj_y + (1.0 - reloj_avance) * 22.0;
        let capa_reloj = column![
            Space::new().height(Length::Fixed((reloj_y - origen_y).max(0.0))),
            container(reloj)
                .width(Length::Fixed(ancho))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto));

        let avatar = self.avatar_tamano(pantalla.1) * (0.90 + 0.10 * acceso_avance);
        let mut acceso = column![self.avatar(avatar, acceso_avance)];
        if self.config.nombre {
            acceso = acceso.push(Space::new().height(Length::Fixed(12.0))).push(
                text(self.usuario.nombre.clone())
                    .size(20.0)
                    .font(iced_core::Font {
                        weight: iced_core::font::Weight::Semibold,
                        ..iced_core::Font::DEFAULT
                    })
                    .color(Color {
                        a: acceso_avance,
                        ..Color::WHITE
                    }),
            );
        }
        let acceso = acceso
            .extend([
                Space::new().height(Length::Fixed(16.0)).into(),
                self.campo(acceso_avance),
                Space::new().height(Length::Fixed(10.0)).into(),
                self.mensaje(acceso_avance),
                self.aviso_huella(acceso_avance),
            ])
            .align_x(Horizontal::Center);

        let sacudida = self.sacudida();
        let acceso_desplazado = row![
            Space::new().width(Length::Fixed((sacudida * 2.0).max(0.0))),
            acceso,
            Space::new().width(Length::Fixed((-sacudida * 2.0).max(0.0))),
        ]
        .align_y(Vertical::Center);
        let acceso_y = self.acceso_y(pantalla.1) + (1.0 - acceso_avance) * 30.0;
        let capa_acceso = column![
            Space::new().height(Length::Fixed((acceso_y - origen_y).max(0.0))),
            container(acceso_desplazado)
                .width(Length::Fixed(ancho))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto));

        let mut capas = iced_widget::stack![];
        if capa.is_none_or(|c| c == Capa::Reloj) {
            capas = capas.push(capa_reloj);
        }
        if capa.is_none_or(|c| c == Capa::Acceso) {
            capas = capas.push(capa_acceso);
        }
        if capa.is_none_or(|c| c == Capa::Medios) && self.config.medios && !self.medios.is_empty() {
            let mut tarjetas = column![].align_x(Horizontal::Center);
            for medio in self.medios.iter().take(2) {
                tarjetas = tarjetas.push(self.tarjeta(medio, medios_avance));
                tarjetas = tarjetas.push(Space::new().height(Length::Fixed(10.0)));
            }
            let medios_y = self.medios_y_actual(pantalla.1, medios_avance);
            let capa_medios = column![
                Space::new().height(Length::Fixed((medios_y - origen_y).max(0.0))),
                container(tarjetas)
                    .width(Length::Fixed(ancho))
                    .center_x(Length::Fill),
            ]
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto));
            capas = capas.push(capa_medios);
        }

        if capa.is_none_or(|c| c == Capa::Energia) && self.config.energia {
            let esquina = container(self.energia(energia_avance))
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(alto))
                .align_x(Horizontal::Right)
                .align_y(Vertical::Top);
            capas = capas.push(esquina);
        }
        if capa.is_none_or(|c| c == Capa::Energia)
            && let Some((_, c)) = &self.confirmacion
        {
            capas = capas.push(
                container(c.view())
                    .center_x(Length::Fixed(ancho))
                    .center_y(Length::Fixed(alto))
                    .style(|_| container::Style {
                        background: Some(
                            Color {
                                a: 0.45,
                                ..Color::BLACK
                            }
                            .into(),
                        ),
                        ..Default::default()
                    }),
            );
        }
        capas.into()
    }

    /// Del avatar al renglón de la huella, que se reserva aunque esté vacío.
    // added 0.2.3
    fn alto_acceso(&self, alto_pantalla: f32) -> f32 {
        self.avatar_tamano(alto_pantalla) + Self::BAJO_AVATAR
    }

    // added 0.2.3
    const BAJO_AVATAR: f32 = 12.0 + ALTO_NOMBRE + 16.0 + ALTO_CAMPO + 10.0 + ALTO_MENSAJE + ALTO_HUELLA;

    /// Dónde empieza el bloque de acceso: la fracción de la configuración,
    /// subida lo justo para que quepa debajo la tarjeta de medios.
    ///
    /// A 1280×800 lógicos el bloque acaba en 642 con `acceso_y` 0,435 y la
    /// tarjeta, de 208, no cabía: el piso le ganaba al techo y se salía por
    /// abajo. Depende de que los medios estén activados y no de que suene
    /// algo, para que el campo no se mueva bajo el dedo al empezar una
    /// canción.
    // added 0.2.3
    fn acceso_y(&self, alto_pantalla: f32) -> f32 {
        let y = alto_pantalla * self.config.acceso_y;
        if !self.config.medios {
            return y;
        }
        y.min(alto_pantalla - Self::BAJO_ACCESO - self.alto_acceso(alto_pantalla))
    }

    /// Lo que va bajo el bloque de acceso cuando hay medios: aire, tarjeta y
    /// margen inferior.
    // added 0.2.3
    const BAJO_ACCESO: f32 = MARGEN_MEDIOS + MEDIOS_ALTO + MARGEN_INFERIOR;

    /// El avatar de la configuración, encogido lo justo para que el bloque de
    /// acceso, al subir para dejar sitio a la tarjeta, no pise la fecha. A
    /// 1280×800 con el reloj de 174 el avatar de 132 tapaba la fecha; queda
    /// en 74. No baja de 72: antes que una foto del tamaño de un sello, que
    /// se apriete el conjunto.
    // added 0.2.3
    fn avatar_tamano(&self, alto_pantalla: f32) -> f32 {
        let completo = self.config.avatar_tamano;
        if !self.config.medios {
            return completo;
        }
        let fin_reloj = alto_pantalla * self.config.reloj_y
            + self.config.reloj_tamano * 1.3
            + if self.config.fecha { FECHA * 1.3 } else { 0.0 }
            + 16.0;
        let cabe = alto_pantalla - Self::BAJO_ACCESO - Self::BAJO_AVATAR - fin_reloj;
        completo.min(cabe).max(72.0_f32.min(completo))
    }

    /// Dónde empieza la tarjeta de medios en reposo, sin el término de la
    /// animación de entrada.
    ///
    /// Nunca por debajo de donde termina el bloque de acceso. `medios_y` es
    /// una fracción fija de la pantalla y el bloque de acceso mide un número
    /// fijo de píxeles: en una pantalla lo bastante baja —1280×800 lógicos, la
    /// de este portátil a escala 1,75— la fracción caía **dentro** del bloque
    /// de acceso y la tarjeta tapaba el aviso de la contraseña. En una
    /// pantalla más alta este piso no cambia nada, porque la fracción ya cae
    /// más abajo que él.
    ///
    /// No es la posición donde se dibuja ni donde se prueba el clic: esa es
    /// [`Self::medios_y_actual`], que le suma el término de la animación de
    /// entrada. Esta existe aparte para poder fijar el piso sin depender de
    /// en qué punto de la animación está, que es lo único que hace falta en
    /// los tests de abajo.
    // 0.1.0
    fn medios_y_asentada(&self, alto_pantalla: f32) -> f32 {
        let acceso_fin = self.acceso_y(alto_pantalla) + self.alto_acceso(alto_pantalla);
        let piso = acceso_fin + MARGEN_MEDIOS;
        // modified 0.2.3 -- techo en el centro del hueco, no a 24 del borde
        // Tampoco baja más que el centro del hueco que deja el acceso: con
        // `medios_y` = 0,856 se quedaba a 24 px del borde y se veía pegada
        // abajo. Centrada, el aire de arriba y el de abajo son el
        // mismo. El piso manda sobre este techo, porque tapar la contraseña
        // es peor.
        let techo = acceso_fin + (alto_pantalla - acceso_fin - MEDIOS_ALTO) / 2.0;
        (alto_pantalla * self.config.medios_y).min(techo).max(piso)
    }

    /// Dónde está la tarjeta de medios **ahora mismo**, con la animación de
    /// entrada incluida: es lo que pintan `view()` y lo que prueba
    /// `medio_en()`, y tiene que ser la misma cuenta en los dos sitios.
    ///
    /// Antes no lo era: `view()` sumaba `(1.0 - medios_avance) * 28.0` a mano
    /// y `medio_en()` llamaba a [`Self::medios_y_asentada`] sin ese término,
    /// así que durante los primeros ~500 ms tras echar el bloqueo la tarjeta
    /// se dibujaba hasta 28 px más abajo de donde el clic la buscaba: tocar
    /// play nada más pulsar Meta+L no hacía nada hasta que la entrada
    /// terminaba de asentarse.
    // 0.1.0
    fn medios_y_actual(&self, alto_pantalla: f32, medios_avance: f32) -> f32 {
        self.medios_y_asentada(alto_pantalla) + (1.0 - medios_avance) * 28.0
    }

    // 0.1.0
    pub fn medio_avance(&self) -> f32 {
        if !self.entrada_animada() {
            return 1.0;
        }
        self.medio_desde.map_or(0.0, |desde| {
            tema::C_ENTRADA.eval(tema::fraccion(desde.elapsed(), Duration::from_millis(500)))
        })
    }

    // 0.1.0
    fn sacudida(&self) -> f32 {
        // La misma sacudida para la contraseña mala y para el dedo que no
        // casa: las dos dicen «no eres tú», y decirlo de dos formas haría
        // pensar que una es más grave.
        let desde = if self.estado == Estado::Fallo {
            self.estado_desde
        } else if self.huella == Huella::NoCoincide {
            self.huella_desde
        } else {
            return 0.0;
        };
        if !self.config.animaciones {
            return 0.0;
        }
        let t = tema::fraccion(desde.elapsed(), ESTADO_TOTAL);
        (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t) * 11.0
    }

    /// Cambia lo que dice la huella. `true` si hay que repintar.
    // 0.1.0
    pub fn actualizar_huella(&mut self, huella: Huella) -> bool {
        // Un segundo «no coincide» o reintento de lectura vuelve a pulsar: es otro toque.
        if self.huella == huella && !matches!(huella, Huella::NoCoincide | Huella::Aviso(_)) {
            return false;
        }
        self.huella = huella;
        self.huella_desde = Instant::now();
        true
    }

    /// Pulso breve (0.0 en reposo, 1.0 en pico) tras una lectura del sensor.
    fn pulso_huella(&self) -> f32 {
        if !self.config.animaciones || self.huella == Huella::Callada {
            return 0.0;
        }
        let duracion = match self.huella {
            Huella::NoCoincide => ESTADO_TOTAL,
            Huella::Reconocida => D_RECONOCIDA,
            Huella::Aviso(_) => PULSO_HUELLA,
            Huella::Callada => return 0.0,
        };
        let elapsed = self.huella_desde.elapsed();
        if elapsed >= duracion {
            return 0.0;
        }
        let t = tema::fraccion(elapsed, duracion);
        match self.huella {
            // En reconocimiento sube rápido hasta 1.0 y se mantiene lleno antes de salir.
            Huella::Reconocida => tema::C_ENTRADA.eval((t / 0.55).min(1.0)),
            _ => (t * std::f32::consts::PI).sin().clamp(0.0, 1.0),
        }
    }

    // 0.1.0
    fn aviso_huella<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let (texto, senal) = match self.huella {
            Huella::Callada => ("", None),
            Huella::Aviso(texto) => (texto, None),
            // En rojo como la contraseña mala, y diciendo que PAM sigue
            // escuchando: no hace falta hacer nada más que volver a poner el
            // dedo.
            Huella::NoCoincide => (
                t(
                    "Huella no reconocida. Vuelve a intentarlo.",
                    "Fingerprint not recognized. Try again.",
                ),
                Some(tema::rojo()),
            ),
            Huella::Reconocida => (
                t("Huella reconocida", "Fingerprint recognized"),
                Some(tema::verde()),
            ),
        };
        if texto.is_empty() {
            return container(Space::new().height(Length::Fixed(ALTO_MENSAJE)))
                .height(Length::Fixed(ALTO_HUELLA))
                .align_y(Vertical::Bottom)
                .into();
        }
        let pulso = self.pulso_huella();
        let reconocida = self.huella == Huella::Reconocida;
        let color_acento = senal.unwrap_or_else(tema::acento);
        let icono_px = 18.0 + if reconocida { 5.0 * pulso } else { 4.0 * pulso };
        let color_icono = match senal {
            Some(c) => con_alfa(c, alfa),
            None => con_alfa(
                tema::mezclar(tinta_campo(), tema::acento(), 0.75 * pulso),
                (0.85 + 0.15 * pulso) * alfa,
            ),
        };
        let insignia_fondo = if senal.is_some() {
            con_alfa(color_acento, (0.20 + 0.22 * pulso) * alfa)
        } else {
            con_alfa(tema::acento(), 0.24 * pulso * alfa)
        };
        let insignia_borde = if senal.is_some() {
            con_alfa(color_acento, (0.55 + 0.45 * pulso) * alfa)
        } else {
            con_alfa(tema::acento(), 0.65 * pulso * alfa)
        };
        let insignia = container(crate::icono::ver_teñido_propio(
            &self.icono_huella,
            icono_px,
            color_icono,
        ))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(28.0))
        .center_x(Length::Fixed(28.0))
        .center_y(Length::Fixed(28.0))
        .style(move |_| container::Style {
            background: Some(insignia_fondo.into()),
            border: Border {
                radius: 14.0.into(),
                width: if senal.is_some() || pulso > 0.04 {
                    1.5
                } else {
                    0.0
                },
                color: insignia_borde,
            },
            ..Default::default()
        });
        let fila = row![
            insignia,
            text(texto).size(13).color(con_alfa(tinta_campo(), alfa)),
        ]
        .spacing(8.0)
        .align_y(Vertical::Center);
        let pildora = container(fila)
            .height(Length::Fixed(30.0))
            .center_y(Length::Fixed(30.0))
            .padding(iced_core::Padding::ZERO.left(4.0).right(12.0))
            .style(move |_| container::Style {
                background: Some(con_alfa(fondo_campo(), alfa).into()),
                border: Border {
                    radius: 15.0.into(),
                    width: if senal.is_some() || pulso > 0.04 {
                        1.2
                    } else {
                        0.0
                    },
                    color: match senal {
                        Some(c) => con_alfa(c, (0.50 + 0.45 * pulso) * alfa),
                        None => con_alfa(tema::acento(), 0.55 * pulso * alfa),
                    },
                },
                ..Default::default()
            });
        container(pildora)
            .height(Length::Fixed(ALTO_HUELLA))
            .align_y(Vertical::Bottom)
            .into()
    }

    // 0.1.0
    pub fn actualizar_estado(&mut self, escritos: usize, estado: Estado) -> bool {
        if self.escritos == escritos && self.estado == estado {
            return false;
        }
        if self.estado != estado || (self.escritos == 0) != (escritos == 0) {
            self.estado_desde = Instant::now();
        }
        self.escritos = escritos;
        self.estado = estado;
        true
    }

    // 0.1.0
    pub fn actualizar_caps_lock(&mut self, activo: bool) -> bool {
        if self.caps_lock == activo {
            return false;
        }
        self.caps_lock = activo;
        true
    }

    /// Actualiza la tarjeta multimedia cuando termina la consulta asíncrona.
    // 0.1.0
    pub fn poner_medio(&mut self, sonando: Option<crate::medios::Sonando>) -> bool {
        let nuevos = if self.config.medios {
            sonando
                .map(|s| Medio::desde_sonando(s, self.medios.first()))
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        if self.medios == nuevos {
            return false;
        }
        let cancion = |m: &[Medio]| m.first().map(|m| (m.bus.clone(), m.titulo.clone()));
        let otra = cancion(&self.medios) != cancion(&nuevos);
        self.medios = nuevos;
        // Una tarjeta que llega unas décimas después del bloqueo entra desde
        // cero, no aparece ya terminada en mitad de la pantalla. Solo al
        // llegar otra canción: con la lectura de cada segundo, la misma
        // volvía a entrar a cada tic.
        if otra {
            self.medio_desde = Some(Instant::now());
        }
        true
    }

    /// Aleatorio y repetición del reproductor, de la misma lectura que
    /// [`Self::poner_medio`]. `true` si cambia algo.
    // añadido en la 0.2.3
    pub fn poner_modos(
        &mut self,
        (aleatorio, repetir): (Option<bool>, Option<crate::medios::Repetir>),
    ) -> bool {
        let antes = (self.aleatorio, self.repetir);
        self.aleatorio = aleatorio;
        self.repetir = repetir;
        antes != (aleatorio, repetir)
    }

    /// Si van los cinco mandos del Player o los tres de siempre.
    // añadido en la 0.2.3
    fn con_modos(&self) -> bool {
        self.aleatorio.is_some() || self.repetir.is_some()
    }

    /// El renglón bajo el campo: qué está pasando con lo que se ha escrito.
    ///
    /// Ocupa sitio siempre, aunque no diga nada: si apareciera y desapareciera,
    /// el campo daría un salto de veinte píxeles justo al pulsar Intro.
    // 0.1.0
    // modified 0.1.1 -- no repite «introduce tu contraseña» con el aviso de la huella
    // modificado en la 0.2.3 -- sobre píldora, con el color en una señal aparte
    fn mensaje<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let (texto_estado, senal) = match self.estado {
            Estado::Comprobando => (t("Comprobando…", "Checking…").to_string(), None),
            Estado::Fallo => (
                t("Contraseña incorrecta", "Incorrect password").to_string(),
                Some(tema::rojo()),
            ),
            Estado::Espera(segundos) => (
                if segundos == 1 {
                    t(
                        "Espera un segundo para volver a intentarlo",
                        "Wait a second to try again",
                    )
                    .to_string()
                } else if t("es", "en") == "es" {
                    format!("Espera {segundos} segundos para volver a intentarlo")
                } else {
                    format!("Wait {segundos} seconds to try again")
                },
                Some(tema::rojo()),
            ),
            Estado::Escribiendo if self.caps_lock => (
                t("Bloq Mayús está activado", "Caps Lock is on").to_string(),
                Some(tema::amarillo()),
            ),
            // El aviso de la huella ya dice «o introduce tu contraseña»; con los
            // dos renglones seguidos el mismo ruego salía dos veces.
            Estado::Escribiendo
                if self.escritos == 0 && !matches!(self.huella, Huella::Aviso(_)) =>
            {
                (
                    t(
                        "Introduce tu contraseña para desbloquear",
                        "Enter your password to unlock",
                    )
                    .to_string(),
                    None,
                )
            }
            Estado::Escribiendo => (String::new(), None),
        };
        aviso(texto_estado, senal, alfa)
    }

    /// Dónde cae el botón de apagado, en lógicos y para la pantalla dada.
    ///
    /// Se calcula aquí y no se guarda: el tamaño de la pantalla lo trae quien
    /// dibuja, y dos copias de la misma medida es como se desalinean el dibujo
    /// y el clic.
    // 0.1.0
    // modificado en la 0.2.0 -- arriba a la derecha, donde el greeter pone sus opciones.
    pub fn boton_energia(&self, pantalla: (f32, f32)) -> (f32, f32, f32, f32) {
        (
            pantalla.0 - MARGEN_ENERGIA - LADO_ENERGIA,
            MARGEN_ENERGIA,
            LADO_ENERGIA,
            LADO_ENERGIA,
        )
    }

    /// Y dónde cae cada entrada del menú desplegado, de arriba abajo.
    // 0.1.0
    fn entrada_energia(&self, i: usize, pantalla: (f32, f32)) -> (f32, f32, f32, f32) {
        let (_, y_boton, _, _) = self.boton_energia(pantalla);
        // Cuelga del botón con su hueco, alineado a su borde derecho.
        let x = pantalla.0 - MARGEN_ENERGIA - ANCHO_MENU;
        let y0 = y_boton + LADO_ENERGIA + HUECO_MENU;
        (x, y0 + i as f32 * ALTO_ENTRADA, ANCHO_MENU, ALTO_ENTRADA)
    }

    /// Un clic en la pantalla de bloqueo.
    ///
    /// Devuelve la petición cuando se elige una entrada del menú. Pulsar el
    /// botón lo despliega o lo recoge, y pulsar en cualquier otro sitio lo
    /// cierra: un menú que se queda abierto al pulsar fuera es el que acaba
    /// apagando el equipo sin querer.
    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32, pantalla: (f32, f32)) -> (Option<Peticion>, bool) {
        if let Some((peticion, c)) = &self.confirmacion {
            let (cw, ch) = c.size();
            let resultado = c.pulsar(
                x - (pantalla.0 - cw) / 2.0,
                y - (pantalla.1 - ch) / 2.0,
            );
            let accion = resultado.filter(|si| *si).map(|_| *peticion);
            if resultado.is_some() {
                self.confirmacion = None;
            }
            return (accion, resultado.is_some());
        }
        let dentro =
            |r: (f32, f32, f32, f32)| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3;
        if self.menu {
            for (i, (_, _, peticion)) in entradas().iter().enumerate() {
                if dentro(self.entrada_energia(i, pantalla)) {
                    self.cerrar_menu();
                    let Some(energia) = peticion.energia() else {
                        return (Some(*peticion), true);
                    };
                    self.confirmacion =
                        Some((*peticion, crate::confirmacion::Confirmacion::new(energia)));
                    return (None, true);
                }
            }
        }
        if self.config.energia && dentro(self.boton_energia(pantalla)) {
            if self.menu {
                self.cerrar_menu();
            } else {
                self.menu = true;
                self.menu_desde = Some((std::time::Instant::now(), true));
            }
            return (None, true);
        }
        if self.en_barra(x, y, pantalla) {
            self.arrastre = Some(self.fraccion_barra(x, pantalla));
            self.engordar_barra();
            return (None, true);
        }
        // modified 0.2.3 -- se hunde al apretar y actúa al soltar
        let mando = self
            .lateral_en(x, y, pantalla)
            .map(Mando::Lateral)
            .or_else(|| self.medio_en(x, y, pantalla).map(Mando::Orden));
        if let Some(mando) = mando {
            self.apretado = Some((mando, (x, y)));
            self.hundido_cual = Some(mando);
            self.hundido.ir_a(1.0);
            self.hundido_desde = Instant::now();
            self.vuelta_pendiente = false;
            return (None, true);
        }
        let estaba_abierto = self.menu;
        self.cerrar_menu();
        (None, estaba_abierto)
    }

    /// Lo que hace un mando al soltarlo. `true` si cambió algo que se ve.
    // added 0.2.3 -- antes iba dentro de `pulsar`, al apretar
    fn accionar(&mut self, mando: Mando) -> bool {
        if let Mando::Lateral(lateral) = mando
            && let Some(medio) = self.medios.first()
        {
            // Sin esperar a releerlo, como el play: la lectura de cada
            // segundo lo confirma o lo corrige.
            return match (lateral, self.aleatorio, self.repetir) {
                (Lateral::Aleatorio, Some(a), _)
                    if crate::medios::poner_aleatorio(&medio.bus, !a) =>
                {
                    self.aleatorio = Some(!a);
                    true
                }
                (Lateral::Repetir, _, Some(r))
                    if crate::medios::poner_repetir(&medio.bus, r.siguiente()) =>
                {
                    self.repetir = Some(r.siguiente());
                    true
                }
                _ => false,
            };
        }
        if let Mando::Orden(orden) = mando
            && let Some(medio) = self.medios.first_mut()
        {
            let ejecutada = crate::medios::ejecutar_en(&medio.bus, orden);
            if orden == crate::medios::Orden::Alternar && ejecutada {
                medio.reproduciendo = !medio.reproduciendo;
            }
            return ejecutada;
        }
        false
    }

    // 0.1.0
    fn medio_en(&self, x: f32, y: f32, pantalla: (f32, f32)) -> Option<crate::medios::Orden> {
        if !self.config.medios || self.medios.is_empty() {
            return None;
        }
        let (x, y) = self.en_tarjeta(x, y, pantalla);
        if !(MEDIOS_CONTROLES_Y..=MEDIOS_CONTROLES_Y + MEDIOS_CONTROL).contains(&y) {
            return None;
        }
        let centro = MEDIOS_ANCHO / 2.0;
        let centros = [centro - MEDIOS_PASO, centro, centro + MEDIOS_PASO];
        let i = centros
            .iter()
            .position(|cx| (x - cx).abs() <= MEDIOS_CONTROL / 2.0)?;
        Some(
            [
                crate::medios::Orden::Anterior,
                crate::medios::Orden::Alternar,
                crate::medios::Orden::Siguiente,
            ][i],
        )
    }

    /// Aleatorio o repetir bajo el punto, a dos pasos del centro; solo con
    /// los cinco mandos.
    // añadido en la 0.2.3
    fn lateral_en(&self, x: f32, y: f32, pantalla: (f32, f32)) -> Option<Lateral> {
        if !self.config.medios || self.medios.is_empty() || !self.con_modos() {
            return None;
        }
        let (x, y) = self.en_tarjeta(x, y, pantalla);
        if !(MEDIOS_CONTROLES_Y..=MEDIOS_CONTROLES_Y + MEDIOS_CONTROL).contains(&y) {
            return None;
        }
        let centro = MEDIOS_ANCHO / 2.0;
        let cerca = |cx: f32| (x - cx).abs() <= MEDIOS_CONTROL / 2.0;
        if cerca(centro - 2.0 * MEDIOS_PASO) && self.aleatorio.is_some() {
            Some(Lateral::Aleatorio)
        } else if cerca(centro + 2.0 * MEDIOS_PASO) && self.repetir.is_some() {
            Some(Lateral::Repetir)
        } else {
            None
        }
    }

    /// El punto relativo a la esquina de la tarjeta, tal como se ve ahora.
    // añadido en la 0.2.3
    fn en_tarjeta(&self, x: f32, y: f32, pantalla: (f32, f32)) -> (f32, f32) {
        (
            x - (pantalla.0 - MEDIOS_ANCHO) / 2.0,
            y - self.medios_y_actual(pantalla.1, self.medio_avance()),
        )
    }

    /// Qué fracción de la canción hay bajo `x`. La barra mide 5 de alto; se
    /// atiende la fila entera y 8 más a cada lado, o acertarla con el
    /// touchpad era cuestión de suerte.
    // añadido en la 0.2.3
    fn fraccion_barra(&self, x: f32, pantalla: (f32, f32)) -> f32 {
        let (x, _) = self.en_tarjeta(x, 0.0, pantalla);
        ((x - MEDIOS_BARRA_X) / MEDIOS_BARRA_W).clamp(0.0, 1.0)
    }

    // añadido en la 0.2.3
    fn en_barra(&self, x: f32, y: f32, pantalla: (f32, f32)) -> bool {
        let con_duracion = self.config.medios
            && self
                .medios
                .first()
                .is_some_and(|m| m.duracion.is_some_and(|d| d > 0));
        let (x, y) = self.en_tarjeta(x, y, pantalla);
        con_duracion
            && (MEDIOS_BARRA_Y - 8.0..=MEDIOS_BARRA_Y + MEDIOS_BARRA_FILA + 8.0).contains(&y)
            && (MEDIOS_BARRA_X - 8.0..=MEDIOS_BARRA_X + MEDIOS_BARRA_W + 8.0).contains(&x)
    }

    /// El puntero se mueve: arrastra la barra si está agarrada y, si no,
    /// sigue si está encima. `true` si hay que repintar; pasar por el resto
    /// de la pantalla no repinta nada.
    // añadido en la 0.2.3
    pub fn mover(&mut self, x: f32, y: f32, pantalla: (f32, f32)) -> bool {
        if let Some((_, (x0, y0))) = self.apretado {
            if (x - x0).hypot(y - y0) <= CANCELA_TOQUE {
                return false;
            }
            self.apretado = None;
            self.hundido.ir_a(0.0);
            return true;
        }
        if self.arrastre.is_some() {
            let f = self.fraccion_barra(x, pantalla);
            return self.arrastre.replace(f) != Some(f);
        }
        let encima = self.en_barra(x, y, pantalla);
        let cambia = std::mem::replace(&mut self.sobre_barra, encima) != encima;
        self.engordar_barra();
        cambia
    }

    /// ¿Cae `(x, y)` en algo de la tarjeta de música que responde al apoyar
    /// (los mandos y la barra)? Con el dedo, el compositor solo convierte el
    /// toque en clic al levantarlo, por si era un arrastre; aquí no hace falta
    /// esperar, porque los mandos ya se cancelan solos a los 10 pt.
    // added 0.2.3
    pub fn toque_directo(&self, x: f32, y: f32, pantalla: (f32, f32)) -> bool {
        self.en_barra(x, y, pantalla)
            || self.lateral_en(x, y, pantalla).is_some()
            || self.medio_en(x, y, pantalla).is_some()
    }

    /// Lleva el grosor de la barra a lo que piden el cursor y el agarre.
    // added 0.2.3
    fn engordar_barra(&mut self) {
        let gruesa = self.arrastre.is_some() || self.sobre_barra;
        self.grosor_barra.ir_a(if gruesa { 1.0 } else { 0.0 });
    }

    /// ¿Se mueve algo de la tarjeta de música: la barra engordando o un
    /// mando hundiéndose o volviendo?
    // added 0.2.3
    pub fn tarjeta_animando(&self) -> bool {
        self.grosor_barra.animando() || self.hundido.animando() || self.vuelta_pendiente
    }

    /// Devuelve el mando a su tamaño, o lo deja pendiente si aún no ha
    /// cumplido [`HUNDIDO_MINIMO`]: con el dedo, el compositor manda apretar
    /// y soltar juntos al levantarlo, y el hundido no llegaba a verse.
    // added 0.2.3
    fn devolver_hundido(&mut self, ahora: Instant) {
        self.vuelta_pendiente = ahora.duration_since(self.hundido_desde) < HUNDIDO_MINIMO;
        if !self.vuelta_pendiente {
            self.hundido.ir_a(0.0);
        }
    }

    /// Cuánto está hundido `mando` ahora, de 0 a 1.
    // added 0.2.3
    fn hundimiento(&self, mando: Mando) -> f32 {
        if self.hundido_cual == Some(mando) {
            self.hundido.valor().clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Suelta la barra: salta a donde quedó el dedo. `Seek` es relativo
    /// (ver [`crate::medios::saltar_en`]), así que se manda la diferencia con
    /// la última posición leída; con la lectura de cada segundo, el error es
    /// de menos de un segundo.
    // añadido en la 0.2.3
    pub fn soltar(&mut self) -> bool {
        if let Some((mando, _)) = self.apretado.take() {
            // La orden sale ya; lo que espera es solo la vuelta del icono.
            self.accionar(mando);
            self.devolver_hundido(Instant::now());
            return true;
        }
        let Some(f) = self.arrastre.take() else {
            return false;
        };
        self.engordar_barra();
        if let Some(m) = self.medios.first_mut()
            && let Some(d) = m.duracion
        {
            let objetivo = (f * d as f32).round() as u64;
            crate::medios::saltar_en(&m.bus, objetivo as i64 - m.posicion.unwrap_or(0) as i64);
            m.posicion = Some(objetivo);
            m.progreso = Some(f);
        }
        true
    }

    /// Cancela un toque sobre los mandos o la barra de medios sin disparar
    /// `accionar` ni `saltar_en`.
    pub fn cancelar_interaccion(&mut self) -> bool {
        let tenia_apretado = self.apretado.take().is_some();
        let tenia_arrastre = self.arrastre.take().is_some();
        if !tenia_apretado && !tenia_arrastre && !self.vuelta_pendiente {
            return false;
        }
        self.vuelta_pendiente = false;
        if tenia_apretado {
            self.hundido.ir_a(0.0);
        }
        if tenia_arrastre {
            self.engordar_barra();
        }
        true
    }

    // 0.1.0
    pub fn confirmar_tecla(&mut self, tecla: crate::TeclaPulsada) -> (bool, Option<Peticion>) {
        let Some((p, c)) = self.confirmacion.as_mut() else {
            return (false, None);
        };
        let resultado = c.tecla(tecla);
        let accion = resultado.filter(|si| *si).map(|_| *p);
        if resultado.is_some() {
            self.confirmacion = None;
        }
        (true, accion)
    }

    // 0.1.0
    fn cerrar_menu(&mut self) {
        if self.menu {
            self.menu = false;
            self.menu_desde = Some((std::time::Instant::now(), false));
        }
    }

    /// ¿Hay que seguir repintando el bloqueo?
    ///
    /// Mira si la animación **está en marcha**, no si existe: preguntar por
    /// `is_some()` deja al compositor dibujando para siempre.
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.contenido_animando()
            || self.tarjeta_animando()
            || (self.entrada_animada() && self.entrada.iter().any(|m| !m.quieto()))
            || (!self.medios.is_empty() && self.medio_avance() < 1.0)
    }

    // añadido en la 0.2.0 -- las animaciones de contenido necesitan píxeles nuevos; la entrada no.
    pub fn contenido_animando(&self) -> bool {
        if tema::efectos_reducidos() {
            return false;
        }
        let estado = self.config.animaciones
            && match self.estado {
                Estado::Fallo => self.estado_desde.elapsed() < ESTADO_TOTAL,
                Estado::Comprobando => self.estado_desde.elapsed() < D_OLA,
                _ => false,
            };
        let duracion_huella = match self.huella {
            Huella::NoCoincide => ESTADO_TOTAL,
            Huella::Reconocida => D_RECONOCIDA,
            Huella::Aviso(_) => PULSO_HUELLA,
            Huella::Callada => Duration::ZERO,
        };
        let huella = self.config.animaciones
            && self.huella != Huella::Callada
            && self.huella_desde.elapsed() < duracion_huella;
        estado
            || huella
            || self
                .menu_desde
                .is_some_and(|(t, _)| t.elapsed() < tema::D_POPOVER)
    }

    /// Cuánto lleva abierto el menú, de 0 (recogido) a 1 (del todo).
    ///
    /// Con el muelle del sistema de diseño, que **se pasa de 1 y vuelve**: es lo
    /// que separa «aparece» de «se despliega». Al cerrar recorre lo mismo del
    /// revés y sin rebote, porque un menú que rebota al irse se lee como que no
    /// se ha ido.
    // 0.1.0
    fn menu_avance(&self) -> f32 {
        match self.menu_desde {
            None => {
                if self.menu {
                    1.0
                } else {
                    0.0
                }
            }
            Some((desde, abriendo)) => {
                let t = tema::fraccion(desde.elapsed(), tema::D_POPOVER);
                if abriendo {
                    tema::C_MUELLE_POPOVER.eval(t)
                } else {
                    1.0 - tema::C_ENTRADA.eval(t)
                }
            }
        }
    }

    /// El botón de apagado y, si está desplegado, su menú.
    ///
    /// Arriba a la derecha, con el menú colgando debajo: el mismo sitio que las
    /// opciones del greeter, para que bloquear y entrar se manejen igual.
    // 0.1.0
    // modificado en la 0.2.0 -- arriba a la derecha y con glifos blancos
    fn energia<'a>(&'a self, alfa_entrada: f32) -> PanelElement<'a> {
        // Icono blanco sobre el fondo, con la sombra que evita perderlo en un
        // fondo claro, igual que la fila de estado del greeter.
        let pictograma = |icono: &crate::icono::Icono, lado: f32, alfa: f32| -> PanelElement<'a> {
            let capa = |tinta: Color, arriba: f32| {
                container(crate::icono::ver_teñido_propio(icono, lado, tinta)).padding(
                    iced_core::Padding {
                        top: arriba,
                        bottom: 1.0 - arriba,
                        ..iced_core::Padding::ZERO
                    },
                )
            };
            iced_widget::stack![
                capa(con_alfa(Color::BLACK, 0.35 * alfa), 1.0),
                capa(con_alfa(Color::WHITE, alfa), 0.0),
            ]
            .into()
        };
        let caja = |contenido: PanelElement<'a>| {
            container(contenido)
                .width(Length::Fixed(LADO_ENERGIA))
                .height(Length::Fixed(LADO_ENERGIA))
                .center_x(Length::Fixed(LADO_ENERGIA))
                .center_y(Length::Fixed(LADO_ENERGIA))
        };
        let abierto = self.menu;
        let mut boton = row![].spacing(6.0).align_y(Vertical::Center);
        if let Some((icono, (w, h), cifra, tinta)) = &self.bateria {
            let cifra = text(cifra.clone())
                .size(13.0)
                .font(iced_core::Font {
                    weight: iced_core::font::Weight::Semibold,
                    ..iced_core::Font::DEFAULT
                })
                .color(con_alfa(*tinta, alfa_entrada));
            boton = boton.push(
                container(
                    row![crate::icono::ver(icono, *w, *h), cifra]
                        .spacing(5.0)
                        .align_y(Vertical::Center),
                )
                .padding([0.0, 9.0]),
            );
        }
        // Sin red son los mismos arcos apagados, como en el greeter.
        let con_red = self.red.is_some_and(|r| r.up);
        boton = boton.push(caja(pictograma(
            &self.wifi,
            20.0,
            alfa_entrada * if con_red { 1.0 } else { 0.35 },
        )));
        boton = boton.push(caja(pictograma(&self.encendido, 20.0, alfa_entrada)).style(
            move |_| container::Style {
                background: abierto.then(|| con_alfa(Color::WHITE, 0.18).into()),
                border: Border {
                    radius: (LADO_ENERGIA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        ));

        // modificado en la 0.2.0 -- el menú cuelga del botón: el hueco de
        // arriba deja sitio al botón y el desplazamiento lo baja desde él.
        let mut columna = column![Space::new().height(Length::Fixed(LADO_ENERGIA))].spacing(0.0);
        // El menú se dibuja mientras el avance no sea cero: al cerrar hay que
        // seguir viéndolo mientras se va, y `self.menu` ya es `false` entonces.
        let avance = self.menu_avance();
        let mut desplazamiento_menu = 0.0;
        if avance > 0.001 {
            let alfa = avance.clamp(0.0, 1.0);
            let mut filas = column![];
            for (i, (_, icono, peticion)) in entradas().iter().enumerate() {
                // modificado en la 0.2.3 -- la etiqueta en el idioma de la sesión, como el greeter
                let etiqueta = match peticion {
                    Peticion::Apagar => t("Apagar", "Shut Down"),
                    Peticion::Reiniciar => t("Reiniciar", "Restart"),
                    Peticion::CerrarSesion => t("Cerrar sesión", "Log Out"),
                    Peticion::Suspender => t("Suspender", "Sleep"),
                    Peticion::Hibernar => t("Hibernar", "Hibernate"),
                    Peticion::CambiarUsuario => t("Cambiar usuario", "Switch User"),
                };
                let tinta = Color {
                    a: alfa,
                    ..tinta_campo()
                };
                // El fundido va en la opacidad: el tinte del SVG no respeta el
                // alfa del color y el glifo salía entero con el menú aún invisible.
                let dibujo: PanelElement<'a> = match crate::icono::propio(icono) {
                    Some(crate::icono::Icono::Svg(handle)) => iced_widget::svg(handle)
                        .width(Length::Fixed(ICONO_ENTRADA))
                        .height(Length::Fixed(ICONO_ENTRADA))
                        .style(|_, _| iced_widget::svg::Style {
                            color: Some(tinta_campo()),
                        })
                        .opacity(alfa)
                        .into(),
                    _ => Space::new().width(Length::Fixed(ICONO_ENTRADA)).into(),
                };
                let fila = row![
                    dibujo,
                    Space::new().width(Length::Fixed(12.0)),
                    // Medium y no Bold: los glifos son de trazo fino (1,5
                    // sobre 24) y al lado de una negrita parecían de otro
                    // juego; así pesa lo mismo la palabra que su icono.
                    text(etiqueta)
                        .size(15.0)
                        .color(tinta)
                        .font(iced_core::Font {
                            weight: iced_core::font::Weight::Medium,
                            ..iced_core::Font::DEFAULT
                        }),
                ]
                .align_y(Vertical::Center);
                // El divisor va **dentro** de la altura de la entrada, no entre
                // entradas: si sumara un píxel propio, el hit-test dejaría de
                // caer donde se dibuja a partir de la segunda.
                let ultima = i + 1 == entradas().len();
                let alto_fila = if ultima {
                    ALTO_ENTRADA
                } else {
                    ALTO_ENTRADA - 1.0
                };
                filas = filas.push(
                    container(fila)
                        .padding([0.0, 18.0])
                        .width(Length::Fixed(ANCHO_MENU))
                        .height(Length::Fixed(alto_fila))
                        .center_y(Length::Fixed(alto_fila)),
                );
                if !ultima {
                    filas = filas.push(
                        container(
                            container(Space::new().height(Length::Fixed(1.0)))
                                .width(Length::Fixed(ANCHO_MENU - 36.0))
                                .style(move |_| container::Style {
                                    background: Some(
                                        Color {
                                            a: 0.12 * alfa,
                                            ..tinta_campo()
                                        }
                                        .into(),
                                    ),
                                    ..Default::default()
                                }),
                        )
                        .width(Length::Fixed(ANCHO_MENU))
                        .center_x(Length::Fixed(ANCHO_MENU)),
                    );
                }
            }
            // Baja desde el botón y se desvanece: son las dos cosas que el
            // sistema de diseño deja animar. El desplazamiento sale del muelle,
            // así que se pasa un poco de su sitio y vuelve.
            // El rebote del muelle se gasta en el desplazamiento, no en la
            // altura: pasarse de alto dejaría un hueco vacío dentro del menú.
            desplazamiento_menu = (1.0 - avance) * 18.0;
            // Entero desde el primer fotograma, sin desplegarse. Antes crecía
            // recortando la altura, y el recorte enseñaba las **primeras**
            // filas en el sitio de las últimas: a medio abrir, «Apagar» estaba
            // dibujado donde el clic encuentra «Suspender». El HIG, además,
            // solo deja animar opacidad y desplazamiento.
            columna = columna.push(Space::new().height(Length::Fixed(HUECO_MENU)));
            columna = columna.push(
                container(filas)
                    .style(move |_| container::Style {
                        // Más opaco que el campo: es un menú de acciones que
                        // apagan el equipo, y leerlo no puede depender de lo que
                        // haya en el fondo de pantalla.
                        background: Some(
                            Color {
                                a: 0.94 * alfa,
                                ..fondo_campo()
                            }
                            .into(),
                        ),
                        border: Border {
                            radius: tema::R_TARJETA.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .clip(true)
                    .width(Length::Fixed(ANCHO_MENU)),
            );
        }

        let ancho = ANCHO_MENU + 2.0 * MARGEN_ENERGIA;
        let esquina = |contenido: PanelElement<'a>, arriba: f32| {
            container(contenido)
                .width(Length::Fixed(ancho))
                .align_x(Horizontal::Right)
                .padding(iced_core::Padding {
                    top: arriba,
                    right: MARGEN_ENERGIA,
                    bottom: 0.0,
                    left: MARGEN_ENERGIA,
                })
        };
        iced_widget::stack![
            esquina(
                columna.into(),
                (MARGEN_ENERGIA - desplazamiento_menu).max(0.0)
            ),
            esquina(boton.into(), MARGEN_ENERGIA),
        ]
        .into()
    }

    /// El círculo del usuario. Gris liso mientras no haya foto: una silueta
    /// genérica dice menos que un hueco limpio.
    // 0.1.0
    fn avatar(&self, lado: f32, alfa: f32) -> PanelElement<'_> {
        let dentro: PanelElement<'_> = match &self.usuario.foto {
            Some(handle) => iced_widget::image(handle.clone())
                .width(Length::Fixed(lado))
                .height(Length::Fixed(lado))
                // La foto se recorta al cuadrado y luego el contenedor la
                // redondea; sin `Cover` una foto apaisada saldría con franjas.
                .content_fit(iced_core::ContentFit::Cover)
                .into(),
            None => {
                let iniciales: String = self
                    .usuario
                    .nombre
                    .split_whitespace()
                    .filter_map(|p| p.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                text(iniciales)
                    .size(lado * 0.34)
                    .color(Color {
                        a: alfa,
                        ..Color::WHITE
                    })
                    .into()
            }
        };
        // Con música, el aro del avatar lleva el color de la carátula: ata la
        // identidad a lo que suena, igual que el velo de arriba. Sin música,
        // el blanco tenue de siempre.
        let aro = self.medios.first().map_or(
            Color {
                a: 0.35 * alfa,
                ..tema::tinta()
            },
            |m| Color {
                a: 0.85 * alfa,
                ..m.color
            },
        );
        let ancho_aro = if self.medios.is_empty() { 2.0 } else { 3.0 };
        let redondo = container(dentro)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .clip(true)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.55 * alfa,
                        ..tema::acento()
                    }
                    .into(),
                ),
                border: Border {
                    radius: (lado / 2.0).into(),
                    width: ancho_aro,
                    color: aro,
                },
                ..Default::default()
            });
        redondo.into()
    }

    /// Cuántos puntos caben en el campo sin salirse.
    // 0.1.0
    fn caben(&self) -> usize {
        // Quitando el candado con su aire (16 + 22 + 10) y el botón (36 + 6).
        ((CAMPO - 90.0) / (PUNTO + 6.0)) as usize
    }

    /// El campo de contraseña: una sola píldora con el candado, los puntos y
    /// el botón de entrar dentro.
    // 0.1.0
    fn campo<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let visibles = self.escritos.min(self.caben());
        let animar = self.config.animaciones;
        let ola = (animar && self.estado == Estado::Comprobando)
            .then(|| tema::fraccion(self.estado_desde.elapsed(), D_OLA))
            .filter(|t| *t < 1.0);
        let mut puntos = row![].spacing(6.0).align_y(Vertical::Center);
        // Un punto por carácter, hasta donde caben. Más allá no se añaden: una
        // fila que se sale del campo no dice nada que no diga ya.
        for i in 0..visibles {
            // Aparece de golpe, sin crecer. Antes nacía con un muelle de
            // 250 ms, y eso eran ~30 repintados a pantalla completa por tecla
            // —cada uno de 7 a 10 ms a escala 2—: escribiendo seguido la CPU
            // no paraba y las teclas esperaban detrás de los fotogramas, que
            // es lo que se veía como letras llegando poco a poco.
            let lado = PUNTO;
            // La ola sube cada punto un poco después que el anterior.
            let alza = ola.map_or(0.0, |t| {
                ((t * 2.0 - i as f32 * 0.08) * std::f32::consts::TAU)
                    .sin()
                    .max(0.0)
                    * 5.0
            });
            let punto = container(Space::new())
                .width(Length::Fixed(lado.max(0.0)))
                .height(Length::Fixed(lado.max(0.0)))
                .style(move |_| container::Style {
                    background: Some(con_alfa(tinta_campo(), alfa).into()),
                    border: Border {
                        radius: (lado / 2.0).max(0.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            puntos = puntos.push(
                container(punto)
                    .width(Length::Fixed(PUNTO))
                    .height(Length::Fixed(PUNTO + 10.0))
                    .center_x(Length::Fixed(PUNTO))
                    .padding(iced_core::Padding::ZERO.bottom(alza * 2.0))
                    .center_y(Length::Fixed(PUNTO + 10.0)),
            );
        }
        // El cursor va detrás del último punto y **no parpadea**: parpadear
        // obliga a repintar la pantalla entera dos veces por segundo, y este
        // escritorio no gasta despertares en eso. Quieto dice lo mismo.
        if self.estado != Estado::Comprobando {
            puntos = puntos.push(
                container(Space::new())
                    .width(Length::Fixed(2.0))
                    .height(Length::Fixed(ALTO_CAMPO * 0.42))
                    .style(move |_| container::Style {
                        background: Some(
                            Color {
                                a: 0.8 * alfa,
                                ..tinta_campo()
                            }
                            .into(),
                        ),
                        border: Border {
                            radius: 1.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        // El borde de la píldora es el foco: en el bloqueo no se puede ir a
        // ninguna otra parte, y sin él no hay nada que diga dónde va lo que se
        // escribe. En rojo cuando la contraseña no era.
        let fallo_huella =
            self.huella == Huella::NoCoincide && self.huella_desde.elapsed() < ESTADO_TOTAL;
        let borde = con_alfa(
            match self.estado {
                Estado::Fallo | Estado::Espera(_) => tema::rojo(),
                _ if fallo_huella => tema::rojo(),
                _ if self.huella == Huella::Reconocida => tema::verde(),
                Estado::Comprobando => Color {
                    a: 0.35,
                    ..tinta_campo()
                },
                _ => Color {
                    a: 0.85,
                    ..tema::acento()
                },
            },
            alfa,
        );
        // El botón se enciende con el acento cuando hay algo que mandar. Vacío
        // no lleva a ninguna parte, y un botón encendido que no hace nada se
        // prueba dos veces antes de mirar el teclado.
        let lleno = self.escritos > 0;
        let fondo_boton = if lleno {
            con_alfa(tema::acento(), alfa)
        } else {
            Color {
                a: 0.12 * alfa,
                ..tinta_campo()
            }
        };
        let tinta_flecha = if lleno {
            con_alfa(tema::tinta_sobre(tema::acento()), alfa)
        } else {
            con_alfa(tinta_campo(), 0.45 * alfa)
        };
        let boton = container(crate::icono::ver_teñido_propio(
            &self.flecha,
            18.0,
            tinta_flecha,
        ))
        .width(Length::Fixed(BOTON))
        .height(Length::Fixed(BOTON))
        .center_x(Length::Fixed(BOTON))
        .center_y(Length::Fixed(BOTON))
        .style(move |_| container::Style {
            background: Some(fondo_boton.into()),
            border: Border {
                radius: (BOTON / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let dentro = row![
            crate::icono::ver_teñido_propio(
                &self.candado,
                16.0,
                Color {
                    a: 0.6 * alfa,
                    ..tinta_campo()
                },
            ),
            Space::new().width(Length::Fixed(10.0)),
            puntos,
            Space::new().width(Length::Fill),
            boton,
        ]
        .align_y(Vertical::Center);
        container(dentro)
            .width(Length::Fixed(CAMPO))
            .height(Length::Fixed(ALTO_CAMPO))
            .padding(iced_core::Padding::ZERO.left(18.0).right(4.0))
            // Fijo y no `Fill`: `center_y(Fill)` **fija la altura a Fill**, no
            // solo centra, y el campo se estiraba hasta ocupar media pantalla.
            .center_y(Length::Fixed(ALTO_CAMPO))
            .style(move |_| container::Style {
                background: Some(con_alfa(fondo_campo(), alfa).into()),
                border: Border {
                    radius: (ALTO_CAMPO / 2.0).into(),
                    width: 2.0,
                    color: borde,
                },
                ..Default::default()
            })
            .into()
    }

    /// Una tarjeta de lo que está sonando.
    /// La tarjeta de «esto está sonando».
    ///
    /// El mismo cristal y la misma tinta que el campo de la contraseña
    /// ([`fondo_campo`], [`tinta_campo`]), con botones redondos y el acento
    /// solo en lo que está activo. Antes usaba `tema::card()` opaco y en claro
    /// salía una losa blanca bajo una píldora oscura: si sigue al tema, lo
    /// sigue con el campo, no por su cuenta.
    ///
    /// Los altos van fijos: `medio_en` busca los botones a una distancia
    /// conocida del borde de abajo, y con un alto que dependiera del texto el
    /// clic y el dibujo volverían a separarse.
    // 0.1.0
    // modified 0.1.1 -- tarjeta de música en blanco sobre cristal, sin seguir el tema
    // modificado en la 0.2.3 -- campo, menú y tarjeta siguen el tema claro
    fn tarjeta<'a>(&self, medio: &'a Medio, alfa: f32) -> PanelElement<'a> {
        use crate::actividad::{ANTERIOR, PAUSA, PLAY, SIGUIENTE};
        let blanco = move |a: f32| Color {
            a: a * alfa,
            ..tinta_campo()
        };
        let texto = blanco(1.0);
        let texto2 = blanco(0.6);

        let arte: PanelElement<'a> = match medio.caratula.clone() {
            Some(h) => iced_widget::image(h)
                .width(MEDIOS_ARTE)
                .height(MEDIOS_ARTE)
                .opacity(alfa)
                .into(),
            None => container(match crate::icono::propio("musica") {
                Some(ic) => crate::icono::ver_teñido_propio(&ic, 34.0, texto2),
                None => Space::new().width(Length::Fixed(34.0)).into(),
            })
            .width(Length::Fixed(MEDIOS_ARTE))
            .height(Length::Fixed(MEDIOS_ARTE))
            .center_x(Length::Fixed(MEDIOS_ARTE))
            .center_y(Length::Fixed(MEDIOS_ARTE))
            .style(move |_| container::Style {
                background: Some(blanco(0.08).into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
        };

        let linea = |cadena: &str, tamaño: f32, peso, color: Color, alto: f32| {
            container(
                text(crate::emergente::recortar_texto(
                    cadena,
                    MEDIOS_INFO,
                    tamaño,
                ))
                .size(tamaño)
                .font(iced_core::Font {
                    weight: peso,
                    ..iced_core::Font::DEFAULT
                })
                .wrapping(iced_core::text::Wrapping::None)
                .color(color),
            )
            .height(Length::Fixed(alto))
            .align_y(Vertical::Center)
        };
        use iced_core::font::Weight;
        let mut info = column![
            linea(&medio.titulo, tema::T_TITULO, Weight::Bold, texto, 24.0),
            linea(&medio.detalle, tema::T_CUERPO, Weight::Normal, texto2, 18.0),
        ];
        // La pastilla de la calidad, como «Opus | 134 kb/s · 48 kHz» del
        // Player: el nombre en negrita, el resto apagado.
        // Solo sin pérdida y solo la etiqueta, como Apple Music: «Opus | 134
        // kbps» no le dice nada a quien está delante del bloqueo, y «FLAC · 24
        // bits · 96 kHz» no cabía al lado y partía la línea. El detalle sigue
        // en el centro de control.
        let sin_perdida = medio.calidad.as_ref().and_then(|c| match c.nivel() {
            crate::medios::Nivel::HiRes => Some("Hi-Res Lossless"),
            crate::medios::Nivel::SinPerdida => Some("Lossless"),
            crate::medios::Nivel::ConPerdida => None,
        });
        if let Some(nombre) = sin_perdida {
            let fila = text(nombre)
                .size(tema::T_PEQUENO)
                .font(iced_core::Font {
                    weight: Weight::Semibold,
                    ..iced_core::Font::DEFAULT
                })
                .color(texto);
            info = info.push(Space::new().height(Length::Fixed(8.0))).push(
                container(fila)
                    .padding([4.0, 9.0])
                    .style(move |_| container::Style {
                        background: Some(blanco(0.1).into()),
                        border: Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        let arriba = row![
            arte,
            Space::new().width(Length::Fixed(14.0)),
            container(info.width(Length::Fixed(MEDIOS_INFO)))
                .height(Length::Fixed(MEDIOS_ARTE))
                .align_y(Vertical::Center),
        ];

        // Mientras se arrastra manda el dedo, tiempos incluidos.
        let progreso = self
            .arrastre
            .or(medio.progreso)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let posicion = match (self.arrastre, medio.duracion) {
            (Some(f), Some(d)) => Some((f * d as f32).round() as u64),
            _ => medio.posicion,
        };
        // Engorda bajo el cursor y agarrada, como la del centro de control:
        // dice que se puede arrastrar. Cabe en los 16 de la fila sin mover nada.
        let grosor = 5.0 + 5.0 * self.grosor_barra.valor().clamp(0.0, 1.0);
        let tramo = |ancho: f32, color: Color| {
            container(Space::new())
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(grosor))
                .style(move |_| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: (grosor / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let lleno = MEDIOS_BARRA_W * progreso;
        let tiempo = |cadena: String, derecha: bool| {
            container(text(cadena).size(tema::T_PEQUENO).color(texto2))
                .width(Length::Fixed(MEDIOS_TIEMPO))
                .align_x(if derecha {
                    Horizontal::Right
                } else {
                    Horizontal::Left
                })
        };
        let restante = match (posicion, medio.duracion) {
            (Some(p), Some(d)) => format!("-{}", crate::medios::reloj(d.saturating_sub(p))),
            (_, Some(d)) => crate::medios::reloj(d),
            _ => String::new(),
        };
        let barra = row![
            tiempo(
                posicion.map(crate::medios::reloj).unwrap_or_default(),
                false
            ),
            Space::new().width(Length::Fixed(8.0)),
            // modified 0.2.3 -- lleno en el color de la carátula, no la tinta
            // Del color de la carátula y no del acento, que sobre una carátula
            // de cualquier color competía con ella.
            // modified 0.2.3 -- una pista entera con lo recorrido encima
            // Encima de la pista entera y no al lado de lo que falta: dos
            // tramos redondeados se leían como dos barras, con un hueco entre
            // las puntas que se movía al avanzar.
            iced_widget::stack![
                tramo(MEDIOS_BARRA_W, blanco(0.18)),
                tramo(
                    lleno.max(grosor),
                    con_alfa(realce_medio(medio, fondo_medio(medio)), alfa)
                ),
            ],
            Space::new().width(Length::Fixed(8.0)),
            tiempo(restante, true),
        ]
        .height(Length::Fixed(MEDIOS_BARRA_FILA))
        .align_y(Vertical::Center);

        // Sin placa, como en el Player: el de reproducir manda por tamaño.
        // Apretado se hunde un 8 % y se apaga hacia el gris (HIG §8).
        let hundir = |cual: Mando, lado: f32, color: Color| {
            let h = self.hundimiento(cual);
            (lado * (1.0 - 0.08 * h), tema::mezclar(color, texto2, h))
        };
        let mando = |svg: &str, lado: f32, cual: Mando| -> PanelElement<'a> {
            let (lado, color) = hundir(cual, lado, texto);
            container(crate::icono::ver_teñido_propio(
                &crate::icono::desde_svg(svg),
                lado,
                color,
            ))
            .width(Length::Fixed(MEDIOS_PASO))
            .height(Length::Fixed(MEDIOS_CONTROL))
            .center_x(Length::Fixed(MEDIOS_PASO))
            .center_y(Length::Fixed(MEDIOS_CONTROL))
            .into()
        };
        // Tres celdas de `MEDIOS_PASO` centradas: los centros quedan donde
        // los busca `medio_en`.
        let centrales = row![
            mando(ANTERIOR, 22.0, Mando::Orden(crate::medios::Orden::Anterior)),
            mando(
                if medio.reproduciendo { PAUSA } else { PLAY },
                30.0,
                Mando::Orden(crate::medios::Orden::Alternar)
            ),
            mando(SIGUIENTE, 22.0, Mando::Orden(crate::medios::Orden::Siguiente)),
        ];
        // Con BookOS Player, aleatorio y repetir a los lados, como en el
        // Player: puestos, sobre una placa; quitados, en gris.
        let lateral =
            |icono: crate::icono::Icono, puesto: Option<bool>, uno: bool, cual: Lateral| -> PanelElement<'a> {
                let on = puesto == Some(true);
                let (lado, color) = hundir(Mando::Lateral(cual), 18.0, if on { texto } else { texto2 });
                let glifo = crate::icono::ver_teñido_propio(&icono, lado, color);
                let mut capa = iced_widget::stack![
                    container(glifo)
                        .width(Length::Fixed(36.0))
                        .height(Length::Fixed(36.0))
                        .center_x(Length::Fixed(36.0))
                        .center_y(Length::Fixed(36.0))
                        .style(move |_| container::Style {
                            background: on.then(|| blanco(0.14).into()),
                            border: Border {
                                radius: 18.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                ];
                // Repetir la canción lleva el «1», como en el centro de control.
                if uno {
                    capa = capa.push(
                        container(
                            container(
                                text("1")
                                    .size(8.0)
                                    .font(iced_core::Font {
                                        weight: Weight::Bold,
                                        ..iced_core::Font::DEFAULT
                                    })
                                    .color(con_alfa(fondo_campo(), alfa)),
                            )
                            .width(Length::Fixed(11.0))
                            .height(Length::Fixed(11.0))
                            .center_x(Length::Fixed(11.0))
                            .center_y(Length::Fixed(11.0))
                            .style(move |_| container::Style {
                                background: Some(texto.into()),
                                border: Border {
                                    radius: 5.5.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }),
                        )
                        .width(Length::Fixed(36.0))
                        .padding(iced_core::Padding {
                            top: 4.0,
                            right: 4.0,
                            ..Default::default()
                        })
                        .align_x(Horizontal::Right),
                    );
                }
                container(capa)
                    .width(Length::Fixed(MEDIOS_PASO))
                    .height(Length::Fixed(MEDIOS_CONTROL))
                    .center_x(Length::Fixed(MEDIOS_PASO))
                    .center_y(Length::Fixed(MEDIOS_CONTROL))
                    .into()
            };
        let fila: PanelElement<'a> = if self.con_modos() {
            use crate::medios::Repetir;
            row![
                lateral(
                    crate::icono::desde_svg(ALEATORIO_PLAYER),
                    self.aleatorio,
                    false,
                    Lateral::Aleatorio
                ),
                centrales,
                lateral(
                    crate::icono::propio_solido("repetir")
                        .expect("repetir va incluido en el binario"),
                    self.repetir.map(|r| r != Repetir::No),
                    self.repetir == Some(Repetir::Cancion),
                    Lateral::Repetir
                ),
            ]
            .into()
        } else {
            centrales.into()
        };
        let controles = container(fila).width(Length::Fill).center_x(Length::Fill);

        container(column![
            arriba,
            Space::new().height(Length::Fixed(MEDIOS_BARRA_Y - MEDIOS_PAD - MEDIOS_ARTE)),
            barra,
            Space::new().height(Length::Fixed(
                MEDIOS_CONTROLES_Y - MEDIOS_BARRA_Y - MEDIOS_BARRA_FILA
            )),
            controles,
        ])
        .width(Length::Fixed(MEDIOS_ANCHO))
        .height(Length::Fixed(MEDIOS_ALTO))
        .padding(MEDIOS_PAD)
        .style(move |_| container::Style {
            background: Some(con_alfa(fondo_medio(medio), alfa).into()),
            // El borde que el HIG pide a una tarjeta superpuesta, al 9 % del
            // texto: es lo que la separa del fondo de pantalla.
            border: Border {
                radius: tema::R_TARJETA.into(),
                width: 1.0,
                color: blanco(0.09),
            },
            // Sin sombra, a diferencia de las tarjetas del escritorio: iced la
            // calcula en CPU píxel a píxel en cada repintado, y el bloqueo se
            // repinta en cada tecla de la contraseña. Medido a escala 2 con
            // música, 16 ms de los 26 de cada tecla.
            ..Default::default()
        })
        .into()
    }
}

/// El aleatorio de BookOS Player (`#immShuffleBtn`): flechas cruzadas de
/// trazo. El sólido del sistema son dos flechas paralelas y, junto al del
/// Player, parecía otro botón.
// añadido en la 0.2.3
const ALEATORIO_PLAYER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"/><path d="M4 20 21 3"/><path d="M21 16v5h-5"/><path d="M15 15l6 6"/><path d="M4 4l5 5"/></svg>"##;

/// Un mando de la tarjeta de música: los tres del centro o los de los lados.
// added 0.2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mando {
    Orden(crate::medios::Orden),
    Lateral(Lateral),
}

/// Lo que puede moverse el puntero con un mando apretado antes de que el
/// toque se cancele (HIG §8: más de 10 pt).
// added 0.2.3
const CANCELA_TOQUE: f32 = 10.0;

/// Lo mínimo que se queda hundido un mando aunque se suelte antes. Con
/// `M_RAPIDO` (0,21 s) a los 120 ms lleva el 87 % del recorrido: se ve
/// el hundido sin retrasar nada, porque la orden ya ha salido.
// added 0.2.3
const HUNDIDO_MINIMO: Duration = Duration::from_millis(120);

/// Los dos mandos de los lados, que solo salen con BookOS Player.
// añadido en la 0.2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lateral {
    Aleatorio,
    Repetir,
}

/// Un renglón de estado bajo el campo, sobre el mismo cristal que el campo.
///
/// Iba como texto suelto sobre la foto y, en un fondo claro, el blanco al
/// 55 % de «Introduce tu contraseña» no se leía. El HIG (§2) pide 4,5:1 para
/// todo texto y una base opaca bajo el que va sobre fotos; el cristal del
/// campo da ~10:1 con su tinta sobre cualquier fondo. El rojo y el naranja del
/// tema no llegan a 4,5:1 sobre claro (3,6 y 2,1), así que el texto va en la
/// tinta y el color, en un punto delante: el significado lo lleva la frase.
// añadido en la 0.2.3
fn aviso<'a>(texto: String, senal: Option<Color>, alfa: f32) -> PanelElement<'a> {
    if texto.is_empty() {
        return Space::new().height(Length::Fixed(ALTO_MENSAJE)).into();
    }
    let mut fila = row![].spacing(7.0).align_y(Vertical::Center);
    if let Some(color) = senal {
        fila = fila.push(
            container(Space::new())
                .width(Length::Fixed(7.0))
                .height(Length::Fixed(7.0))
                .style(move |_| container::Style {
                    background: Some(con_alfa(color, alfa).into()),
                    border: Border {
                        radius: 3.5.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    fila = fila.push(text(texto).size(13).color(con_alfa(tinta_campo(), alfa)));
    container(fila)
        .height(Length::Fixed(ALTO_MENSAJE))
        .center_y(Length::Fixed(ALTO_MENSAJE))
        .padding([0.0, 12.0])
        .style(move |_| container::Style {
            background: Some(con_alfa(fondo_campo(), alfa).into()),
            border: Border {
                radius: (ALTO_MENSAJE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Conserva el color y modula su opacidad para las entradas escalonadas.
// 0.1.0
fn con_alfa(mut color: Color, alfa: f32) -> Color {
    color.a *= alfa.clamp(0.0, 1.0);
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los puntos no se salen del campo por larga que sea la contraseña.
    #[test]
    fn los_puntos_no_desbordan_el_campo() {
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes, 17 de agosto".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        b.escritos = 500;
        let caben = b.caben();
        assert!(caben > 8, "caben muy pocos puntos: {caben}");
        assert!(caben as f32 * (PUNTO + 6.0) <= CAMPO - 40.0 + PUNTO);
    }

    /// El botón de apagado despliega su menú y sus entradas devuelven lo suyo.
    /// El dibujo y el hit-test salen del mismo cálculo, así que si uno se mueve
    /// este test lo dice.
    #[test]
    fn el_menu_de_apagado_responde_donde_se_dibuja() {
        let pantalla = (1645.0, 1029.0);
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        let (bx, by, bw, bh) = b.boton_energia(pantalla);
        assert_eq!(b.pulsar(bx + bw / 2.0, by + bh / 2.0, pantalla).0, None);
        assert!(b.menu, "el botón no desplegó el menú");

        // La última entrada es Suspender, y el menú cuelga del botón sin
        // salirse por la derecha.
        let (ex, ey, ew, eh) = b.entrada_energia(entradas().len() - 1, pantalla);
        assert!(ey > by + bh, "el menú cuelga debajo del botón");
        assert!(ex + ew <= pantalla.0, "el menú no se sale de la pantalla");
        assert_eq!(b.pulsar(ex + ew / 2.0, ey + eh / 2.0, pantalla).0, None);
        assert!(!b.menu, "elegir una entrada tiene que cerrar el menú");
        assert!(
            b.confirmacion.is_some(),
            "hay que confirmar antes de suspender"
        );
        b.confirmar_tecla(crate::TeclaPulsada::Derecha);
        assert_eq!(
            b.confirmar_tecla(crate::TeclaPulsada::Intro),
            (true, Some(Peticion::Suspender))
        );

        // Y pulsar en cualquier otro sitio lo recoge sin hacer nada.
        b.menu = true;
        assert_eq!(
            b.pulsar(pantalla.0 / 2.0, pantalla.1 / 2.0, pantalla).0,
            None
        );
        assert!(!b.menu, "pulsar fuera tiene que cerrar el menú");
    }

    /// La contraseña nunca vive en esta estructura: solo su longitud.
    #[test]
    fn no_se_guarda_la_contrasena() {
        let b = Bloqueo::new(
            "12:30".into(),
            "lunes, 17 de agosto".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        // Si algún día alguien añade aquí un `String` con lo escrito, este test
        // no lo verá; queda como recordatorio de dónde está la frontera.
        assert_eq!(b.escritos, 0);
        assert_eq!(b.estado, Estado::Escribiendo);
    }

    #[test]
    fn confirmar_energia_no_filtra_teclas_a_la_contrasena() {
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        b.confirmacion = Some((
            Peticion::Apagar,
            crate::confirmacion::Confirmacion::new(
                Peticion::Apagar.energia().expect("apagar se confirma"),
            ),
        ));
        assert_eq!(
            b.confirmar_tecla(crate::TeclaPulsada::Caracter('x')),
            (true, None)
        );
        assert_eq!(b.confirmar_tecla(crate::TeclaPulsada::Intro), (true, None));
        assert!(b.confirmacion.is_none());
        assert_eq!(b.escritos, 0);
    }

    #[test]
    fn bloq_mayus_solo_repinta_cuando_cambia() {
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        assert!(b.actualizar_caps_lock(true));
        assert!(b.caps_lock);
        assert!(!b.actualizar_caps_lock(true));
        assert!(b.actualizar_caps_lock(false));
    }

    #[test]
    fn sin_animaciones_el_bloqueo_nace_quieto() {
        let config = crate::config::Bloqueo {
            animaciones: false,
            ..Default::default()
        };
        let b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        assert!(!b.entrada_animada());
        assert!(!b.animando());
    }

    #[test]
    fn los_controles_multimedia_coinciden_con_la_tarjeta() {
        // Sin animaciones: la prueba mete el medio a mano, sin pasar por
        // `poner_medio`, así que `medio_desde` nunca se pone y la entrada se
        // quedaría a medias para siempre si las animaciones estuvieran
        // activas. Lo que se comprueba aquí es que el dibujo y el clic
        // coinciden en la posición **asentada**, no a mitad de una animación
        // que este montaje no dispara de verdad.
        let config = crate::config::Bloqueo {
            animaciones: false,
            ..Default::default()
        };
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.medios.push(Medio {
            fuente: None,
            caratula: None,
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Canción".into(),
            detalle: "Artista".into(),
            color: tema::acento(),
            progreso: Some(0.5),
            posicion: Some(30),
            duracion: Some(60),
            reproduciendo: true,
            calidad: None,
        });
        let pantalla = (1645.0, 1029.0);
        let x0 = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let centro = pantalla.0 / 2.0;
        let y = b.medios_y_actual(pantalla.1, b.medio_avance())
            + MEDIOS_CONTROLES_Y
            + MEDIOS_CONTROL / 2.0;
        assert_eq!(
            b.medio_en(centro - MEDIOS_PASO, y, pantalla),
            Some(crate::medios::Orden::Anterior)
        );
        assert_eq!(
            b.medio_en(centro, y, pantalla),
            Some(crate::medios::Orden::Alternar)
        );
        assert_eq!(b.medio_en(x0 - 2.0, y, pantalla), None);
    }

    fn sonando_a(posicion: u64) -> crate::medios::Sonando {
        crate::medios::Sonando {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Canción".into(),
            artista: "Artista".into(),
            aplicacion: "Prueba".into(),
            caratula: None,
            posicion: Some(posicion),
            duracion: Some(60),
            posicion_ms: Some(posicion * 1000),
            duracion_ms: Some(60_000),
            calidad: None,
            reproduciendo: true,
        }
    }

    /// Pulsar la barra la agarra, moverse arrastra (los tiempos siguen al
    /// dedo) y soltar deja la posición donde quedó.
    #[test]
    fn la_barra_se_arrastra_y_salta_al_soltar() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.poner_medio(Some(sonando_a(30)));
        let pantalla = (1645.0, 1029.0);
        let x0 = (pantalla.0 - MEDIOS_ANCHO) / 2.0 + MEDIOS_BARRA_X;
        let y = b.medios_y_actual(pantalla.1, b.medio_avance())
            + MEDIOS_BARRA_Y
            + MEDIOS_BARRA_FILA / 2.0;
        let (peticion, repintar) = b.pulsar(x0 + MEDIOS_BARRA_W * 0.25, y, pantalla);
        assert!(peticion.is_none() && repintar);
        assert_eq!(b.arrastre, Some(0.25));
        // Fuera de la barra, en horizontal, sigue arrastrando y se queda en el tope.
        assert!(b.mover(x0 + MEDIOS_BARRA_W * 2.0, 0.0, pantalla));
        assert_eq!(b.arrastre, Some(1.0));
        assert!(b.mover(x0 + MEDIOS_BARRA_W * 0.75, 0.0, pantalla));
        assert!(b.soltar());
        assert_eq!(b.arrastre, None);
        assert_eq!(b.medios[0].posicion, Some(45));
        assert!(!b.soltar(), "soltar dos veces no salta otra vez");
    }

    /// Pasar por encima de la barra la engorda; salir la devuelve, y moverse
    /// por el resto de la pantalla no pide repintar.
    #[test]
    fn la_barra_engorda_bajo_el_cursor() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.poner_medio(Some(sonando_a(30)));
        let pantalla = (1645.0, 1029.0);
        let x = (pantalla.0 - MEDIOS_ANCHO) / 2.0 + MEDIOS_BARRA_X + 20.0;
        let y = b.medios_y_actual(pantalla.1, b.medio_avance())
            + MEDIOS_BARRA_Y
            + MEDIOS_BARRA_FILA / 2.0;
        assert!(b.mover(x, y, pantalla));
        assert!(b.sobre_barra);
        assert!(!b.mover(x + 5.0, y, pantalla), "seguir encima no repinta");
        assert!(b.mover(x, 10.0, pantalla));
        assert!(!b.sobre_barra);
        assert!(!b.mover(x, 20.0, pantalla));
    }

    /// Con BookOS Player salen aleatorio y repetir a dos pasos del centro y
    /// se pulsan; sin modos, ahí no hay nada y los tres de en medio no se mueven.
    #[test]
    fn aleatorio_y_repetir_solo_con_modos() {
        use crate::medios::Repetir;
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.poner_medio(Some(sonando_a(30)));
        let pantalla = (1645.0, 1029.0);
        let centro = pantalla.0 / 2.0;
        let y = b.medios_y_actual(pantalla.1, b.medio_avance())
            + MEDIOS_CONTROLES_Y
            + MEDIOS_CONTROL / 2.0;
        let izquierda = centro - 2.0 * MEDIOS_PASO;
        let derecha = centro + 2.0 * MEDIOS_PASO;
        assert_eq!(b.lateral_en(izquierda, y, pantalla), None);

        assert!(b.poner_modos((Some(false), Some(Repetir::No))));
        assert!(
            !b.poner_modos((Some(false), Some(Repetir::No))),
            "lo mismo no repinta"
        );
        assert_eq!(
            b.lateral_en(izquierda, y, pantalla),
            Some(Lateral::Aleatorio)
        );
        assert_eq!(b.lateral_en(derecha, y, pantalla), Some(Lateral::Repetir));
        assert_eq!(
            b.medio_en(centro, y, pantalla),
            Some(crate::medios::Orden::Alternar)
        );
        // Apretar solo hunde; soltar cambia el estado ya, sin esperar a la
        // lectura siguiente (HIG §8: actuar al soltar).
        b.pulsar(izquierda, y, pantalla);
        assert_eq!(b.aleatorio, Some(false), "apretar no actúa");
        assert!(b.soltar());
        assert_eq!(b.aleatorio, Some(true));
        b.pulsar(derecha, y, pantalla);
        b.soltar();
        assert_eq!(b.repetir, Some(Repetir::Lista));
        // Un toque rápido, como el del dedo: la orden sale al soltar, pero el
        // icono se queda hundido hasta cumplir `HUNDIDO_MINIMO`.
        b.pulsar(izquierda, y, pantalla);
        b.soltar();
        assert_eq!(b.aleatorio, Some(false), "la orden no espera");
        assert!(b.vuelta_pendiente && b.tarjeta_animando());
        assert_eq!(b.hundido.destino(), 1.0);
        std::thread::sleep(HUNDIDO_MINIMO);
        b.avanzar_entrada(Instant::now());
        assert!(!b.vuelta_pendiente);
        assert_eq!(b.hundido.destino(), 0.0);
        // Irse más de 10 pt con el botón apretado lo cancela.
        b.pulsar(derecha, y, pantalla);
        assert!(b.mover(derecha, y + CANCELA_TOQUE + 1.0, pantalla));
        b.soltar();
        assert_eq!(b.repetir, Some(Repetir::Lista), "el toque cancelado no actúa");
    }

    /// La lectura de cada segundo cambia la posición; con la misma canción la
    /// tarjeta no vuelve a entrar.
    #[test]
    fn la_misma_cancion_no_vuelve_a_entrar() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.poner_medio(Some(sonando_a(30)));
        let desde = b.medio_desde;
        assert!(b.poner_medio(Some(sonando_a(31))), "otra posición repinta");
        assert_eq!(b.medio_desde, desde);
        let mut otra = sonando_a(0);
        otra.titulo = "Otra".into();
        assert!(b.poner_medio(Some(otra)));
        assert_ne!(b.medio_desde, desde);
    }

    /// El clic tiene que acertar donde la tarjeta **se ve**, aunque todavía
    /// esté entrando.
    ///
    /// Antes `view()` sumaba el término de la animación a mano y `medio_en`
    /// llamaba a la posición asentada sin él: durante los primeros ~500 ms
    /// tras `Meta+L`, la tarjeta se dibujaba hasta 28 px más abajo de donde
    /// el clic la buscaba, así que tocar play nada más echar el bloqueo no
    /// hacía nada.
    #[test]
    fn el_clic_acierta_aunque_la_tarjeta_siga_entrando() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        // `poner_medio`, no un `push` a mano: es lo que arranca de verdad la
        // animación de entrada, con `medio_desde` recién puesto.
        b.poner_medio(Some(crate::medios::Sonando {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Canción".into(),
            artista: "Artista".into(),
            aplicacion: "Prueba".into(),
            caratula: None,
            posicion: Some(30),
            duracion: Some(60),
            posicion_ms: Some(30_000),
            duracion_ms: Some(60_000),
            calidad: None,
            reproduciendo: true,
        }));
        assert!(
            b.medio_avance() < 1.0,
            "la prueba no prueba nada si ya asentó"
        );

        let pantalla = (1645.0, 1029.0);
        let centro = pantalla.0 / 2.0;
        // La `y` de verdad: la misma cuenta que hace `view()` para dibujar,
        // con el término de la animación incluido. Si `medio_en` la ignora,
        // este punto cae fuera de lo que el hit-test busca y el `assert`
        // de abajo falla con `None`.
        let y = b.medios_y_actual(pantalla.1, b.medio_avance())
            + MEDIOS_CONTROLES_Y
            + MEDIOS_CONTROL / 2.0;
        assert_eq!(
            b.medio_en(centro, y, pantalla),
            Some(crate::medios::Orden::Alternar),
            "el clic no acertó la tarjeta mientras seguía entrando"
        );
    }

    /// En una resolución lógica baja, la tarjeta de medios no puede tapar el
    /// bloque de acceso.
    ///
    /// 1280×800 es la de este portátil a escala 1,75 —comprobado en la
    /// sesión real—: con el cálculo antiguo, `0.68 * 800 = 544` caía dentro
    /// del bloque de acceso, que a esa altura llega hasta `288 + 274 = 562`.
    #[test]
    fn la_tarjeta_de_medios_no_tapa_el_acceso_en_pantallas_bajas() {
        let config = crate::config::Bloqueo::default();
        let b = Bloqueo::new("23:50".into(), "jueves".into(), None, config);
        let alto = 800.0;
        let fin_acceso = b.acceso_y(alto) + b.alto_acceso(alto);
        assert!(
            b.medios_y_asentada(alto) >= fin_acceso + MARGEN_MEDIOS,
            "la tarjeta de medios se solapa con el bloque de acceso"
        );
        assert!(
            b.medios_y_asentada(alto) + MEDIOS_ALTO <= alto - MARGEN_INFERIOR + 0.5,
            "la tarjeta de medios se sale por abajo o queda pegada al borde"
        );
    }

    /// Y en una pantalla normal, el piso no debe moverla: sigue en la
    /// fracción de siempre.
    #[test]
    fn en_pantallas_normales_la_tarjeta_no_se_mueve() {
        let config = crate::config::Bloqueo::default();
        let b = Bloqueo::new("23:50".into(), "jueves".into(), None, config);
        let alto = 1029.0;
        assert_eq!(b.medios_y_asentada(alto), alto * config.medios_y);
    }

    #[test]
    fn huella_pulsa_al_fallar_y_al_reconocer_y_luego_reposa() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("23:50".into(), "jueves".into(), None, config);
        assert_eq!(b.pulso_huella(), 0.0);

        assert!(b.actualizar_huella(Huella::NoCoincide));
        assert!(b.contenido_animando());
        assert!(b.sacudida().abs() >= 0.0);

        assert!(b.actualizar_huella(Huella::Reconocida));
        assert!(b.contenido_animando());
        assert!(!b.actualizar_huella(Huella::Reconocida));

        // Al envejecer más allá de ESTADO_TOTAL, deja de animar (cero CPU en reposo).
        b.huella_desde = Instant::now() - ESTADO_TOTAL - Duration::from_millis(10);
        assert_eq!(b.pulso_huella(), 0.0);
        assert!(!b.contenido_animando());
    }

    #[test]
    fn cancelar_interaccion_suelta_mando_y_arrastre_sin_accionar() {
        let mut config = crate::config::Bloqueo::default();
        config.medios = true;
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.poner_medio(Some(crate::medios::Sonando {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Canción".into(),
            artista: "Artista".into(),
            aplicacion: "Prueba".into(),
            caratula: None,
            posicion: Some(15),
            duracion: Some(60),
            posicion_ms: Some(15_000),
            duracion_ms: Some(60_000),
            calidad: None,
            reproduciendo: false,
        }));
        b.apretado = Some((Mando::Orden(crate::medios::Orden::Alternar), (100.0, 200.0)));
        b.arrastre = Some(0.9);
        assert!(b.cancelar_interaccion());
        assert!(b.apretado.is_none());
        assert!(b.arrastre.is_none());
        assert!(!b.vuelta_pendiente);
        assert!(!b.medios[0].reproduciendo, "cancelar no debe disparar play/pause");
        assert_eq!(b.medios[0].posicion, Some(15), "cancelar no debe hacer seek");
        assert!(!b.cancelar_interaccion());
    }
}
