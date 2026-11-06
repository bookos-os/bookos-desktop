//! La fecha y la hora, a la derecha del panel.
//!
//! Va con la fecha delante —`15/8/26 1:54`— como el plasmoide `bookos-clock` y
//! como la referencia del panel: en un portátil la fecha se mira tanto como la
//! hora, y ponerla aquí ahorra abrir el calendario para saber en qué día vives.

use std::time::{Duration, Instant};

use iced_widget::{container, text};

use crate::state::{Fecha, local_hm};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Reloj {
    texto: String,
    /// Cuándo cambió el texto, para que el minuto nuevo entre subiendo.
    cambio: Option<Instant>,
}

impl Reloj {
    // 0.1.0
    pub fn new() -> Self {
        Self {
            texto: ahora(),
            cambio: None,
        }
    }
}

/// Lo que sube el texto nuevo al cambiar de minuto.
const SUBIDA: f32 = 6.0;

/// El reloj va en seminegrita y un punto más grande que el resto del panel: es
/// lo que más se mira de la barra, y en la referencia de Plasma es lo que
/// ancla el lado derecho. Los mismos 14/Semibold que el título de fila y los
/// botones del sistema de diseño.
const TAMANO: f32 = 14.0;
// 0.1.0
fn fuente() -> iced_core::Font {
    iced_core::Font {
        weight: iced_core::font::Weight::Semibold,
        ..iced_core::Font::DEFAULT
    }
}

/// `d/M/yy H:mm`, el formato del plasmoide: la hora **sin** cero delante,
/// `1:54` y no `01:54`, como la fecha.
// 0.1.0
fn ahora() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let f = Fecha::hoy();
    let segundos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (h, m) = local_hm(segundos);
    format!("{}/{}/{:02} {h}:{m:02}", f.dia, f.mes, f.anio % 100)
}

impl Widget for Reloj {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "reloj"
    }

    // 0.1.0
    fn refrescar(&mut self) -> bool {
        let texto = ahora();
        if texto == self.texto {
            return false;
        }
        self.texto = texto;
        self.cambio = (!tema::efectos_reducidos()).then(Instant::now);
        true
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.cambio.is_some_and(|t| t.elapsed() < tema::D_TARJETA)
    }

    /// Despierta en el cambio de minuto, no una vez por segundo.
    ///
    /// Son 59 despertares menos por minuto. Cada uno saca a la CPU de su
    /// C-state, y ese goteo es justo lo que arruina la autonomía en reposo.
    // 0.1.0
    fn proxima_alarma(&self) -> Option<Duration> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Duration::from_secs(60 - (secs % 60)))
    }

    /// Lo que ocupa su texto: "15/8/26 19:23" son trece caracteres.
    // 0.1.0
    fn ancho(&self) -> f32 {
        crate::widget::ancho_con_fuente(&self.texto, TAMANO, fuente())
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        let t = self.cambio.map_or(1.0, |desde| {
            tema::C_ENTRADA.eval(tema::fraccion(desde.elapsed(), tema::D_TARJETA))
        });
        let mut color = TEXT();
        color.a *= t;
        // El relleno de arriba es el doble de lo que se quiere bajar el texto:
        // la fila centra el elemento entero en vertical, así que la mitad del
        // relleno se come subiendo la caja.
        container(
            text(self.texto.clone())
                .size(TAMANO)
                .font(fuente())
                .color(color),
        )
        .padding(iced_core::Padding::ZERO.top(2.0 * SUBIDA * (1.0 - t)))
        .into()
    }
}
