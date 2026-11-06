//! Vista general y gestión de escritorios virtuales.
//!
//! Toma la forma de la barra de espacios de Mission Control: una **franja de
//! lado a lado pegada al borde de arriba**, translúcida sobre el escritorio,
//! con las miniaturas centradas, su nombre debajo y el botón de añadir suelto
//! en el extremo derecho. Se probó como tarjeta flotante centrada y no es lo
//! mismo: una tarjeta se lee como un diálogo que hay que atender, y esto es un
//! sitio al que se va —quedarse mirando el escritorio de al lado por el rabillo
//! del ojo— así que la franja tiene que seguir dejando ver lo que hay debajo.
//!
//! Por eso **no lleva velo**: el escritorio de debajo se ve tal cual, como en
//! macOS. Apagarlo diría «esto es modal, resuélvelo» y no lo es.
//!
//! Las miniaturas son huecos: el compositor coloca dentro el fondo de pantalla
//! y las superficies vivas de cada escritorio, **por delante** de esta franja.
//! Van 2 px hacia dentro de la previa para que el marco redondeado siga
//! viéndose alrededor: lo que se compone encima tiene esquinas rectas.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length, Rectangle};
use iced_widget::{Space, column, container, row, text};

use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

use super::{Ancla, Tecla};

/// Aire entre el borde de arriba de la pantalla y las miniaturas.
///
/// La franja empieza en el borde —está pegada arriba— pero las previas caen por
/// debajo del panel, que mide 32 px lógicos y se sigue viendo delante.
const AIRE_SUPERIOR: f32 = 40.0;
/// Aire por debajo del nombre, hasta el final de la franja.
const AIRE_INFERIOR: f32 = 14.0;
/// Margen lateral: lo que queda libre a los lados de la franja.
const MARGEN: f32 = 24.0;
const HUECO: f32 = 18.0;
/// Ancho de una previa, dentro de lo que caben las que haya.
const PREVIA_MIN: f32 = 130.0;
const PREVIA_MAX: f32 = 250.0;
/// Cuánto se mete hacia dentro lo que compone el compositor, para que el marco
/// redondeado de la previa no quede tapado por un contenido de esquinas rectas.
const PREVIA_INSET: f32 = 2.0;
/// El radio del hueco donde el compositor monta el fondo y las ventanas: el
/// del marco menos lo que se mete hacia dentro, para que las dos curvas vayan
/// paralelas.
pub const RADIO_MINIATURA: f32 = tema::R_BOTON_PEQUENO - PREVIA_INSET;
/// Aire entre la previa y su nombre.
const AIRE_NOMBRE: f32 = 6.0;
const NOMBRE_H: f32 = 22.0;
/// Diámetro del botón de añadir, que es un círculo en el extremo derecho.
const MAS: f32 = 44.0;
/// Lado del botón de cerrar de cada previa: el alto de la línea del nombre,
/// donde vive.
const CERRAR: f32 = NOMBRE_H;
const DOBLE_CLIC: std::time::Duration = std::time::Duration::from_millis(450);

pub struct Escritorios {
    pantalla: (f32, f32),
    nombres: Vec<String>,
    activo: usize,
    hover: tema::Realce,
    editando: Option<(usize, String)>,
    ultimo_nombre: Option<(usize, std::time::Instant)>,
    cerrar: crate::Icono,
    anadir: crate::Icono,
}

impl Escritorios {
    // 0.1.0
    pub fn new(pantalla: (f32, f32), activo: usize, nombres: Vec<String>) -> Self {
        Self {
            pantalla,
            nombres,
            activo,
            hover: tema::Realce::nuevo(),
            editando: None,
            ultimo_nombre: None,
            cerrar: crate::icono::desde_svg(include_str!("../../assets/iconos/vista-cerrar.svg")),
            anadir: crate::icono::desde_svg(include_str!("../../assets/iconos/vista-anadir.svg")),
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (self.pantalla.0, self.alto_franja())
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Arriba
    }

    // 0.1.0
    fn cabe_otro(&self) -> bool {
        self.nombres.len() < 5
    }

    // 0.1.0
    fn ancho_previa(&self) -> f32 {
        let n = self.nombres.len() as f32;
        // Los lados se reservan **simétricos** aunque el botón solo esté a la
        // derecha: las miniaturas van centradas en la pantalla, y descontar el
        // botón de un solo lado las dejaría torcidas respecto al escritorio.
        let reserva = (MARGEN + MAS + HUECO) * 2.0;
        ((self.pantalla.0 - reserva - HUECO * (n - 1.0).max(0.0)) / n).clamp(PREVIA_MIN, PREVIA_MAX)
    }

    /// El alto sale de la proporción de la pantalla: una previa 16:9 sobre un
    /// monitor 16:10 enseñaría el escritorio recortado, y lo que se compone
    /// dentro es la pantalla entera a escala.
    // 0.1.0
    fn alto_previa(&self) -> f32 {
        (self.ancho_previa() * (self.pantalla.1 / self.pantalla.0.max(1.0))).round()
    }

    // 0.1.0
    fn alto_franja(&self) -> f32 {
        AIRE_SUPERIOR + self.alto_previa() + AIRE_NOMBRE + NOMBRE_H + AIRE_INFERIOR
    }

    /// Lo que ocupan las miniaturas juntas, que es lo que se centra.
    // 0.1.0
    fn ancho_total(&self) -> f32 {
        let n = self.nombres.len() as f32;
        self.ancho_previa() * n + HUECO * (n - 1.0).max(0.0)
    }

    // 0.1.0
    fn inicio_x(&self) -> f32 {
        ((self.pantalla.0 - self.ancho_total()) / 2.0).max(MARGEN)
    }

    // 0.1.0
    fn previa(&self, i: usize) -> Rectangle {
        Rectangle {
            x: self.inicio_x() + i as f32 * (self.ancho_previa() + HUECO),
            y: AIRE_SUPERIOR,
            width: self.ancho_previa(),
            height: self.alto_previa(),
        }
    }

    // 0.1.0
    fn nombre_rect(&self, i: usize) -> Rectangle {
        let p = self.previa(i);
        Rectangle {
            x: p.x,
            y: p.y + p.height + AIRE_NOMBRE,
            width: p.width,
            height: NOMBRE_H,
        }
    }

    /// Va **debajo** de la miniatura, en la línea del nombre. Dentro de ella no
    /// se veía nunca: el compositor monta el fondo y las ventanas por delante de
    /// esta franja, así que todo lo que se dibuje en el hueco queda tapado.
    // 0.1.0
    fn cerrar_rect(&self, i: usize) -> Rectangle {
        let n = self.nombre_rect(i);
        Rectangle {
            x: n.x + n.width - CERRAR,
            y: n.y,
            width: CERRAR,
            height: CERRAR,
        }
    }

    // 0.1.0
    fn mas_rect(&self) -> Rectangle {
        let p = self.previa(0);
        Rectangle {
            x: self.pantalla.0 - MARGEN - MAS,
            y: p.y + (p.height - MAS) / 2.0,
            width: MAS,
            height: MAS,
        }
    }

    // 0.1.0
    fn contiene(r: Rectangle, x: f32, y: f32) -> bool {
        x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height
    }

    /// Huecos en los que el compositor monta el fondo y las ventanas vivas.
    // 0.1.0
    pub fn miniaturas(&self) -> Vec<Rectangle> {
        (0..self.nombres.len())
            .map(|i| {
                let p = self.previa(i);
                Rectangle {
                    x: p.x + PREVIA_INSET,
                    y: p.y + PREVIA_INSET,
                    width: p.width - PREVIA_INSET * 2.0,
                    height: p.height - PREVIA_INSET * 2.0,
                }
            })
            .collect()
    }

    // 0.1.0
    pub fn actualizar(&mut self, activo: usize, nombres: Vec<String>) -> bool {
        if self.activo == activo && self.nombres == nombres {
            return false;
        }
        self.activo = activo.min(nombres.len().saturating_sub(1));
        self.nombres = nombres;
        if self
            .editando
            .as_ref()
            .is_some_and(|(i, _)| *i >= self.nombres.len())
        {
            self.editando = None;
        }
        true
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nuevo = punto.and_then(|(x, y)| {
            // La línea del nombre cuenta como parte de la miniatura: la ✕ está
            // ahí, y si al llegar a ella se apagase el hover desaparecería
            // antes de poder pulsarla.
            (0..self.nombres.len()).find(|i| {
                Self::contiene(self.previa(*i), x, y) || Self::contiene(self.nombre_rect(*i), x, y)
            })
        });
        self.hover.señalar(nuevo)
    }

    /// ¿Se mueve algo dentro de la vista?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.hover.animando()
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.cabe_otro() && Self::contiene(self.mas_rect(), x, y) {
            return Some(Accion::CrearEscritorio);
        }
        for i in 0..self.nombres.len() {
            if self.nombres.len() > 1 && Self::contiene(self.cerrar_rect(i), x, y) {
                return Some(Accion::EliminarEscritorio(i));
            }
            if Self::contiene(self.nombre_rect(i), x, y) {
                let ahora = std::time::Instant::now();
                let doble = self
                    .ultimo_nombre
                    .is_some_and(|(j, t)| j == i && ahora.duration_since(t) <= DOBLE_CLIC);
                self.ultimo_nombre = Some((i, ahora));
                if doble {
                    self.editando = Some((i, self.nombres[i].clone()));
                }
                return None;
            }
            if Self::contiene(self.previa(i), x, y) {
                return Some(Accion::Escritorio(i));
            }
        }
        None
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        let Some((indice, nombre)) = self.editando.as_mut() else {
            return match tecla {
                TeclaPulsada::Escape => Tecla::Cerrar,
                _ => Tecla::Ignorada,
            };
        };
        match tecla {
            TeclaPulsada::Escape => {
                self.editando = None;
                Tecla::Consumida
            }
            TeclaPulsada::Intro => {
                let accion = Accion::RenombrarEscritorio {
                    indice: *indice,
                    nombre: nombre.clone(),
                };
                self.editando = None;
                Tecla::Hacer(accion)
            }
            TeclaPulsada::Retroceso => {
                nombre.pop();
                Tecla::Consumida
            }
            TeclaPulsada::Caracter(c)
                if !c.is_control() && c != ',' && nombre.chars().count() < 32 =>
            {
                nombre.push(c);
                Tecla::Consumida
            }
            _ => Tecla::Consumida,
        }
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let ancho = self.ancho_previa();
        let alto = self.alto_previa();
        let puede_cerrar = self.nombres.len() > 1;
        let mut fila = row![].spacing(HUECO).align_y(Vertical::Top);
        for (i, nombre) in self.nombres.iter().enumerate() {
            let activo = i == self.activo;
            let hover = self.hover.intensidad(i);
            let editando = self.editando.as_ref().is_some_and(|(j, _)| *j == i);

            // La ✕ solo aparece bajo el puntero. Estando siempre, cinco cruces
            // rojas compiten con lo único que hay que mirar aquí, que son las
            // miniaturas.
            // La ✕ aparece con el hover, pero su presencia sigue siendo un
            // sí o un no: lo que se anima es su opacidad, no si existe. Por
            // debajo de la mitad del recorrido no se dibuja, o quedaría un
            // fantasma capturando la mirada en la miniatura de al lado.
            let cierre: PanelElement<'_> = if puede_cerrar && hover > 0.02 {
                container(crate::icono::ver_teñido(
                    &self.cerrar,
                    13.0,
                    Some(tema::alfa(tema::texto(), hover)),
                ))
                .width(Length::Fixed(CERRAR))
                .height(Length::Fixed(CERRAR))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    // Círculo oscuro con el aspa blanca, no rojo: cerrar un
                    // escritorio no cierra nada —las ventanas se van al de
                    // al lado— y pintarlo de peligro promete lo que no es.
                    background: Some(tema::alfa(tema::card(), 0.85 * hover).into()),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
            } else {
                Space::new()
                    .width(Length::Fixed(CERRAR))
                    .height(Length::Fixed(CERRAR))
                    .into()
            };

            let previa = container(Space::new())
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(alto))
                .style(move |_| container::Style {
                    // Solo se ve en el marco de 2 px que queda alrededor de lo que
                    // compone el compositor —el fondo de pantalla y las ventanas—,
                    // y entero mientras el fondo no haya podido cargarse.
                    background: Some(
                        Color {
                            a: 0.92,
                            ..tema::bg()
                        }
                        .into(),
                    ),
                    border: Border {
                        color: if activo {
                            tema::acento()
                        } else {
                            tema::mezclar(
                                tema::alfa(tema::tinta(), 0.18),
                                tema::alfa(tema::tinta(), 0.45),
                                hover,
                            )
                        },
                        width: if activo { 2.0 } else { 1.0 },
                        radius: tema::R_BOTON_PEQUENO.into(),
                    },
                    ..Default::default()
                });

            let texto_etiqueta = self
                .editando
                .as_ref()
                .filter(|(j, _)| *j == i)
                .map(|(_, s)| format!("{s}│"))
                .unwrap_or_else(|| nombre.clone());
            // El hueco de la izquierda mide lo que la ✕: así el nombre sigue
            // centrado en la miniatura aunque la ✕ ocupe el otro extremo.
            let etiqueta = container(row![
                Space::new().width(Length::Fixed(CERRAR)),
                container(
                    text(texto_etiqueta)
                        .size(tema::T_CUERPO)
                        // El activo es el único que va en blanco: el nombre es lo
                        // que dice dónde estás cuando las cinco miniaturas se
                        // parecen entre sí.
                        // Todos en blanco: sobre la franja translúcida el gris
                        // secundario se pierde contra el escritorio que se ve
                        // debajo. Lo que separa al activo es el peso, no el color.
                        .color(tema::texto())
                        .font(iced_core::Font {
                            weight: if activo {
                                iced_core::font::Weight::Semibold
                            } else {
                                iced_core::font::Weight::Normal
                            },
                            ..iced_core::Font::DEFAULT
                        })
                        .align_x(Horizontal::Center)
                )
                .width(Length::Fill)
                .align_x(Horizontal::Center),
                cierre
            ])
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(NOMBRE_H))
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: editando.then_some(tema::hover().into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            fila = fila.push(column![
                previa,
                Space::new().height(Length::Fixed(AIRE_NOMBRE)),
                etiqueta
            ]);
        }

        // El botón de añadir no va en la fila: vive suelto en el extremo
        // derecho, y las miniaturas quedan centradas en la pantalla pase lo que
        // pase con él. Por eso la fila lleva un hueco fijo delante —el mismo que
        // calcula el hit-test— y uno elástico detrás.
        let anadir: PanelElement<'_> = if self.cabe_otro() {
            container(
                container(crate::icono::ver_teñido(
                    &self.anadir,
                    20.0,
                    Some(tema::texto()),
                ))
                .width(Length::Fixed(MAS))
                .height(Length::Fixed(MAS))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(|_| container::Style {
                    background: Some(
                        Color {
                            a: 0.85,
                            ..tema::card()
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_PILL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .height(Length::Fixed(alto))
            .center_y(Length::Fixed(alto))
            .into()
        } else {
            Space::new().width(Length::Fixed(MAS)).into()
        };

        let linea = row![
            Space::new().width(Length::Fixed(self.inicio_x())),
            fila,
            Space::new().width(Length::Fill),
            anadir,
            Space::new().width(Length::Fixed(MARGEN)),
        ]
        .align_y(Vertical::Top);

        // La franja va de lado a lado y sin radio: está pegada al borde de
        // arriba, y redondearla dejaría dos triángulos de escritorio en las
        // esquinas que no significan nada.
        container(linea)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced_core::Padding {
                top: AIRE_SUPERIOR,
                ..iced_core::Padding::ZERO
            })
            .align_y(Vertical::Top)
            .into()
    }

    /// El fondo de la franja. No lo pinta el contenedor de fuera de
    /// [`Self::view`] sino la limpieza del buffer: medido, el repintado pasó
    /// de 31 a 14 ms.
    ///
    /// 0,80 y no 0,62: sin el cristal que iba debajo, con menos opacidad los
    /// nombres de los escritorios se mezclan con la barra de herramientas de
    /// la ventana que pase por detrás.
    // 0.1.0
    pub fn fondo(&self) -> Color {
        Color {
            a: 0.80,
            ..tema::bg()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nunca_ofrece_mas_de_cinco() {
        let mut e = Escritorios::new(
            (1200.0, 800.0),
            0,
            (1..=5).map(|i| format!("E{i}")).collect(),
        );
        assert_eq!(e.miniaturas().len(), 5);
        let r = e.mas_rect();
        assert_eq!(e.pulsar(r.x + 2.0, r.y + 2.0), None);
    }

    #[test]
    fn doble_click_activa_la_edicion() {
        let mut e = Escritorios::new((1200.0, 800.0), 0, vec!["Uno".into(), "Dos".into()]);
        let r = e.nombre_rect(0);
        assert_eq!(e.pulsar(r.x + 2.0, r.y + 2.0), None);
        assert_eq!(e.pulsar(r.x + 2.0, r.y + 2.0), None);
        assert!(e.editando.is_some());
        assert!(matches!(
            e.tecla(TeclaPulsada::Caracter('!')),
            Tecla::Consumida
        ));
        assert!(matches!(
            e.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::RenombrarEscritorio { indice: 0, nombre }) if nombre == "Uno!"
        ));
    }
}
