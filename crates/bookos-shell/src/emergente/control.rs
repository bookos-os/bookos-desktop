//! Los controles que comparten las superficies emergentes.
//!
//! La píldora y el botón redondo salen igual en Sonido, en Brillo y en la
//! Estación de Control, con las mismas medidas del sistema de diseño. Están
//! aquí para que una sola sea la buena: tres copias de un deslizador acaban
//! con tres radios distintos y tres zonas agarrables que no coinciden.
//!
//! Las medidas son las del lienzo «Widgets de BookOS», que junta la anatomía de
//! la página `Applets` de Figma con los tokens de `AI-DESIGN-SYSTEM.md`: la
//! tarjeta blanca o carbón, con grupos grises dentro y baldosas del color de la
//! tarjeta dentro de los grupos.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Alto de la píldora. Barra gruesa sin tirador, como en la referencia: se
/// agarra sin apuntar, que es lo que no da un deslizador de línea fina.
pub const PILDORA: f32 = 24.0;
/// Lado del botón redondo que va a su derecha.
pub const BOTON: f32 = 32.0;
/// Separación entre el icono, la píldora y el botón.
pub const HUECO: f32 = 10.0;
/// Lado del icono que va a la izquierda de la píldora.
pub const ICONO: f32 = 18.0;
/// Alto del renglón de etiqueta y valor que va encima de una píldora.
pub const ETIQUETA: f32 = 18.0;
/// Aire entre la etiqueta y su píldora.
pub const BAJO_ETIQUETA: f32 = 8.0;
/// Alto de la fila de una píldora: la marca el botón, que es lo más alto.
pub const FILA_PILDORA: f32 = BOTON;
/// Relleno interior de un grupo gris.
pub const GRUPO: f32 = 6.0;
/// Alto de un chip de cabecera como «Config».
pub const CHIP: f32 = 28.0;
/// Cuánto se agranda la zona agarrable por arriba y por abajo.
///
/// Apuntar a 24 px de alto con el ratón en movimiento falla más de lo que
/// parece, y fallar aquí significa cerrar la emergente en vez de mover el
/// deslizador.
pub const MARGEN_AGARRE: f32 = 6.0;

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

/// La píldora: el canal lleno hasta `nivel` (de 0 a 100) sobre `surco`.
///
/// `surco` lo decide quien la pone: sobre la tarjeta es `--sbg`, y dentro de un
/// grupo, que ya es `--sbg`, es el color de la tarjeta para que se distinga.
///
/// El relleno nunca es más estrecho que alto. Con menos, iced recorta su radio
/// a la mitad del lado corto y sale una barrita recta asomando por las curvas
/// del surco; así, a 0 % queda un punto, que es lo que dice que ahí hay un
/// control y no un hueco. Ver [`lleno`] para cómo se reparte el resto.
// 0.1.0
pub fn pildora<'a>(
    ancho: f32,
    alto: f32,
    nivel: u8,
    apagado: bool,
    surco: Color,
) -> PanelElement<'a> {
    let lleno = lleno(ancho, alto, nivel);
    // A cero el punto va en el gris de apagado: en acento se leía como «un
    // poco», que es justo lo que no se distinguía del 0.
    let color = if apagado || nivel == 0 {
        tema::alfa(tema::TEXTO2, 0.35)
    } else {
        tema::acento()
    };
    let barra = container(Space::new())
        .width(Length::Fixed(lleno))
        .height(Length::Fixed(alto))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: (alto / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    container(barra)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto))
        .style(move |_| container::Style {
            background: Some(surco.into()),
            border: Border {
                radius: (alto / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// El botón redondo con un icono dentro.
///
/// Encendido va en el acento con el icono en la tinta que se lea encima;
/// apagado, en `fondo_apagado` —el mismo color que el surco de su píldora— con
/// el icono en texto secundario. `señalado`, de 0 a 1, es el puntero por
/// encima: oscurece el fondo sin cambiar de estado, que es lo que distingue «se
/// puede pulsar» de «está pulsado».
// 0.1.0
pub fn boton<'a>(
    icono: Option<&'a Icono>,
    encendido: bool,
    fondo_apagado: Color,
    señalado: f32,
) -> PanelElement<'a> {
    let (fondo, tinta) = if encendido {
        (tema::acento(), tema::sobre_acento())
    } else {
        (fondo_apagado, tema::TEXTO2)
    };
    let fondo = tema::mezclar(fondo, tema::tinta(), 0.08 * señalado);
    let dentro = match icono {
        Some(ic) => icono::ver_teñido(ic, 16.0, Some(tinta)),
        None => crate::widget::vacio(),
    };
    container(dentro)
        .width(Length::Fixed(BOTON))
        .height(Length::Fixed(BOTON))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: (BOTON / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Una fila de deslizador: icono a la izquierda, píldora y, si lo hay, el botón.
// 0.1.0
pub fn fila_pildora<'a>(
    icono: Option<&'a Icono>,
    pildora: PanelElement<'a>,
    boton: Option<PanelElement<'a>>,
) -> PanelElement<'a> {
    let dibujo: PanelElement<'a> = match icono {
        Some(ic) => icono::ver_teñido(ic, ICONO, Some(tema::texto())),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let mut fila = row![dibujo, Space::new().width(Length::Fixed(HUECO)), pildora]
        .align_y(Vertical::Center)
        .height(Length::Fixed(FILA_PILDORA));
    if let Some(boton) = boton {
        fila = fila
            .push(Space::new().width(Length::Fixed(HUECO)))
            .push(boton);
    }
    fila.into()
}

/// Lo que mide la píldora de una fila de `ancho`, con o sin botón.
// 0.1.0
pub fn ancho_pildora(ancho: f32, con_boton: bool) -> f32 {
    ancho - ICONO - HUECO - if con_boton { BOTON + HUECO } else { 0.0 }
}

/// La etiqueta de encima de una píldora: nombre a la izquierda en 13/Semibold y
/// el valor a la derecha en 12 secundario.
// 0.1.0
pub fn etiqueta<'a>(nombre: &'a str, valor: String, ancho: f32) -> PanelElement<'a> {
    container(
        row![
            text(nombre)
                .size(13.0)
                .font(peso(Weight::Semibold))
                .color(tema::texto()),
            Space::new().width(Length::Fill),
            text(valor).size(12.0).color(tema::TEXTO2),
        ]
        .align_y(Vertical::Center),
    )
    .width(Length::Fixed(ancho))
    .height(Length::Fixed(ETIQUETA))
    .center_y(Length::Fixed(ETIQUETA))
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

/// Un grupo gris dentro de la tarjeta, con su relleno.
// 0.1.0
pub fn grupo<'a>(contenido: PanelElement<'a>, ancho: f32, relleno: f32) -> PanelElement<'a> {
    container(contenido)
        .width(Length::Fixed(ancho))
        .padding(relleno)
        .style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: tema::R_ITEM_POPOVER.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
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

/// Lo seleccionado dentro de un grupo: el acento al 10 % sobre la tarjeta.
///
/// Es lo que pide el sistema de diseño para una selección en lista, en vez del
/// relleno sólido de la referencia, que dejaba el texto sin contraste.
///
/// En oscuro el 10 % sobre `#1c1c1e` apenas tiñe y la fila se leía gris
/// sucio; parte de [`tema::elevado`] y con el doble, como el resto de filas.
// 0.1.0
// modified 0.1.1 -- en oscuro el acento al 20 % sobre elevado, para que se note
pub fn seleccionada(señalada: f32) -> Color {
    let (base, mezcla) = if tema::es_claro() {
        (tema::card(), 0.10)
    } else {
        (tema::elevado(), 0.20)
    };
    tema::mezclar(base, tema::acento(), mezcla + 0.06 * señalada)
}

/// Hasta dónde llega el relleno de una píldora al `nivel` (de 0 a 100).
///
/// El punto del 0 % ocupa `alto` y los niveles se reparten en lo que queda.
/// Antes el relleno era `ancho × nivel` con ese punto como mínimo, y en la
/// píldora de la Estación de Control (24 de alto) todo lo que había entre 0 y
/// un 12 % se dibujaba igual: no había forma de saber si el volumen estaba a
/// cero o al diez.
// 0.1.0
pub fn lleno(ancho: f32, alto: f32, nivel: u8) -> f32 {
    alto + (ancho - alto) * (nivel.min(100) as f32 / 100.0)
}

/// De una `x` dentro de una píldora de [`PILDORA`] de alto a su nivel,
/// recortado a [0, 100].
///
/// Es la inversa de [`lleno`]: el final del relleno queda bajo el puntero
/// mientras se arrastra, y el 0 y el 100 caen en el centro de los extremos
/// redondos y no en el borde, que con el ratón no se alcanza sin pasarse.
/// Arrastrar más allá deja el nivel en el tope y no en un número imposible.
// 0.1.0
pub fn nivel_en(x: f32, x0: f32, ancho: f32) -> u8 {
    let recorrido = ancho - PILDORA;
    (((x - x0 - PILDORA / 2.0) / recorrido).clamp(0.0, 1.0) * 100.0).round() as u8
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

#[cfg(test)]
mod tests {
    use super::{PILDORA, lleno, nivel_en};

    #[test]
    fn los_extremos_dan_cero_y_cien() {
        assert_eq!(nivel_en(20.0, 20.0, 200.0), 0);
        assert_eq!(nivel_en(220.0, 20.0, 200.0), 100);
        assert_eq!(nivel_en(120.0, 20.0, 200.0), 50);
    }

    /// El 0 y el 10 % tienen que verse distintos. Con el relleno a
    /// `ancho × nivel` y el punto del 0 como mínimo, en una píldora de 200 los
    /// dos medían 24.
    #[test]
    fn el_cero_y_el_diez_no_se_confunden() {
        let ancho = 200.0;
        assert_eq!(lleno(ancho, PILDORA, 0), PILDORA);
        assert!(lleno(ancho, PILDORA, 10) - lleno(ancho, PILDORA, 0) > 15.0);
        assert_eq!(lleno(ancho, PILDORA, 100), ancho);
    }

    /// Pulsar donde acaba el relleno devuelve el mismo nivel: si no, coger la
    /// píldora la haría saltar.
    #[test]
    fn el_final_del_relleno_queda_bajo_el_puntero() {
        for nivel in [0, 7, 10, 50, 93, 100] {
            let x = 20.0 + lleno(200.0, PILDORA, nivel) - PILDORA / 2.0;
            assert_eq!(nivel_en(x, 20.0, 200.0), nivel);
        }
    }

    #[test]
    fn pasarse_de_largo_no_desborda() {
        assert_eq!(nivel_en(-9000.0, 20.0, 200.0), 0);
        assert_eq!(nivel_en(9000.0, 20.0, 200.0), 100);
    }
}
