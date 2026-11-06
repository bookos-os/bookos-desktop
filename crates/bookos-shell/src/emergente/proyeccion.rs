//! Selector de proyección de Fn+F4.

use iced_core::alignment::Horizontal;
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use super::{Ancla, Tecla};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const MARGEN: f32 = 20.0;
/// Aire transparente dentro del buffer para que quepa la sombra de la tarjeta.
///
/// Sin él la tarjeta ocupaba la superficie entera: iced dibujaba la sombra
/// fuera del rectángulo, el buffer la cortaba por sus cuatro cantos y durante
/// el zoom de entrada ese recorte se veía como un cuadrado desenfocado que
/// tardaba un instante en convertirse en las esquinas redondeadas.
const MARGEN_SOMBRA: f32 = 22.0;
const CELDA_W: f32 = 150.0;
const CELDA_H: f32 = 126.0;
const HUECO: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Principal,
    Externa,
    Duplicar,
    Extender,
    SinCambios,
}

impl Modo {
    const TODOS: [Self; 5] = [
        Self::Principal,
        Self::Externa,
        Self::Duplicar,
        Self::Extender,
        Self::SinCambios,
    ];
    // 0.1.0
    fn nombre(self) -> &'static str {
        match self {
            Self::Principal => "Solo principal",
            Self::Externa => "Solo externa",
            Self::Duplicar => "Duplicar",
            Self::Extender => "Extender",
            Self::SinCambios => "Sin cambios",
        }
    }
    // 0.1.0
    fn simbolo(self) -> &'static str {
        match self {
            Self::Principal => "▣  □",
            Self::Externa => "□  ▣",
            Self::Duplicar => "▣ ▣",
            Self::Extender => "▣│▣",
            Self::SinCambios => "⊘",
        }
    }
}

pub struct Proyeccion {
    conectadas: usize,
    elegida: usize,
}

impl Proyeccion {
    // 0.1.0
    pub fn new(conectadas: usize) -> Self {
        Self {
            conectadas,
            elegida: 3,
        }
    }
    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.contenido_size();
        (w + MARGEN_SOMBRA * 2.0, h + MARGEN_SOMBRA * 2.0)
    }
    // 0.1.0
    fn contenido_size(&self) -> (f32, f32) {
        if self.conectadas < 2 {
            (430.0, 180.0)
        } else {
            (MARGEN * 2.0 + CELDA_W * 5.0 + HUECO * 4.0, 206.0)
        }
    }
    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }
    // 0.1.0
    pub fn animando(&self) -> bool {
        false
    }
    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((x, y)) = punto.map(|(x, y)| (x - MARGEN_SOMBRA, y - MARGEN_SOMBRA)) else {
            return false;
        };
        if self.conectadas < 2 || y < 54.0 || y >= 54.0 + CELDA_H {
            return false;
        }
        let paso = CELDA_W + HUECO;
        let i = ((x - MARGEN) / paso).floor() as isize;
        if !(0..5).contains(&i) || (x - MARGEN) % paso > CELDA_W {
            return false;
        }
        let i = i as usize;
        let cambio = i != self.elegida;
        self.elegida = i;
        cambio
    }
    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (w, h) = self.contenido_size();
        if x < MARGEN_SOMBRA
            || y < MARGEN_SOMBRA
            || x >= MARGEN_SOMBRA + w
            || y >= MARGEN_SOMBRA + h
        {
            return None;
        }
        self.puntero(Some((x, y)));
        (self.conectadas >= 2).then(|| Accion::Proyeccion(Modo::TODOS[self.elegida]))
    }
    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        match tecla {
            TeclaPulsada::Escape => Tecla::Cerrar,
            TeclaPulsada::Izquierda => {
                self.elegida = (self.elegida + 4) % 5;
                Tecla::Consumida
            }
            TeclaPulsada::Derecha => {
                self.elegida = (self.elegida + 1) % 5;
                Tecla::Consumida
            }
            TeclaPulsada::Intro if self.conectadas >= 2 => {
                Tecla::Hacer(Accion::Proyeccion(Modo::TODOS[self.elegida]))
            }
            _ => Tecla::Ignorada,
        }
    }
    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let titulo = text("Proyección de pantalla")
            .size(tema::T_TITULO)
            .color(tema::texto());
        if self.conectadas < 2 {
            let tarjeta = container(
                column![
                    titulo,
                    Space::new().height(18),
                    text("Conecta otra pantalla para elegir cómo usarla.")
                        .size(tema::T_CUERPO)
                        .color(tema::TEXTO2)
                ]
                .align_x(Horizontal::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .style(tarjeta);
            return container(tarjeta).padding(MARGEN_SOMBRA).into();
        }
        let mut opciones = row![].spacing(HUECO);
        for (i, modo) in Modo::TODOS.into_iter().enumerate() {
            let activa = i == self.elegida;
            opciones = opciones.push(
                container(
                    column![
                        Space::new().height(10),
                        text(modo.simbolo()).size(43).color(if activa {
                            tema::acento()
                        } else {
                            tema::TEXTO2
                        }),
                        Space::new().height(10),
                        text(modo.nombre())
                            .size(tema::T_PEQUENO)
                            .color(tema::texto())
                            .align_x(Horizontal::Center),
                    ]
                    .align_x(Horizontal::Center),
                )
                .width(CELDA_W)
                .height(CELDA_H)
                .center(Length::Fill)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        tema::alfa(
                            if activa { tema::acento() } else { tema::card() },
                            if activa { 0.16 } else { 0.72 },
                        )
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_CONTROL.into(),
                        width: if activa { 2.0 } else { 1.0 },
                        color: if activa {
                            tema::acento()
                        } else {
                            tema::borde()
                        },
                    },
                    ..Default::default()
                }),
            );
        }
        let tarjeta = container(
            column![titulo, Space::new().height(16), opciones].align_x(Horizontal::Center),
        )
        .padding(MARGEN)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(tarjeta);
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }
}

// 0.1.0
fn tarjeta(_: &iced_widget::Theme) -> iced_widget::container::Style {
    iced_widget::container::Style {
        background: Some(tema::card().into()),
        border: Border {
            radius: tema::R_TARJETA.into(),
            width: 1.0,
            color: tema::borde(),
        },
        // Popover y no modal aunque sea un diálogo: la de modal pide 56 px
        // alrededor y el búfer reserva 22.
        shadow: tema::sombra_popover(),
        ..Default::default()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn sin_segunda_pantalla_no_ofrece_acciones() {
        let mut p = Proyeccion::new(1);
        assert!(p.pulsar(100.0, 100.0).is_none());
        assert!(matches!(p.tecla(TeclaPulsada::Intro), Tecla::Ignorada));
    }

    #[test]
    fn raton_y_teclado_eligen_un_modo() {
        let mut p = Proyeccion::new(2);
        let x_extender = MARGEN_SOMBRA + MARGEN + 3.0 * (CELDA_W + HUECO) + CELDA_W / 2.0;
        assert!(!p.puntero(Some((x_extender, 80.0)))); // ya nace en Extender
        assert!(matches!(
            p.pulsar(x_extender, 80.0),
            Some(Accion::Proyeccion(Modo::Extender))
        ));
        assert!(matches!(p.tecla(TeclaPulsada::Derecha), Tecla::Consumida));
        assert!(matches!(
            p.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::Proyeccion(Modo::SinCambios))
        ));
    }
}
