//! En qué escritorio estás: un monitor con el número al lado.
//!
//! Con varios escritorios y sin esto, cambiar de escritorio es teletransportarse
//! a un sitio vacío sin saber a cuál has ido. El número es lo que convierte
//! «mis ventanas han desaparecido» en «estoy en el 3».
//!
//! **El dato lo pone el compositor**, por `Widget::escritorios`. El shell no
//! conoce a Smithay a propósito —esa frontera es lo que permite probar el panel
//! sin arrancar un compositor—, así que el escritorio activo entra como
//! parámetro, igual que las ventanas abiertas del dock.
//!
//! // modified 0.2.0 -- monitor y número en vez de una fila de puntos
//! Antes eran puntos pulsables, uno por escritorio. El número no se puede
//! pulsar «por escritorio»: pulsar el widget abre la vista de escritorios, y
//! es allí donde se elige. Se pierde saltar de golpe al tercero desde el panel
//! y se gana poder leer cuál es sin contar puntos. El número va fuera del
//! monitor, como el de las notificaciones: a 18 px dentro de la pantalla no
//! cabía con aire.

use iced_widget::{row, text};

use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

/// Aire entre el monitor y el número.
const HUECO: f32 = 5.0;
/// Mismo tamaño que el número de las notificaciones: 12 en seminegrita.
const TAMANO: f32 = tema::T_PEQUENO;

pub struct Escritorios {
    /// Cuántos hay. Cero mientras el compositor no lo diga, y entonces el widget
    /// no se dibuja: en el arranque no se sabe todavía, y un monitor suelto
    /// mientras tanto sería peor que nada.
    cuantos: usize,
    activo: usize,
    icono: Option<crate::Icono>,
}

impl Escritorios {
    // 0.1.0
    pub fn new() -> Self {
        Self {
            // La configuración BookOS trae dos escritorios; mostrarlos desde
            // el primer fotograma evita que el panel cambie de ancho al entrar
            // en la sesión.
            cuantos: 2,
            activo: 0,
            icono: crate::icono::propio("monitor"),
        }
    }

    fn cifra(&self) -> String {
        (self.activo + 1).to_string()
    }
}

fn fuente() -> iced_core::Font {
    iced_core::Font {
        weight: iced_core::font::Weight::Semibold,
        ..iced_core::Font::DEFAULT
    }
}

impl Widget for Escritorios {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "escritorios"
    }

    /// No lee nada: su dato entra por `Widget::escritorios`, no de sysfs.
    // 0.1.0
    fn refrescar(&mut self) -> bool {
        false
    }

    // modified 0.2.0 -- siempre abre la vista; ya no hay un punto por escritorio
    fn pulsar(&self, _x: f32) -> Option<crate::Accion> {
        (self.cuantos > 0).then_some(crate::Accion::VistaEscritorios)
    }

    // 0.1.0
    fn escritorios(&mut self, activo: usize, cuantos: usize) -> bool {
        if (activo, cuantos) == (self.activo, self.cuantos) {
            return false;
        }
        self.activo = activo;
        self.cuantos = cuantos;
        true
    }

    // modified 0.2.0 -- icono más número
    fn ancho(&self) -> f32 {
        if self.cuantos == 0 {
            return 0.0;
        }
        tema::ICONO_PANEL + HUECO + crate::widget::ancho_de(&self.cifra(), TAMANO)
    }

    // modified 0.2.0 -- monitor y número
    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        if self.cuantos == 0 {
            return crate::widget::vacio();
        }
        let color = TEXT();
        row![
            crate::icono::ver_teñido_propio(ic, tema::ICONO_PANEL, color),
            text(self.cifra()).size(TAMANO).font(fuente()).color(color),
        ]
        .spacing(HUECO)
        .align_y(iced_core::alignment::Vertical::Center)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Con cero escritorios el widget no ocupa sitio: si ocupara, dejaría un
    /// hueco en el panel.
    ///
    /// Se fuerza el cero en vez de dar por hecho el del constructor: arranca en
    /// dos —los de la configuración de BookOS— para no cambiar el ancho del
    /// panel mientras llega la primera actualización del compositor.
    #[test]
    fn sin_escritorios_no_se_dibuja() {
        let mut e = Escritorios::new();
        e.escritorios(0, 0);
        assert_eq!(e.ancho(), 0.0);
        assert_eq!(e.pulsar(3.0), None);
    }

    /// Pulsar en cualquier punto del widget abre la vista: el número no se
    /// reparte en zonas.
    #[test]
    fn pulsar_abre_la_vista() {
        let mut e = Escritorios::new();
        e.escritorios(1, 4);
        assert_eq!(e.pulsar(0.0), Some(crate::Accion::VistaEscritorios));
        assert_eq!(e.pulsar(e.ancho()), Some(crate::Accion::VistaEscritorios));
    }

    /// El ancho incluye el número: es lo que reparte las zonas de clic del
    /// panel, así que si miente, el widget de al lado se come los clics.
    #[test]
    fn el_ancho_cuenta_icono_y_numero() {
        let mut e = Escritorios::new();
        e.escritorios(0, 3);
        assert!(e.ancho() > tema::ICONO_PANEL + HUECO);
    }

    /// Repetir el mismo dato no pide repintar.
    #[test]
    fn el_mismo_dato_no_repinta() {
        let mut e = Escritorios::new();
        assert!(e.escritorios(1, 4));
        assert!(!e.escritorios(1, 4));
        assert!(e.escritorios(2, 4));
    }
}
