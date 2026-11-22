//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.
//!
//! La lista lleva la red conectada arriba y, debajo, «Redes conocidas» (con
//! perfil guardado) y «Otras redes». Como en la tarjeta de Bluetooth, los
//! rótulos hacen que las filas no midan todas lo mismo y la geometría sale de
//! [`Red::colocar`].
//!
//! La clave de una red protegida sin perfil se escribe en la propia tarjeta:
//! un campo bajo la red, Intro conecta y Esc lo cierra. Con perfil guardado
//! NetworkManager ya la tiene, y una red abierta no la necesita.

use std::time::{Duration, Instant};

use bookos_system::Operation;
use serde_json::Value;

use iced_core::alignment::Vertical;
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

/// Cuánto dura el «buscando…» junto a «Otras redes»: lo que tarda NetworkManager
/// en devolver el primer escaneo.
const BUSCANDO: Duration = Duration::from_secs(8);

/// Cuánto se espera a que una orden de conectar o desconectar se vea reflejada
/// antes de dejar de decir «Conectando…». Pasado eso la orden ya ha fallado, y
/// seguir diciéndolo mentiría.
const PENDIENTE: Duration = Duration::from_secs(30);

/// Lo más alto que se hace la lista: cuatro redes y los dos rótulos.
const MAX_VENTANA: f32 =
    4.0 * lista::FILA + 3.0 * lista::HUECO_FILA + 2.0 * (lista::SECCION + lista::HUECO_FILA);

/// Lo que hace falta para actuar sobre una red, en el orden de `entradas`.
struct Ap {
    ssid: String,
    /// El dispositivo Wi-Fi, para desconectar la activa.
    dispositivo: String,
    guardada: bool,
    abierta: bool,
    señal: u8,
    activa: bool,
}

/// WPA2-PSK no admite claves de menos de 8 caracteres: NetworkManager rechaza
/// el perfil y el fallo llegaría como un «No se pudo conectar» sin motivo.
const CLAVE_MINIMA: usize = 8;

/// La contraseña que se está escribiendo para una red. Va por SSID y no por
/// índice: cada refresco reordena `aps`.
struct Clave {
    ssid: String,
    texto: String,
}

/// Lo que se ha pedido y todavía no se ve.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Orden {
    Conectar,
    Desconectar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seccion {
    Conocidas,
    Otras,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fila {
    Seccion(Seccion),
    Red(usize),
    /// El campo de contraseña, bajo la red a la que pertenece.
    Clave,
}

type Colocada = lista::Colocada<Fila>;

pub struct Red {
    aps: Vec<Ap>,
    entradas: Vec<Entrada>,
    encendida: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendida`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero (siempre el 0).
    pie: tema::Realce,
    ultimo: Value,
    /// Lo que se ha subido la lista, en píxeles.
    desplazamiento: f32,
    estado_carga: Option<String>,
    abierta: Instant,
    /// Ya se pidió el escaneo de esta apertura. Una vez: las consultas no lo
    /// fuerzan, y pedirlo en cada refresco gastaría radio y batería.
    escaneada: bool,
    buscando_visto: bool,
    pendiente: Option<(String, Orden, Instant)>,
    clave: Option<Clave>,
}

/// Las tres intensidades que distingue el icono y el texto.
// añadido en la 0.2.1
fn nivel_señal(señal: u8) -> usize {
    match señal {
        67.. => 2,
        34..=66 => 1,
        _ => 0,
    }
}

fn calidad(señal: u8) -> &'static str {
    ["Débil", "Buena", "Excelente"][nivel_señal(señal)]
}

impl Red {
    // 0.1.0
    // modified 0.2.1 -- secciones, icono por intensidad, candado y escaneo
    pub fn new() -> Self {
        let mut r = Self {
            aps: Vec::new(),
            entradas: Vec::new(),
            encendida: true,
            interruptor: tema::Transicion::nueva(1.0, tema::M_RAPIDO),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            desplazamiento: 0.0,
            estado_carga: Some("Cargando…".into()),
            abierta: Instant::now(),
            escaneada: false,
            buscando_visto: false,
            pendiente: None,
            clave: None,
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
        let todas: Vec<Ap> = data["networks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| Ap {
                ssid: n["ssid"].as_str().unwrap_or_default().into(),
                dispositivo: n["device"].as_str().unwrap_or_default().into(),
                guardada: n["saved"].as_bool().unwrap_or(false),
                abierta: n["security"].as_str().unwrap_or_default().is_empty(),
                señal: u8::try_from(n["signal"].as_u64().unwrap_or(0)).unwrap_or(100),
                activa: n["active"].as_bool().unwrap_or(false),
            })
            .collect();
        // La conectada, luego las guardadas y al final las demás: los índices
        // de `entradas` y de `señalada` siguen este orden.
        let (activas, resto): (Vec<Ap>, Vec<Ap>) = todas.into_iter().partition(|a| a.activa);
        let (conocidas, otras): (Vec<Ap>, Vec<Ap>) = resto.into_iter().partition(|a| a.guardada);
        self.aps = activas.into_iter().chain(conocidas).chain(otras).collect();
        // Si la red se fue de alcance, o ya tiene perfil, el campo sobra.
        if let Some(c) = &self.clave
            && !self
                .aps
                .iter()
                .any(|a| a.ssid == c.ssid && !a.guardada && !a.abierta)
        {
            self.clave = None;
        }
        self.pedir_escaneo();
        self.avanzar_pendiente();
        self.reconstruir();
        self.desplazamiento = self.desplazamiento.min(self.maximo());
        self.interruptor.ir_a(self.encendida as u8 as f32);
        self.ultimo = data;
        true
    }

    /// Pide a NetworkManager que busque redes, una vez por apertura y con el
    /// Wi-Fi encendido.
    // añadido en la 0.2.1
    fn pedir_escaneo(&mut self) {
        if self.escaneada || !self.encendida || self.estado_carga.is_some() {
            return;
        }
        bookos_system::request(Operation::WifiScan {});
        self.escaneada = true;
        self.abierta = Instant::now();
    }

    /// Cierra lo pendiente cuando el estado real ya lo refleja.
    // añadido en la 0.2.1
    fn avanzar_pendiente(&mut self) {
        let Some((ssid, orden, _)) = self.pendiente.as_ref() else {
            return;
        };
        let hecho = self.aps.iter().any(|a| {
            &a.ssid == ssid
                && match orden {
                    Orden::Conectar => a.activa,
                    Orden::Desconectar => !a.activa,
                }
        });
        if hecho {
            self.pendiente = None;
        }
    }

    /// Rehace lo que se dibuja de cada red.
    // añadido en la 0.2.1
    fn reconstruir(&mut self) {
        let pendiente = self.pendiente.as_ref().map(|(s, o, _)| (s.as_str(), *o));
        self.entradas = self
            .aps
            .iter()
            .map(|a| {
                let nivel = nivel_señal(a.señal);
                let estado = match pendiente {
                    Some((s, Orden::Conectar)) if s == a.ssid => "Conectando…".to_string(),
                    Some((s, Orden::Desconectar)) if s == a.ssid => "Desconectando…".to_string(),
                    _ if a.activa => format!("Conectado · {}", calidad(a.señal)),
                    _ => String::new(),
                };
                Entrada {
                    nombre: a.ssid.clone(),
                    icono: icono::propio_solido(["wifi-1", "wifi-2", "wifi"][nivel]),
                    estado,
                    derecha: None,
                    activa: a.activa,
                    candado: !a.abierta,
                }
            })
            .collect();
    }

    fn buscando(&self) -> bool {
        self.escaneada && self.abierta.elapsed() < BUSCANDO
    }

    /// Lo que caduca solo y nadie avisa: el «buscando…» y lo pendiente.
    // añadido en la 0.2.1
    fn caducar(&mut self) -> bool {
        let mut cambio = false;
        if self
            .pendiente
            .as_ref()
            .is_some_and(|(_, _, desde)| desde.elapsed() > PENDIENTE)
        {
            self.pendiente = None;
            self.reconstruir();
            cambio = true;
        }
        let buscando = self.buscando();
        cambio || std::mem::replace(&mut self.buscando_visto, buscando) != buscando
    }

    /// Todo lo que hay en la lista, con su altura y su `y` dentro de ella.
    // añadido en la 0.2.1
    fn colocar(&self) -> Vec<Colocada> {
        let mut v = self.colocar_redes();
        let Some(c) = &self.clave else {
            return v;
        };
        let Some(i) = v
            .iter()
            .position(|f| matches!(f.fila, Fila::Red(i) if self.aps[i].ssid == c.ssid))
        else {
            return v;
        };
        // Lo de debajo baja lo que mide el campo y su hueco.
        let y = v[i].y + v[i].alto + lista::HUECO_FILA;
        for f in &mut v[i + 1..] {
            f.y += lista::FILA + lista::HUECO_FILA;
        }
        v.insert(
            i + 1,
            Colocada {
                fila: Fila::Clave,
                y,
                alto: lista::FILA,
            },
        );
        v
    }

    fn colocar_redes(&self) -> Vec<Colocada> {
        let mut v = Vec::new();
        if self.estado_carga.is_some() || !self.encendida {
            return v;
        }
        let mut y = 0.0;
        let conectadas = self.aps.iter().take_while(|a| a.activa).count();
        let conocidas = self.aps[conectadas..]
            .iter()
            .take_while(|a| a.guardada)
            .count();
        for i in 0..conectadas {
            lista::poner(&mut v, &mut y, Fila::Red(i), lista::FILA);
        }
        if conocidas > 0 {
            lista::poner(
                &mut v,
                &mut y,
                Fila::Seccion(Seccion::Conocidas),
                lista::SECCION,
            );
        }
        for i in conectadas..conectadas + conocidas {
            lista::poner(&mut v, &mut y, Fila::Red(i), lista::FILA);
        }
        let otras = conectadas + conocidas;
        if otras < self.aps.len() || self.buscando() {
            lista::poner(
                &mut v,
                &mut y,
                Fila::Seccion(Seccion::Otras),
                lista::SECCION,
            );
        }
        for i in otras..self.aps.len() {
            lista::poner(&mut v, &mut y, Fila::Red(i), lista::FILA);
        }
        v
    }

    fn total(&self) -> f32 {
        self.colocar().last().map_or(0.0, |c| c.y + c.alto)
    }

    // 0.1.0
    // modified 0.2.1 -- las filas ya no miden todas lo mismo
    fn alto_lista(&self) -> f32 {
        lista::ventana_de(self.total(), MAX_VENTANA)
    }

    fn maximo(&self) -> f32 {
        (self.total() - self.alto_lista()).max(0.0)
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

    /// La `y` de la primera fila: bajo la cabecera y dentro del grupo.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA + control::GRUPO
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("red")
    }

    /// Qué red hay bajo un punto de la tarjeta, si hay alguna.
    // añadido en la 0.2.1
    fn red_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = lista::MARGEN + control::GRUPO;
        let y0 = self.y_lista();
        if x < x0 || x > x0 + lista::BALDOSA || y < y0 || y >= y0 + self.alto_lista() {
            return None;
        }
        let y = y - y0 + self.desplazamiento;
        self.colocar()
            .into_iter()
            .find(|c| (c.y..c.y + c.alto).contains(&y))
            .and_then(|c| match c.fila {
                Fila::Red(i) => Some(i),
                Fila::Seccion(_) | Fila::Clave => None,
            })
    }

    // 0.1.0
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| self.red_en(x, y));
        let pie = punto.is_some_and(|(x, y)| self.pie_en(x, y)).then_some(0);
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
        let nuevo = (self.desplazamiento + dy).clamp(0.0, self.maximo());
        let cambio = (nuevo - self.desplazamiento).abs() > f32::EPSILON;
        self.desplazamiento = nuevo;
        cambio
    }

    /// Desplaza lo justo para que se vea `[y, y + alto)` de la lista.
    // añadido en la 0.2.1
    fn mostrar(&mut self, y: f32, alto: f32) {
        let ventana = self.alto_lista();
        if y < self.desplazamiento {
            self.desplazamiento = y;
        } else if y + alto > self.desplazamiento + ventana {
            self.desplazamiento = y + alto - ventana;
        }
        self.desplazamiento = self.desplazamiento.clamp(0.0, self.maximo());
    }

    /// Sobre qué botón del pie cae el punto.
    // 0.1.0
    fn pie_en(&self, x: f32, y: f32) -> bool {
        lista::pie_en(x, y, self.y_lista() + self.alto_lista() + lista::PIE_AIRE)
    }

    // 0.1.0
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        // El interruptor se dibujaba pero nadie miraba si se pulsaba. Mismo
        // trato que en la tarjeta de Bluetooth.
        if lista::interruptor_en(x, y) {
            self.encendida = !self.encendida;
            self.interruptor.ir_a(self.encendida as u8 as f32);
            bookos_system::request(Operation::WifiPower {
                enabled: self.encendida,
            });
            // Al volver a encenderlo se pedirá otro escaneo.
            self.escaneada = false;
            self.ultimo = Value::Null;
            return None;
        }
        if self.pie_en(x, y) {
            return Some(Accion::Lanzar("bookos-settings --page wifi".into()));
        }
        let Some(i) = self.red_en(x, y) else {
            // Pulsar el propio campo no lo cierra; pulsar fuera de la lista
            // tampoco: se está escribiendo.
            return None;
        };
        self.activar(i)
    }

    /// Conecta, desconecta o pide la clave según la red `i`: lo mismo con el
    /// ratón que con Intro.
    // añadido en la 0.2.0
    // modified 0.2.1 -- «Conectando…» con caducidad en vez de escribir en la entrada
    fn activar(&mut self, i: usize) -> Option<Accion> {
        let ap = self.aps.get(i)?;
        if !ap.activa && !ap.guardada && !ap.abierta {
            // Pulsarla otra vez con el campo abierto lo cierra.
            self.clave = match self.clave.take() {
                Some(c) if c.ssid == ap.ssid => None,
                _ => Some(Clave {
                    ssid: ap.ssid.clone(),
                    texto: String::new(),
                }),
            };
            if let Some(c) = self.colocar().into_iter().find(|c| c.fila == Fila::Clave) {
                self.mostrar(c.y, c.alto);
            }
            return None;
        }
        self.clave = None;
        let orden = if ap.activa {
            bookos_system::request(Operation::NetworkDisconnect {
                device: ap.dispositivo.clone(),
            });
            Orden::Desconectar
        } else {
            // Con perfil guardado NetworkManager ya tiene la clave; una red
            // abierta no la necesita. Ninguna de las dos pasa contraseña por
            // el shell.
            bookos_system::request(Operation::WifiConnect {
                ssid: ap.ssid.clone(),
                password: String::new(),
            });
            Orden::Conectar
        };
        self.pendiente = Some((ap.ssid.clone(), orden, Instant::now()));
        self.reconstruir();
        // Se olvida lo último recibido: el próximo refresco restituye el
        // estado real aunque la orden falle.
        self.ultimo = Value::Null;
        None
    }

    /// Manda la clave escrita. Con menos de [`CLAVE_MINIMA`] no hace nada: el
    /// campo lo dice.
    // añadido en la 0.2.1
    fn enviar_clave(&mut self) {
        let Some(c) = self
            .clave
            .take_if(|c| c.texto.chars().count() >= CLAVE_MINIMA)
        else {
            return;
        };
        bookos_system::request(Operation::WifiConnect {
            ssid: c.ssid.clone(),
            password: c.texto,
        });
        self.pendiente = Some((c.ssid, Orden::Conectar, Instant::now()));
        self.reconstruir();
        self.ultimo = Value::Null;
    }

    // 0.1.0
    // modified 0.2.0 -- flechas y Intro recorren y activan la lista
    // modified 0.2.1 -- con el campo de clave abierto, las teclas son suyas
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        if let Some(c) = &mut self.clave {
            match tecla {
                T::Caracter(ch) if !ch.is_control() => c.texto.push(ch),
                T::Retroceso => {
                    c.texto.pop();
                }
                T::BorrarTodo => c.texto.clear(),
                T::Escape => self.clave = None,
                T::Intro => self.enviar_clave(),
                T::Arriba | T::Abajo => {
                    self.clave = None;
                    self.mover(if tecla == T::Arriba { -1 } else { 1 });
                }
                // Lo demás se traga: no debe acabar en la ventana de debajo
                // mientras se teclea una contraseña.
                _ => {}
            }
            return Tecla::Consumida;
        }
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Arriba | T::Abajo => {
                self.mover(if tecla == T::Arriba { -1 } else { 1 });
                Tecla::Consumida
            }
            T::Intro => match self.señalada.actual() {
                Some(i) => self.activar(i).map_or(Tecla::Consumida, Tecla::Hacer),
                None => Tecla::Consumida,
            },
            _ => Tecla::Ignorada,
        }
    }

    /// Mueve la fila señalada con el teclado y desplaza la lista para que se
    /// vea. Sin dar la vuelta: de la última no se salta a la primera.
    // añadido en la 0.2.1
    fn mover(&mut self, paso: i32) {
        let n = self.aps.len();
        if n == 0 {
            return;
        }
        let nueva = match self.señalada.actual() {
            None if paso > 0 => 0,
            None => n - 1,
            Some(a) => (a as i32 + paso).clamp(0, n as i32 - 1) as usize,
        };
        self.señalada.señalar(Some(nueva));
        if let Some(c) = self
            .colocar()
            .into_iter()
            .find(|c| c.fila == Fila::Red(nueva))
        {
            self.mostrar(c.y, c.alto);
        }
    }

    // 0.1.0
    // modified 0.2.1 -- también recoge lo que caduca
    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        let cambio = self.aplicar(state.network, bookos_system::unavailable("network"));
        cambio | self.caducar()
    }

    /// El campo de contraseña. Puntos y no la clave: la tarjeta puede estar
    /// en una captura o a la vista de otro.
    // añadido en la 0.2.1
    fn campo_clave(&self) -> PanelElement<'_> {
        let Some(c) = &self.clave else {
            return Space::new().into();
        };
        let n = c.texto.chars().count();
        let escrito = if n == 0 {
            text("Contraseña").size(14.0).color(tema::texto2())
        } else {
            text(format!("{}▏", "•".repeat(n)))
                .size(14.0)
                .color(tema::texto())
        };
        let aviso = if n >= CLAVE_MINIMA {
            "Intro para conectar"
        } else if n > 0 {
            "Mínimo 8 caracteres"
        } else {
            "Esc para cancelar"
        };
        let acento = tema::acento();
        container(
            row![
                escrito,
                Space::new().width(Length::Fill),
                text(aviso).size(11.0).color(tema::texto2()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::BALDOSA))
        .height(Length::Fixed(lista::FILA))
        .center_y(Length::Fixed(lista::FILA))
        .padding([0, 12])
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(tema::elevado().into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                width: 1.5,
                color: acento,
            },
            ..Default::default()
        })
        .into()
    }

    /// El interruptor, las redes que se ven, el campo de clave y el botón del
    /// pie, con la geometría de `pulsar`. Las filas desplazadas fuera de la
    /// ventana no salen: pulsarlas en su centro no caería en ellas.
    // añadido en la 0.2.2
    pub fn accesible(&self) -> Vec<super::FilaAccesible> {
        use accesskit::Role;
        use super::FilaAccesible as F;
        let mut v = vec![F {
            id: "interruptor".into(),
            rol: Role::Switch,
            nombre: "Wi-Fi".into(),
            valor: String::new(),
            rect: (
                lista::ANCHO - lista::MARGEN - 4.0 - lista::INTERRUPTOR_ANCHO,
                lista::MARGEN + (lista::CABECERA - lista::INTERRUPTOR_ALTO) / 2.0,
                lista::INTERRUPTOR_ANCHO,
                lista::INTERRUPTOR_ALTO,
            ),
            marcada: Some(self.encendida),
            seleccionada: false,
        }];
        let (x0, y0, alto) = (lista::MARGEN + control::GRUPO, self.y_lista(), self.alto_lista());
        for c in self.colocar() {
            let y = y0 + c.y - self.desplazamiento;
            if y < y0 || y + c.alto > y0 + alto + 0.5 {
                continue;
            }
            let rect = (x0, y, lista::BALDOSA, c.alto);
            let fila = match c.fila {
                Fila::Seccion(_) => continue,
                Fila::Red(i) => {
                    let (ap, e) = (&self.aps[i], &self.entradas[i]);
                    let mut valor = e.estado.clone();
                    if !ap.abierta {
                        if !valor.is_empty() {
                            valor.push_str(", ");
                        }
                        valor.push_str("protegida");
                    }
                    F {
                        id: format!("red:{}", ap.ssid),
                        rol: Role::ListBoxOption,
                        nombre: ap.ssid.clone(),
                        valor,
                        rect,
                        marcada: None,
                        seleccionada: self.señalada.actual() == Some(i),
                    }
                }
                Fila::Clave => {
                    let Some(k) = &self.clave else { continue };
                    F {
                        id: "clave".into(),
                        rol: Role::PasswordInput,
                        nombre: format!("Contraseña de {}", k.ssid),
                        valor: format!(
                            "{} caracteres{}",
                            k.texto.chars().count(),
                            if k.texto.chars().count() >= CLAVE_MINIMA {
                                ", Intro para conectar"
                            } else {
                                ", mínimo 8"
                            }
                        ),
                        rect,
                        marcada: None,
                        seleccionada: true,
                    }
                }
            };
            v.push(fila);
        }
        v.push(F {
            id: "pie".into(),
            rol: Role::Button,
            nombre: "Ajustes de Wi-Fi".into(),
            valor: String::new(),
            rect: (
                x0,
                y0 + alto + lista::PIE_AIRE,
                lista::BALDOSA,
                lista::PIE_BOTON,
            ),
            marcada: None,
            seleccionada: false,
        });
        v
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        control::tarjeta(self.contenido(false), lista::ANCHO, lista::MARGEN)
    }

    /// Lo de dentro de la tarjeta, sin la caja: la Estación de Control lo pone
    /// en la suya. `volver` añade la flecha de volver a la cabecera.
    // 0.1.0
    // modified 0.2.1 -- secciones y filas de distinto alto
    pub fn contenido(&self, volver: bool) -> PanelElement<'_> {
        let cabecera = lista::cabecera("Wi-Fi", Some(self.interruptor.valor()), volver);
        let colocadas = self.colocar();
        let cuerpo: PanelElement<'_> = if let Some(status) = &self.estado_carga {
            lista::vacia(status)
        } else if !self.encendida {
            lista::vacia("Wi-Fi apagado")
        } else if colocadas.is_empty() {
            lista::vacia("No hay redes a la vista")
        } else {
            let mut todas = column![];
            let mut fin = 0.0;
            for c in &colocadas {
                if c.y > fin {
                    todas = todas.push(Space::new().height(Length::Fixed(c.y - fin)));
                }
                let pieza: PanelElement<'_> = match c.fila {
                    Fila::Seccion(s) => lista::seccion(
                        match s {
                            Seccion::Conocidas => "Redes conocidas",
                            Seccion::Otras => "Otras redes",
                        },
                        s == Seccion::Otras && self.buscando(),
                    ),
                    Fila::Red(i) => lista::fila(&self.entradas[i], self.señalada.intensidad(i)),
                    Fila::Clave => self.campo_clave(),
                };
                todas = todas.push(pieza);
                fin = c.y + c.alto;
            }
            lista::ventana_alto(
                todas.into(),
                self.alto_lista(),
                self.maximo(),
                self.desplazamiento,
            )
        };
        column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            lista::grupo(column![cuerpo, lista::pie("Ajustes de Wi-Fi", &self.pie)].into()),
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn red(ssid: &str, señal: u64, activa: bool, guardada: bool, seguridad: &str) -> Value {
        json!({"ssid":ssid,"device":"/dev/wlan0","signal":señal,"active":activa,"saved":guardada,"security":seguridad})
    }

    fn con(redes: Vec<Value>) -> Red {
        let mut r = Red::new();
        r.aplicar(json!({"enabled":true,"networks":redes}), None);
        r
    }

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

    /// La conectada va arriba sin rótulo; luego las guardadas y las demás, cada
    /// grupo con el suyo.
    #[test]
    fn la_conectada_arriba_y_dos_secciones() {
        let r = con(vec![
            red("Vecina", 30, false, false, "WPA2"),
            red("Casa", 55, false, true, "WPA2"),
            red("Oficina", 90, true, true, "WPA2"),
        ]);
        let filas: Vec<Fila> = r.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(
            filas,
            [
                Fila::Red(0),
                Fila::Seccion(Seccion::Conocidas),
                Fila::Red(1),
                Fila::Seccion(Seccion::Otras),
                Fila::Red(2),
            ]
        );
        assert_eq!(r.aps[0].ssid, "Oficina");
        assert_eq!(r.entradas[0].estado, "Conectado · Excelente");
        assert!(r.entradas[2].candado, "las protegidas llevan candado");
    }

    /// Una abierta no lleva candado y una guardada se conecta sin abrir nada.
    #[test]
    fn conectar_pone_conectando_y_no_pasa_clave() {
        let mut r = con(vec![red("Casa", 55, false, true, "WPA2")]);
        assert!(r.activar(0).is_none(), "con perfil no hace falta Ajustes");
        assert_eq!(r.entradas[0].estado, "Conectando…");
        let abierta = con(vec![red("Bar", 60, false, false, "")]);
        assert!(!abierta.entradas[0].candado);
    }

    /// Una red protegida sin perfil abre el campo bajo ella; con menos de 8
    /// caracteres Intro no manda nada y con 8 conecta y lo cierra.
    #[test]
    fn una_red_protegida_sin_perfil_pide_la_clave_en_la_tarjeta() {
        use crate::TeclaPulsada as T;
        let mut r = con(vec![
            red("Vecina", 70, false, false, "WPA2"),
            red("Otra", 20, false, false, "WPA2"),
        ]);
        assert!(r.activar(0).is_none(), "no abre Ajustes");
        let filas: Vec<Fila> = r.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(
            filas,
            [
                Fila::Seccion(Seccion::Otras),
                Fila::Red(0),
                Fila::Clave,
                Fila::Red(1)
            ]
        );
        for ch in "corta".chars() {
            assert!(matches!(r.tecla(T::Caracter(ch)), Tecla::Consumida));
        }
        r.tecla(T::Intro);
        assert!(r.pendiente.is_none(), "corta: no se pidió nada");
        assert!(r.clave.is_some());
        for ch in "123".chars() {
            r.tecla(T::Caracter(ch));
        }
        r.tecla(T::Intro);
        assert!(r.clave.is_none());
        assert_eq!(r.entradas[0].estado, "Conectando…");
        assert!(
            matches!(r.tecla(T::Escape), Tecla::Cerrar),
            "sin campo, Esc cierra la tarjeta"
        );
    }

    /// Esc con el campo abierto lo cierra a él, no a la tarjeta.
    #[test]
    fn esc_cierra_el_campo_primero() {
        use crate::TeclaPulsada as T;
        let mut r = con(vec![red("Vecina", 70, false, false, "WPA2")]);
        r.activar(0);
        assert!(matches!(r.tecla(T::Escape), Tecla::Consumida));
        assert!(r.clave.is_none());
    }

    /// Pulsar donde se dibuja cada cosa: una fila, un rótulo (que no es de
    /// nadie) y fuera de la lista.
    #[test]
    fn se_pulsa_lo_que_se_ve() {
        let r = con(vec![
            red("Casa", 55, false, true, "WPA2"),
            red("Vecina", 30, false, false, "WPA2"),
        ]);
        let x = lista::MARGEN + control::GRUPO + 40.0;
        let y0 = r.y_lista();
        assert_eq!(r.red_en(x, y0 + 4.0), None, "el rótulo no se pulsa");
        let c = r
            .colocar()
            .into_iter()
            .find(|c| c.fila == Fila::Red(1))
            .expect("hay una red no guardada");
        assert_eq!(r.red_en(x, y0 + c.y + 4.0), Some(1));
        assert_eq!(
            r.red_en(2.0, y0 + c.y + 4.0),
            None,
            "fuera por la izquierda"
        );
    }

    /// Apagado no se enseña la lista: con todo «No conectado» parecía que algo
    /// fallaba, cuando lo único que pasa es que está apagado.
    #[test]
    fn apagado_no_hay_lista() {
        let mut r = Red::new();
        r.aplicar(
            json!({"enabled":false,"networks":[red("Casa", 55, false, true, "WPA2")]}),
            None,
        );
        assert!(r.colocar().is_empty());
        assert_eq!(r.alto_lista(), lista::FILA);
    }

    /// La intensidad se reparte en tres, con el mismo corte para el icono y
    /// para la palabra.
    #[test]
    fn tres_intensidades() {
        assert_eq!(calidad(90), "Excelente");
        assert_eq!(calidad(50), "Buena");
        assert_eq!(calidad(10), "Débil");
        assert_eq!(nivel_señal(67), 2);
        assert_eq!(nivel_señal(66), 1);
    }

    /// Vuelca la tarjeta a PNG para mirarla, en oscuro y en claro.
    /// `BOOKOS_RED_PNG=/tmp/red` escribe `/tmp/red-<tema>.png`.
    #[test]
    fn vuelca_la_tarjeta_a_png() {
        let Ok(prefijo) = std::env::var("BOOKOS_RED_PNG") else {
            return;
        };
        for (tema, nombre) in [
            (crate::tema::Tema::Oscuro, "oscuro"),
            (crate::tema::Tema::Claro, "claro"),
        ] {
            let mut r = con(vec![
                red("gencat_ENS_EDU_PORTAL", 54, true, true, "WPA2"),
                red("Casa", 80, false, true, "WPA2"),
                red("Redmi Note 13", 100, false, true, "WPA2"),
                red("Vodafone-7F21", 52, false, false, "WPA2"),
                red("Cafe_Libre", 30, false, false, ""),
            ]);
            r.activar(1);
            r.activar(3);
            for ch in "clave123".chars() {
                r.tecla(crate::TeclaPulsada::Caracter(ch));
            }
            let config = crate::Config {
                tema,
                ..crate::Config::default()
            };
            // El interruptor se anima desde su estado inicial: sin esperar la
            // captura lo pilla a mitad de recorrido.
            std::thread::sleep(Duration::from_millis(400));
            let mut shell = crate::Shell::con_config(1646, 1.0, config);
            shell.abrir(super::super::Emergente::Red(r));
            let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);
            for p in buf.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            image::save_buffer(
                format!("{prefijo}-{nombre}.png"),
                &buf,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("se escribe el PNG");
        }
    }
}
