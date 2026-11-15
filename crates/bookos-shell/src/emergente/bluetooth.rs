//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.
//!
//! La lista tiene dos secciones, «Mis dispositivos» (emparejados) y «Otros
//! dispositivos» (lo que BlueZ va descubriendo mientras la tarjeta está
//! abierta), y los Galaxy Buds conectados se despliegan con su batería y el
//! control de ruido. Por eso las filas ya no miden todas lo mismo y la
//! geometría (ventana, ratón, teclado) sale de [`Bluetooth::colocar`] y no del
//! `n × FILA` de `lista.rs`, que sigue sirviendo a la tarjeta de Wi-Fi.

use std::time::{Duration, Instant};

use bookos_system::Operation;
use serde_json::Value;

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::buds::{self, Modo};
use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

/// Cuánto dura el «buscando…» junto a «Otros dispositivos». BlueZ sigue
/// descubriendo mientras la tarjeta esté abierta; esto solo dice que la lista
/// aún se está llenando, y 8 s es lo que tardan en aparecer los primeros.
const BUSCANDO: Duration = Duration::from_secs(8);

/// Cuánto se espera a que un dispositivo acabe de conectarse o emparejarse
/// antes de dejar de decir «Conectando…». Es el tiempo que el agente de BlueZ
/// espera la confirmación del usuario (`pairing.rs`): pasado eso, la orden ya
/// ha fallado y mantener el aviso mentiría.
const PENDIENTE: Duration = Duration::from_secs(60);

/// Lo más alto que se hace la lista: cuatro dispositivos y los dos rótulos.
/// Con más, se desplaza; la tarjeta no crece hasta el dock.
const MAX_VENTANA: f32 =
    4.0 * lista::FILA + 3.0 * lista::HUECO_FILA + 2.0 * (lista::SECCION + lista::HUECO_FILA);

/// Lo desplegado de unos Buds: relleno, línea de batería, selector de ruido y
/// los dos enlaces. Los `Y_*` son dónde empieza cada parte dentro del mando,
/// que es de donde sale tanto el dibujo como lo que se puede pulsar.
const MANDO_X: f32 = 12.0;
const MANDO_V: f32 = 10.0;
const MANDO_HUECO: f32 = 8.0;
const LINEA: f32 = 16.0;
const SEGMENTO: f32 = 30.0;
/// La pista del selector: el segmento y 2 de margen por arriba y por abajo.
const PISTA: f32 = SEGMENTO + 4.0;
const ENLACES: f32 = 20.0;
const Y_PISTA: f32 = MANDO_V + LINEA + MANDO_HUECO;
const Y_ENLACES: f32 = Y_PISTA + PISTA + MANDO_HUECO;
const MANDO_H: f32 = Y_ENLACES + ENLACES + MANDO_V;

/// Un dispositivo tal como lo cuenta BlueZ.
struct Dispositivo {
    /// Conectar va por MAC y no por nombre: dos auriculares del mismo modelo
    /// se llaman igual.
    mac: String,
    nombre: String,
    glifo: &'static str,
    /// Auriculares, altavoz o similar: lo que puede ser unos Buds.
    audio: bool,
    conectado: bool,
    emparejado: bool,
    bateria: Option<u8>,
}

/// Algo que se ha pedido y aún no ha pasado.
struct Pendiente {
    mac: String,
    desde: Instant,
    /// Tras emparejar hay que conectar: BlueZ no lo hace solo con todos los
    /// aparatos, y pedirlo antes de que termine el emparejamiento lo dispara
    /// otra vez.
    conectar_al_emparejar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seccion {
    Mios,
    Otros,
}

/// Lo que hay en cada altura de la lista.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fila {
    Seccion(Seccion),
    Dispositivo(usize),
    /// Lo desplegado bajo unos Buds, pegado a su fila.
    Mando(usize),
}

type Colocada = lista::Colocada<Fila>;

/// Lo que hay bajo un punto de la tarjeta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zona {
    Dispositivo(usize),
    Ruido(Modo),
    Desconectar,
    AjustesBuds,
}

pub struct Bluetooth {
    dispositivos: Vec<Dispositivo>,
    /// Lo que se dibuja de cada dispositivo, en el mismo orden. Se rehace en
    /// [`Self::reconstruir`] cada vez que cambia algo de lo que depende.
    entradas: Vec<Entrada>,
    encendido: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendido`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero (siempre el 0).
    pie: tema::Realce,
    ultimo: Value,
    /// Lo que se ha subido la lista, en píxeles.
    desplazamiento: f32,
    estado_carga: Option<String>,
    /// Se le ha pedido a BlueZ que descubra dispositivos.
    escaneando: bool,
    abierta: Instant,
    /// Lo que se pintó de «buscando…», para repintar cuando caduque sin que
    /// nada más cambie.
    buscando_visto: bool,
    pendiente: Option<Pendiente>,
    /// Los Buds: lo último que dijo budsd, si hay hilo leyéndolo.
    buds: buds::Buzon,
    vigilando: bool,
    buds_estado: Option<buds::Estado>,
    /// El modo recién elegido, que se enseña ya: budsd tarda en reflejarlo y
    /// la lectura siguiente devolvería el anterior, con el selector
    /// parpadeando hacia atrás.
    elegido: Option<(Modo, Instant)>,
    desplegado: bool,
}

impl Bluetooth {
    // 0.1.0
    // modified 0.2.1 -- dos secciones, escaneo y mando de los Buds
    pub fn new() -> Self {
        let mut b = Self {
            dispositivos: Vec::new(),
            entradas: Vec::new(),
            encendido: false,
            interruptor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            desplazamiento: 0.0,
            estado_carga: Some("Cargando…".into()),
            escaneando: false,
            abierta: Instant::now(),
            buscando_visto: false,
            pendiente: None,
            buds: Default::default(),
            vigilando: false,
            buds_estado: None,
            elegido: None,
            desplegado: false,
        };
        b.refrescar();
        b
    }

    // 0.1.0
    // modified 0.2.1 -- los no emparejados entran: son «Otros dispositivos»
    fn aplicar(&mut self, data: Value, error: Option<String>) -> bool {
        let status = error.or_else(|| data.is_null().then(|| "Cargando…".into()));
        if self.ultimo == data && self.estado_carga == status {
            return false;
        }
        self.estado_carga = status;
        self.encendido = data["enabled"].as_bool().unwrap_or(false);
        let todos = data["devices"].as_array().into_iter().flatten();
        let (mut mios, mut otros) = (Vec::new(), Vec::new());
        for d in todos.map(Dispositivo::de) {
            if d.emparejado {
                mios.push(d);
            } else if tiene_nombre(&d.nombre) {
                otros.push(d);
            }
        }
        // Emparejados primero, en el orden del servicio (conectados delante);
        // los índices de `señalada` y de `entradas` siguen este orden.
        mios.append(&mut otros);
        self.dispositivos = mios;
        self.ajustar_escaneo();
        self.avanzar_pendiente();
        self.reconstruir();
        self.desplazamiento = self.desplazamiento.min(self.maximo());
        self.interruptor.ir_a(self.encendido as u8 as f32);
        self.ultimo = data;
        true
    }

    /// Descubre mientras la tarjeta esté abierta y el adaptador encendido, y
    /// solo entonces: con el adaptador apagado BlueZ rechaza la orden y salía
    /// un aviso de error al abrir la tarjeta.
    // añadido en la 0.2.1
    fn ajustar_escaneo(&mut self) {
        if self.encendido == self.escaneando || self.estado_carga.is_some() {
            return;
        }
        bookos_system::request(Operation::BluetoothScan {
            enabled: self.encendido,
        });
        self.escaneando = self.encendido;
        if self.encendido {
            self.abierta = Instant::now();
        }
    }

    /// Cierra lo pendiente cuando el dispositivo ya está conectado, y conecta
    /// los que acaban de emparejarse.
    // añadido en la 0.2.1
    fn avanzar_pendiente(&mut self) {
        let Some(p) = self.pendiente.as_mut() else {
            return;
        };
        let Some(d) = self.dispositivos.iter().find(|d| d.mac == p.mac) else {
            return;
        };
        if d.conectado {
            self.pendiente = None;
        } else if d.emparejado && p.conectar_al_emparejar {
            p.conectar_al_emparejar = false;
            bookos_system::request(Operation::BluetoothConnect {
                address: p.mac.clone(),
            });
        }
    }

    /// Rehace lo que se dibuja de cada dispositivo.
    // añadido en la 0.2.1
    fn reconstruir(&mut self) {
        let pendiente = self.pendiente.as_ref().map(|p| p.mac.as_str());
        let buds = self.indice_buds();
        self.entradas = self
            .dispositivos
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let espera = pendiente == Some(d.mac.as_str());
                let bateria = if Some(i) == buds {
                    self.buds_estado.and_then(peor_pila).or(d.bateria)
                } else {
                    d.bateria
                };
                Entrada {
                    nombre: d.nombre.clone(),
                    icono: icono::propio_solido(d.glifo),
                    estado: match (espera, d.conectado, d.emparejado) {
                        (true, _, true) => "Conectando…".into(),
                        (true, _, false) => "Emparejando…".into(),
                        (false, true, _) => "Conectado".into(),
                        _ => String::new(),
                    },
                    derecha: bateria.map(|v| format!("{v}%")),
                    activa: d.conectado,
                    candado: false,
                }
            })
            .collect();
    }

    /// Qué dispositivo son los Buds: el conectado con pinta de auricular que
    /// se llame «Buds» o, si ninguno, el primero. budsd no dice la MAC de lo
    /// que tiene abierto, así que se casa por lo que hay.
    // añadido en la 0.2.1
    fn indice_buds(&self) -> Option<usize> {
        self.buds_estado?;
        let audio = |d: &&Dispositivo| d.conectado && d.audio;
        let con_nombre = self
            .dispositivos
            .iter()
            .position(|d| audio(&d) && d.nombre.contains("Buds"));
        con_nombre.or_else(|| self.dispositivos.iter().position(|d| audio(&d)))
    }

    /// Los pone a leer en cuanto hay algo conectado que pueda ser unos Buds.
    /// Con la tarjeta sin auriculares no hay hilo ni `busctl` cada 3 s.
    // añadido en la 0.2.1
    fn vigilar_buds(&mut self) {
        if !self.vigilando && self.dispositivos.iter().any(|d| d.conectado && d.audio) {
            self.vigilando = true;
            buds::vigilar(&self.buds);
        }
    }

    /// Recoge lo que dejó el hilo de los Buds. `true` si cambió.
    // añadido en la 0.2.1
    fn leer_buds(&mut self) -> bool {
        let leido = *self.buds.lock().expect("nadie suelta el cerrojo a medias");
        if leido == self.buds_estado {
            return false;
        }
        self.buds_estado = leido;
        if leido.is_none() {
            self.desplegado = false;
        }
        self.reconstruir();
        self.desplazamiento = self.desplazamiento.min(self.maximo());
        true
    }

    /// Lo que caduca solo y nadie avisa: el «buscando…», lo pendiente y el
    /// modo recién elegido. `true` si hay que repintar.
    // añadido en la 0.2.1
    fn caducar(&mut self) -> bool {
        let mut cambio = false;
        if self
            .pendiente
            .as_ref()
            .is_some_and(|p| p.desde.elapsed() > PENDIENTE)
        {
            self.pendiente = None;
            self.reconstruir();
            cambio = true;
        }
        if self
            .elegido
            .is_some_and(|(_, cuando)| cuando.elapsed() > ELEGIDO)
        {
            self.elegido = None;
            cambio = true;
        }
        let buscando = self.buscando();
        cambio || std::mem::replace(&mut self.buscando_visto, buscando) != buscando
    }

    fn buscando(&self) -> bool {
        self.escaneando && self.abierta.elapsed() < BUSCANDO
    }

    /// El modo que se enseña como activo: el recién elegido y, si no hay, el
    /// que dice budsd.
    // añadido en la 0.2.1
    fn modo(&self) -> Option<Modo> {
        self.elegido
            .map(|(m, _)| m)
            .or(self.buds_estado.map(|e| e.anc))
    }

    /// Todo lo que hay en la lista, con su altura y su `y` dentro de ella.
    /// Entre filas van [`lista::HUECO_FILA`], salvo entre unos Buds y lo que
    /// llevan desplegado, que forman una sola pieza.
    // añadido en la 0.2.1
    fn colocar(&self) -> Vec<Colocada> {
        let mut v = Vec::new();
        if self.estado_carga.is_some() || !self.encendido {
            return v;
        }
        let mut y = 0.0;
        let buds = self.indice_buds().filter(|_| self.desplegado);
        let mios = self
            .dispositivos
            .iter()
            .take_while(|d| d.emparejado)
            .count();
        if mios > 0 {
            lista::poner(&mut v, &mut y, Fila::Seccion(Seccion::Mios), lista::SECCION);
        }
        for i in 0..mios {
            lista::poner(&mut v, &mut y, Fila::Dispositivo(i), lista::FILA);
            if buds == Some(i) {
                // Sin el hueco: el mando continúa la fila.
                y -= lista::HUECO_FILA;
                lista::poner(&mut v, &mut y, Fila::Mando(i), MANDO_H);
            }
        }
        if mios < self.dispositivos.len() || self.buscando() {
            lista::poner(
                &mut v,
                &mut y,
                Fila::Seccion(Seccion::Otros),
                lista::SECCION,
            );
        }
        for i in mios..self.dispositivos.len() {
            lista::poner(&mut v, &mut y, Fila::Dispositivo(i), lista::FILA);
        }
        v
    }

    /// Lo que mide la lista entera, sin el hueco de después de la última.
    fn total(&self) -> f32 {
        self.colocar().last().map_or(0.0, |c| c.y + c.alto)
    }

    /// El alto de la ventana: lo que mide la lista hasta [`MAX_VENTANA`], o
    /// una fila para el aviso de «vacío».
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
        Ancla::BajoWidget("bluetooth")
    }

    /// Qué hay bajo un punto de la tarjeta (coordenadas de la tarjeta).
    // añadido en la 0.2.1
    fn zona(&self, x: f32, y: f32) -> Option<Zona> {
        let x0 = lista::MARGEN + control::GRUPO;
        let y0 = self.y_lista();
        if x < x0 || x > x0 + lista::BALDOSA || y < y0 || y >= y0 + self.alto_lista() {
            return None;
        }
        let (x, y) = (x - x0, y - y0 + self.desplazamiento);
        let c = self
            .colocar()
            .into_iter()
            .find(|c| (c.y..c.y + c.alto).contains(&y))?;
        match c.fila {
            Fila::Dispositivo(i) => Some(Zona::Dispositivo(i)),
            Fila::Seccion(_) => None,
            Fila::Mando(_) => {
                let y = y - c.y;
                let dentro = x - MANDO_X;
                if (Y_PISTA..Y_PISTA + PISTA).contains(&y) {
                    let ancho = (lista::BALDOSA - MANDO_X * 2.0) / Modo::TODOS.len() as f32;
                    let i = (dentro / ancho).floor();
                    return (i >= 0.0)
                        .then(|| Modo::TODOS.get(i as usize).copied())
                        .flatten()
                        .map(Zona::Ruido);
                }
                if (Y_ENLACES..Y_ENLACES + ENLACES).contains(&y) {
                    let mitad = (lista::BALDOSA - MANDO_X * 2.0) / 2.0;
                    return Some(if dentro < mitad {
                        Zona::Desconectar
                    } else {
                        Zona::AjustesBuds
                    });
                }
                None
            }
        }
    }

    // 0.1.0
    // modified 0.2.1 -- el realce sigue a la zona, no a `n × FILA`
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| match self.zona(x, y) {
            Some(Zona::Dispositivo(i)) => Some(i),
            _ => None,
        });
        let pie = punto.is_some_and(|(x, y)| self.pie_en(x, y)).then_some(0);
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
    // modified 0.2.1 -- dispositivos, mando de los Buds y emparejar
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
        if self.pie_en(x, y) {
            return Some(Accion::Lanzar("bookos-settings --page bluetooth".into()));
        }
        match self.zona(x, y)? {
            Zona::Dispositivo(i) => self.activar(i),
            Zona::Ruido(modo) => {
                buds::poner(modo);
                self.elegido = Some((modo, Instant::now()));
                None
            }
            Zona::Desconectar => {
                let i = self.indice_buds()?;
                self.desplegado = false;
                self.desconectar(i);
                None
            }
            Zona::AjustesBuds => Some(Accion::Lanzar("bookos-settings --page buds".into())),
        }
    }

    fn desconectar(&self, i: usize) {
        bookos_system::request(Operation::BluetoothDisconnect {
            address: self.dispositivos[i].mac.clone(),
        });
    }

    /// Lo que hace la fila `i`, igual con el ratón que con Intro: los Buds
    /// conectados se despliegan, otro conectado se desconecta, uno emparejado
    /// se conecta y uno nuevo se empareja.
    ///
    /// Emparejar lo confirma el usuario en Ajustes (`pairing.rs` lo manda por
    /// `PairingRequested`), así que se abre su página de Bluetooth: sin ella,
    /// si Ajustes no está abierto, la orden esperaría 60 s sin que nadie la
    /// viera.
    // añadido en la 0.2.0
    // modified 0.2.1 -- conectar, emparejar y desplegar los Buds
    fn activar(&mut self, i: usize) -> Option<Accion> {
        let d = self.dispositivos.get(i)?;
        if d.conectado {
            if self.indice_buds() == Some(i) {
                self.desplegado = !self.desplegado;
                if self.desplegado
                    && let Some(c) = self
                        .colocar()
                        .into_iter()
                        .find(|c| c.fila == Fila::Dispositivo(i))
                {
                    // Desde arriba: del mando se ve lo de abajo si hay que
                    // elegir, pero la fila tiene que seguir a la vista.
                    self.mostrar(c.y, (lista::FILA + MANDO_H).min(self.alto_lista()));
                }
                self.desplazamiento = self.desplazamiento.min(self.maximo());
            } else {
                self.desconectar(i);
            }
            return None;
        }
        let mac = d.mac.clone();
        let emparejado = d.emparejado;
        self.pendiente = Some(Pendiente {
            mac: mac.clone(),
            desde: Instant::now(),
            conectar_al_emparejar: !emparejado,
        });
        self.reconstruir();
        if emparejado {
            bookos_system::request(Operation::BluetoothConnect { address: mac });
            None
        } else {
            bookos_system::request(Operation::BluetoothPair { address: mac });
            Some(Accion::Lanzar("bookos-settings --page bluetooth".into()))
        }
    }

    // 0.1.0
    // modified 0.2.0 -- flechas y Intro recorren y activan la lista
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
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
        let n = self.dispositivos.len();
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
            .find(|c| c.fila == Fila::Dispositivo(nueva))
        {
            self.mostrar(c.y, c.alto);
        }
    }

    // 0.1.0
    // modified 0.2.1 -- también recoge a los Buds y lo que caduca
    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        let mut cambio = self.aplicar(state.bluetooth, bookos_system::unavailable("bluetooth"));
        self.vigilar_buds();
        cambio |= self.leer_buds();
        cambio | self.caducar()
    }

    // 0.1.0
    pub fn view(&self) -> PanelElement<'_> {
        control::tarjeta(self.contenido(false), lista::ANCHO, lista::MARGEN)
    }

    /// Lo de dentro de la tarjeta, sin la caja: la Estación de Control lo pone
    /// en la suya. `volver` añade la flecha de volver a la cabecera.
    // 0.1.0
    // modified 0.2.1 -- secciones, filas de distinto alto y mando de los Buds
    pub fn contenido(&self, volver: bool) -> PanelElement<'_> {
        let cabecera = lista::cabecera("Bluetooth", Some(self.interruptor.valor()), volver);
        let colocadas = self.colocar();
        let cuerpo: PanelElement<'_> = if let Some(status) = &self.estado_carga {
            lista::vacia(status)
        } else if !self.encendido {
            lista::vacia("Bluetooth apagado")
        } else if colocadas.is_empty() {
            lista::vacia("No hay dispositivos cerca")
        } else {
            let mut todas = column![];
            let mut fin = 0.0;
            for c in &colocadas {
                // El mando se dibuja con su fila, en la misma pieza.
                if matches!(c.fila, Fila::Mando(_)) {
                    fin = c.y + c.alto;
                    continue;
                }
                if c.y > fin {
                    todas = todas.push(Space::new().height(Length::Fixed(c.y - fin)));
                }
                let (pieza, alto): (PanelElement<'_>, f32) = match c.fila {
                    Fila::Seccion(s) => (
                        lista::seccion(
                            match s {
                                Seccion::Mios => "Mis dispositivos",
                                Seccion::Otros => "Otros dispositivos",
                            },
                            s == Seccion::Otros && self.buscando(),
                        ),
                        c.alto,
                    ),
                    Fila::Dispositivo(i) if self.desplegado && self.indice_buds() == Some(i) => {
                        (self.buds_desplegados(i), lista::FILA + MANDO_H)
                    }
                    Fila::Dispositivo(i) => (
                        lista::fila(&self.entradas[i], self.señalada.intensidad(i)),
                        c.alto,
                    ),
                    Fila::Mando(_) => continue,
                };
                todas = todas.push(pieza);
                fin = c.y + alto;
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
            lista::grupo(column![cuerpo, lista::pie("Ajustes de Bluetooth", &self.pie)].into()),
        ]
        .into()
    }

    /// La fila de unos Buds y, pegado debajo, su mando, sobre un mismo fondo:
    /// dos baldosas redondeadas apiladas dejaban una cintura en la unión.
    // añadido en la 0.2.1
    fn buds_desplegados(&self, i: usize) -> PanelElement<'_> {
        let (disco, glifo) = control::encendido();
        let activo = self.modo();
        let ancho = (lista::BALDOSA - MANDO_X * 2.0 - 4.0) / Modo::TODOS.len() as f32;
        let mut segmentos = row![];
        for modo in Modo::TODOS {
            let marcado = activo == Some(modo);
            segmentos = segmentos.push(
                container(
                    text(modo.etiqueta())
                        .size(12.0)
                        .font(control::peso(Weight::Semibold))
                        .color(if marcado { glifo } else { tema::texto() }),
                )
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(SEGMENTO))
                .center_x(Length::Fixed(ancho))
                .center_y(Length::Fixed(SEGMENTO))
                .style(move |_| container::Style {
                    background: marcado.then(|| disco.into()),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        let pista = container(segmentos).padding(2).style(|_| container::Style {
            background: Some(tema::superficie_en_tarjeta().into()),
            border: Border {
                radius: (tema::R_BOTON_PEQUENO + 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let enlace = |t: &'static str| text(t).size(12.0).color(tema::TEXTO2);
        let mando = container(
            column![
                Space::new().height(Length::Fixed(MANDO_V)),
                container(text(self.linea_de_pilas()).size(11.0).color(tema::TEXTO2))
                    .height(Length::Fixed(LINEA)),
                Space::new().height(Length::Fixed(MANDO_HUECO)),
                pista,
                Space::new().height(Length::Fixed(MANDO_HUECO)),
                container(
                    row![
                        enlace("Desconectar"),
                        Space::new().width(Length::Fill),
                        enlace("Ajustes de los Buds ›"),
                    ]
                    .align_y(Vertical::Center)
                )
                .height(Length::Fixed(ENLACES))
                .center_y(Length::Fixed(ENLACES)),
            ]
            .width(Length::Fill),
        )
        .padding(iced_core::Padding {
            left: MANDO_X,
            right: MANDO_X,
            ..Default::default()
        })
        .width(Length::Fixed(lista::BALDOSA))
        .height(Length::Fixed(MANDO_H));
        container(column![lista::fila(&self.entradas[i], 1.0), mando])
            .width(Length::Fixed(lista::BALDOSA))
            .style(|_| container::Style {
                background: Some(tema::elevado().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// «Izq. 82 % · Der. 79 % · Estuche 45 %», sin lo que no se sabe: budsd
    /// manda 0 mientras el aparato no ha contestado.
    // añadido en la 0.2.1
    fn linea_de_pilas(&self) -> String {
        let Some(e) = self.buds_estado else {
            return String::new();
        };
        [
            ("Izq.", e.izquierdo),
            ("Der.", e.derecho),
            ("Estuche", e.estuche),
        ]
        .into_iter()
        .filter(|(_, v)| *v > 0)
        .map(|(n, v)| format!("{n} {v} %"))
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

impl Drop for Bluetooth {
    /// Deja de descubrir al cerrar la tarjeta: BlueZ seguiría buscando, con el
    /// coste en radio y batería, hasta que alguien lo parase.
    // añadido en la 0.2.1
    fn drop(&mut self) {
        if self.escaneando && self.encendido {
            bookos_system::request(Operation::BluetoothScan { enabled: false });
        }
    }
}

impl Dispositivo {
    // añadido en la 0.2.1
    fn de(d: &Value) -> Self {
        let icono = d["icon"].as_str().unwrap_or_default();
        let conectado = d["connected"] == true;
        Self {
            mac: d["mac"].as_str().unwrap_or_default().into(),
            nombre: d["name"].as_str().unwrap_or_default().into(),
            glifo: glifo_de(icono),
            audio: icono.starts_with("audio-"),
            conectado,
            emparejado: d["paired"] == true || conectado,
            bateria: d["battery"].as_u64().and_then(|v| u8::try_from(v).ok()),
        }
    }
}

/// El icono de la fila según el `Icon` que da BlueZ. Lo que no se reconoce es
/// el logotipo de Bluetooth, que siempre dice algo.
// añadido en la 0.2.1
fn glifo_de(icono: &str) -> &'static str {
    match icono {
        "audio-headset" | "audio-headphones" => "auriculares",
        "audio-card" | "audio-speakers" => "volumen-alto",
        "phone" => "movil",
        "input-mouse" => "raton",
        "input-keyboard" => "teclado",
        _ => "bluetooth",
    }
}

/// BlueZ pone de alias la dirección cuando el aparato no dice su nombre
/// (`4C-3F-1B-…`): en «Otros dispositivos» solo ruido, que no se puede
/// reconocer ni elegir con criterio.
// añadido en la 0.2.1
fn tiene_nombre(nombre: &str) -> bool {
    let es_mac = nombre.len() == 17
        && nombre
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == ':');
    !nombre.is_empty() && !es_mac
}

/// La menor de las pilas de los auriculares: la que decide cuánto queda.
/// El estuche no cuenta, y un 0 es «no lo sé».
// añadido en la 0.2.1
fn peor_pila(e: buds::Estado) -> Option<u8> {
    [e.izquierdo, e.derecho]
        .into_iter()
        .filter(|v| *v > 0)
        .min()
}

/// Cuánto se enseña el modo recién elegido antes de fiarse otra vez de budsd.
const ELEGIDO: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn disp(nombre: &str, mac: &str, emparejado: bool, conectado: bool, icono: &str) -> Value {
        json!({"name":nombre,"mac":mac,"paired":emparejado,"connected":conectado,"icon":icono})
    }

    fn con(dispositivos: Vec<Value>) -> Bluetooth {
        let mut b = Bluetooth::new();
        b.aplicar(json!({"enabled":true,"devices":dispositivos}), None);
        b
    }

    fn buds_conectados() -> Bluetooth {
        let mut b = con(vec![
            disp("Galaxy Buds3 Pro", "AA", true, true, "audio-headset"),
            disp("S22 Ultra", "BB", true, false, "phone"),
            disp("JBL Flip 6", "CC", false, false, "audio-card"),
            disp("Pixel Buds", "DD", false, false, "audio-headset"),
        ]);
        b.buds_estado = Some(buds::Estado {
            anc: Modo::Cancelacion,
            izquierdo: 82,
            derecho: 79,
            estuche: 45,
        });
        b.reconstruir();
        b
    }

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

    /// Los emparejados van primero y los descubiertos después, cada grupo con
    /// su rótulo; y los que solo traen su dirección por nombre no salen.
    #[test]
    fn dos_secciones_y_sin_direcciones_por_nombre() {
        let b = con(vec![
            disp("JBL Flip 6", "CC", false, false, "audio-card"),
            disp("4C-3F-1B-22-AA-01", "EE", false, false, ""),
            disp("S22 Ultra", "BB", true, false, "phone"),
        ]);
        let filas: Vec<Fila> = b.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(
            filas,
            [
                Fila::Seccion(Seccion::Mios),
                Fila::Dispositivo(0),
                Fila::Seccion(Seccion::Otros),
                Fila::Dispositivo(1),
            ]
        );
        assert_eq!(b.dispositivos[0].nombre, "S22 Ultra");
        assert_eq!(b.dispositivos[1].nombre, "JBL Flip 6");
    }

    /// Apagado no se enseña lista: con todo «No conectado» parecía que algo
    /// fallaba, cuando lo único que pasa es que está apagado.
    #[test]
    fn apagado_no_hay_lista() {
        let mut b = Bluetooth::new();
        b.aplicar(
            json!({"enabled":false,"devices":[disp("S22", "BB", true, false, "phone")]}),
            None,
        );
        assert!(b.colocar().is_empty());
        assert_eq!(b.alto_lista(), lista::FILA);
    }

    /// Pulsar donde se dibuja cada cosa: una fila de las de arriba, otra de
    /// las de abajo y un rótulo, que no es de nadie.
    #[test]
    fn se_pulsa_lo_que_se_ve() {
        let b = buds_conectados();
        let x = lista::MARGEN + control::GRUPO + 40.0;
        let y0 = b.y_lista();
        assert_eq!(b.zona(x, y0 + 4.0), None, "el rótulo no se pulsa");
        let c = b
            .colocar()
            .into_iter()
            .find(|c| c.fila == Fila::Dispositivo(2))
            .expect("hay un dispositivo no emparejado");
        assert_eq!(b.zona(x, y0 + c.y + 4.0), Some(Zona::Dispositivo(2)));
        assert_eq!(b.zona(2.0, y0 + c.y + 4.0), None, "fuera por la izquierda");
    }

    /// Un dispositivo nuevo se empareja y abre Ajustes, donde se confirma; uno
    /// emparejado se conecta sin abrir nada.
    #[test]
    fn emparejar_abre_ajustes_y_conectar_no() {
        let mut b = buds_conectados();
        assert!(
            matches!(b.activar(3), Some(Accion::Lanzar(c)) if c.contains("--page bluetooth")),
            "el nuevo se empareja desde Ajustes"
        );
        assert_eq!(b.entradas[3].estado, "Emparejando…");
        let mut b = buds_conectados();
        assert!(b.activar(1).is_none(), "uno emparejado solo se conecta");
        assert_eq!(b.entradas[1].estado, "Conectando…");
    }

    /// Los Buds conectados se despliegan en vez de desconectarse, y el mando
    /// responde donde se dibuja: el selector elige y los enlaces actúan.
    #[test]
    fn los_buds_se_despliegan_y_el_mando_responde() {
        let mut b = buds_conectados();
        let sin = b.total();
        assert!(b.activar(0).is_none());
        assert!(b.desplegado);
        assert_eq!(b.total(), sin + MANDO_H, "el mando va pegado a su fila");

        let mando = b
            .colocar()
            .into_iter()
            .find(|c| matches!(c.fila, Fila::Mando(_)))
            .expect("desplegado hay mando");
        let x0 = lista::MARGEN + control::GRUPO + MANDO_X;
        let y = b.y_lista() + mando.y - b.desplazamiento;
        let ancho = (lista::BALDOSA - MANDO_X * 2.0) / 3.0;
        let en_pista = y + Y_PISTA + PISTA / 2.0;
        assert_eq!(
            b.zona(x0 + 4.0, en_pista),
            Some(Zona::Ruido(Modo::Cancelacion))
        );
        assert_eq!(
            b.zona(x0 + ancho * 1.5, en_pista),
            Some(Zona::Ruido(Modo::Ambiente))
        );
        assert_eq!(
            b.zona(x0 + ancho * 2.5, en_pista),
            Some(Zona::Ruido(Modo::Apagado))
        );
        let en_enlaces = y + Y_ENLACES + ENLACES / 2.0;
        assert_eq!(b.zona(x0 + 4.0, en_enlaces), Some(Zona::Desconectar));
        assert_eq!(
            b.zona(x0 + ancho * 2.5, en_enlaces),
            Some(Zona::AjustesBuds)
        );
        assert_eq!(
            b.zona(x0 + 4.0, y + 2.0),
            None,
            "la línea de pilas no se pulsa"
        );

        assert!(b.activar(0).is_none());
        assert!(!b.desplegado, "otra pulsación los pliega");
    }

    /// El modo recién elegido se enseña ya, sin esperar a que budsd lo
    /// refleje: si no, el selector volvía al anterior hasta la lectura
    /// siguiente.
    #[test]
    fn el_modo_elegido_se_ve_al_momento() {
        let mut b = buds_conectados();
        assert_eq!(b.modo(), Some(Modo::Cancelacion));
        b.elegido = Some((Modo::Ambiente, Instant::now()));
        assert_eq!(b.modo(), Some(Modo::Ambiente));
    }

    /// Sin canal con los Buds no hay mando: la fila se comporta como otra
    /// cualquiera y una pulsación desconecta.
    #[test]
    fn sin_budsd_no_hay_mando() {
        let mut b = con(vec![disp("Galaxy Buds", "AA", true, true, "audio-headset")]);
        assert_eq!(b.indice_buds(), None);
        assert!(b.activar(0).is_none());
        assert!(!b.desplegado);
    }

    #[test]
    fn la_linea_de_pilas_omite_lo_que_no_se_sabe() {
        let mut b = buds_conectados();
        assert_eq!(b.linea_de_pilas(), "Izq. 82 % · Der. 79 % · Estuche 45 %");
        b.buds_estado = Some(buds::Estado {
            anc: Modo::Apagado,
            izquierdo: 0,
            derecho: 79,
            estuche: 0,
        });
        assert_eq!(b.linea_de_pilas(), "Der. 79 %");
    }

    /// La lista nunca pasa de su tope, y desplegado cabe lo que se ve.
    #[test]
    fn la_ventana_no_pasa_de_su_tope() {
        let mut b = buds_conectados();
        b.activar(0);
        assert!(b.alto_lista() <= MAX_VENTANA);
        assert!(b.maximo() >= 0.0);
        let todo = b.desplazar(10_000.0);
        assert!(!todo || b.desplazamiento <= b.maximo());
    }

    /// Vuelca la tarjeta a PNG para mirarla: plegada y con los Buds
    /// desplegados, en oscuro y en claro. `BOOKOS_BLUETOOTH_PNG=/tmp/bt`
    /// escribe `/tmp/bt-<variante>-<tema>.png`.
    #[test]
    fn vuelca_la_tarjeta_a_png() {
        let Ok(prefijo) = std::env::var("BOOKOS_BLUETOOTH_PNG") else {
            return;
        };
        for (tema, nombre) in [
            (crate::tema::Tema::Oscuro, "oscuro"),
            (crate::tema::Tema::Claro, "claro"),
        ] {
            for desplegados in [false, true] {
                let mut b = buds_conectados();
                if desplegados {
                    b.activar(0);
                }
                let config = crate::Config {
                    tema,
                    ..crate::Config::default()
                };
                // El interruptor se anima desde apagado; sin esperar, la captura
                // lo pilla a mitad de recorrido y parece que está apagado.
                std::thread::sleep(Duration::from_millis(400));
                let mut shell = crate::Shell::con_config(1646, 1.0, config);
                shell.abrir(super::super::Emergente::Bluetooth(b));
                let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
                let mut buf = vec![0u8; (w * h * 4) as usize];
                shell.draw_emergente(&mut buf);
                for p in buf.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                }
                let variante = if desplegados { "desplegado" } else { "plegado" };
                image::save_buffer(
                    format!("{prefijo}-{variante}-{nombre}.png"),
                    &buf,
                    w,
                    h,
                    image::ExtendedColorType::Rgba8,
                )
                .expect("se escribe el PNG");
            }
        }
    }
}
