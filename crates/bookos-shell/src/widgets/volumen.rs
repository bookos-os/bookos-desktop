//! Volumen almacenado por el cliente del servicio compartido.

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::{Cruce, Widget};

pub struct Volumen {
    /// De 0 a 100. `None` mientras no se haya podido leer: sin PipeWire
    /// delante, el widget no se dibuja en vez de mentir con un 0 %.
    nivel: Option<u8>,
    silenciado: bool,
    icono: Option<Icono>,
    /// Con qué nombre se cargó, para no rasterizarlo otra vez sin motivo.
    icono_nombre: &'static str,
    cruce: Cruce,
}

impl Volumen {
    // 0.1.0
    pub fn new() -> Self {
        let mut v = Self {
            nivel: None,
            silenciado: false,
            icono: None,
            icono_nombre: "",
            cruce: Cruce::default(),
        };
        v.releer();
        v
    }

    /// Guarda lo leído y elige el icono. `true` si cambió algo que se ve.
    // 0.1.0
    fn aplicar(&mut self, nivel: u8, silenciado: bool) -> bool {
        if Some(nivel) == self.nivel && silenciado == self.silenciado {
            return false;
        }
        let color_previo = self.color();
        self.nivel = Some(nivel);
        self.silenciado = silenciado;
        self.actualizar_icono(color_previo);
        true
    }

    /// Silenciado en gris: es la misma señal que da el plasmoide, y se
    /// distingue del aspa a distancia de un vistazo.
    // 0.1.0
    fn color(&self) -> iced_core::Color {
        if self.nivel.is_none() || self.silenciado {
            tema::TEXTO2
        } else {
            TEXT()
        }
    }

    // 0.1.0
    fn releer(&mut self) -> bool {
        if let Some((nivel, silenciado)) = consultar("@DEFAULT_AUDIO_SINK@") {
            self.aplicar(nivel, silenciado)
        } else {
            let color_previo = self.color();
            let changed = self.nivel.take().is_some() || self.icono.is_none();
            // El widget permanece visible aunque PipeWire aún no tenga una
            // salida: el icono atenuado explica que no hay dispositivo, en vez
            // de hacer desaparecer un control del panel.
            if self.icono_nombre != "volumen-silencio" {
                let previo = std::mem::replace(&mut self.icono, icono::propio("volumen-silencio"));
                self.cruce.empezar(previo, color_previo);
                self.icono_nombre = "volumen-silencio";
            }
            changed
        }
    }

    /// Los cuatro escalones del plasmoide: sin ondas por debajo del 40 %, una
    /// hasta el 75 % y dos por encima. Con el aspa si está silenciado o a cero.
    ///
    /// Cruza solo al cambiar de escalón: mover el volumen punto a punto no
    /// cambia el dibujo y no tiene por qué animar nada.
    // 0.1.0
    fn actualizar_icono(&mut self, color_previo: iced_core::Color) {
        let nombre = match (self.silenciado, self.nivel.unwrap_or(0)) {
            (true, _) | (_, 0) => "volumen-silencio",
            (_, n) if n < 40 => "volumen-bajo",
            (_, n) if n < 75 => "volumen-medio",
            _ => "volumen-alto",
        };
        if nombre == self.icono_nombre {
            return;
        }
        let previo = std::mem::replace(&mut self.icono, icono::propio(nombre));
        self.cruce.empezar(previo, color_previo);
        self.icono_nombre = nombre;
    }
}

impl Widget for Volumen {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "volumen"
    }

    // 0.1.0
    fn esperar_arranque(&mut self) {
        self.releer();
    }

    // 0.1.0
    fn refrescar(&mut self) -> bool {
        self.releer()
    }

    /// Por lo mismo que en el bluetooth: sin icono no se dibuja nada, así que
    /// tampoco puede ocupar sitio ni recibir clics.
    // 0.1.0
    fn ancho(&self) -> f32 {
        tema::ICONO_PANEL
    }

    /// Solo el icono, como el plasmoide en su forma compacta: el número vive en
    /// el emergente, y una fila de porcentajes seguidos —brillo, volumen,
    /// batería— se lee peor que un icono que ya dice el nivel por su forma.
    // 0.1.0
    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        self.cruce.ver(ic, self.color())
    }

    // 0.1.0
    fn animando(&self) -> bool {
        self.cruce.animando()
    }
}

/// Lee la copia en memoria, sin esperar al servicio.
// 0.1.0
pub(crate) fn consultar(destino: &str) -> Option<(u8, bool)> {
    bookos_system::volume(destino == "@DEFAULT_AUDIO_SOURCE@")
}
