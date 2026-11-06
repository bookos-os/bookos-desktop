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
const D_OLA: Duration = Duration::from_millis(1040);
/// Alto del campo y lado del botón redondo que lleva al lado.
pub const ALTO_CAMPO: f32 = 44.0;
/// Cuerpo de la fecha, bajo el reloj.
const FECHA: f32 = 22.0;
/// Diámetro de los puntos de la contraseña.
pub const PUNTO: f32 = 10.0;
const ESTADO_TOTAL: Duration = Duration::from_millis(520);
const MEDIOS_ANCHO: f32 = 372.0;
const MEDIOS_ARTE: f32 = 78.0;
const MEDIOS_INFO: f32 = 246.0;
const MEDIOS_ALTO: f32 = 124.0;
const MEDIOS_PAD: f32 = 12.0;
/// Alto de la fila de botones, que es también el diámetro del central.
const MEDIOS_CONTROL: f32 = 34.0;
/// Diámetro de anterior y siguiente.
const MEDIOS_BOTON: f32 = 30.0;
/// De centro a centro entre dos botones.
const MEDIOS_PASO: f32 = 39.0;

/// Alto de la línea del nombre de usuario: tamaño 20 con el interlineado
/// habitual de 1,2.
const ALTO_NOMBRE: f32 = 24.0;
/// Alto del aviso de la huella: tamaño 13 con el mismo interlineado. Está
/// vacío la mayor parte del tiempo, pero reserva su hueco igual —la tarjeta de
/// medios no puede depender de si hoy hay algo que decir sobre la huella.
const ALTO_HUELLA: f32 = 16.0;
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

/// Fondo del campo y del botón: casi negro y translúcido, para que se vea el
/// fondo por debajo sin perder el contraste de los puntos.
pub const FONDO_CAMPO: Color = tema::hexa(0x16191e, 0.82);

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
const ICONO_ENTRADA: f32 = 18.0;

/// Lo que ofrece el menú de apagado, en el orden del diseño. Hibernar solo si
/// el equipo puede: ver [`crate::confirmacion::hibernar_disponible`].
// 0.1.0
pub fn entradas() -> &'static [(&'static str, &'static str, Peticion)] {
    const SIN: &[(&str, &str, Peticion)] = &[
        ("Apagar", "apagar", Peticion::Apagar),
        ("Reiniciar", "reiniciar", Peticion::Reiniciar),
        ("Cerrar sesión", "salir", Peticion::CerrarSesion),
        ("Suspender", "dormir", Peticion::Suspender),
    ];
    const CON: &[(&str, &str, Peticion)] = &[
        ("Apagar", "apagar", Peticion::Apagar),
        ("Reiniciar", "reiniciar", Peticion::Reiniciar),
        ("Cerrar sesión", "salir", Peticion::CerrarSesion),
        ("Suspender", "dormir", Peticion::Suspender),
        ("Hibernar", "hibernar", Peticion::Hibernar),
    ];
    if crate::confirmacion::hibernar_disponible() {
        CON
    } else {
        SIN
    }
}

/// Lo que el bloqueo puede pedirle al compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peticion {
    Apagar,
    Reiniciar,
    CerrarSesion,
    Suspender,
    Hibernar,
}

impl Peticion {
    // 0.1.0
    fn energia(self) -> crate::confirmacion::Energia {
        use crate::confirmacion::Energia;
        match self {
            Self::Apagar => Energia::Apagar,
            Self::Reiniciar => Energia::Reiniciar,
            Self::CerrarSesion => Energia::CerrarSesion,
            Self::Suspender => Energia::Suspender,
            Self::Hibernar => Energia::Hibernar,
        }
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
    let lado = 440_u32;
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
            usuario: Usuario::leer(avatar),
            config,
            estado_desde: ahora - ESTADO_TOTAL,
            medio_desde: None,
            candado: crate::icono::desde_svg(CANDADO),
            flecha: crate::icono::desde_svg(FLECHA),
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
                let ancho = (CAMPO + 100.0).max(self.config.avatar_tamano + 40.0).min(w);
                let alto = self.config.avatar_tamano
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
                ((w - ancho) / 2.0, h * self.config.acceso_y, ancho, alto)
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

        let avatar = self.config.avatar_tamano * (0.90 + 0.10 * acceso_avance);
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
        let acceso_y = pantalla.1 * self.config.acceso_y + (1.0 - acceso_avance) * 30.0;
        let capa_acceso = column![
            Space::new().height(Length::Fixed((acceso_y - origen_y).max(0.0))),
            container(acceso_desplazado)
                .width(Length::Fixed(ancho))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto));

        let mut capas = iced_widget::stack![];
        // El velo de la canción no va aquí: lo pinta el compositor en la GPU.
        // Ver `velo_medios`.
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
        let piso = alto_pantalla * self.config.acceso_y
            + self.config.avatar_tamano
            + 12.0
            + ALTO_NOMBRE
            + 16.0
            + ALTO_CAMPO
            + 10.0
            + 20.0
            + ALTO_HUELLA
            + MARGEN_MEDIOS;
        // Tampoco se sale por abajo: con `medios_y` cerca del borde, la
        // fracción deja menos sitio del que mide la tarjeta. El piso manda
        // sobre este techo, porque tapar la contraseña es peor.
        let techo = alto_pantalla - MEDIOS_ALTO - MARGEN_MEDIOS;
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

    /// El color con que empieza, arriba, el velo de la canción: baja desde
    /// arriba y se apaga del todo al 60 % del alto. `None` sin música.
    ///
    /// Lo pinta el compositor, no esta vista. Era un degradado translúcido a
    /// pantalla completa rasterizado en CPU, y medido a escala 2 con música
    /// sonando cada tecla de la contraseña costaba **55 ms** en vez de 3–7
    /// (`examples/bloqueo_bench.rs`): el punto de cada carácter nacía a cuatro
    /// fotogramas por segundo y parecía que las letras llegaban con retraso.
    // 0.1.0
    pub fn velo_medios(&self) -> Option<Color> {
        let m = self.medios.first().filter(|_| self.config.medios)?;
        Some(Color {
            // modificado en la 0.2.0 -- el compositor funde la textura sin volver a rasterizar el degradado.
            a: 0.30,
            ..m.color
        })
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
        // Un segundo «no coincide» seguido también sacude: es otro dedo.
        if self.huella == huella && huella != Huella::NoCoincide {
            return false;
        }
        self.huella = huella;
        self.huella_desde = Instant::now();
        true
    }

    // 0.1.0
    fn aviso_huella<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let (texto, color) = match self.huella {
            Huella::Callada => ("", Color::WHITE),
            Huella::Aviso(texto) => (texto, con_alfa(Color::WHITE, 0.82)),
            // En rojo como la contraseña mala, y diciendo que PAM sigue
            // escuchando: no hace falta hacer nada más que volver a poner el
            // dedo.
            Huella::NoCoincide => ("Huella no reconocida. Vuelve a intentarlo.", tema::rojo()),
            Huella::Reconocida => ("Huella reconocida", Color::WHITE),
        };
        text(texto)
            .size(13)
            .color(Color {
                a: color.a * alfa,
                ..color
            })
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
        self.medios = nuevos;
        // Una tarjeta que llega unas décimas después del bloqueo entra desde
        // cero, no aparece ya terminada en mitad de la pantalla.
        self.medio_desde = Some(Instant::now());
        true
    }

    /// El renglón bajo el campo: qué está pasando con lo que se ha escrito.
    ///
    /// Ocupa sitio siempre, aunque no diga nada: si apareciera y desapareciera,
    /// el campo daría un salto de veinte píxeles justo al pulsar Intro.
    // 0.1.0
    // modified 0.1.1 -- no repite «introduce tu contraseña» con el aviso de la huella
    fn mensaje<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let (texto_estado, color) = match self.estado {
            Estado::Comprobando => (
                "Comprobando…".to_string(),
                Color {
                    a: 0.75,
                    ..Color::WHITE
                },
            ),
            Estado::Fallo => ("Contraseña incorrecta".to_string(), tema::rojo()),
            Estado::Espera(segundos) => (
                if segundos == 1 {
                    "Espera un segundo para volver a intentarlo".to_string()
                } else {
                    format!("Espera {segundos} segundos para volver a intentarlo")
                },
                tema::rojo(),
            ),
            Estado::Escribiendo if self.caps_lock => (
                "Bloq Mayús está activado".to_string(),
                Color {
                    a: 0.82,
                    ..tema::amarillo()
                },
            ),
            // El aviso de la huella ya dice «o introduce tu contraseña»; con los
            // dos renglones seguidos el mismo ruego salía dos veces.
            Estado::Escribiendo
                if self.escritos == 0 && !matches!(self.huella, Huella::Aviso(_)) =>
            {
                (
                    "Introduce tu contraseña para desbloquear".to_string(),
                    Color {
                        a: 0.55,
                        ..Color::WHITE
                    },
                )
            }
            Estado::Escribiendo => ("".to_string(), Color::TRANSPARENT),
        };
        container(
            text(texto_estado)
                .size(tema::T_CUERPO)
                .color(con_alfa(color, alfa)),
        )
        .height(Length::Fixed(20.0))
        .center_y(Length::Fixed(20.0))
        .into()
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
            let resultado = c.pulsar(
                x - (pantalla.0 - crate::confirmacion::Confirmacion::ANCHO) / 2.0,
                y - (pantalla.1 - crate::confirmacion::Confirmacion::ALTO) / 2.0,
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
                    self.confirmacion = Some((
                        *peticion,
                        crate::confirmacion::Confirmacion::new(peticion.energia()),
                    ));
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
        if let Some(orden) = self.medio_en(x, y, pantalla)
            && let Some(medio) = self.medios.first_mut()
        {
            let ejecutada = crate::medios::ejecutar_en(&medio.bus, orden);
            if orden == crate::medios::Orden::Alternar && ejecutada {
                medio.reproduciendo = !medio.reproduciendo;
            }
            return (None, ejecutada);
        }
        let estaba_abierto = self.menu;
        self.cerrar_menu();
        (None, estaba_abierto)
    }

    // 0.1.0
    fn medio_en(&self, x: f32, y: f32, pantalla: (f32, f32)) -> Option<crate::medios::Orden> {
        if !self.config.medios || self.medios.is_empty() {
            return None;
        }
        let tarjeta_x = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let tarjeta_y = self.medios_y_actual(pantalla.1, self.medio_avance());
        if y < tarjeta_y || y > tarjeta_y + MEDIOS_ALTO {
            return None;
        }
        let centro = tarjeta_x + MEDIOS_PAD + MEDIOS_ARTE + 14.0 + MEDIOS_INFO / 2.0;
        let control_y = tarjeta_y + MEDIOS_ALTO - MEDIOS_PAD - MEDIOS_CONTROL;
        if y < control_y || y > control_y + MEDIOS_CONTROL {
            return None;
        }
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
            || (self.entrada_animada() && self.entrada.iter().any(|m| !m.quieto()))
            || (!self.medios.is_empty() && self.medio_avance() < 1.0)
    }

    // añadido en la 0.2.0 -- las animaciones de contenido necesitan píxeles nuevos; la entrada no.
    pub fn contenido_animando(&self) -> bool {
        let estado = self.config.animaciones
            && match self.estado {
                Estado::Fallo => self.estado_desde.elapsed() < ESTADO_TOTAL,
                Estado::Comprobando => self.estado_desde.elapsed() < D_OLA,
                _ => false,
            };
        let huella = self.config.animaciones
            && self.huella == Huella::NoCoincide
            && self.huella_desde.elapsed() < ESTADO_TOTAL;
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
            for (i, (etiqueta, icono, _)) in entradas().iter().enumerate() {
                let tinta = Color {
                    a: alfa,
                    ..Color::WHITE
                };
                // El fundido va en la opacidad: el tinte del SVG no respeta el
                // alfa del color y el glifo salía entero con el menú aún invisible.
                let dibujo: PanelElement<'a> = match crate::icono::propio(icono) {
                    Some(crate::icono::Icono::Svg(handle)) => iced_widget::svg(handle)
                        .width(Length::Fixed(ICONO_ENTRADA))
                        .height(Length::Fixed(ICONO_ENTRADA))
                        .style(|_, _| iced_widget::svg::Style {
                            color: Some(Color::WHITE),
                        })
                        .opacity(alfa)
                        .into(),
                    _ => Space::new().width(Length::Fixed(ICONO_ENTRADA)).into(),
                };
                let fila = row![
                    dibujo,
                    Space::new().width(Length::Fixed(12.0)),
                    text(*etiqueta)
                        .size(15.0)
                        .color(tinta)
                        .font(iced_core::Font {
                            weight: iced_core::font::Weight::Bold,
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
                                            ..Color::WHITE
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
                                ..FONDO_CAMPO
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
                    background: Some(con_alfa(Color::WHITE, alfa).into()),
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
                                ..Color::WHITE
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
        let borde = con_alfa(
            match self.estado {
                Estado::Fallo | Estado::Espera(_) => tema::rojo(),
                Estado::Comprobando => Color {
                    a: 0.35,
                    ..Color::WHITE
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
                ..Color::WHITE
            }
        };
        let tinta_flecha = Color {
            a: if lleno { alfa } else { 0.45 * alfa },
            ..Color::WHITE
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
                    ..Color::WHITE
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
                background: Some(con_alfa(FONDO_CAMPO, alfa).into()),
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
    /// Habla el idioma del propio bloqueo, no el del escritorio: el mismo
    /// cristal oscuro y el mismo blanco que el campo de la contraseña, con
    /// botones redondos y el acento solo en lo que está activo —el progreso y
    /// el botón central—. Con la paleta del tema, en claro salía una losa
    /// blanca opaca bajo una píldora oscura, y en oscuro un gris macizo al
    /// lado del cristal: dos piezas de sitios distintos. El bloqueo no cambia
    /// con el tema —reloj, campo y menú van en blanco sobre el fondo—, y la
    /// tarjeta tampoco.
    ///
    /// Los altos van fijos: `medio_en` busca los botones a una distancia
    /// conocida del borde de abajo, y con un alto que dependiera del texto el
    /// clic y el dibujo volverían a separarse.
    // 0.1.0
    // modified 0.1.1 -- tarjeta de música en blanco sobre cristal, sin seguir el tema
    fn tarjeta<'a>(&self, medio: &'a Medio, alfa: f32) -> PanelElement<'a> {
        use crate::actividad::{ANTERIOR, PAUSA, PLAY, SIGUIENTE};
        let blanco = move |a: f32| Color {
            a: a * alfa,
            ..Color::WHITE
        };
        let texto = blanco(1.0);
        let texto2 = blanco(0.6);
        let acento = tema::acento();

        let arte: PanelElement<'a> = match medio.caratula.clone() {
            Some(h) => iced_widget::image(h)
                .width(MEDIOS_ARTE)
                .height(MEDIOS_ARTE)
                .opacity(alfa)
                .into(),
            None => container(match crate::icono::propio("musica") {
                Some(ic) => crate::icono::ver_teñido_propio(&ic, 30.0, texto2),
                None => Space::new().width(Length::Fixed(30.0)).into(),
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

        let tramo = |ancho: f32, color: Color| {
            container(Space::new())
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(4.0))
                .style(move |_| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        let lleno = MEDIOS_INFO * medio.progreso.unwrap_or(0.0).clamp(0.0, 1.0);
        let barra = row![
            tramo(lleno, con_alfa(acento, alfa)),
            tramo((MEDIOS_INFO - lleno).max(0.0), blanco(0.18)),
        ];
        let tiempo = |segundos: Option<u64>| {
            text(segundos.map(crate::medios::reloj).unwrap_or_default())
                .size(tema::T_PEQUENO)
                .color(texto2)
        };
        let tiempos = row![
            tiempo(medio.posicion),
            Space::new().width(Length::Fill),
            tiempo(medio.duracion),
        ]
        .width(Length::Fixed(MEDIOS_INFO));

        let boton = |svg: &str, lado: f32, principal: bool| -> PanelElement<'a> {
            // Los secundarios, como el botón del campo cuando aún no hay
            // nada que mandar.
            let (relleno, tinta) = if principal {
                (
                    con_alfa(acento, alfa),
                    con_alfa(tema::tinta_sobre(acento), alfa),
                )
            } else {
                (blanco(0.12), texto)
            };
            container(crate::icono::ver_teñido_propio(
                &crate::icono::desde_svg(svg),
                lado * 0.4,
                tinta,
            ))
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: Some(relleno.into()),
                border: Border {
                    radius: (lado / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        };
        // Los centros quedan a `MEDIOS_PASO` uno de otro, que es donde los
        // busca `medio_en`.
        let hueco = MEDIOS_PASO - (MEDIOS_BOTON + MEDIOS_CONTROL) / 2.0;
        let controles = row![
            boton(ANTERIOR, MEDIOS_BOTON, false),
            Space::new().width(Length::Fixed(hueco)),
            boton(
                if medio.reproduciendo { PAUSA } else { PLAY },
                MEDIOS_CONTROL,
                true
            ),
            Space::new().width(Length::Fixed(hueco)),
            boton(SIGUIENTE, MEDIOS_BOTON, false),
        ]
        .align_y(Vertical::Center);

        let info = column![
            text(crate::emergente::recortar_texto(
                &medio.titulo,
                MEDIOS_INFO,
                tema::T_CUERPO
            ))
            .size(tema::T_CUERPO)
            .font(iced_core::Font {
                weight: iced_core::font::Weight::Semibold,
                ..iced_core::Font::DEFAULT
            })
            .wrapping(iced_core::text::Wrapping::None)
            .color(texto),
            text(crate::emergente::recortar_texto(
                &medio.detalle,
                MEDIOS_INFO,
                tema::T_PEQUENO
            ))
            .size(tema::T_PEQUENO)
            .wrapping(iced_core::text::Wrapping::None)
            .color(texto2),
            Space::new().height(Length::Fixed(7.0)),
            barra,
            Space::new().height(Length::Fixed(3.0)),
            tiempos,
            Space::new().height(Length::Fill),
            container(controles)
                .width(Length::Fixed(MEDIOS_INFO))
                .height(Length::Fixed(MEDIOS_CONTROL))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(MEDIOS_INFO))
        .height(Length::Fill);

        container(
            row![arte, Space::new().width(Length::Fixed(14.0)), info].align_y(Vertical::Center),
        )
        .width(Length::Fixed(MEDIOS_ANCHO))
        .height(Length::Fixed(MEDIOS_ALTO))
        .padding(MEDIOS_PAD)
        .style(move |_| container::Style {
            background: Some(con_alfa(FONDO_CAMPO, alfa).into()),
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
            crate::confirmacion::Confirmacion::new(Peticion::Apagar.energia()),
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
        });
        let pantalla = (1645.0, 1029.0);
        let x0 = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let centro = x0 + 12.0 + MEDIOS_ARTE + 14.0 + MEDIOS_INFO / 2.0;
        let y = pantalla.1 * config.medios_y + MEDIOS_ALTO - MEDIOS_PAD - MEDIOS_CONTROL / 2.0;
        assert_eq!(
            b.medio_en(centro - 39.0, y, pantalla),
            Some(crate::medios::Orden::Anterior)
        );
        assert_eq!(
            b.medio_en(centro, y, pantalla),
            Some(crate::medios::Orden::Alternar)
        );
        assert_eq!(b.medio_en(x0 - 2.0, y, pantalla), None);
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
            reproduciendo: true,
        }));
        assert!(
            b.medio_avance() < 1.0,
            "la prueba no prueba nada si ya asentó"
        );

        let pantalla = (1645.0, 1029.0);
        let x0 = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let centro = x0 + 12.0 + MEDIOS_ARTE + 14.0 + MEDIOS_INFO / 2.0;
        // La `y` de verdad: la misma cuenta que hace `view()` para dibujar,
        // con el término de la animación incluido. Si `medio_en` la ignora,
        // este punto cae fuera de lo que el hit-test busca y el `assert`
        // de abajo falla con `None`.
        let y = b.medios_y_actual(pantalla.1, b.medio_avance()) + MEDIOS_ALTO
            - MEDIOS_PAD
            - MEDIOS_CONTROL / 2.0;
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
        let fin_acceso = alto * config.acceso_y
            + config.avatar_tamano
            + 12.0
            + ALTO_NOMBRE
            + 16.0
            + ALTO_CAMPO
            + 10.0
            + 20.0
            + ALTO_HUELLA;
        assert!(
            b.medios_y_asentada(alto) >= fin_acceso + MARGEN_MEDIOS,
            "la tarjeta de medios se solapa con el bloque de acceso"
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
}
