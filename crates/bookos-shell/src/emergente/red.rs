//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.

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

/// Lo que hace falta para actuar sobre una red, en el orden de `entradas`.
struct Datos {
    ssid: String,
    /// El dispositivo Wi-Fi, para desconectar la activa.
    dispositivo: String,
    guardada: bool,
    abierta: bool,
}

pub struct Red {
    entradas: Vec<Entrada>,
    datos: Vec<Datos>,
    encendida: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendida`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero: el 0 es el izquierdo y el 1 el otro.
    pie: tema::Realce,
    ultimo: Value,
    /// Lo que se ha subido la lista, en píxeles. Ver `lista::desplazar`.
    desplazamiento: f32,
    estado_carga: Option<String>,
}

impl Red {
    // 0.1.0
    pub fn new() -> Self {
        let mut r = Self {
            entradas: Vec::new(),
            datos: Vec::new(),
            encendida: true,
            interruptor: tema::Transicion::nueva(1.0, tema::M_RAPIDO),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            desplazamiento: 0.0,
            estado_carga: Some("Cargando…".into()),
        };
        r.refrescar();
        r
    }

    // 0.1.0
    fn aplicar(&mut self, data: Value, error: Option<String>) -> bool {
        let status = error.or_else(|| data.is_null().then(|| "Cargando…".into()));
        if self.ultimo == data && self.estado_carga == status {
            return false;
        }
        self.estado_carga = status;
        self.encendida = data["enabled"].as_bool().unwrap_or(false);
        let redes: Vec<&Value> = data["networks"].as_array().into_iter().flatten().collect();
        self.datos = redes
            .iter()
            .map(|n| Datos {
                ssid: n["ssid"].as_str().unwrap_or_default().into(),
                dispositivo: n["device"].as_str().unwrap_or_default().into(),
                guardada: n["saved"].as_bool().unwrap_or(false),
                abierta: n["security"].as_str().unwrap_or_default().is_empty(),
            })
            .collect();
        self.entradas = redes
            .into_iter()
            .map(|n| {
                let signal = n["signal"].as_u64().unwrap_or(0);
                let active = n["active"].as_bool().unwrap_or(false);
                Entrada {
                    nombre: n["ssid"].as_str().unwrap_or_default().into(),
                    icono: icono::propio(if active || signal >= 40 {
                        "wifi"
                    } else {
                        "sin-red"
                    }),
                    estado: if active {
                        "Conectado".into()
                    } else {
                        "No conectado".into()
                    },
                    derecha: Some(format!("{signal}%")),
                    activa: active,
                }
            })
            .collect();
        self.desplazamiento = self
            .desplazamiento
            .min(lista::maximo_desplazamiento(self.visibles().len()));
        self.interruptor.ir_a(self.encendida as u8 as f32);
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
        if self.encendida { &self.entradas } else { &[] }
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
        Ancla::BajoWidget("red")
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
        // Los dos `señalar`, sin `||`: con el cortocircuito, salir de una fila
        // hacia un botón del pie apaga la fila y deja el botón sin encender.
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
        // El interruptor se dibujaba pero nadie miraba si se pulsaba. Mismo
        // trato que en la tarjeta de Bluetooth.
        if lista::interruptor_en(x, y) {
            self.encendida = !self.encendida;
            self.interruptor.ir_a(self.encendida as u8 as f32);
            bookos_system::request(bookos_system::Operation::WifiPower {
                enabled: self.encendida,
            });
            self.ultimo = Value::Null;
            return None;
        }
        if self.pie_en(x, y).is_some() {
            return Some(Accion::Lanzar("bookos-settings --page wifi".into()));
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

    /// Conecta, desconecta o remite a Ajustes según la red `i`: lo mismo con
    /// el ratón que con Intro.
    // añadido en la 0.2.0
    fn activar(&mut self, i: usize) -> Option<Accion> {
        let datos = self.datos.get(i)?;
        if self.entradas[i].activa {
            bookos_system::request(bookos_system::Operation::NetworkDisconnect {
                device: datos.dispositivo.clone(),
            });
            self.entradas[i].estado = "Desconectando…".into();
        } else if datos.guardada || datos.abierta {
            // Con perfil guardado NetworkManager ya tiene la clave; una red
            // abierta no la necesita. Ninguna de las dos pasa contraseña por
            // el shell.
            bookos_system::request(bookos_system::Operation::WifiConnect {
                ssid: datos.ssid.clone(),
                password: String::new(),
            });
            self.entradas[i].estado = "Conectando…".into();
        } else {
            // Red con clave sin perfil: la contraseña la pide Ajustes, nunca el shell.
            return Some(Accion::Lanzar("bookos-settings --page wifi".into()));
        }
        // Se olvida lo último recibido: el próximo refresco restituye el
        // estado real aunque la orden falle.
        self.ultimo = Value::Null;
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
        self.aplicar(state.network, bookos_system::unavailable("network"))
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
        let cabecera = lista::cabecera("Wi-Fi", Some(self.interruptor.valor()), volver);
        if let Some(status) = &self.estado_carga {
            filas = filas.push(lista::vacia(status));
        } else if self.visibles().is_empty() {
            filas = filas.push(lista::vacia(if self.encendida {
                "No hay redes a la vista"
            } else {
                "Wi-Fi apagado"
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actualiza_sin_cambiar_el_numero_de_elementos() {
        let mut card = Red::new();
        let first = serde_json::json!({"enabled":true,"networks":[{"ssid":"A","name":"A","mac":"00","paired":true,"signal":20}]});
        assert!(card.aplicar(first.clone(), None));
        assert!(!card.aplicar(first, None));
        let next = serde_json::json!({"enabled":true,"networks":[{"ssid":"B","name":"B","mac":"00","paired":true,"signal":80}]});
        assert!(card.aplicar(next, None));
        assert_eq!(card.entradas[0].nombre, "B");
    }
}
