//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.

use bookos_system::Operation;
use serde_json::Value;

use iced_core::Length;
use iced_widget::{Space, column};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

pub struct Bluetooth {
    entradas: Vec<Entrada>,
    /// La dirección de cada entrada, en el mismo orden. Conectar va por MAC y
    /// no por nombre: dos auriculares del mismo modelo se llaman igual.
    direcciones: Vec<String>,
    encendido: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendido`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero: el 0 es el izquierdo y el 1 el otro.
    pie: tema::Realce,
    ultimo: Value,
    /// Lo que se ha subido la lista, en píxeles. Ver `lista::desplazar`.
    desplazamiento: f32,
    estado_carga: Option<String>,
}

impl Bluetooth {
    // 0.1.0
    pub fn new() -> Self {
        let mut b = Self {
            entradas: Vec::new(),
            direcciones: Vec::new(),
            encendido: false,
            interruptor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            desplazamiento: 0.0,
            estado_carga: Some("Cargando…".into()),
        };
        b.refrescar();
        b
    }

    // 0.1.0
    fn aplicar(&mut self, data: Value, error: Option<String>) -> bool {
        let status = error.or_else(|| data.is_null().then(|| "Cargando…".into()));
        if self.ultimo == data && self.estado_carga == status {
            return false;
        }
        self.estado_carga = status;
        self.encendido = data["enabled"].as_bool().unwrap_or(false);
        let devices: Vec<_> = data["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["paired"] == true || d["connected"] == true)
            .collect();
        self.direcciones = devices
            .iter()
            .map(|d| d["mac"].as_str().unwrap_or_default().into())
            .collect();
        self.entradas = devices
            .iter()
            .map(|d| {
                let connected = d["connected"] == true;
                Entrada {
                    nombre: d["name"].as_str().unwrap_or_default().into(),
                    icono: icono::propio(if connected {
                        "bluetooth"
                    } else {
                        "bluetooth-apagado"
                    }),
                    estado: if connected {
                        "Conectado".into()
                    } else {
                        "No conectado".into()
                    },
                    derecha: d["battery"].as_u64().map(|v| format!("{v}%")),
                    activa: connected,
                }
            })
            .collect();
        self.desplazamiento = self
            .desplazamiento
            .min(lista::maximo_desplazamiento(self.visibles().len()));
        self.interruptor.ir_a(self.encendido as u8 as f32);
        self.ultimo = data;
        true
    }

    /// El alto reservado: la lista y el pie van dentro del grupo, así que al
    /// final se suman su relleno de abajo y el margen de la tarjeta.
    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        (
            lista::ANCHO,
            self.y_lista() + self.alto_lista() + lista::PIE + control::GRUPO + lista::MARGEN,
        )
    }

    /// Apagado no se enseña la lista: con todo «No conectado» parecía que
    /// algo fallaba, cuando lo único que pasa es que está apagado.
    // añadido en la 0.2.0
    fn visibles(&self) -> &[Entrada] {
        if self.encendido { &self.entradas } else { &[] }
    }

    // 0.1.0
    fn alto_lista(&self) -> f32 {
        lista::alto_ventana(self.visibles().len())
    }

    /// La `y` de la primera fila: bajo la cabecera y dentro del grupo.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA + control::GRUPO
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("bluetooth")
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| {
            lista::fila_en_ventana(
                x,
                y,
                self.y_lista(),
                self.visibles().len(),
                self.desplazamiento,
            )
        });
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y)).map(usize::from);
        // Sin cortocircuito: salir de una fila hacia un botón del pie tiene que
        // apagar la fila **y** encender el botón.
        let a = self.señalada.señalar(señalada);
        let b = self.pie.señalar(pie);
        a || b
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.pie.animando() || self.interruptor.animando()
    }

    /// Rueda o touchpad sobre la lista. `true` si se movió.
    // añadido en la 0.2.0
    pub fn desplazar(&mut self, dy: f32) -> bool {
        let n = self.visibles().len();
        lista::desplazar(&mut self.desplazamiento, dy, n)
    }

    /// Sobre qué botón del pie cae el punto.
    // 0.1.0
    fn pie_en(&self, x: f32, y: f32) -> Option<bool> {
        lista::pie_en(x, y, self.y_lista() + self.alto_lista() + lista::PIE_AIRE)
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        // El interruptor se dibujaba pero nadie miraba si se pulsaba.
        if lista::interruptor_en(x, y) {
            self.encendido = !self.encendido;
            self.interruptor.ir_a(self.encendido as u8 as f32);
            bookos_system::request(Operation::BluetoothPower {
                enabled: self.encendido,
            });
            // Se olvida lo último recibido para que el próximo refresco lo
            // aplique aunque no haya cambiado: si la orden falla, el
            // interruptor vuelve a su sitio en vez de quedarse mintiendo.
            self.ultimo = Value::Null;
            return None;
        }
        if self.pie_en(x, y).is_some() {
            return Some(Accion::Lanzar("bookos-settings --page bluetooth".into()));
        }
        let i = lista::fila_en_ventana(
            x,
            y,
            self.y_lista(),
            self.visibles().len(),
            self.desplazamiento,
        )?;
        self.activar(i)
    }

    /// Conecta o desconecta la fila `i`: lo mismo con el ratón que con Intro.
    // añadido en la 0.2.0
    fn activar(&mut self, i: usize) -> Option<Accion> {
        let address = self.direcciones.get(i)?.clone();
        bookos_system::request(if self.entradas[i].activa {
            Operation::BluetoothDisconnect { address }
        } else {
            Operation::BluetoothConnect { address }
        });
        None
    }

    // 0.1.0
    // modified 0.2.0 -- flechas y Intro recorren y activan la lista
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        let n = self.visibles().len();
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Arriba | T::Abajo => {
                let paso = if tecla == T::Arriba { -1 } else { 1 };
                lista::mover_seleccion(&mut self.señalada, &mut self.desplazamiento, n, paso);
                Tecla::Consumida
            }
            T::Intro => match self.señalada.actual() {
                Some(i) => self.activar(i).map_or(Tecla::Consumida, Tecla::Hacer),
                None => Tecla::Consumida,
            },
            _ => Tecla::Ignorada,
        }
    }

    // 0.1.0
    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        self.aplicar(state.bluetooth, bookos_system::unavailable("bluetooth"))
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        control::tarjeta(self.contenido(false), lista::ANCHO, lista::MARGEN)
    }

    /// Lo de dentro de la tarjeta, sin la caja: la Estación de Control lo pone
    /// en la suya. `volver` añade la flecha de volver a la cabecera.
    // 0.1.0
    pub fn contenido(&self, volver: bool) -> PanelElement<'_> {
        let mut filas = column![];
        let cabecera = lista::cabecera("Bluetooth", Some(self.interruptor.valor()), volver);
        if let Some(status) = &self.estado_carga {
            filas = filas.push(lista::vacia(status));
        } else if self.visibles().is_empty() {
            filas = filas.push(lista::vacia(if self.encendido {
                "No hay dispositivos emparejados"
            } else {
                "Bluetooth apagado"
            }));
        } else {
            let mut todas = column![];
            for (i, entrada) in self.visibles().iter().enumerate() {
                if i > 0 {
                    todas = todas.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                todas = todas.push(lista::fila(entrada, self.señalada.intensidad(i)));
            }
            filas = filas.push(lista::ventana(
                todas.into(),
                self.visibles().len(),
                self.desplazamiento,
            ));
        }
        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            lista::grupo(
                filas
                    .push(lista::pie("Detalles", "Configuración", &self.pie))
                    .into()
            ),
        ];
        contenido.into()
    }
}

/// Estado del adaptador y los dispositivos que conoce, en un solo proceso.
///
/// `devices Connected` va antes que `devices Paired` para poder marcar cuáles
/// están puestos: la segunda lista incluye a los de la primera.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actualiza_sin_cambiar_el_numero_de_elementos() {
        let mut card = Bluetooth::new();
        let first = serde_json::json!({"enabled":true,"devices":[{"ssid":"A","name":"A","mac":"00","paired":true,"signal":20}]});
        assert!(card.aplicar(first.clone(), None));
        assert!(!card.aplicar(first, None));
        let next = serde_json::json!({"enabled":true,"devices":[{"ssid":"B","name":"B","mac":"00","paired":true,"signal":80}]});
        assert!(card.aplicar(next, None));
        assert_eq!(card.entradas[0].nombre, "B");
    }
}
