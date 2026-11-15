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
    /// Tercio del nivel (0, 1 o 2): lo único del nivel que cambia el dibujo.
    tercio: u8,
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
        let tercio = tercio(read_brightness());
        Self {
            legible: read_brightness().is_some(),
            tercio,
            automatico,
            sensor,
            icono: icono::propio(nombre_icono(automatico, tercio)),
            cruce: Cruce::default(),
        }
    }

    // 0.1.0
    fn color(&self) -> iced_core::Color {
        if self.legible { TEXT() } else { tema::TEXTO2 }
    }
}

/// Los rayos que sobran quedan atenuados, no ausentes: el sol mide siempre lo
/// mismo. Con el automático, la «A» manda: dice más que el nivel, que ya cambia
/// solo.
// modified 0.2.0 -- rayos según el nivel
fn nombre_icono(automatico: bool, tercio: u8) -> &'static str {
    match (automatico, tercio) {
        (true, _) => "brillo-automatico",
        (_, 0) => "brillo-bajo",
        (_, 1) => "brillo-medio",
        _ => "brillo",
    }
}

fn tercio(nivel: Option<u8>) -> u8 {
    match nivel {
        Some(0..=33) => 0,
        Some(34..=66) => 1,
        _ => 2,
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

    /// `true` solo si cambia el dibujo: cambiar de tercio sí, cada paso de la
    /// rampa del automático no (un repintado del panel entero por un sol que se
    /// ve igual).
    // 0.1.0
    fn refrescar(&mut self) -> bool {
        let nivel = read_brightness();
        let legible = nivel.is_some();
        let automatico = self.sensor && crate::retroiluminacion::automatico().pantalla;
        let tercio = tercio(nivel);
        if (legible, automatico, tercio) == (self.legible, self.automatico, self.tercio) {
            return false;
        }
        let color_previo = self.color();
        let antes = nombre_icono(self.automatico, self.tercio);
        self.legible = legible;
        self.automatico = automatico;
        self.tercio = tercio;
        if nombre_icono(automatico, tercio) != antes {
            let previo = std::mem::replace(
                &mut self.icono,
                icono::propio(nombre_icono(automatico, tercio)),
            );
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
