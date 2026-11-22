//! Los controles que comparten las superficies emergentes.
//!
//! La barra de las tarjetas sale igual en Sonido, en Brillo y en la Estación
//! de Control, con las mismas medidas del sistema de diseño. Está aquí para
//! que una sola sea la buena: tres copias de un deslizador acaban con tres
//! radios distintos y tres zonas agarrables que no coinciden.
//!
//! Las medidas son las del lienzo «Widgets de BookOS», que junta la anatomía de
//! la página `Applets` de Figma con los tokens de `AI-DESIGN-SYSTEM.md`: la
//! tarjeta blanca o carbón, con grupos grises dentro y baldosas del color de la
//! tarjeta dentro de los grupos.

use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, container, text};

use crate::tema;
use crate::view::PanelElement;

/// Relleno interior de un grupo gris.
pub const GRUPO: f32 = 6.0;
/// Alto de un chip de cabecera como «Config».
pub const CHIP: f32 = 28.0;

/// La fuente del sistema con otro peso. El sistema de diseño solo admite
/// Regular, Medium, Semibold, Bold y Heavy, y por eso se pide el peso y no un
/// número.
// 0.1.0
pub fn peso(weight: Weight) -> Font {
    Font {
        weight,
        ..Font::DEFAULT
    }
}

/// El título de una tarjeta: 17/Bold, el del diálogo en el sistema de diseño.
// 0.1.0
pub fn titulo<'a>(texto: impl iced_core::text::IntoFragment<'a>) -> PanelElement<'a> {
    text(texto)
        .size(17.0)
        .font(peso(Weight::Bold))
        .color(tema::texto())
        .into()
}

/// Un chip de cabecera: «Config», «Borrar todo», «Hoy».
// 0.1.0
pub fn chip<'a>(etiqueta: &'a str, fondo: Color, color: Color) -> PanelElement<'a> {
    container(
        text(etiqueta)
            .size(13.0)
            .font(peso(Weight::Semibold))
            .color(color)
            .wrapping(iced_core::text::Wrapping::None),
    )
    .height(Length::Fixed(CHIP))
    .center_y(Length::Fixed(CHIP))
    .padding([0, 12])
    .style(move |_| container::Style {
        background: Some(fondo.into()),
        border: Border {
            radius: tema::R_BOTON_PEQUENO.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// Lo que mide un chip, para las zonas de clic.
// 0.1.0
pub fn ancho_chip(etiqueta: &str) -> f32 {
    crate::widget::ancho_de(etiqueta, 13.0) + 24.0
}

/// La línea de medio píxel que separa bloques dentro de un grupo.
// 0.1.0
pub fn divisor<'a>(ancho: f32) -> PanelElement<'a> {
    container(Space::new().height(Length::Fixed(0.5)))
        .width(Length::Fixed(ancho))
        .style(|_| container::Style {
            background: Some(tema::divisor().into()),
            ..Default::default()
        })
        .into()
}

/// El color de una baldosa dentro de un grupo, con el puntero encima a
/// `señalada` (de 0 a 1).
///
/// Se mezcla hacia la tinta y no se superpone el `--hover`: el hover es un velo
/// con alfa, y encima de una baldosa opaca lo que se ve es la tarjeta de detrás
/// asomando, no un realce.
// 0.1.0
// modified 0.1.1 -- fondo elevado en vez de card
pub fn baldosa(señalada: f32) -> Color {
    tema::mezclar(tema::elevado(), tema::tinta(), 0.05 * señalada)
}

/// Grueso de la barra de una tarjeta en reposo y agarrada. Engorda al tocarla
/// para confirmar el agarre; la misma en Pantalla, Sonido y Brillo.
// añadido en la 0.2.1
pub const PISTA: f32 = 10.0;
pub const PISTA_AGARRADA: f32 = 15.0;
/// Alto de la fila de una barra, con su icono a cada lado: lo que se agarra.
pub const FILA_BARRA: f32 = 28.0;
/// Ancho de la caja de cada icono de los extremos y hueco que los separa de la
/// barra.
pub const ICONO_BARRA: f32 = 28.0;
pub const ENTRE_BARRA: f32 = 8.0;

/// El grueso de la barra con el agarre a `grosor` (de 0 a 1, el muelle).
// añadido en la 0.2.1
pub fn grueso_barra(grosor: f32) -> f32 {
    PISTA + (PISTA_AGARRADA - PISTA) * grosor
}

/// La barra de una tarjeta: surco, relleno hasta `fraccion` (de 0 a 1) y, si
/// hace falta, cortes para los niveles de un control por pasos y la marca de
/// dónde pondría el sensor la barra.
///
/// Sin tirador: el final del relleno es el agarre. El relleno nunca es más
/// estrecho que alto —a cero queda un punto—, porque con menos iced recorta su
/// radio y sale una barrita recta asomando por las curvas del surco.
///
/// Mide [`FILA_BARRA`] de alto aunque la barra sea más fina, para que la marca
/// quepa debajo y la fila no cambie de tamaño cuando engorda.
// añadido en la 0.2.1
pub fn barra<'a>(
    ancho: f32,
    grueso: f32,
    fraccion: f32,
    lleno: Color,
    surco: Color,
    cortes: &[f32],
    marca: Option<f32>,
) -> PanelElement<'a> {
    let pieza = |w: f32, h: f32, color: Color, radio: f32| {
        container(Space::new())
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: radio.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
    };
    let centrada = |dentro: PanelElement<'a>| {
        container(dentro)
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(FILA_BARRA))
            .center_y(Length::Fixed(FILA_BARRA))
    };
    let base: PanelElement<'a> = if fraccion <= 0.0 {
        pieza(ancho, grueso, surco, grueso / 2.0).into()
    } else {
        let relleno = (ancho * fraccion.clamp(0.0, 1.0)).max(grueso);
        iced_widget::stack![
            pieza(ancho, grueso, surco, grueso / 2.0),
            pieza(relleno, grueso, lleno, grueso / 2.0),
        ]
        .into()
    };
    let mut capas = vec![centrada(base).into()];
    // Los cortes van del color de la placa de detrás, para que se lean como
    // huecos tanto sobre el relleno como sobre el surco.
    for c in cortes {
        let x = (ancho * c.clamp(0.0, 1.0) - 2.0).max(0.0);
        let corte = container(pieza(4.0, 4.0, tema::superficie_en_tarjeta(), 2.0))
            .padding(iced_core::Padding::ZERO.left(x))
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(FILA_BARRA))
            .center_y(Length::Fixed(FILA_BARRA));
        capas.push(corte.into());
    }
    if let Some(f) = marca {
        let x = (ancho * f.clamp(0.0, 1.0) - 2.5).max(0.0);
        // Debajo de la barra, con el grueso máximo: apuntándole, no cruzándola,
        // para que no se tome por un tirador.
        let y = (FILA_BARRA + PISTA_AGARRADA) / 2.0 + 1.0;
        capas.push(
            container(pieza(5.0, 5.0, tema::texto2(), 2.5))
                .padding(iced_core::Padding::ZERO.left(x).top(y))
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(FILA_BARRA))
                .into(),
        );
    }
    iced_widget::Stack::with_children(capas)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(FILA_BARRA))
        .into()
}

/// De una `x` sobre una barra (`x0` es su borde izquierdo) a su nivel de 0 a
/// 100, recortado. El final del relleno queda bajo el puntero mientras se
/// arrastra; pasarse de los extremos deja el tope y no un número imposible.
// añadido en la 0.2.1
pub fn nivel_barra(x: f32, x0: f32, ancho: f32) -> u8 {
    (((x - x0) / ancho).clamp(0.0, 1.0) * 100.0).round() as u8
}

/// La caja de una tarjeta emergente: fondo, borde, radio y margen del sistema.
// 0.1.0
pub fn tarjeta<'a>(contenido: PanelElement<'a>, ancho: f32, margen: f32) -> PanelElement<'a> {
    container(contenido)
        .padding(margen)
        .width(Length::Fixed(ancho))
        // El radio de fondo no recorta a los hijos por sí solo. Sin este clip,
        // una tarjeta alta podía pintar hasta las esquinas cuadradas del
        // buffer, especialmente al quedar pegada al borde inferior.
        .clip(true)
        .style(|_| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_POPOVER.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        })
        .into()
}

/// El encendido: disco blanco con glifo en acento en oscuro; en claro, al revés.
/// Lo comparten los conmutadores de la estación y las filas activas de Wi-Fi y
/// Bluetooth, que tienen que leerse como el mismo «encendido».
// añadido en la 0.2.0
// modified 0.2.1 -- pasa de centro.rs aquí
pub fn encendido() -> (Color, Color) {
    if tema::es_claro() {
        (tema::acento(), tema::sobre_acento())
    } else {
        (Color::WHITE, tema::acento())
    }
}
