//! El botón que abre el centro de control del shell.

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Control {
    icono: Option<Icono>,
}

impl Control {
    // 0.1.0
    pub fn new() -> Self {
        Self {
            icono: icono::propio("control"),
        }
    }
}

impl Widget for Control {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "control"
    }

    // 0.1.0
    fn refrescar(&mut self) -> bool {
        false
    }

    // 0.1.0
    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL,
            None => 0.0,
        }
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        match &self.icono {
            Some(ic) => icono::ver_teñido(ic, tema::ICONO_PANEL, Some(TEXT())),
            None => crate::widget::vacio(),
        }
    }
}
