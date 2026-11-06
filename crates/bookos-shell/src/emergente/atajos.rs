//! La hoja de atajos: todos los atajos de teclado y gestos en una tarjeta.
//!
//! Se abre manteniendo Meta sola un segundo y se va al soltarla, como
//! CheatSheet en macOS; también desde el buscador («Atajos de teclado»), y
//! entonces se cierra con Esc o pulsando fuera.
//!
//! La lista está escrita a mano y tiene que decir lo mismo que
//! `bookos-comp/src/keybinds.rs` (`resolver` y `conmutador_de`): es la única
//! fuente de lo que hace cada tecla. Si cambia un atajo allí, cambia aquí.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{Column, Row, Space, column, container, row, text};

use super::{Ancla, Tecla, t};
use crate::tema;
use crate::view::PanelElement;

const ANCHO: f32 = 1040.0;
const MARGEN: f32 = 32.0;
const HUECO_COLUMNAS: f32 = 28.0;
const COLUMNA: f32 = (ANCHO - MARGEN * 2.0 - HUECO_COLUMNAS * 2.0) / 3.0;
/// Ancho de las teclas en cada columna. Uno común no servía (visto en las
/// capturas): a 128, «Meta+Ctrl+Mayús+←/→» saltaba de línea y pisaba su
/// descripción; a 168, la que saltaba era «Ocultar la barra superior» (con las flechas
/// como teclas sueltas la segunda va a 144 y la tercera a 176). Solo
/// la columna de escritorios lleva la combinación larga. La tarjeta mide 1040
/// lógicos, que entran en una pantalla de 1280.
const TECLAS: [f32; 3] = [128.0, 144.0, 176.0];
/// Ancho de una tecla de una sola marca.
const TECLA_CUADRADA: f32 = 20.0;
/// Alto de toda tecla. Igual para todas: el cuerpo de una flecha o de ⇧ es
/// mayor que el de una letra y, sin esto, su tecla salía más alta y la fila
/// se descuadraba (visto en la captura).
const TECLA_ALTO: f32 = 22.0;
const TITULO: f32 = 40.0;
const CABECERA: f32 = 26.0;
const RENGLON: f32 = 28.0;
const ENTRE_GRUPOS: f32 = 18.0;
const PIE: f32 = 36.0;

/// Una combinación (las teclas separadas por `+`) y qué hace, en los dos
/// idiomas del escritorio.
type Atajo = (&'static str, &'static str, &'static str);
type Grupo = (&'static str, &'static str, &'static [Atajo]);

const GENERAL: Grupo = (
    "General",
    "General",
    &[
        ("Meta", "Launchpad", "Launchpad"),
        ("Meta+Espacio", "Buscar", "Search"),
        ("Meta+Z", "Menú de BookOS", "BookOS menu"),
        ("Meta+X", "Centro de control", "Control Center"),
        ("Meta+E", "Archivos", "Files"),
        ("Meta+Intro", "Terminal", "Terminal"),
        ("Meta+.", "Emojis", "Emoji"),
        ("Meta+Alt+A", "Asistente", "Assistant"),
    ],
);

const CAPTURA: Grupo = (
    "Captura y pantalla",
    "Capture and display",
    &[
        ("Impr", "Captura de pantalla", "Screenshot"),
        ("Meta+Mayús+S", "Captura de pantalla", "Screenshot"),
        ("Meta+Alt+C", "Cuentagotas", "Color picker"),
        ("Meta+P", "Pantallas", "Displays"),
        ("Meta+Alt+O", "Apagar las pantallas", "Turn off displays"),
    ],
);

const VENTANAS: Grupo = (
    "Ventanas",
    "Windows",
    &[
        ("Alt+Tab", "Cambiar de aplicación", "Switch apps"),
        ("Meta+Tab", "Todas las ventanas", "All windows"),
        ("Meta+Q", "Cerrar", "Close"),
        ("Meta+F", "Maximizar", "Maximize"),
        ("Meta+H", "Minimizar", "Minimize"),
        ("Meta+Flechas", "Encajar a un lado", "Snap to a side"),
        ("Meta+Alt+T", "Siempre encima", "Always on top"),
        ("Alt+F3", "Menú de la ventana", "Window menu"),
    ],
);

const SISTEMA: Grupo = (
    "Sistema",
    "System",
    &[
        ("Meta+L", "Bloquear", "Lock"),
        ("Meta+Esc", "Apagar o reiniciar", "Shut down or restart"),
        ("Ctrl+Alt+Supr", "Cerrar sesión", "Log out"),
        ("Meta+Alt+S", "Salvapantallas", "Screen saver"),
        (
            "Meta+Alt+B",
            "Ocultar la barra superior",
            "Hide the top bar",
        ),
        ("Meta+Alt+D", "Ocultar el dock", "Hide the dock"),
    ],
);

const ESCRITORIOS: Grupo = (
    "Escritorios",
    "Desktops",
    &[
        ("Meta+W", "Vista de escritorios", "Desktop overview"),
        ("Meta+1…5", "Ir al escritorio", "Go to desktop"),
        ("Meta+Ctrl+←/→", "Anterior o siguiente", "Previous or next"),
        (
            "Meta+Ctrl+Mayús+←/→",
            "Llevar la ventana",
            "Move the window",
        ),
        ("Meta+Ctrl+D", "Ver el escritorio", "Show desktop"),
    ],
);

/// Los del touchpad, que son la otra mitad de cómo se maneja el escritorio.
const GESTOS: Grupo = (
    "Gestos del touchpad",
    "Trackpad gestures",
    &[
        ("3 dedos ↑", "Todas las ventanas", "All windows"),
        ("4 dedos ↑", "Vista de escritorios", "Desktop overview"),
        ("4 dedos ←/→", "Cambiar de escritorio", "Switch desktop"),
        ("4 dedos ↓", "Ver el escritorio", "Show desktop"),
        ("Pellizco 5 dedos", "Launchpad", "Launchpad"),
    ],
);

/// Tres columnas, repartidas a mano para que queden parejas en alto.
const COLUMNAS: [&[Grupo]; 3] = [
    &[GENERAL, CAPTURA],
    &[VENTANAS, SISTEMA],
    &[ESCRITORIOS, GESTOS],
];

pub struct Atajos;

impl Atajos {
    // añadido en la 0.2.0
    pub fn new() -> Self {
        Self
    }

    // añadido en la 0.2.0
    pub fn size(&self) -> (f32, f32) {
        let alto_columna = |grupos: &[Grupo]| {
            grupos
                .iter()
                .map(|(_, _, atajos)| CABECERA + atajos.len() as f32 * RENGLON)
                .sum::<f32>()
                + ENTRE_GRUPOS * grupos.len().saturating_sub(1) as f32
        };
        let alto = COLUMNAS.iter().map(|c| alto_columna(c)).fold(0.0, f32::max);
        (ANCHO, MARGEN + TITULO + alto + PIE + MARGEN)
    }

    // añadido en la 0.2.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    // añadido en la 0.2.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// Como en «Acerca de»: centrada y sin velo se leería como una ventana.
    // añadido en la 0.2.0
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..tema::bg()
        }
    }

    // añadido en la 0.2.0
    pub fn view(&self) -> PanelElement<'_> {
        let columnas = COLUMNAS.iter().zip(TECLAS).fold(
            Row::new().spacing(HUECO_COLUMNAS),
            |fila, (grupos, teclas)| {
                let columna = grupos.iter().enumerate().fold(
                    Column::new().width(Length::Fixed(COLUMNA)),
                    |col, (i, grupo)| {
                        let col = if i > 0 {
                            col.push(Space::new().height(Length::Fixed(ENTRE_GRUPOS)))
                        } else {
                            col
                        };
                        col.push(grupo_view(grupo, teclas))
                    },
                );
                fila.push(columna)
            },
        );
        let (_, alto) = self.size();
        container(column![
            container(
                text(t("Atajos de teclado", "Keyboard shortcuts"))
                    .size(tema::T_TITULO)
                    .color(tema::texto())
            )
            .height(Length::Fixed(TITULO)),
            columnas,
            Space::new().height(Length::Fill),
            container(
                text(t(
                    "Mantén pulsada Meta para ver esta hoja.",
                    "Hold Meta to see this sheet.",
                ))
                .size(tema::T_PEQUENO)
                .color(tema::TEXTO2)
            )
            .height(Length::Fixed(PIE))
            .align_y(Vertical::Bottom),
        ])
        .padding(MARGEN as u16)
        .width(Length::Fixed(ANCHO))
        .height(Length::Fixed(alto))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                width: 1.0,
                color: tema::borde(),
            },
            ..Default::default()
        })
        .into()
    }
}

// añadido en la 0.2.0
fn grupo_view(&(es, en, atajos): &Grupo, ancho_teclas: f32) -> PanelElement<'static> {
    let cabecera = container(text(t(es, en)).size(tema::T_PEQUENO).color(tema::TEXTO2))
        .height(Length::Fixed(CABECERA))
        .align_y(Vertical::Top);
    atajos
        .iter()
        .fold(column![cabecera], |col, &(teclas, es, en)| {
            col.push(
                row![
                    container(combinacion(teclas)).width(Length::Fixed(ancho_teclas)),
                    text(t(es, en)).size(tema::T_CUERPO).color(tema::texto()),
                ]
                .height(Length::Fixed(RENGLON))
                .align_y(Vertical::Center),
            )
        })
        .into()
}

/// Cada tecla en su chip, como se ven en un teclado: «Meta» «Espacio» y no
/// «Meta+Espacio» en texto corrido, que a golpe de vista es una palabra más.
/// Los gestos van en texto, sin chip: no son teclas.
// añadido en la 0.2.0
fn combinacion(teclas: &'static str) -> PanelElement<'static> {
    if teclas.contains("dedos") {
        // El gesto se escribe una vez, en castellano; solo tiene dos palabras
        // que traducir y una tabla aparte para ellas sería repetir la lista.
        let gesto = if t("es", "en") == "en" {
            teclas
                .replace("Pellizco", "Pinch")
                .replace("dedos", "fingers")
        } else {
            teclas.to_string()
        };
        let corte = gesto.find(['←', '↑', '→', '↓']).unwrap_or(gesto.len());
        let (palabras, flechas) = gesto.split_at(corte);
        let mut fila = Row::new().spacing(6.0).align_y(Vertical::Center).push(
            text(palabras.trim_end().to_owned())
                .size(tema::T_PEQUENO)
                .color(tema::TEXTO2),
        );
        for flecha in flechas.chars().filter_map(flecha) {
            fila = fila.push(tecla_view(flecha, true));
        }
        return fila.into();
    }
    teclas
        .split('+')
        .flat_map(expandir_tecla)
        .fold(
            Row::new().spacing(4.0).align_y(Vertical::Center),
            |fila, tecla| fila.push(tecla_view(traducir_tecla(tecla), false)),
        )
        .into()
}

/// Una tecla. `tenue` es para las de un gesto, que se leen como pista y no
/// como orden: mismo dibujo, con el color del texto secundario.
// añadido en la 0.2.0
fn tecla_view(nombre: &'static str, tenue: bool) -> PanelElement<'static> {
    // Una sola marca —letra, flecha, ⇧— va en una tecla cuadrada, como en un
    // teclado; las de palabra se ajustan a su texto.
    let cuadrada = nombre.chars().count() == 1;
    // Las flechas y ⇧ vienen de otra fuente y a 13 salían diminutas y bajas
    // dentro de su tecla (visto en la captura): una letra de más.
    let tamano = if cuadrada && !nombre.chars().all(char::is_alphanumeric) {
        tema::T_CUERPO + 3.0
    } else if cuadrada {
        tema::T_CUERPO
    } else {
        tema::T_PEQUENO
    };
    let color = if tenue { tema::TEXTO2 } else { tema::texto() };
    let marca = container(text(nombre).size(tamano).color(color))
        .height(Length::Fixed(TECLA_ALTO))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center);
    if cuadrada {
        marca.width(Length::Fixed(TECLA_CUADRADA))
    } else {
        marca.padding([0.0, 6.0])
    }
    .style(|_theme: &iced_widget::Theme| container::Style {
        background: Some(tema::alfa(tema::tinta(), 0.12).into()),
        border: Border {
            radius: tema::R_CHIP.into(),
            width: 1.0,
            color: tema::borde(),
        },
        ..Default::default()
    })
    .into()
}

/// La flecha de un gesto como texto fijo; la barra que separa dos no es una.
// añadido en la 0.2.0
fn flecha(c: char) -> Option<&'static str> {
    match c {
        '←' => Some("←"),
        '↑' => Some("↑"),
        '→' => Some("→"),
        '↓' => Some("↓"),
        _ => None,
    }
}

/// Las flechas son teclas de verdad, cada una la suya: «←/→» como una sola
/// tecla con una barra en medio no existe en ningún teclado.
// añadido en la 0.2.0
fn expandir_tecla(tecla: &'static str) -> Vec<&'static str> {
    match tecla {
        "←/→" => vec!["←", "→"],
        "Flechas" => vec!["←", "↑", "→", "↓"],
        otra => vec![otra],
    }
}

/// El nombre de la tecla en inglés cuando el escritorio va en inglés. Las
/// letras y los símbolos se quedan como están.
// añadido en la 0.2.0
fn traducir_tecla(tecla: &'static str) -> &'static str {
    match tecla {
        "Espacio" => t("Espacio", "Space"),
        "Intro" => t("Intro", "Enter"),
        // El símbolo de Mayús: en el teclado lo lleva en la tecla, y es el
        // mismo en los dos idiomas.
        "Mayús" => "⇧",
        "Supr" => t("Supr", "Delete"),
        "Impr" => t("Impr", "PrtSc"),
        otra => otra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada tecla de una combinación tiene que ser un nombre que alguien
    /// encuentre en el teclado: sin espacios sueltos ni `+` dobles.
    #[test]
    fn las_combinaciones_estan_bien_escritas() {
        for grupos in COLUMNAS {
            for (_, _, atajos) in grupos.iter() {
                for (teclas, es, en) in atajos.iter() {
                    assert!(!es.is_empty() && !en.is_empty(), "{teclas} sin descripción");
                    if teclas.contains("dedos") {
                        continue;
                    }
                    for tecla in teclas.split('+') {
                        assert!(!tecla.is_empty() && tecla.trim() == tecla, "«{teclas}»");
                    }
                }
            }
        }
    }

    /// Las flechas son teclas sueltas, no una con una barra; y Mayús es su
    /// símbolo en cualquier idioma.
    #[test]
    fn las_flechas_se_reparten_en_teclas_y_mayus_es_su_simbolo() {
        assert_eq!(expandir_tecla("←/→"), vec!["←", "→"]);
        assert_eq!(expandir_tecla("Flechas"), vec!["←", "↑", "→", "↓"]);
        assert_eq!(expandir_tecla("Meta"), vec!["Meta"]);
        assert_eq!(traducir_tecla("Mayús"), "⇧");
        assert_eq!(flecha('↑'), Some("↑"));
        assert_eq!(flecha('/'), None);
    }

    /// El número de escritorios de Meta+1…N es el máximo del compositor.
    #[test]
    fn meta_numero_llega_al_maximo_de_escritorios() {
        let esperado = format!("Meta+1…{}", crate::MAXIMO_ESCRITORIOS);
        assert!(
            ESCRITORIOS
                .2
                .iter()
                .any(|(teclas, _, _)| *teclas == esperado)
        );
    }
}
