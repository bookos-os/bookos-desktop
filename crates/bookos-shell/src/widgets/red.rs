//! Wi-Fi o cable, y si hay enlace.

use crate::icono::{self, Icono};
use crate::state::{Link, Network};
use crate::tema;
use crate::view::PanelElement;
use crate::widget::{AvisoDesconexion, Cruce, Widget};

pub struct Red {
    dato: Option<Network>,
    icono: Option<Icono>,
    cruce: Cruce,
    desconexion: AvisoDesconexion,
}

impl Red {
    // 0.1.0
    // modified 0.1.1 -- nace sin aviso de desconexión
    pub fn new() -> Self {
        let dato = estado_red();
        let icono = icono::propio(nombre_icono(dato));
        Self {
            dato,
            icono,
            cruce: Cruce::default(),
            desconexion: AvisoDesconexion::default(),
        }
    }

    /// Conectada va en acento, con la misma regla que el Bluetooth: el acento
    /// dice «esto está funcionando». Caída se atenúa: no es un fallo, es una
    /// cosa menos encendida.
    // 0.1.0
    fn color(&self) -> iced_core::Color {
        match self.dato {
            Some(red) if red.up => tema::acento(),
            _ => tema::TEXTO2,
        }
    }
}

/// El icono de BookOS que toca. Wi-Fi caído sale con el icono de
/// desconectado en vez de con el de señal, que sería mentir.
// 0.1.0
fn nombre_icono(dato: Option<Network>) -> &'static str {
    match dato {
        Some(Network {
            kind: Link::Wifi,
            up: true,
        }) => "wifi",
        Some(Network {
            kind: Link::Cable,
            up: true,
        }) => "cable",
        Some(_) => "sin-red",
        None => "sin-red",
    }
}

impl Widget for Red {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "red"
    }

    // 0.1.0
    // modified 0.1.1 -- suma el ancho de «Desconectado» mientras se ve
    fn ancho(&self) -> f32 {
        if self.dato.is_some() && self.icono.is_some() {
            tema::ICONO_PANEL + self.desconexion.ancho()
        } else {
            0.0
        }
    }

    // 0.1.0
    fn subsistemas(&self) -> &'static [&'static str] {
        &["net"]
    }

    // 0.1.0
    // modified 0.1.1 -- al caerse el enlace saca «Desconectado» unos segundos
    fn refrescar(&mut self) -> bool {
        let caducado = self.desconexion.caducar();
        let fresco = estado_red();
        if fresco == self.dato {
            return caducado;
        }
        let estaba = self.dato.is_some_and(|red| red.up);
        if estaba && !fresco.is_some_and(|red| red.up) {
            self.desconexion.empezar();
        }
        let color_previo = self.color();
        self.dato = fresco;
        let previo = std::mem::replace(&mut self.icono, icono::propio(nombre_icono(self.dato)));
        self.cruce.empezar(previo, color_previo);
        true
    }

    // 0.1.0
    // modified 0.2.0 -- también mientras «Desconectado» entra o sale
    fn animando(&self) -> bool {
        self.cruce.animando() || self.desconexion.animando()
    }

    // 0.1.1
    fn proxima_alarma(&self) -> Option<std::time::Duration> {
        self.desconexion.alarma()
    }

    /// Solo el icono, sin texto: "Wi-Fi" escrito al lado de un icono de Wi-Fi
    /// no añade nada y ocupa el ancho de tres estados más. La excepción es
    /// «Desconectado», unos segundos al caerse: eso sí es noticia.
    // 0.1.0
    // modified 0.1.1 -- «Desconectado» al lado del icono al perder la red
    fn ver(&self) -> PanelElement<'_> {
        if self.dato.is_none() {
            return crate::widget::vacio();
        }
        match &self.icono {
            Some(ic) => self.desconexion.ver(self.cruce.ver(ic, self.color())),
            None => crate::widget::vacio(),
        }
    }
}

// 0.1.0
fn estado_red() -> Option<Network> {
    let s = bookos_system::snapshot().network;
    if s.is_null() {
        return None;
    }
    if s["ethernet"]["connected"] == true {
        return Some(Network {
            kind: Link::Cable,
            up: true,
        });
    }
    Some(Network {
        kind: Link::Wifi,
        up: s["ssid"].as_str().is_some_and(|s| !s.is_empty()),
    })
}
