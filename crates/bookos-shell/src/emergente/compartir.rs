//! El permiso de «compartir pantalla»: qué aplicación lo pide y qué verá.
//!
//! Lo abre el portal de escritorio ([`org.freedesktop.impl.portal.ScreenCast`])
//! antes de crear ningún nodo de vídeo. Es un **diálogo**: hay que contestarlo,
//! y por eso `Esc` no lo cierra sin más sino que manda la negativa —el portal
//! está esperando una respuesta al otro lado y dejarlo colgado hasta que venza
//! el plazo se ve como una aplicación congelada.
//!
//! El puntero **siempre** sale en lo que se comparte, así que no hay
//! interruptor para eso: el compositor pinta el cursor como un elemento más de
//! la escena y ofrecer una casilla que no hace nada sería mentir.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, text};

use super::{Ancla, Tecla};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const MARGEN: f32 = 24.0;
/// Aire transparente para que quepa la sombra de la tarjeta, como en el resto
/// de emergentes centradas.
const MARGEN_SOMBRA: f32 = 22.0;
const ICONO_CABECERA: f32 = 44.0;
const ICONO_AIRE: f32 = 12.0;
const CELDA_W: f32 = 176.0;
const CELDA_H: f32 = 104.0;
const HUECO: f32 = 12.0;
const LH_TITULO: f32 = 22.0;
const LH_SUB: f32 = 18.0;
/// Lo que ocupa la cabecera: icono, aire, título, espacio y subtítulo.
const CABECERA_H: f32 = ICONO_CABECERA + ICONO_AIRE + LH_TITULO + 4.0 + LH_SUB;
/// Aire entre la cabecera y la fila de pantallas.
const CABECERA_HUECO: f32 = 18.0;
/// Dónde empieza la fila de pantallas dentro del contenido.
const FILA_Y: f32 = MARGEN + CABECERA_H + CABECERA_HUECO;
const BOTON_W: f32 = 132.0;
const BOTON_H: f32 = 38.0;
/// Aire entre la fila de pantallas y la de botones.
const BOTONES_HUECO: f32 = 18.0;

/// Una pantalla que se puede compartir, tal y como se le enseña al usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pantalla {
    pub nombre: String,
    pub ancho: u32,
    pub alto: u32,
}

pub struct Compartir {
    /// El identificador de la sesión del portal. Viaja en la respuesta porque
    /// puede haber más de una petición en vuelo.
    sesion: u32,
    /// Quién lo pide, ya en forma legible.
    app: String,
    pantallas: Vec<Pantalla>,
    elegida: usize,
    sobre_pantalla: Option<usize>,
    /// 0 = Cancelar, 1 = Compartir. Solo para el realce del ratón.
    sobre_boton: Option<usize>,
    icono_cabecera: Option<Icono>,
    icono_monitor: Option<Icono>,
}

impl Compartir {
    // 0.1.0
    pub fn new(sesion: u32, app: String, pantallas: Vec<Pantalla>) -> Self {
        let icono_cabecera = icono::insignia_con("compartir", "#2F75FF")
            .or_else(|| icono::insignia("monitor"));
        let icono_monitor = icono::cargar("monitor").or_else(|| icono::cargar("pantalla"));
        Self {
            sesion,
            app,
            pantallas,
            elegida: 0,
            sobre_pantalla: None,
            sobre_boton: None,
            icono_cabecera,
            icono_monitor,
        }
    }

    /// La negativa que hay que mandar si esto se cierra por cualquier otro
    /// camino: un clic fuera, o el compositor recogiendo la tarjeta.
    // 0.1.0
    pub fn denegar(&self) -> Accion {
        Accion::Compartir {
            sesion: self.sesion,
            pantalla: None,
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.contenido_size();
        (w + MARGEN_SOMBRA * 2.0, h + MARGEN_SOMBRA * 2.0)
    }

    // 0.1.0
    fn contenido_size(&self) -> (f32, f32) {
        let n = self.pantallas.len().max(1) as f32;
        let ancho = MARGEN * 2.0 + CELDA_W * n + HUECO * (n - 1.0);
        let alto = FILA_Y + CELDA_H + BOTONES_HUECO + BOTON_H + MARGEN;
        // El mínimo cubre el caso de una sola pantalla: la tarjeta tiene que
        // dar de sí para la cabecera y para los dos botones, que juntos miden
        // más que una celda suelta.
        let minimo = MARGEN * 2.0 + BOTON_W * 2.0 + HUECO;
        (ancho.max(minimo), alto)
    }

    /// La `x` del primer botón, dentro del contenido. Los dos van a la derecha.
    // 0.1.0
    fn botones_x(&self) -> f32 {
        let (w, _) = self.contenido_size();
        w - MARGEN - BOTON_W * 2.0 - HUECO
    }

    // 0.1.0
    fn botones_y(&self) -> f32 {
        FILA_Y + CELDA_H + BOTONES_HUECO
    }

    /// La `x` de la primera celda. Las pantallas van centradas, no pegadas al
    /// margen: con una sola, dejarla a la izquierda de una tarjeta ancha se ve
    /// descolgada.
    // 0.1.0
    fn celdas_x(&self) -> f32 {
        let n = self.pantallas.len().max(1) as f32;
        let fila = CELDA_W * n + HUECO * (n - 1.0);
        let (w, _) = self.contenido_size();
        (w - fila) / 2.0
    }

    /// Qué hay bajo ese punto, ya en coordenadas del **contenido**.
    // 0.1.0
    fn zona(&self, x: f32, y: f32) -> Zona {
        if (FILA_Y..FILA_Y + CELDA_H).contains(&y) {
            let paso = CELDA_W + HUECO;
            let rel = x - self.celdas_x();
            if rel >= 0.0 && rel % paso <= CELDA_W {
                let i = (rel / paso).floor() as usize;
                if i < self.pantallas.len() {
                    return Zona::Pantalla(i);
                }
            }
        }
        let by = self.botones_y();
        if y >= by && y < by + BOTON_H {
            let bx = self.botones_x();
            for i in 0..2 {
                let izq = bx + (BOTON_W + HUECO) * i as f32;
                if x >= izq && x < izq + BOTON_W {
                    return Zona::Boton(i);
                }
            }
        }
        Zona::Nada
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
    pub fn velo(&self) -> Color {
        // El mismo velo que el diálogo de apagar, y por lo mismo: hay que
        // contestarle antes de seguir.
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((x, y)) = punto.map(|(x, y)| (x - MARGEN_SOMBRA, y - MARGEN_SOMBRA)) else {
            let cambio = self.sobre_boton.is_some() || self.sobre_pantalla.is_some();
            self.sobre_boton = None;
            self.sobre_pantalla = None;
            return cambio;
        };
        match self.zona(x, y) {
            Zona::Pantalla(i) => {
                let cambio = self.sobre_pantalla != Some(i) || self.sobre_boton.is_some();
                self.sobre_pantalla = Some(i);
                self.sobre_boton = None;
                cambio
            }
            Zona::Boton(i) => {
                let cambio = self.sobre_boton != Some(i) || self.sobre_pantalla.is_some();
                self.sobre_boton = Some(i);
                self.sobre_pantalla = None;
                cambio
            }
            Zona::Nada => {
                let cambio = self.sobre_boton.is_some() || self.sobre_pantalla.is_some();
                self.sobre_boton = None;
                self.sobre_pantalla = None;
                cambio
            }
        }
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (x, y) = (x - MARGEN_SOMBRA, y - MARGEN_SOMBRA);
        match self.zona(x, y) {
            Zona::Pantalla(i) => {
                self.elegida = i;
                None
            }
            Zona::Boton(0) => Some(self.denegar()),
            Zona::Boton(_) => (!self.pantallas.is_empty()).then(|| self.aceptar()),
            Zona::Nada => None,
        }
    }

    // 0.1.0
    fn aceptar(&self) -> Accion {
        Accion::Compartir {
            sesion: self.sesion,
            pantalla: Some(self.elegida),
        }
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        let n = self.pantallas.len().max(1);
        match tecla {
            // `Hacer` y no `Cerrar`: el portal espera respuesta.
            TeclaPulsada::Escape => Tecla::Hacer(self.denegar()),
            TeclaPulsada::Izquierda => {
                self.elegida = (self.elegida + n - 1) % n;
                Tecla::Consumida
            }
            TeclaPulsada::Derecha => {
                self.elegida = (self.elegida + 1) % n;
                Tecla::Consumida
            }
            TeclaPulsada::Intro if !self.pantallas.is_empty() => Tecla::Hacer(self.aceptar()),
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        let icono_el: PanelElement<'_> = match &self.icono_cabecera {
            Some(ic) => icono::ver(ic, ICONO_CABECERA, ICONO_CABECERA),
            None => Space::new().width(ICONO_CABECERA).height(ICONO_CABECERA).into(),
        };
        let cabecera = column![
            icono_el,
            Space::new().height(ICONO_AIRE),
            container(
                text(super::t("Compartir pantalla", "Share screen"))
                    .size(17.0)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto()),
            )
            .height(LH_TITULO)
            .center_y(LH_TITULO),
            Space::new().height(4),
            container(
                text(if super::t("es", "en") == "es" {
                    format!("«{}» quiere ver tu pantalla.", self.app)
                } else {
                    format!("“{}” wants to see your screen.", self.app)
                })
                .size(tema::T_CUERPO)
                .color(tema::texto2()),
            )
            .height(LH_SUB)
            .center_y(LH_SUB),
        ]
        .align_x(Horizontal::Center);

        let (ancho_contenido, _) = self.contenido_size();
        let ancho_max_nombre = CELDA_W - 24.0;
        let mut celdas = row![].spacing(HUECO);
        for (i, pantalla) in self.pantallas.iter().enumerate() {
            let activa = i == self.elegida;
            let sobre = self.sobre_pantalla == Some(i);
            let icono_pantalla: PanelElement<'_> = match &self.icono_monitor {
                Some(ic) => icono::ver_teñido(
                    ic,
                    34.0,
                    Some(if activa {
                        tema::acento()
                    } else {
                        tema::texto2()
                    }),
                ),
                None => text("▣")
                    .size(32)
                    .color(if activa {
                        tema::acento()
                    } else {
                        tema::texto2()
                    })
                    .into(),
            };
            let nombre = super::recortar_texto(&pantalla.nombre, ancho_max_nombre, tema::T_CUERPO);
            let estado_txt = if activa {
                super::t("Seleccionada", "Selected")
            } else {
                super::t("Pantalla completa", "Full screen")
            };
            celdas = celdas.push(
                container(
                    column![
                        icono_pantalla,
                        Space::new().height(8),
                        text(nombre)
                            .size(tema::T_CUERPO)
                            .font(Font {
                                weight: Weight::Medium,
                                ..Font::DEFAULT
                            })
                            .color(tema::texto()),
                        Space::new().height(2),
                        text(estado_txt)
                            .size(11.5)
                            .font(Font {
                                weight: if activa { Weight::Medium } else { Weight::Normal },
                                ..Font::DEFAULT
                            })
                            .color(if activa {
                                tema::acento()
                            } else {
                                tema::texto2()
                            }),
                    ]
                    .align_x(Horizontal::Center),
                )
                // `center_x/center_y` con el tamaño, no `.width().height()`
                // seguido de `.center(Fill)`: `center(len)` fija **ancho y
                // alto** a `len`, así que el `Fill` se comía el tamaño de la
                // celda y esta se estiraba hasta el borde de la tarjeta,
                // dejando el nombre de la pantalla por fuera.
                .center_x(CELDA_W)
                .center_y(CELDA_H)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        if activa {
                            tema::alfa(tema::acento(), if sobre { 0.22 } else { 0.14 })
                        } else if sobre {
                            tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.06)
                        } else {
                            tema::superficie_en_tarjeta()
                        }
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

        let botones = row![
            self.boton(super::t("Cancelar", "Cancel"), 0),
            Space::new().width(HUECO),
            self.boton(super::t("Compartir", "Share"), 1),
        ];

        let tarjeta = container(
            column![
                cabecera,
                Space::new().height(CABECERA_HUECO),
                container(celdas)
                    .width(ancho_contenido - MARGEN * 2.0)
                    .align_x(Horizontal::Center),
                Space::new().height(BOTONES_HUECO),
                container(botones)
                    .width(Length::Fill)
                    .align_x(Horizontal::Right),
            ]
            .align_x(Horizontal::Center),
        )
        .padding(MARGEN)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(tarjeta);
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }

    // 0.1.0
    fn boton(&self, texto: &'static str, indice: usize) -> PanelElement<'_> {
        let principal = indice == 1;
        let sobre = self.sobre_boton == Some(indice);
        let fondo = if principal {
            if sobre {
                tema::mezclar(tema::acento(), Color::WHITE, 0.10)
            } else {
                tema::acento()
            }
        } else if sobre {
            tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.08)
        } else {
            tema::superficie_en_tarjeta()
        };
        container(
            text(texto)
                .size(tema::T_CUERPO)
                .font(Font {
                    weight: Weight::Medium,
                    ..Font::DEFAULT
                })
                .color(if principal {
                    Color::WHITE
                } else {
                    tema::texto()
                }),
        )
        .center_x(BOTON_W)
        .center_y(BOTON_H)
        .align_y(Vertical::Center)
        .style(move |_| iced_widget::container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: if principal { 0.0 } else { 1.0 },
                color: if principal {
                    Color::TRANSPARENT
                } else {
                    tema::borde()
                },
            },
            ..Default::default()
        })
        .into()
    }
}

enum Zona {
    Pantalla(usize),
    Boton(usize),
    Nada,
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

    fn dos() -> Compartir {
        Compartir::new(
            7,
            "Firefox".into(),
            vec![
                Pantalla {
                    nombre: "eDP-1".into(),
                    ancho: 1920,
                    alto: 1080,
                },
                Pantalla {
                    nombre: "HDMI-A-1".into(),
                    ancho: 2560,
                    alto: 1440,
                },
            ],
        )
    }

    #[test]
    fn el_raton_elige_pantalla_y_comparte() {
        let mut c = dos();
        let x = MARGEN_SOMBRA + c.celdas_x() + CELDA_W + HUECO + CELDA_W / 2.0;
        let y = MARGEN_SOMBRA + FILA_Y + CELDA_H / 2.0;
        assert!(c.puntero(Some((x, y))));
        assert!(c.pulsar(x, y).is_none());

        let bx = MARGEN_SOMBRA + c.botones_x() + BOTON_W + HUECO + BOTON_W / 2.0;
        let by = MARGEN_SOMBRA + c.botones_y() + BOTON_H / 2.0;
        assert!(matches!(
            c.pulsar(bx, by),
            Some(Accion::Compartir {
                sesion: 7,
                pantalla: Some(1)
            })
        ));
    }

    #[test]
    fn cancelar_y_escape_contestan_que_no() {
        let mut c = dos();
        let bx = MARGEN_SOMBRA + c.botones_x() + BOTON_W / 2.0;
        let by = MARGEN_SOMBRA + c.botones_y() + BOTON_H / 2.0;
        assert!(matches!(
            c.pulsar(bx, by),
            Some(Accion::Compartir {
                sesion: 7,
                pantalla: None
            })
        ));
        // Escape no cierra a secas: manda la negativa, porque el portal está
        // bloqueado esperándola.
        assert!(matches!(
            c.tecla(TeclaPulsada::Escape),
            Tecla::Hacer(Accion::Compartir { pantalla: None, .. })
        ));
    }

    #[test]
    fn pulsar_compartir_sin_pantallas_no_acepta_indice_invalido() {
        let mut c = Compartir::new(9, "App".into(), Vec::new());
        let by = MARGEN_SOMBRA + c.botones_y() + BOTON_H / 2.0;
        let bx_ok = MARGEN_SOMBRA + c.botones_x() + BOTON_W + HUECO + BOTON_W / 2.0;
        assert_eq!(c.pulsar(bx_ok, by), None);
        let bx_cancel = MARGEN_SOMBRA + c.botones_x() + BOTON_W / 2.0;
        assert!(matches!(
            c.pulsar(bx_cancel, by),
            Some(Accion::Compartir {
                sesion: 9,
                pantalla: None
            })
        ));
    }
}
