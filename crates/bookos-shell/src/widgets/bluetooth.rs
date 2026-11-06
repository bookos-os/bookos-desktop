//! Bluetooth almacenado por el cliente del servicio compartido.

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::{AvisoDesconexion, Cruce, Widget};

/// En qué estado está el adaptador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// No hay adaptador: el icono no se dibuja. Un portátil sin bluetooth no
    /// tiene por qué enseñar un icono tachado para siempre.
    SinAdaptador,
    Apagado,
    Encendido,
    /// Encendido y con algo conectado. Es el único caso que merece el acento:
    /// lo que importa de un vistazo es si tus auriculares están puestos.
    Conectado,
}

pub struct Bluetooth {
    estado: Estado,
    icono: Option<Icono>,
    icono_nombre: &'static str,
    cruce: Cruce,
    desconexion: AvisoDesconexion,
}

impl Bluetooth {
    // 0.1.0
    // modified 0.1.1 -- nace sin aviso de desconexión
    pub fn new() -> Self {
        let mut b = Self {
            estado: Estado::SinAdaptador,
            icono: None,
            icono_nombre: "",
            cruce: Cruce::default(),
            desconexion: AvisoDesconexion::default(),
        };
        b.refrescar();
        b
    }

    // 0.1.0
    // modified 0.1.1 -- al soltarse el último dispositivo saca «Desconectado»
    fn aplicar(&mut self, estado: Estado) -> bool {
        if estado == self.estado {
            return false;
        }
        // Solo de conectado a no conectado: encender o apagar el adaptador sin
        // nada enganchado no ha desconectado nada.
        if self.estado == Estado::Conectado && matches!(estado, Estado::Encendido | Estado::Apagado)
        {
            self.desconexion.empezar();
        }
        let color_previo = self.color();
        self.estado = estado;
        let nombre = match estado {
            Estado::SinAdaptador => "",
            Estado::Apagado => "bluetooth-apagado",
            Estado::Encendido => "bluetooth",
            Estado::Conectado => "bluetooth-conectado",
        };
        if nombre != self.icono_nombre {
            let nuevo = (!nombre.is_empty())
                .then(|| icono::propio(nombre))
                .flatten();
            let previo = std::mem::replace(&mut self.icono, nuevo);
            self.cruce.empezar(previo, color_previo);
            self.icono_nombre = nombre;
        }
        true
    }

    /// Solo lo conectado va en acento: el azul del sistema de diseño es «esto
    /// está activo», y un adaptador encendido sin nada enganchado no lo está.
    /// Antes encendido y conectado salían iguales, en azul, y no había forma de
    /// saber de un vistazo si los auriculares estaban puestos.
    // 0.1.0
    fn color(&self) -> iced_core::Color {
        match self.estado {
            Estado::Apagado => tema::TEXTO2,
            Estado::Conectado => tema::acento(),
            Estado::Encendido | Estado::SinAdaptador => TEXT(),
        }
    }
}

impl Widget for Bluetooth {
    // 0.1.0
    fn nombre(&self) -> &'static str {
        "bluetooth"
    }

    // 0.1.0
    fn esperar_arranque(&mut self) {
        self.refrescar();
    }

    // 0.1.0
    // modified 0.1.1 -- también repinta al quitar «Desconectado»
    fn refrescar(&mut self) -> bool {
        let caducado = self.desconexion.caducar();
        let s = bookos_system::snapshot().bluetooth;
        let estado = if s["present"] != true {
            Estado::SinAdaptador
        } else if s["enabled"] != true {
            Estado::Apagado
        } else if s["devices"]
            .as_array()
            .is_some_and(|ds| ds.iter().any(|d| d["connected"] == true))
        {
            Estado::Conectado
        } else {
            Estado::Encendido
        };
        self.aplicar(estado) || caducado
    }

    /// Sale del icono y no del estado: `ver` no dibuja nada sin icono, y si
    /// `ancho` dijera otra cosa quedaría una zona que se come los clics del
    /// vecino sin enseñar nada. Que las dos respuestas salgan del mismo dato es
    /// lo que mantiene el invariante "ocupa sitio si y solo si se ve".
    // 0.1.0
    // modified 0.1.1 -- suma el ancho de «Desconectado» mientras se ve
    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL + self.desconexion.ancho(),
            None => 0.0,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- «Desconectado» al lado del icono al soltarse
    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        self.desconexion.ver(self.cruce.ver(ic, self.color()))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encendido y conectado tienen que distinguirse de un vistazo: antes los
    /// dos salían en acento y con el mismo dibujo.
    #[test]
    fn desconectado_sale_solo_al_soltarse_algo() {
        let mut b = Bluetooth::new();
        b.aplicar(Estado::Apagado);
        b.aplicar(Estado::Encendido);
        assert_eq!(b.desconexion.ancho(), 0.0, "encender no desconecta nada");
        b.aplicar(Estado::Conectado);
        b.aplicar(Estado::Encendido);
        // El aviso abre su hueco animado: recién empezado aún mide cero.
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(b.desconexion.ancho() > 0.0, "soltar los auriculares sí");
        assert!(
            b.proxima_alarma().is_some(),
            "y pide despertar para quitarlo"
        );
    }

    #[test]
    fn solo_lo_conectado_va_en_acento() {
        let mut b = Bluetooth::new();
        b.aplicar(Estado::Encendido);
        assert_eq!(b.color(), TEXT());
        assert_eq!(b.icono_nombre, "bluetooth");
        b.aplicar(Estado::Conectado);
        assert_eq!(b.color(), tema::acento());
        assert_eq!(b.icono_nombre, "bluetooth-conectado");
        b.aplicar(Estado::Apagado);
        assert_eq!(b.color(), tema::TEXTO2);
    }
}
