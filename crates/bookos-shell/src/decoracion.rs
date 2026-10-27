//! La barra de título que dibuja el escritorio: los tres botones de ventana.
//!
//! Existe porque en Wayland la decoración es del cliente salvo que el
//! compositor la reclame (`xdg-decoration`). Un escritorio donde cada programa
//! pinta sus propios botones —GTK unos, Qt otros, Electron los suyos— no es un
//! escritorio, es una colección de aplicaciones; esto es la mitad que sí
//! podemos unificar.
//!
//! Los glifos son los del boceto: una raya para minimizar, un cuadrado para
//! maximizar y una cruz para cerrar. Van en SVG generado aquí y no como ficheros
//! del tema de iconos por lo mismo que el pictograma de la batería: el color y
//! el grosor cambian con el estado (activa, señalada, maximizada) y un tema de
//! iconos solo tiene escalones fijos.
//!
//! El dibujo vive en el shell y no en el compositor porque aquí están el tema y
//! el rasterizador; el compositor decide **qué** ventana lleva barra y qué pasa
//! al pulsarla.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Length};
use iced_widget::{Space, container, row, text};

use crate::tema;
use crate::view::PanelElement;

/// Alto de la barra, en píxeles lógicos. El mismo que el panel: dos franjas de
/// distinto grosor en la misma pantalla se ven como un error de montaje.
pub const ALTO: f32 = 32.0;

/// Lado de la zona sensible de cada botón, y separación entre ellos.
///
/// El HIG reserva 28 px dentro de una titlebar de 36 px. Aquí la barra es de
/// 32 px, así que 28 deja 2 px de aire arriba y abajo y permite que la tarjeta
/// redondeada se vea como la referencia sin robar altura a la ventana.
const LADO: f32 = 28.0;
const HUECO: f32 = 4.0;
/// Aire entre el último botón y el borde derecho de la ventana.
const MARGEN: f32 = 6.0;
/// Lado del dibujo dentro del botón.
const GLIFO: f32 = 16.0;

/// Los tres de siempre, en el orden en que se pintan de izquierda a derecha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boton {
    Minimizar,
    Maximizar,
    Cerrar,
}

impl Boton {
    /// De derecha a izquierda: el cierre pegado al borde, que es donde lo busca
    /// la mano.
    const ORDEN: [Boton; 3] = [Boton::Cerrar, Boton::Maximizar, Boton::Minimizar];
}

/// Lo que hay que saber de una ventana para dibujar su barra.
///
/// Se pasa entero en cada repintado en vez de guardarse: el compositor ya tiene
/// estos datos y duplicarlos aquí es la forma segura de que una de las dos
/// copias se quede atrás.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Estado {
    pub pestanas: Vec<String>,
    pub pestana_activa: usize,
    pub titulo: String,
    /// La que tiene el foco. Las demás se apagan, que es lo único que distingue
    /// de un vistazo dónde van a ir las teclas.
    pub activa: bool,
    pub maximizada: bool,
    /// El botón bajo el cursor, y el que se está pulsando.
    pub señalado: Option<Boton>,
    pub pulsado: Option<Boton>,
}

/// Qué botón cae en `(x, y)`, con las coordenadas relativas a la esquina
/// superior izquierda de la barra y en píxeles lógicos.
///
/// El hit-test lo hace el compositor antes de que iced calcule ningún layout
/// —igual que en el panel—, así que la geometría está aquí escrita a mano y es
/// la que el dibujo tiene que respetar.
pub fn boton_en(ancho: f32, x: f32, y: f32) -> Option<Boton> {
    if y < 0.0 || y >= ALTO || x < 0.0 || x >= ancho {
        return None;
    }
    let aire_vertical = (ALTO - LADO) / 2.0;
    if y < aire_vertical || y >= aire_vertical + LADO {
        return None;
    }
    let paso = LADO + HUECO;
    // Desde el borde derecho hacia dentro, que es como están colocados.
    let desde_derecha = ancho - MARGEN - x;
    if desde_derecha < 0.0 {
        return None;
    }
    let i = (desde_derecha / paso).floor() as usize;
    // El hueco visual entre dos tarjetas no pertenece a ninguna. Antes esos
    // cuatro píxeles encendían el botón de su derecha y el hover parecía ir
    // desalineado respecto a lo que se veía.
    if desde_derecha % paso >= LADO {
        return None;
    }
    Boton::ORDEN.get(i).copied()
}

/// Ancho que ocupan los tres botones con su margen: lo que el título no puede
/// invadir.
fn ancho_botones() -> f32 {
    MARGEN + 3.0 * (LADO + HUECO)
}

pub fn pestana_en(ancho: f32, cantidad: usize, x: f32) -> Option<usize> {
    let libre = (ancho - ancho_botones()).max(0.0);
    if cantidad == 0 || x < 0.0 || x >= libre {
        return None;
    }
    Some(((x / libre * cantidad as f32) as usize).min(cantidad - 1))
}

/// El SVG de un glifo. `viewBox` de 16 y trazo de 1,4.
///
/// Sale en blanco y lo tiñe iced al dibujarlo, como cualquier icono de estado:
/// el color depende del foco y del cursor, y meterlo dentro del SVG obligaría a
/// rasterizar de nuevo por cada cambio de tinta.
fn glifo(boton: Boton, maximizada: bool) -> String {
    const TRAZO: &str = r##"fill="none" stroke="#ffffff" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round""##;
    let dibujo = match boton {
        // La referencia usa una raya horizontal, no el chevron anterior.
        Boton::Minimizar => format!(r#"<path d="M3.5 8 H12.5" {TRAZO}/>"#),
        // Maximizar es un cuadrado. Cuando ya está maximizada se enseñan dos
        // cuadrados superpuestos para comunicar que el botón restaura.
        Boton::Maximizar if maximizada => {
            r##"<path d="M5.5 3.5 H12.5 V10.5 H5.5 Z M3.5 5.5 V12.5 H10.5" fill="none" stroke="#ffffff" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"##.to_string()
        }
        Boton::Maximizar => format!(r#"<path d="M3.5 3.5 H12.5 V12.5 H3.5 Z" {TRAZO}/>"#),
        Boton::Cerrar => format!(r#"<path d="M4.6 4.6 L11.4 11.4 M11.4 4.6 L4.6 11.4" {TRAZO}/>"#),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">{dibujo}</svg>"#
    )
}

/// Un botón: la tarjeta redondeada y el glifo encima.
fn boton(cual: Boton, estado: &Estado) -> PanelElement<'static> {
    let señalado = estado.señalado == Some(cual);
    let pulsado = estado.pulsado == Some(cual);

    // El glifo de una ventana sin foco se apaga en vez de desaparecer: en el
    // boceto los tres siguen ahí, más flojos. Bajo el cursor sube a tinta
    // entera, que es lo que dice «esto responde».
    let tinta = match (estado.activa, señalado || pulsado) {
        (_, true) => tema::texto(),
        (true, false) => tema::alfa(tema::texto(), 0.55),
        (false, false) => tema::alfa(tema::texto(), 0.30),
    };
    // En reposo no hay placa, solo el glifo: con una tarjeta gris detrás de
    // cada uno, los tres botones pesaban más que el título y se veían en todas
    // las ventanas a la vez. La placa sale al señalar, que es cuando dice
    // «esto responde», con contraste de sobra para notarse en HiDPI. El cierre
    // solo se vuelve rojo al señalarlo.
    let fondo = match (cual, señalado, pulsado) {
        (Boton::Cerrar, _, true) => Some(tema::alfa(tema::rojo(), 1.0)),
        (Boton::Cerrar, true, false) => Some(tema::alfa(tema::rojo(), 0.95)),
        (_, _, true) => Some(tema::alfa(tema::tinta(), 0.22)),
        (_, true, false) => Some(tema::alfa(
            tema::tinta(),
            if tema::es_claro() { 0.14 } else { 0.17 },
        )),
        (_, false, false) => None,
    };
    // Solo la variante destructiva lleva la tinta calculada sobre rojo. En el
    // estado normal la X sigue el mismo color que los demás glifos.
    let tinta = match (fondo, cual == Boton::Cerrar && (señalado || pulsado)) {
        (Some(f), true) => tema::tinta_sobre(f),
        _ => tinta,
    };

    let icono = crate::icono::desde_svg(&glifo(cual, estado.maximizada));
    container(crate::icono::ver_teñido_propio(&icono, GLIFO, tinta))
        .width(Length::Fixed(LADO))
        .height(Length::Fixed(LADO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: fondo.map(Into::into),
            border: Border {
                radius: tema::R_CHIP.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La barra entera, para un ancho de ventana dado en píxeles lógicos.
pub fn vista(estado: &Estado, ancho: f32) -> PanelElement<'static> {
    let mut botones = row![].align_y(Vertical::Center);
    for (i, cual) in [Boton::Minimizar, Boton::Maximizar, Boton::Cerrar]
        .into_iter()
        .enumerate()
    {
        if i > 0 {
            botones = botones.push(Space::new().width(Length::Fixed(HUECO)));
        }
        botones = botones.push(boton(cual, estado));
    }

    // El título se centra en la **barra** y no entre sus vecinos: con `row` se
    // corría cada vez que el texto cambiaba de largo. El recorte se hace por
    // caracteres contra el hueco libre —el layout de iced no llega aquí— y deja
    // el mismo margen a los dos lados para que el centrado siga siendo cierto.
    let libre = (ancho - ancho_botones() * 2.0 - 16.0).max(0.0);
    let titulo = recortar(&estado.titulo, libre);
    let color = if estado.activa {
        tema::alfa(tema::texto(), 0.85)
    } else {
        tema::TEXTO2
    };
    let rotulo: PanelElement<'static> = container(
        text(titulo)
            .size(tema::T_PEQUENO)
            .color(color)
            .align_x(Horizontal::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .into();

    let fila = if !estado.pestanas.is_empty() {
        let libre = (ancho - ancho_botones()).max(0.0);
        let ancho_pestana = libre / estado.pestanas.len() as f32;
        let mut pestanas = row![].height(Length::Fill);
        for (i, titulo) in estado.pestanas.iter().enumerate() {
            let activa = i == estado.pestana_activa;
            let titulo = recortar(titulo, (ancho_pestana - 20.0).max(0.0));
            pestanas = pestanas.push(
                container(text(titulo).size(tema::T_PEQUENO).color(if activa {
                    tema::texto()
                } else {
                    tema::TEXTO2
                }))
                .width(Length::Fixed(ancho_pestana))
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: activa.then(|| tema::alfa(tema::tinta(), 0.12).into()),
                    ..Default::default()
                }),
            );
        }
        row![pestanas, botones, Space::new().width(Length::Fixed(MARGEN))]
            .align_y(Vertical::Center)
            .height(Length::Fill)
    } else {
        row![
            Space::new().width(Length::Fixed(ancho_botones())),
            rotulo,
            botones,
            Space::new().width(Length::Fixed(MARGEN)),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fill)
    };

    // Restaurada, la barra forma las dos esquinas superiores de la ventana.
    // Maximizada toca los bordes de la salida y debe volver a ser rectangular:
    // un radio ahí dejaría ver dos cuñas del fondo en las esquinas.
    let radio = if estado.maximizada {
        iced_core::border::Radius::default()
    } else {
        iced_core::border::Radius::default()
            .top_left(tema::R_CONTROL)
            .top_right(tema::R_CONTROL)
    };
    container(fila)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: radio,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// Corta el título a lo que quepa en `ancho`, con puntos suspensivos.
///
/// Mide con el mismo motor que pinta ([`crate::widget::ancho_de`]) en vez de
/// contar caracteres: un título es texto arbitrario —«IIIII» y «MMMMM» no
/// ocupan lo mismo— y aquí el error se ve como un rótulo que se mete debajo de
/// los botones.
fn recortar(titulo: &str, ancho: f32) -> String {
    let titulo = titulo.trim();
    if ancho <= 0.0 {
        return String::new();
    }
    if crate::widget::ancho_de(titulo, tema::T_PEQUENO) <= ancho {
        return titulo.to_string();
    }
    let letras: Vec<char> = titulo.chars().collect();
    // Búsqueda binaria sobre cuántos caracteres caben: medir uno a uno son
    // tantos shapings como letras tenga el título, y esto se hace en cada
    // repintado de cada barra.
    let (mut bajo, mut alto) = (0usize, letras.len());
    while bajo < alto {
        let medio = (bajo + alto).div_ceil(2);
        let prueba: String = letras[..medio].iter().collect::<String>() + "…";
        if crate::widget::ancho_de(&prueba, tema::T_PEQUENO) <= ancho {
            bajo = medio;
        } else {
            alto = medio - 1;
        }
    }
    if bajo == 0 {
        return String::new();
    }
    letras[..bajo].iter().collect::<String>() + "…"
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_botones_se_pulsan_de_derecha_a_izquierda() {
        let ancho = 800.0;
        // El centro de cada zona, contando desde el borde derecho.
        let centro = |i: f32| ancho - MARGEN - i * (LADO + HUECO) - LADO / 2.0;
        assert_eq!(boton_en(ancho, centro(0.0), 16.0), Some(Boton::Cerrar));
        assert_eq!(boton_en(ancho, centro(1.0), 16.0), Some(Boton::Maximizar));
        assert_eq!(boton_en(ancho, centro(2.0), 16.0), Some(Boton::Minimizar));
    }

    #[test]
    fn fuera_de_la_barra_no_hay_boton() {
        assert_eq!(boton_en(800.0, 400.0, 16.0), None);
        assert_eq!(boton_en(800.0, 799.0, ALTO + 1.0), None);
        // El margen de la derecha no es un botón: es el aire del borde.
        assert_eq!(boton_en(800.0, 799.0, 16.0), None);
        // Tampoco el aire vertical ni la separación entre dos tarjetas.
        assert_eq!(boton_en(800.0, 780.0, 1.0), None);
        assert_eq!(boton_en(800.0, 764.0, 16.0), None);
    }
}
