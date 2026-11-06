//! La campana de notificaciones.
//!
//! El contador viene de fuera: quien atiende `org.freedesktop.Notifications` es
//! el compositor —el shell no habla D-Bus, para poder estar pintado en el
//! primer frame— y le pasa aquí cuántas hay sin leer.
//!
//! Lo pendiente lo dice **solo la chapa**. Antes la campana además cambiaba de
//! dibujo y se ponía en acento: tres señales para un solo dato, y el azul del
//! sistema de diseño es «esto está activo», no «tienes avisos». Con «No
//! molestar» puesto la campana pasa a la luna, en gris.

use std::f32::consts::PI;
use std::time::Instant;

use iced_core::{Border, Length};
use iced_widget::{container, stack, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::{Cruce, Widget};

/// Diámetro de la chapa del contador. 14 px es lo que necesita un número de dos
/// cifras a 9 px sin que el círculo se convierta en una cápsula.
const CHAPA: f32 = 14.0;

/// Qué está haciendo la chapa.
#[derive(Clone, Copy)]
enum Chapa {
    /// Llega la primera: crece desde cero con muelle.
    Aparece,
    /// Cambia la cifra: un rebote hasta 1,25 para que se note que es otra.
    Rebota,
    /// Se han leído todas: encoge hasta desaparecer. Guarda la cifra que había,
    /// porque `pendientes` ya vale cero y la chapa tiene que irse con su número.
    Sale(u32),
}

impl Chapa {
    // 0.1.0
    fn duracion(self) -> std::time::Duration {
        match self {
            Self::Aparece | Self::Rebota => tema::D_MODAL,
            Self::Sale(_) => tema::D_POPOVER,
        }
    }
}

pub struct Notificaciones {
    /// Cuántas hay sin leer, tal como las cuenta el compositor.
    pendientes: u32,
    silencio: bool,
    icono: Option<Icono>,
    icono_nombre: &'static str,
    cruce: Cruce,
    chapa: Option<(Chapa, Instant)>,
}

impl Notificaciones {
    // 0.1.0
    pub fn new() -> Self {
        let mut n = Self {
            // Escotilla para poder mirar la chapa sin provocar notificaciones
            // de verdad. Sigue valiendo con el servidor puesto: el panel se
            // pinta antes de que llegue ninguna.
            pendientes: std::env::var("BOOKOS_NOTIF_TEST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            silencio: false,
            icono: None,
            icono_nombre: "",
            cruce: Cruce::default(),
            chapa: None,
        };
        n.actualizar_icono();
        n
    }

    // 0.1.0
    fn color(&self) -> iced_core::Color {
        if self.silencio { tema::TEXTO2 } else { TEXT() }
    }

    // 0.1.0
    fn actualizar_icono(&mut self) {
        let nombre = if self.silencio {
            "noche"
        } else {
            "notificaciones"
        };
        if nombre == self.icono_nombre {
            return;
        }
        self.icono = icono::propio(nombre);
        self.icono_nombre = nombre;
    }

    /// La chapa en curso y cuánto lleva de su animación, o `None` si está
    /// quieta.
    // 0.1.0
    fn chapa_en_curso(&self) -> Option<(Chapa, f32)> {
        let (chapa, desde) = self.chapa?;
        let pasado = desde.elapsed();
        (pasado < chapa.duracion()).then(|| (chapa, tema::fraccion(pasado, chapa.duracion())))
    }

    // 0.1.0
    fn chapa_visible(&self) -> bool {
        self.pendientes > 0 || matches!(self.chapa_en_curso(), Some((Chapa::Sale(_), _)))
    }
}

impl Widget for Notificaciones {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "notificaciones"
    }

    // 0.1.0
    fn refrescar(&mut self) -> bool {
        false
    }

    /// El contador lo pone el compositor, que es quien recibe las
    /// notificaciones. Cambiarlo puede cambiar también el ancho del widget —la
    /// chapa se sale por la derecha—, y por eso repinta el panel entero y no
    /// solo su hueco.
    // 0.1.0
    fn notificaciones(&mut self, cuantas: u32) -> bool {
        if self.pendientes == cuantas {
            return false;
        }
        let chapa = match (self.pendientes, cuantas) {
            (0, _) => Chapa::Aparece,
            (antes, 0) => Chapa::Sale(antes),
            _ => Chapa::Rebota,
        };
        self.chapa = (!tema::efectos_reducidos()).then(|| (chapa, Instant::now()));
        self.pendientes = cuantas;
        true
    }

    // 0.1.0
    fn no_molestar(&mut self, activo: bool) -> bool {
        if self.silencio == activo {
            return false;
        }
        let color_previo = self.color();
        let previo = self.icono.clone();
        self.silencio = activo;
        self.actualizar_icono();
        self.cruce.empezar(previo, color_previo);
        true
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.cruce.animando() || self.chapa_en_curso().is_some()
    }

    /// La chapa se sale por la derecha del icono, así que el widget ocupa un
    /// poco más cuando hay algo pendiente: si no, se comería el hueco del
    /// vecino y el reparto de clics del panel dejaría de cuadrar. Mientras
    /// encoge sigue contando, para que los vecinos no salten antes de que se
    /// haya ido.
    // 0.1.0
    fn ancho(&self) -> f32 {
        match (&self.icono, self.chapa_visible()) {
            (None, _) => 0.0,
            (Some(_), false) => tema::ICONO_PANEL,
            (Some(_), true) => tema::ICONO_PANEL + CHAPA / 2.0,
        }
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        let campana = self.cruce.ver(ic, self.color());
        if !self.chapa_visible() {
            return campana;
        }
        let (cifra, escala) = match self.chapa_en_curso() {
            None => (self.pendientes, 1.0),
            Some((Chapa::Aparece, t)) => (self.pendientes, tema::C_MUELLE.eval(t)),
            Some((Chapa::Rebota, t)) => (
                self.pendientes,
                1.0 + 0.25 * (PI * tema::C_SUAVE.eval(t)).sin(),
            ),
            Some((Chapa::Sale(antes), t)) => (antes, 1.0 - tema::C_SUAVE.eval(t)),
        };
        // El número se corta en 9+: "12" dentro de un círculo de 14 px no se
        // lee, y lo que hace falta saber es que hay varias, no cuántas.
        let cuenta = if cifra > 9 {
            "9+".to_string()
        } else {
            cifra.to_string()
        };
        let lado = (CHAPA * escala).max(0.0);
        let chapa: PanelElement<'_> = container(
            container(
                container(
                    text(cuenta)
                        .size((9.0 * escala).max(1.0))
                        .color(iced_core::Color::WHITE)
                        .align_x(iced_core::alignment::Horizontal::Center),
                )
                .center_x(Length::Fixed(lado))
                .center_y(Length::Fixed(lado))
                .style(move |_theme: &iced_widget::Theme| container::Style {
                    background: Some(tema::rojo().into()),
                    border: Border {
                        radius: (lado / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            // La caja se queda en 14 px mientras la chapa crece o encoge
            // dentro: así escala desde su centro y no desde la esquina.
            .center_x(Length::Fixed(CHAPA))
            .center_y(Length::Fixed(CHAPA)),
        )
        // Arriba a la derecha de la campana, mordiéndola: pegada al borde
        // parecería un icono aparte.
        .padding(iced_core::Padding::ZERO.left(tema::ICONO_PANEL - CHAPA / 2.0))
        .into();

        // La primera capa de un `stack` decide su tamaño: con la campana sola
        // medía 18 y la chapa, que llega hasta 25, salía cortada por la mitad
        // en cuanto la ranura la envolvía en su resalte.
        let base = container(campana).width(Length::Fixed(self.ancho()));
        stack![base, chapa].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mientras la chapa encoge al leerlo todo, el widget sigue midiendo lo
    /// mismo: si soltara su sitio al empezar, los vecinos saltarían antes de
    /// que la chapa se hubiera ido.
    #[test]
    fn la_chapa_que_se_va_conserva_su_sitio() {
        let mut n = Notificaciones::new();
        n.notificaciones(3);
        let con_chapa = n.ancho();
        assert!(con_chapa > tema::ICONO_PANEL);
        n.notificaciones(0);
        if !tema::efectos_reducidos() {
            assert_eq!(n.ancho(), con_chapa, "encogiendo, la chapa sigue ahí");
        }
        n.chapa = Some((Chapa::Sale(3), Instant::now() - tema::D_POPOVER * 2));
        assert_eq!(n.ancho(), tema::ICONO_PANEL, "ya se fue");
        assert!(!n.animando());
    }

    #[test]
    fn no_molestar_cambia_la_campana_por_la_luna() {
        let mut n = Notificaciones::new();
        assert!(n.no_molestar(true));
        assert_eq!(n.icono_nombre, "noche");
        assert!(!n.no_molestar(true), "el mismo dato no repinta");
        assert!(n.no_molestar(false));
        assert_eq!(n.icono_nombre, "notificaciones");
    }
}
