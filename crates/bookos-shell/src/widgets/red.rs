//! Wi-Fi o cable, y si hay enlace.

use crate::icono::{self, Icono};
use crate::state::{Link, Network};
use crate::tema;
use crate::view::PanelElement;
use crate::widget::{AvisoDesconexion, Cruce, Widget};

pub struct Red {
    dato: Option<Network>,
    /// Intensidad de la red activa, de 0 a 100. Aparte de `Network` porque ese
    /// tipo también lo construye el greeter y no tiene por qué cargar con ella.
    senal: Option<u8>,
    icono: Option<Icono>,
    cruce: Cruce,
    desconexion: AvisoDesconexion,
}

impl Red {
    // 0.1.0
    // modified 0.1.1 -- nace sin aviso de desconexión
    pub fn new() -> Self {
        let s = &bookos_system::snapshot().network;
        let dato = estado_red(s);
        let senal = senal_activa(s);
        let icono = icono::propio(nombre_icono(dato, senal));
        Self {
            dato,
            senal,
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
            _ => tema::texto2(),
        }
    }
}

/// El icono de BookOS que toca. Wi-Fi caído sale con el icono de
/// desconectado en vez de con el de señal, que sería mentir.
///
/// Los arcos que sobran quedan atenuados en vez de desaparecer, así el icono
/// no cambia de tamaño con la señal. Tercios: NetworkManager da 0–100 y no hay
/// una escala mejor que repartirlo en tres. Sin dato de señal, completo.
// modified 0.2.0 -- por intensidad
fn nombre_icono(dato: Option<Network>, senal: Option<u8>) -> &'static str {
    match dato {
        Some(Network {
            kind: Link::Wifi,
            up: true,
        }) => match senal {
            Some(0..=33) => "wifi-senal-1",
            Some(34..=66) => "wifi-senal-2",
            _ => "wifi",
        },
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

    fn accesible(&self) -> (&'static str, String) {
        use crate::state::Link;
        let valor = match self.dato {
            Some(n) if n.up && n.kind == Link::Cable => crate::emergente::t("Cable conectado", "Wired, connected").to_string(),
            Some(n) if n.up => match self.senal {
                Some(s) => format!("{}, {s} %", crate::emergente::t("Conectado", "Connected")),
                None => crate::emergente::t("Conectado", "Connected").into(),
            },
            _ => crate::emergente::t("Sin conexión", "Not connected").into(),
        };
        (crate::emergente::t("Red", "Network"), valor)
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
        let s = &bookos_system::snapshot().network;
        let fresco = estado_red(s);
        let senal = senal_activa(s);
        if fresco == self.dato && nombre_icono(fresco, senal) == nombre_icono(self.dato, self.senal)
        {
            self.senal = senal;
            return caducado;
        }
        let estaba = self.dato.is_some_and(|red| red.up);
        if estaba && !fresco.is_some_and(|red| red.up) {
            self.desconexion.empezar();
        }
        let color_previo = self.color();
        self.dato = fresco;
        self.senal = senal;
        let previo = std::mem::replace(
            &mut self.icono,
            icono::propio(nombre_icono(self.dato, self.senal)),
        );
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
fn estado_red(s: &serde_json::Value) -> Option<Network> {
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

/// La señal de la red a la que se está conectado, si la lista la trae.
// añadido en la 0.2.0
fn senal_activa(s: &serde_json::Value) -> Option<u8> {
    s["networks"]
        .as_array()?
        .iter()
        .find(|n| n["active"] == true)
        .and_then(|n| n["signal"].as_u64())
        .map(|v| v.min(100) as u8)
}
