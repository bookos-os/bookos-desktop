//! El brillo de la pantalla.
//!
//! Solo el icono: el nivel se ve en la tarjeta y en el aviso al cambiarlo. Con
//! el automático de la pantalla puesto —y un sensor que lo haga posible— el sol
//! lleva una «A» en lugar del núcleo, que es como se dice «brillo automático»
//! en los sistemas móviles.

use crate::icono::{self, Icono};
use crate::state::read_brightness;
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::{Cruce, Widget};

pub struct Brillo {
    /// Si se puede leer el brillo. El nivel en sí no se guarda: no se dibuja.
    legible: bool,
    automatico: bool,
    /// Se mira una vez: un sensor de luz no aparece ni desaparece con la
    /// sesión abierta, y mirarlo es recorrer `/sys/bus/iio/devices`.
    sensor: bool,
    icono: Option<Icono>,
    cruce: Cruce,
}

impl Brillo {
    // 0.1.0
    pub fn new() -> Self {
        let sensor = crate::retroiluminacion::hay_sensor_luz();
        let automatico = sensor && crate::retroiluminacion::automatico().pantalla;
        Self {
            legible: read_brightness().is_some(),
            automatico,
            sensor,
            icono: icono::propio(nombre_icono(automatico)),
            cruce: Cruce::default(),
        }
    }

    // 0.1.0
    fn color(&self) -> iced_core::Color {
        if self.legible { TEXT() } else { tema::TEXTO2 }
    }
}

// 0.1.0
fn nombre_icono(automatico: bool) -> &'static str {
    if automatico {
        "brillo-automatico"
    } else {
        "brillo"
    }
}

impl Widget for Brillo {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "brillo"
    }

    // 0.1.0
    fn subsistemas(&self) -> &'static [&'static str] {
        &["backlight"]
    }

    /// `true` solo si cambia el dibujo. Antes bastaba con que cambiara el nivel,
    /// y con el automático eso es cada paso de su rampa: un repintado del panel
    /// entero por un sol que se ve igual.
    // 0.1.0
    fn refrescar(&mut self) -> bool {
        let legible = read_brightness().is_some();
        let automatico = self.sensor && crate::retroiluminacion::automatico().pantalla;
        if (legible, automatico) == (self.legible, self.automatico) {
            return false;
        }
        let color_previo = self.color();
        self.legible = legible;
        if automatico != self.automatico {
            self.automatico = automatico;
            let previo =
                std::mem::replace(&mut self.icono, icono::propio(nombre_icono(automatico)));
            self.cruce.empezar(previo, color_previo);
        }
        true
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.cruce.animando()
    }

    // 0.1.0
    fn ancho(&self) -> f32 {
        tema::ICONO_PANEL
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        self.icono
            .as_ref()
            .map(|ic| self.cruce.ver(ic, self.color()))
            .unwrap_or_else(crate::widget::vacio)
    }
}
