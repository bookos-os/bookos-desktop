//! Diálogo modal de permisos del portal (`org.freedesktop.impl.portal.Access`).
//!
//! Responde a peticiones de aplicaciones externas o Flatpak (cámara,
//! ubicación, segundo plano, acceso a dispositivos…) siguiendo el lenguaje
//! visual del HIG §9 de BookOS con soporte opcional de conmutadores o
//! selectores (`choices`).

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, stack, text};

use super::color_carpeta::{SOMBRA, sombra_dialogo};
use super::{Ancla, FilaAccesible, Tecla, t};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

const ANCHO: f32 = 360.0;
const LATERAL: f32 = 24.0;
const INTERIOR: f32 = ANCHO - LATERAL * 2.0;
const ARRIBA: f32 = 24.0;
const ABAJO: f32 = 20.0;
const ICONO_H: f32 = 48.0;
const AIRE_TITULO: f32 = 12.0;
const T_TITULO: f32 = 17.0;
const LH_TITULO: f32 = 22.0;
const T_SUB: f32 = 13.0;
const LH_SUB: f32 = 19.0;
const T_CUERPO: f32 = 12.5;
const LH_CUERPO: f32 = 18.0;
const FILA_OPCION: f32 = 44.0;
const DIVISOR: f32 = 0.5;
const AIRE_BOTONES: f32 = 16.0;
const BOTON: f32 = 44.0;
const HUECO_BOTON: f32 = 10.0;

/// Una opción interactiva enviada por el portal junto a la pregunta de acceso.
///
/// Con `opciones` vacío se comporta como un interruptor booleano (`"true"` /
/// `"false"`). Con `opciones` poblado rota entre los pares `(id, etiqueta)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcionAcceso {
    pub id: String,
    pub etiqueta: String,
    pub opciones: Vec<(String, String)>,
    pub elegida: String,
}

/// Datos de apertura del diálogo `org.freedesktop.impl.portal.Access`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeticionAcceso {
    pub id: u32,
    pub app_id: String,
    pub titulo: String,
    pub subtitulo: String,
    pub cuerpo: String,
    pub icono: String,
    pub denegar: String,
    pub permitir: String,
    pub opciones: Vec<OpcionAcceso>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pieza {
    Opcion(usize),
    Denegar,
    Permitir,
}

pub struct Acceso {
    id: u32,
    app_id: String,
    icono_nombre: String,
    icono: Option<Icono>,
    titulo: Vec<String>,
    subtitulo: Vec<String>,
    cuerpo: Vec<String>,
    etiqueta_denegar: String,
    etiqueta_permitir: String,
    opciones: Vec<OpcionAcceso>,
    alto: f32,
    sombra: [Icono; 4],
    señalado: Option<Pieza>,
}

fn cargar_icono(icono_nombre: &str, app_id: &str) -> Option<Icono> {
    if !icono_nombre.is_empty() {
        if let Some(ic) = icono::cargar(icono_nombre) {
            return Some(ic);
        }
        if let Some(base) = icono_nombre.strip_suffix("-symbolic")
            && let Some(ic) = icono::cargar(base)
        {
            return Some(ic);
        }
    }
    if !app_id.is_empty()
        && let Some(app) = crate::apps::por_desktop_id(app_id)
        && let Some(ic) = icono::cargar(&app.icono)
    {
        return Some(ic);
    }
    icono::insignia("bloquear")
}

impl Acceso {
    pub fn new(peticion: PeticionAcceso) -> Self {
        let titulo_base = if peticion.titulo.trim().is_empty() {
            t("Solicitud de permiso", "Permission request").to_string()
        } else {
            peticion.titulo
        };
        let titulo = crate::escritorio::lineas(&titulo_base, INTERIOR, T_TITULO, 2);
        let subtitulo = if peticion.subtitulo.trim().is_empty() {
            Vec::new()
        } else {
            crate::escritorio::lineas(&peticion.subtitulo, INTERIOR, T_SUB, 3)
        };
        let cuerpo = if peticion.cuerpo.trim().is_empty() {
            Vec::new()
        } else {
            crate::escritorio::lineas(&peticion.cuerpo, INTERIOR, T_CUERPO, 4)
        };
        let icono = cargar_icono(&peticion.icono, &peticion.app_id);

        let mut dialogo = Self {
            id: peticion.id,
            app_id: peticion.app_id,
            icono_nombre: peticion.icono,
            icono,
            titulo,
            subtitulo,
            cuerpo,
            etiqueta_denegar: peticion.denegar,
            etiqueta_permitir: peticion.permitir,
            opciones: peticion.opciones,
            alto: 0.0,
            sombra: sombra_dialogo(ANCHO, 200.0),
            señalado: None,
        };
        dialogo.alto = dialogo.y_botones() + BOTON + ABAJO;
        dialogo.sombra = sombra_dialogo(ANCHO, dialogo.alto);
        dialogo
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn denegar(&self) -> Accion {
        Accion::PortalAcceso {
            id: self.id,
            permitido: false,
            opciones: Vec::new(),
        }
    }

    pub fn permitir(&self) -> Accion {
        Accion::PortalAcceso {
            id: self.id,
            permitido: true,
            opciones: self
                .opciones
                .iter()
                .map(|o| (o.id.clone(), o.elegida.clone()))
                .collect(),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        (ANCHO + SOMBRA * 2.0, self.alto + SOMBRA * 2.0)
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    pub fn animando(&self) -> bool {
        false
    }

    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    pub fn recargar_iconos(&mut self) {
        self.icono = cargar_icono(&self.icono_nombre, &self.app_id);
    }

    fn y_opciones(&self) -> f32 {
        let mut y = ARRIBA + ICONO_H + AIRE_TITULO + self.titulo.len() as f32 * LH_TITULO;
        if !self.subtitulo.is_empty() {
            y += 6.0 + self.subtitulo.len() as f32 * LH_SUB;
        }
        if !self.cuerpo.is_empty() {
            y += 6.0 + self.cuerpo.len() as f32 * LH_CUERPO;
        }
        y + 14.0
    }

    fn alto_opciones(&self) -> f32 {
        if self.opciones.is_empty() {
            return 0.0;
        }
        let n = self.opciones.len() as f32;
        n * FILA_OPCION + (n - 1.0) * DIVISOR
    }

    fn y_botones(&self) -> f32 {
        let y_op = self.y_opciones();
        if self.opciones.is_empty() {
            y_op + (AIRE_BOTONES - 14.0).max(0.0)
        } else {
            y_op + self.alto_opciones() + AIRE_BOTONES
        }
    }

    fn pieza_en(&self, x: f32, y: f32) -> Option<Pieza> {
        let (x, y) = (x - SOMBRA - LATERAL, y - SOMBRA);
        if !(0.0..=INTERIOR).contains(&x) {
            return None;
        }
        if !self.opciones.is_empty() {
            let mut top = self.y_opciones();
            for i in 0..self.opciones.len() {
                if (top..top + FILA_OPCION).contains(&y) {
                    return Some(Pieza::Opcion(i));
                }
                top += FILA_OPCION + DIVISOR;
            }
        }
        let y_bot = self.y_botones();
        if (y_bot..y_bot + BOTON).contains(&y) {
            let medio = (INTERIOR - HUECO_BOTON) / 2.0;
            if x <= medio {
                return Some(Pieza::Denegar);
            }
            if x >= medio + HUECO_BOTON {
                return Some(Pieza::Permitir);
            }
        }
        None
    }

    fn alternar_opcion(&mut self, indice: usize) {
        let Some(op) = self.opciones.get_mut(indice) else {
            return;
        };
        if op.opciones.is_empty() {
            op.elegida = if op.elegida == "true" {
                "false".into()
            } else {
                "true".into()
            };
        } else {
            let actual = op
                .opciones
                .iter()
                .position(|(id, _)| id == &op.elegida)
                .unwrap_or(0);
            let siguiente = (actual + 1) % op.opciones.len();
            op.elegida = op.opciones[siguiente].0.clone();
        }
    }

    pub fn accesible(&self) -> Vec<FilaAccesible> {
        let mut out = Vec::new();
        let mut top = SOMBRA + self.y_opciones();
        for (i, op) in self.opciones.iter().enumerate() {
            let (rol, marcada, valor) = if op.opciones.is_empty() {
                (
                    accesskit::Role::Switch,
                    Some(op.elegida == "true"),
                    String::new(),
                )
            } else {
                let etiqueta_actual = op
                    .opciones
                    .iter()
                    .find(|(id, _)| id == &op.elegida)
                    .map_or_else(|| op.elegida.clone(), |(_, v)| v.clone());
                (accesskit::Role::Button, None, etiqueta_actual)
            };
            out.push(FilaAccesible {
                id: format!("acceso:opcion:{i}"),
                rol,
                nombre: op.etiqueta.clone(),
                valor,
                rect: (SOMBRA + LATERAL, top, INTERIOR, FILA_OPCION),
                marcada,
                seleccionada: self.señalado == Some(Pieza::Opcion(i)),
            });
            top += FILA_OPCION + DIVISOR;
        }
        let y_bot = SOMBRA + self.y_botones();
        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        out.push(FilaAccesible {
            id: "acceso:denegar".into(),
            rol: accesskit::Role::Button,
            nombre: self.etiqueta_denegar.clone(),
            valor: String::new(),
            rect: (SOMBRA + LATERAL, y_bot, medio, BOTON),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Denegar),
        });
        out.push(FilaAccesible {
            id: "acceso:permitir".into(),
            rol: accesskit::Role::Button,
            nombre: self.etiqueta_permitir.clone(),
            valor: String::new(),
            rect: (
                SOMBRA + LATERAL + medio + HUECO_BOTON,
                y_bot,
                medio,
                BOTON,
            ),
            marcada: None,
            seleccionada: self.señalado == Some(Pieza::Permitir),
        });
        out
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.pieza_en(x, y));
        let cambia = nueva != self.señalado;
        self.señalado = nueva;
        cambia
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let pieza = self.pieza_en(x, y)?;
        match pieza {
            Pieza::Opcion(i) => {
                self.alternar_opcion(i);
                None
            }
            Pieza::Denegar => Some(self.denegar()),
            Pieza::Permitir => Some(self.permitir()),
        }
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        match tecla {
            TeclaPulsada::Escape => Tecla::Hacer(self.denegar()),
            TeclaPulsada::Intro => Tecla::Hacer(self.permitir()),
            TeclaPulsada::Arriba | TeclaPulsada::Abajo | TeclaPulsada::Tabulador
                if !self.opciones.is_empty() =>
            {
                let n = self.opciones.len();
                let actual = match self.señalado {
                    Some(Pieza::Opcion(i)) => Some(i),
                    _ => None,
                };
                let siguiente = match (tecla, actual) {
                    (TeclaPulsada::Arriba, None) => n - 1,
                    (TeclaPulsada::Arriba, Some(i)) => (i + n - 1) % n,
                    (_, None) => 0,
                    (_, Some(i)) => (i + 1) % n,
                };
                self.señalado = Some(Pieza::Opcion(siguiente));
                Tecla::Consumida
            }
            TeclaPulsada::Caracter(' ') => {
                if let Some(Pieza::Opcion(i)) = self.señalado {
                    self.alternar_opcion(i);
                }
                Tecla::Consumida
            }
            _ => Tecla::Consumida,
        }
    }

    fn boton(&self, etiqueta: &str, pieza: Pieza, ancho: f32) -> PanelElement<'_> {
        let principal = pieza == Pieza::Permitir;
        let base = if principal {
            tema::acento()
        } else {
            tema::superficie_en_tarjeta()
        };
        let fondo = if self.señalado == Some(pieza) {
            tema::mezclar(base, tema::tinta(), 0.12)
        } else {
            base
        };
        let tinta = if principal {
            tema::tinta_sobre(tema::acento())
        } else {
            tema::texto()
        };
        container(
            text(super::recortar_texto(etiqueta, ancho - 16.0, 15.0))
                .size(15)
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tinta),
        )
        .center(Length::Fill)
        .width(ancho)
        .height(BOTON)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn vista_opciones(&self) -> PanelElement<'_> {
        let mut col = column![];
        for (i, op) in self.opciones.iter().enumerate() {
            if i > 0 {
                col = col.push(
                    container(Space::new().height(DIVISOR))
                        .width(INTERIOR)
                        .style(|_| container::Style {
                            background: Some(tema::divisor().into()),
                            ..Default::default()
                        }),
                );
            }
            let señalada = self.señalado == Some(Pieza::Opcion(i));
            let control: PanelElement<'_> = if op.opciones.is_empty() {
                super::lista::interruptor(if op.elegida == "true" { 1.0 } else { 0.0 })
            } else {
                let etiqueta = op
                    .opciones
                    .iter()
                    .find(|(id, _)| id == &op.elegida)
                    .map_or_else(|| op.elegida.clone(), |(_, v)| v.clone());
                container(
                    text(super::recortar_texto(&etiqueta, 120.0, 12.0))
                        .size(12)
                        .font(Font {
                            weight: Weight::Medium,
                            ..Font::DEFAULT
                        })
                        .color(tema::acento()),
                )
                .padding([3, 8])
                .style(|_| container::Style {
                    background: Some(tema::alfa(tema::acento(), 0.14).into()),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
            };
            let max_lbl = INTERIOR - 28.0 - 12.0 - 60.0;
            let fila = container(
                row![
                    text(super::recortar_texto(&op.etiqueta, max_lbl, 13.0))
                        .size(13)
                        .font(Font {
                            weight: Weight::Medium,
                            ..Font::DEFAULT
                        })
                        .color(tema::texto()),
                    Space::new().width(Length::Fill),
                    control,
                ]
                .align_y(Vertical::Center),
            )
            .padding([0, 14])
            .width(INTERIOR)
            .height(FILA_OPCION)
            .center_y(FILA_OPCION)
            .style(move |_| container::Style {
                background: señalada
                    .then(|| tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.06).into()),
                ..Default::default()
            });
            col = col.push(fila);
        }
        container(col)
            .width(INTERIOR)
            .style(|_| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(ARRIBA)].align_x(Horizontal::Center);
        let icono_el: PanelElement<'_> = match &self.icono {
            Some(ic) => icono::ver(ic, ICONO_H, ICONO_H),
            None => Space::new().width(ICONO_H).height(ICONO_H).into(),
        };
        col = col.push(icono_el).push(Space::new().height(AIRE_TITULO));
        for l in &self.titulo {
            col = col.push(
                container(
                    text(l.clone())
                        .size(T_TITULO)
                        .font(Font {
                            weight: Weight::Bold,
                            ..Font::DEFAULT
                        })
                        .color(tema::texto()),
                )
                .height(LH_TITULO)
                .center_y(LH_TITULO),
            );
        }
        if !self.subtitulo.is_empty() {
            col = col.push(Space::new().height(6.0));
            for l in &self.subtitulo {
                col = col.push(
                    container(
                        text(l.clone())
                            .size(T_SUB)
                            .font(Font {
                                weight: Weight::Medium,
                                ..Font::DEFAULT
                            })
                            .color(tema::texto()),
                    )
                    .height(LH_SUB)
                    .center_y(LH_SUB),
                );
            }
        }
        if !self.cuerpo.is_empty() {
            col = col.push(Space::new().height(6.0));
            for l in &self.cuerpo {
                col = col.push(
                    container(text(l.clone()).size(T_CUERPO).color(tema::texto2()))
                        .height(LH_CUERPO)
                        .center_y(LH_CUERPO),
                );
            }
        }
        if !self.opciones.is_empty() {
            col = col
                .push(Space::new().height(14.0))
                .push(self.vista_opciones())
                .push(Space::new().height(AIRE_BOTONES));
        } else {
            col = col.push(Space::new().height(14.0_f32.max(AIRE_BOTONES)));
        }

        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        col = col.push(
            row![
                self.boton(&self.etiqueta_denegar, Pieza::Denegar, medio),
                self.boton(&self.etiqueta_permitir, Pieza::Permitir, medio),
            ]
            .spacing(HUECO_BOTON),
        );

        let tarjeta = container(container(col).padding([0, LATERAL as u16]))
            .width(ANCHO)
            .height(self.alto)
            .style(|_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            });
        let (ancho, alto) = self.size();
        let franja = SOMBRA + tema::R_DIALOGO;
        let lado = alto - franja * 2.0;
        let [arriba, izquierda, derecha, abajo] = &self.sombra;
        stack![
            column![
                icono::ver(arriba, ancho, franja),
                row![
                    icono::ver(izquierda, SOMBRA, lado),
                    Space::new().width(ANCHO),
                    icono::ver(derecha, SOMBRA, lado),
                ],
                icono::ver(abajo, ancho, franja),
            ],
            container(tarjeta).padding(SOMBRA)
        ]
        .into()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn peticion_ejemplo() -> PeticionAcceso {
        PeticionAcceso {
            id: 9,
            app_id: String::new(),
            titulo: "Permitir acceso a la cámara".into(),
            subtitulo: "Firefox quiere usar la cámara integrada.".into(),
            cuerpo: "Puedes cambiar este permiso cuando quieras.".into(),
            icono: String::new(),
            denegar: "Denegar".into(),
            permitir: "Permitir".into(),
            opciones: vec![
                OpcionAcceso {
                    id: "remember".into(),
                    etiqueta: "Recordar decisión".into(),
                    opciones: Vec::new(),
                    elegida: "false".into(),
                },
                OpcionAcceso {
                    id: "scope".into(),
                    etiqueta: "Alcance".into(),
                    opciones: vec![
                        ("once".into(), "Solo esta vez".into()),
                        ("always".into(), "Siempre".into()),
                    ],
                    elegida: "once".into(),
                },
            ],
        }
    }

    #[test]
    fn alterna_interruptor_y_selector_y_mantiene_tamano() {
        let mut a = Acceso::new(peticion_ejemplo());
        let tam = a.size();
        let cx = SOMBRA + LATERAL + INTERIOR / 2.0;
        let y0 = SOMBRA + a.y_opciones() + FILA_OPCION / 2.0;
        let y1 = y0 + FILA_OPCION + DIVISOR;

        assert!(a.pulsar(cx, y0).is_none());
        assert!(a.pulsar(cx, y1).is_none());
        assert_eq!(a.size(), tam);

        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        let cx_ok = SOMBRA + LATERAL + medio + HUECO_BOTON + medio / 2.0;
        let cy_bot = SOMBRA + a.y_botones() + BOTON / 2.0;
        assert_eq!(
            a.pulsar(cx_ok, cy_bot),
            Some(Accion::PortalAcceso {
                id: 9,
                permitido: true,
                opciones: vec![
                    ("remember".into(), "true".into()),
                    ("scope".into(), "always".into()),
                ],
            })
        );
    }

    #[test]
    fn escape_deniega_intro_permite_y_clic_deniega() {
        let mut a = Acceso::new(peticion_ejemplo());
        assert!(matches!(
            a.tecla(TeclaPulsada::Escape),
            Tecla::Hacer(Accion::PortalAcceso {
                id: 9,
                permitido: false,
                ..
            })
        ));
        assert!(matches!(
            a.tecla(TeclaPulsada::Intro),
            Tecla::Hacer(Accion::PortalAcceso {
                id: 9,
                permitido: true,
                ..
            })
        ));

        let medio = (INTERIOR - HUECO_BOTON) / 2.0;
        let cx_no = SOMBRA + LATERAL + medio / 2.0;
        let cy_bot = SOMBRA + a.y_botones() + BOTON / 2.0;
        assert!(matches!(
            a.pulsar(cx_no, cy_bot),
            Some(Accion::PortalAcceso {
                id: 9,
                permitido: false,
                ..
            })
        ));
    }
}
