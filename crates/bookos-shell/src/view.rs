//! La vista del panel.
//!
//! Los colores no se eligen aquí: salen de [`crate::tema`], que es el sistema de
//! diseño que ya siguen los widgets de Plasma de BookOS. Antes esta paleta era
//! propia y aproximada —el acento era `#5A8FFA` en vez del `#0A84FF` del
//! sistema— y eso hacía que el panel y los plasmoides no pareciesen del mismo
//! escritorio.

use iced_core::theme::Palette;
use iced_core::{Element, Length, Theme};
use iced_tiny_skia::Renderer;
use iced_widget::{Space, container, row, stack};

use crate::tema;
use crate::widget::Panel;

// Los nombres viejos siguen valiendo dentro del crate: son los roles que usan
// los widgets, y apuntan a los tokens del sistema.
pub use tema::acento as ACENTO;
pub use tema::rojo as PELIGRO;
pub use tema::texto as TEXT;
pub use tema::verde as OK;

// 0.1.0
pub fn theme() -> Theme {
    Theme::custom(
        "BookOS",
        Palette {
            background: tema::panel(),
            text: tema::texto(),
            primary: tema::acento(),
            success: tema::verde(),
            warning: tema::amarillo(),
            danger: tema::rojo(),
        },
    )
}

pub type PanelElement<'a> = Element<'a, (), Theme, Renderer>;

/// Separación entre los estados de la derecha.
///
/// 22 px entre un icono y el siguiente —12 más 10 a petición—. Con 5 los
/// widgets se pegaban unos a otros y la fila se leía como un bloque sin
/// separar bluetooth de red, de volumen, etc.; con 20 —el valor original— sí
/// se distinguían, pero con nueve widgets se comía 160 px de panel en puro
/// aire, así que bajó a medio camino de los dos. Se volvió a subir porque a
/// 12 seguía leyéndose apretado.
///
/// El hueco no es un espacio suelto entre elementos: va **dentro** de cada
/// ranura, mitad a cada lado. Así el resalte del puntero cubre el icono con
/// aire alrededor, y la zona de clic —que ya repartía medio hueco a cada
/// vecino— coincide exactamente con la píldora que se pinta.
pub const HUECO: f32 = 22.0;
/// Margen izquierdo y derecho del panel.
///
/// 16 y no 12: un margen menor deja el reloj demasiado pegado al borde de la
/// pantalla, y se lee como si estuviera a punto de salirse.
pub const MARGEN_PANEL: f32 = 16.0;
/// Lado del logo, en lógicos. 20 sobre un panel de 32 deja 6 de aire arriba y
/// abajo, que es lo que hace que no parezca metido con calzador.
pub const LADO_LOGO: f32 = 20.0;
/// Zona sensible de la marca, a la izquierda: pulsar ahí abre el menú.
///
/// Es un ancho fijo y no el del elemento porque el layout lo calcula iced y
/// aquí hace falta antes, para el hit-test. Cubre el margen de 16 y el logo de
/// 20, más un blanco de cortesía para no fallar el clic por dos píxeles: el
/// logo va solo, sin el nombre escrito al lado, y una zona de 90 se comía el
/// hueco de la izquierda sin nada que la justificara.
pub const ANCHO_LOGO: f32 = 44.0;

// 0.1.0
pub fn panel(widgets: &Panel) -> PanelElement<'_> {
    // Los widgets se montan en una lista y no en un `row!` fijo porque casi
    // todos pueden faltar: sin batería, sin red o sin retroiluminación no se
    // dibuja un hueco vacío, se dibuja una cosa menos.
    let mut derecha = row![].align_y(iced_core::alignment::Vertical::Center);
    for estado in widgets.derecha() {
        derecha = derecha.push(estado);
    }

    let fila = row![
        Space::new().width(Length::Fixed(MARGEN_PANEL)),
        crate::marca::ver(LADO_LOGO),
        Space::new().width(crate::FILL),
        derecha,
        // Cada ranura ya trae medio hueco a su derecha: se descuenta aquí para
        // que el último icono quede a `MARGEN_PANEL` del borde, que es donde lo
        // colocan las zonas de clic.
        Space::new().width(Length::Fixed(MARGEN_PANEL - HUECO / 2.0)),
    ]
    .align_y(iced_core::alignment::Vertical::Center)
    .height(Length::Fill);

    // El widget del centro va en su propia capa, no dentro de la fila, para
    // que esté centrado en la **pantalla** y no entre sus vecinos. Metido en la
    // fila se desplazaba cada vez que la derecha cambiaba de ancho: bastaba con
    // que la batería pasara de "85%" a "85% 2:15" para que la hora diese un
    // salto.
    let contenido: PanelElement<'_> = match widgets.centro() {
        Some(centro) => stack![fila, centro].into(),
        None => fila.into(),
    };

    container(contenido)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(tema::panel().into()),
            ..Default::default()
        })
        .into()
}
