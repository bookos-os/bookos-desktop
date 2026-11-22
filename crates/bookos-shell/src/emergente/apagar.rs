//! El diálogo del botón de encendido: dormir, bloquear, salir, reiniciar,
//! apagar.
//!
//! Sale con la tecla de encendido del portátil y con `Meta+Esc`. Las cinco
//! acciones se ordenan de reversible a destructiva; al confirmar una acción con
//! cierre de aplicaciones, pasa en el sitio a [`crate::confirmacion::Confirmacion`].
//!
//! La opción señalada al abrirse es **Dormir**, que es reversible y segura si
//! el botón se pulsa por accidente.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema::{self, Realce};
use crate::view::PanelElement;

use super::{Ancla, FilaAccesible, Tecla, t};

const ANCHO: f32 = 468.0;
const MARGEN: f32 = 24.0;
const MARGEN_INF: f32 = 18.0;
/// Alto del renglón del título (17/700, lh 1,3).
const TITULO: f32 = 24.0;
/// Aire entre el título y la fila de botones.
const HUECO_TITULO: f32 = 16.0;
/// Medidas de un botón: celda, disco y glifo.
const BOTON_ANCHO: f32 = 80.0;
const BOTON_ALTO: f32 = 86.0;
const HUECO: f32 = 5.0;
const DISCO: f32 = 52.0;
const ANILLO: f32 = 60.0;
const ICONO: f32 = 22.0;
/// Pie del paso 1: separador y botón «Cancelar».
const HUECO_SEP: f32 = 14.0;
const SEP: f32 = 1.0;
const HUECO_PIE: f32 = 10.0;
const PIE_ALTO: f32 = 44.0;
const R_DIALOGO_CIRCULAR: f32 = 34.0;

/// Una opción del diálogo.
struct Opcion {
    etiqueta_es: &'static str,
    etiqueta_en: &'static str,
    accesible_es: &'static str,
    accesible_en: &'static str,
    icono: &'static str,
    /// Si cierra sesión o apaga el equipo (semántica destructiva).
    grave: bool,
    accion: fn() -> Accion,
}

impl Opcion {
    fn etiqueta(&self) -> &'static str {
        t(self.etiqueta_es, self.etiqueta_en)
    }

    fn nombre_accesible(&self) -> &'static str {
        t(self.accesible_es, self.accesible_en)
    }
}

/// Las cinco, en orden de menos a más definitivo: es el orden en que se leen y
/// deja lo irreversible lejos de donde arranca la selección.
// 0.1.0
fn opciones() -> [Opcion; 5] {
    [
        Opcion {
            etiqueta_es: "Dormir",
            etiqueta_en: "Sleep",
            accesible_es: "Dormir",
            accesible_en: "Sleep",
            icono: "dormir",
            grave: false,
            accion: || Accion::Lanzar("systemctl suspend".into()),
        },
        Opcion {
            etiqueta_es: "Bloquear",
            etiqueta_en: "Lock",
            accesible_es: "Bloquear",
            accesible_en: "Lock",
            icono: "bloquear",
            grave: false,
            accion: || Accion::Bloquear,
        },
        Opcion {
            etiqueta_es: "Salir",
            etiqueta_en: "Log out",
            accesible_es: "Cerrar sesión",
            accesible_en: "Log out",
            icono: "salir",
            grave: false,
            accion: || Accion::CerrarSesion,
        },
        Opcion {
            etiqueta_es: "Reiniciar",
            etiqueta_en: "Restart",
            accesible_es: "Reiniciar",
            accesible_en: "Restart",
            icono: "reiniciar",
            grave: false,
            accion: || Accion::Lanzar("systemctl reboot".into()),
        },
        Opcion {
            etiqueta_es: "Apagar",
            etiqueta_en: "Shut down",
            accesible_es: "Apagar",
            accesible_en: "Shut down",
            icono: "apagar",
            grave: true,
            accion: || Accion::Lanzar("systemctl poweroff".into()),
        },
    ]
}

/// La que sale señalada al abrirse.
const INICIAL: usize = 0;

pub struct Apagar {
    opciones: [Opcion; 5],
    /// La opción señalada, compartida entre puntero y teclado.
    señalada: Realce,
    /// Si el foco de teclado está en el botón inferior «Cancelar».
    foco_cancelar: bool,
    /// Si el puntero está sobre el botón inferior «Cancelar».
    cancelar_señalado: bool,
    confirmar: Option<crate::confirmacion::Confirmacion>,
}

impl Apagar {
    // 0.1.0
    pub fn confirmar(accion: crate::confirmacion::Energia) -> Self {
        Self::confirmar_con(crate::confirmacion::Confirmacion::new(accion))
    }

    // añadido en la 0.2.3
    pub fn confirmar_con(confirmacion: crate::confirmacion::Confirmacion) -> Self {
        let mut dialogo = Self::new();
        dialogo.confirmar = Some(confirmacion);
        dialogo
    }

    // 0.1.0
    pub fn new() -> Self {
        let mut señalada = Realce::nuevo();
        señalada.señalar(Some(INICIAL));
        señalada.terminar();
        Self {
            opciones: opciones(),
            señalada,
            foco_cancelar: false,
            cancelar_señalado: false,
            confirmar: None,
        }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        if let Some(c) = &self.confirmar {
            return c.size();
        }
        (ANCHO, self.y_cancelar() + PIE_ALTO + MARGEN_INF)
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// El velo de detrás. Es un diálogo modal: 0,45 del HIG §6.
    // 0.1.0
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..Color::BLACK
        }
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.confirmar.is_none() && self.señalada.animando()
    }

    /// `x` del borde izquierdo del botón `i`. Los cinco van centrados.
    // 0.1.0
    fn x_boton(&self, i: usize) -> f32 {
        let fila = BOTON_ANCHO * 5.0 + HUECO * 4.0;
        (ANCHO - fila) / 2.0 + i as f32 * (BOTON_ANCHO + HUECO)
    }

    // 0.1.0
    fn y_botones(&self) -> f32 {
        MARGEN + TITULO + HUECO_TITULO
    }

    fn y_cancelar(&self) -> f32 {
        self.y_botones() + BOTON_ALTO + HUECO_SEP + SEP + HUECO_PIE
    }

    fn en_cancelar(&self, x: f32, y: f32) -> bool {
        let y0 = self.y_cancelar();
        (MARGEN..=ANCHO - MARGEN).contains(&x) && (y0..=y0 + PIE_ALTO).contains(&y)
    }

    // 0.1.0
    fn boton_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_botones();
        if y < y0 || y > y0 + BOTON_ALTO {
            return None;
        }
        (0..self.opciones.len()).find(|i| {
            let x0 = self.x_boton(*i);
            x >= x0 && x < x0 + BOTON_ANCHO
        })
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if let Some(c) = &mut self.confirmar {
            return c.puntero(punto);
        }
        if let Some((x, y)) = punto {
            if let Some(nueva) = self.boton_en(x, y) {
                let c1 = self.cancelar_señalado;
                self.cancelar_señalado = false;
                self.foco_cancelar = false;
                return self.señalada.señalar(Some(nueva)) || c1;
            }
            if self.en_cancelar(x, y) {
                let antes = self.cancelar_señalado;
                self.cancelar_señalado = true;
                return !antes;
            }
        }
        // Salir del diálogo **no** apaga la selección circular: Intro tiene que
        // seguir teniendo qué hacer. Solo apaga el hover de «Cancelar».
        let antes = self.cancelar_señalado;
        self.cancelar_señalado = false;
        antes
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(c) = &self.confirmar {
            return match c.pulsar(x, y) {
                Some(true) => Some(Accion::EnergiaConfirmada(c.accion)),
                Some(false) => Some(Accion::CerrarEmergente),
                None => None,
            };
        }
        if self.en_cancelar(x, y) {
            return Some(Accion::CerrarEmergente);
        }
        let i = self.boton_en(x, y)?;
        Some((self.opciones[i].accion)())
    }

    // 0.1.0
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        if let Some(c) = &mut self.confirmar {
            return match c.tecla(tecla) {
                Some(true) => Tecla::Hacer(Accion::EnergiaConfirmada(c.accion)),
                Some(false) => Tecla::Cerrar,
                None => Tecla::Consumida,
            };
        }
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda => self.mover(-1),
            T::Derecha => self.mover(1),
            T::Abajo => {
                self.foco_cancelar = true;
                Tecla::Consumida
            }
            T::Arriba => {
                self.foco_cancelar = false;
                Tecla::Consumida
            }
            T::Tabulador => {
                self.foco_cancelar = !self.foco_cancelar;
                Tecla::Consumida
            }
            T::Intro | T::Caracter(' ') => {
                if self.foco_cancelar {
                    Tecla::Cerrar
                } else {
                    let i = self.señalada.actual().unwrap_or(INICIAL);
                    Tecla::Hacer((self.opciones[i].accion)())
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    /// Mueve la selección **sin dar la vuelta**: pasar de «Dormir» a «Apagar»
    /// por el borde izquierdo es justo el salto que no se quiere en una fila
    /// donde el último elemento apaga el equipo.
    // 0.1.0
    fn mover(&mut self, paso: isize) -> Tecla {
        self.foco_cancelar = false;
        let n = self.opciones.len() as isize;
        let actual = self.señalada.actual().unwrap_or(INICIAL) as isize;
        let siguiente = (actual + paso).clamp(0, n - 1) as usize;
        self.señalada.señalar(Some(siguiente));
        self.confirmar = None;
        Tecla::Consumida
    }

    /// Controles expuestos a AT-SPI2 / Orca.
    // añadido en la 0.2.3
    pub fn accesible(&self) -> Vec<FilaAccesible> {
        if let Some(c) = &self.confirmar {
            return c.accesible();
        }
        let y0 = self.y_botones();
        let mut items: Vec<FilaAccesible> = self
            .opciones
            .iter()
            .enumerate()
            .map(|(i, o)| FilaAccesible {
                id: format!("apagar:{}", o.icono),
                rol: accesskit::Role::RadioButton,
                nombre: o.nombre_accesible().into(),
                valor: String::new(),
                rect: (self.x_boton(i), y0, BOTON_ANCHO, BOTON_ALTO),
                marcada: Some(!self.foco_cancelar && self.señalada.actual() == Some(i)),
                seleccionada: !self.foco_cancelar && self.señalada.actual() == Some(i),
            })
            .collect();
        items.push(FilaAccesible {
            id: "apagar:cancelar".into(),
            rol: accesskit::Role::Button,
            nombre: t("Cancelar", "Cancel").into(),
            valor: String::new(),
            rect: (MARGEN, self.y_cancelar(), ANCHO - MARGEN * 2.0, PIE_ALTO),
            marcada: None,
            seleccionada: self.foco_cancelar,
        });
        items
    }

    // 0.1.0
    // modified 0.2.3 -- radio 26, título 17/700, discos opacos accesibles y pie Cancelar
    pub fn view(&self) -> PanelElement<'_> {
        if let Some(c) = &self.confirmar {
            return c.view();
        }
        let mut fila = row![];
        for (i, opcion) in self.opciones.iter().enumerate() {
            if i > 0 {
                fila = fila.push(Space::new().width(Length::Fixed(HUECO)));
            }
            let intensidad = if self.foco_cancelar {
                0.0
            } else {
                self.señalada.intensidad(i)
            };
            let foco = !self.foco_cancelar && self.señalada.actual() == Some(i);
            fila = fila.push(boton(opcion, intensidad, foco));
        }

        let pie_fondo = if self.cancelar_señalado || self.foco_cancelar {
            tema::mezclar(tema::superficie_en_tarjeta(), tema::tinta(), 0.08)
        } else {
            tema::superficie_en_tarjeta()
        };
        let pie_interior = container(
            text(t("Cancelar", "Cancel"))
                .size(14)
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tema::texto()),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0 - 8.0))
        .height(Length::Fixed(PIE_ALTO - 8.0))
        .center_x(Length::Fixed(ANCHO - MARGEN * 2.0 - 8.0))
        .center_y(Length::Fixed(PIE_ALTO - 8.0))
        .style(move |_| container::Style {
            background: Some(pie_fondo.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                width: if !self.foco_cancelar && tema::alto_contraste() {
                    1.0
                } else {
                    0.0
                },
                color: tema::borde(),
            },
            ..Default::default()
        });

        let pie = container(pie_interior)
            .width(Length::Fill)
            .height(Length::Fixed(PIE_ALTO))
            .center_x(Length::Fill)
            .center_y(Length::Fixed(PIE_ALTO))
            .style(move |_| container::Style {
                border: Border {
                    radius: tema::R_PILL.into(),
                    width: if self.foco_cancelar { 2.0 } else { 0.0 },
                    color: tema::texto(),
                },
                ..Default::default()
            });

        let divisor = container(Space::new().height(Length::Fixed(SEP)))
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            });

        let contenido = column![
            container(
                text(t("¿Qué quieres hacer?", "What do you want to do?"))
                    .size(17)
                    .font(Font {
                        weight: Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tema::texto())
            )
            .width(Length::Fill)
            .height(Length::Fixed(TITULO))
            .center_x(Length::Fill)
            .center_y(Length::Fixed(TITULO)),
            Space::new().height(Length::Fixed(HUECO_TITULO)),
            container(fila).width(Length::Fill).center_x(Length::Fill),
            Space::new().height(Length::Fixed(HUECO_SEP)),
            divisor,
            Space::new().height(Length::Fixed(HUECO_PIE)),
            pie,
        ];

        let borde_tarjeta = if tema::alto_contraste() { 1.5 } else { 1.0 };

        container(contenido)
            .padding(iced_core::Padding {
                top: MARGEN,
                right: MARGEN,
                bottom: MARGEN_INF,
                left: MARGEN,
            })
            .width(Length::Fixed(ANCHO))
            .clip(true)
            .style(move |_| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: R_DIALOGO_CIRCULAR.into(),
                    width: borde_tarjeta,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// Un botón: disco opaco con el icono arriba y su etiqueta legible debajo.
// 0.1.0
fn boton<'a>(opcion: &'a Opcion, señalado: f32, foco: bool) -> PanelElement<'a> {
    let reposo_fondo = tema::superficie_en_tarjeta();
    let reposo_tinta = if opcion.grave {
        tema::rojo_en(reposo_fondo)
    } else {
        tema::texto()
    };

    let (lleno, sobre_lleno) = if opcion.grave {
        tema::pareja_destructiva()
    } else {
        (tema::acento(), tema::sobre_acento())
    };

    let fondo = tema::mezclar(reposo_fondo, lleno, señalado);
    let tinta = tema::mezclar(reposo_tinta, sobre_lleno, señalado);

    let dibujo: PanelElement<'a> = match crate::icono::propio_firme(opcion.icono) {
        Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO, tinta),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let disco = container(dibujo)
        .width(Length::Fixed(DISCO))
        .height(Length::Fixed(DISCO))
        .center_x(Length::Fixed(DISCO))
        .center_y(Length::Fixed(DISCO))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                width: if foco { 0.0 } else { 1.0 },
                color: tema::borde(),
            },
            ..Default::default()
        });

    // Anillo de foco exterior con 2 px de separación del disco (como `outline-offset`).
    let aro = container(disco)
        .width(Length::Fixed(ANILLO))
        .height(Length::Fixed(ANILLO))
        .center_x(Length::Fixed(ANILLO))
        .center_y(Length::Fixed(ANILLO))
        .style(move |_| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                width: if foco { 2.0 } else { 0.0 },
                color: tema::texto(),
            },
            ..Default::default()
        });

    let peso = if foco || señalado > 0.5 {
        Weight::Semibold
    } else {
        Weight::Medium
    };

    container(
        column![
            aro,
            Space::new().height(Length::Fixed(6.0)),
            text(opcion.etiqueta())
                .size(tema::T_PEQUENO)
                .font(Font {
                    weight: peso,
                    ..Font::DEFAULT
                })
                .color(tema::mezclar(tema::texto2(), tema::texto(), señalado)),
        ]
        .align_x(Horizontal::Center),
    )
    .width(Length::Fixed(BOTON_ANCHO))
    .height(Length::Fixed(BOTON_ALTO))
    .align_x(Horizontal::Center)
    .align_y(Vertical::Top)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los cinco botones caben en la tarjeta y cada uno responde en su sitio.
    #[test]
    fn los_cinco_botones_caen_donde_se_dibujan() {
        let a = Apagar::new();
        let y = a.y_botones() + 10.0;
        for i in 0..5 {
            let centro = a.x_boton(i) + BOTON_ANCHO / 2.0;
            assert_eq!(a.boton_en(centro, y), Some(i));
            assert!(a.x_boton(i) >= 0.0 && a.x_boton(i) + BOTON_ANCHO <= ANCHO);
        }
        assert_eq!(a.boton_en(2.0, y), None, "el margen no es de nadie");
        assert_eq!(a.boton_en(ANCHO / 2.0, 4.0), None, "el título tampoco");
        assert!(a.y_botones() + BOTON_ALTO <= a.size().1);
        assert!(a.en_cancelar(ANCHO / 2.0, a.y_cancelar() + PIE_ALTO / 2.0));
    }

    /// Arranca en «Dormir» y las flechas no dan la vuelta: llegar a «Apagar»
    /// desde el principio son cuatro pulsaciones, no una hacia atrás.
    #[test]
    fn la_seleccion_no_da_la_vuelta() {
        let mut a = Apagar::new();
        assert_eq!(a.señalada.actual(), Some(0));
        a.tecla(crate::TeclaPulsada::Izquierda);
        assert_eq!(
            a.señalada.actual(),
            Some(0),
            "a la izquierda de la primera, ella"
        );
        for _ in 0..9 {
            a.tecla(crate::TeclaPulsada::Derecha);
        }
        assert_eq!(
            a.señalada.actual(),
            Some(4),
            "y a la derecha de la última, ella"
        );
    }

    /// Intro ejecuta lo que está señalado, y con el ratón encima ejecuta eso.
    #[test]
    fn intro_hace_lo_señalado() {
        let mut a = Apagar::new();
        assert!(
            matches!(a.tecla(crate::TeclaPulsada::Intro), Tecla::Hacer(Accion::Lanzar(cmd)) if cmd.contains("suspend"))
        );
        // Con el puntero sobre «Bloquear», Intro bloquea.
        let y = a.y_botones() + 10.0;
        a.puntero(Some((a.x_boton(1) + 4.0, y)));
        assert!(matches!(
            a.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::Bloquear)
        ));
        // Y pulsar «Apagar» pide apagar.
        let accion = a.pulsar(a.x_boton(4) + 4.0, y);
        assert!(matches!(accion, Some(Accion::Lanzar(cmd)) if cmd.contains("poweroff")));
        // Pulsar «Cancelar» cierra.
        assert!(matches!(
            a.pulsar(ANCHO / 2.0, a.y_cancelar() + PIE_ALTO / 2.0),
            Some(Accion::CerrarEmergente)
        ));
    }

    #[test]
    fn tab_y_flechas_alcanzan_cancelar() {
        let mut a = Apagar::new();
        a.tecla(crate::TeclaPulsada::Tabulador);
        assert!(a.foco_cancelar);
        assert!(matches!(a.tecla(crate::TeclaPulsada::Intro), Tecla::Cerrar));
    }

    /// Salir del diálogo con el ratón no puede dejarlo sin selección: Intro
    /// tiene que seguir teniendo qué hacer.
    #[test]
    fn el_raton_fuera_no_apaga_la_seleccion() {
        let mut a = Apagar::new();
        a.puntero(Some((a.x_boton(3) + 4.0, a.y_botones() + 10.0)));
        assert_eq!(a.señalada.actual(), Some(3));
        a.puntero(None);
        assert_eq!(a.señalada.actual(), Some(3));
    }
}
