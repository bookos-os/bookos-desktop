//! Indicadores de privacidad activos en la sesión (micrófono naranja,
//! cámara verde, pantalla compartida morada) al estilo macOS.
//!
//! Vive en su propio widget para poder colocarlo junto al indicador de
//! escritorios y permitir que el usuario cambie su posición en el panel sin
//! poder quitarlo. Cuando no hay ningún sensor activo ocupa `0.0` px para no
//! dejar un hueco vacío.

use iced_core::{Alignment, Border, Color, Length};
use iced_widget::{container, row};

use crate::view::PanelElement;
use crate::widget::Widget;

/// Indicadores de privacidad activos en la sesión.
// añadido en la 0.2.4
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privacidad {
    pub microfono: bool,
    pub camara: bool,
    pub pantalla: bool,
}

impl Privacidad {
    // añadido en la 0.2.4
    pub fn cuantos(self) -> usize {
        usize::from(self.microfono) + usize::from(self.camara) + usize::from(self.pantalla)
    }
}

const PUNTO: f32 = 6.0;
const HUECO_PUNTO: f32 = 3.0;
const COLOR_MICRO: Color = Color::from_rgb(1.0, 0.624, 0.039); // #FF9F0A
const COLOR_CAMARA: Color = Color::from_rgb(0.188, 0.820, 0.345); // #30D158
const COLOR_PANTALLA: Color = Color::from_rgb(0.749, 0.353, 0.949); // #BF5AF2

/// Widget del panel que muestra los puntos de privacidad activos.
pub struct Indicador {
    estado: Privacidad,
}

impl Indicador {
    pub fn new() -> Self {
        Self {
            estado: Privacidad::default(),
        }
    }
}

fn punto(color: Color) -> PanelElement<'static> {
    container(crate::widget::vacio())
        .width(Length::Fixed(PUNTO))
        .height(Length::Fixed(PUNTO))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: (PUNTO / 2.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

impl Widget for Indicador {
    fn nombre(&self) -> &'static str {
        "privacidad"
    }

    fn accesible(&self) -> (&'static str, String) {
        let mut activos = Vec::new();
        if self.estado.microfono {
            activos.push(crate::emergente::t("micrófono activo", "microphone active"));
        }
        if self.estado.camara {
            activos.push(crate::emergente::t("cámara activa", "camera active"));
        }
        if self.estado.pantalla {
            activos.push(crate::emergente::t(
                "compartiendo pantalla",
                "screen sharing active",
            ));
        }
        (
            crate::emergente::t("Privacidad", "Privacy"),
            activos.join(", "),
        )
    }

    fn refrescar(&mut self) -> bool {
        false
    }

    fn privacidad(&mut self, estado: Privacidad) -> bool {
        if self.estado == estado {
            return false;
        }
        self.estado = estado;
        true
    }

    fn ancho(&self) -> f32 {
        let n = self.estado.cuantos();
        if n == 0 {
            0.0
        } else {
            n as f32 * PUNTO + (n.saturating_sub(1)) as f32 * HUECO_PUNTO
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        if self.estado.cuantos() == 0 {
            return crate::widget::vacio();
        }
        let mut puntos = row![].spacing(HUECO_PUNTO).align_y(Alignment::Center);
        if self.estado.microfono {
            puntos = puntos.push(punto(COLOR_MICRO));
        }
        if self.estado.camara {
            puntos = puntos.push(punto(COLOR_CAMARA));
        }
        if self.estado.pantalla {
            puntos = puntos.push(punto(COLOR_PANTALLA));
        }
        puntos.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_indicadores_no_ocupa_sitio() {
        let ind = Indicador::new();
        assert_eq!(ind.ancho(), 0.0);
    }

    #[test]
    fn con_indicadores_ocupa_sitio_y_solo_repinta_si_cambia() {
        let mut ind = Indicador::new();
        let uno = Privacidad {
            camara: true,
            ..Default::default()
        };
        assert!(ind.privacidad(uno));
        assert!(!ind.privacidad(uno));
        assert_eq!(ind.ancho(), PUNTO);

        let dos = Privacidad {
            microfono: true,
            camara: true,
            pantalla: false,
        };
        assert!(ind.privacidad(dos));
        assert_eq!(ind.ancho(), 2.0 * PUNTO + HUECO_PUNTO);
    }
}
