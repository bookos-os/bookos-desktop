mod ipc;
mod lente;
mod sistema;
mod usuarios;
mod vista;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookos_shell::bloqueo::Peticion;
use bookos_shell::tema::{self, Transicion};
use bookos_shell::{Battery, Icono, Link, Network, Shell};
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};
use iced::{Point, Size, Subscription, Task, Theme};

use sistema::{EstadoBateria, EstadoWifi, Idioma, RedWifi};
use usuarios::Persona;

const SACUDIDA: Duration = Duration::from_millis(520);

// 0.1.1
fn main() -> anyhow::Result<()> {
    let mut previa = false;
    let mut sesion = "/usr/local/bin/bookos-session".to_string();
    let mut argumentos = std::env::args().skip(1);
    while let Some(argumento) = argumentos.next() {
        match argumento.as_str() {
            "--preview" => previa = true,
            "--session" => {
                sesion = argumentos
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Falta la ruta de sesión"))?;
                anyhow::ensure!(
                    std::path::Path::new(&sesion).is_absolute(),
                    "La sesión debe tener una ruta absoluta"
                );
            }
            "--help" | "-h" => {
                println!(
                    "Uso: bookos-greeter [--preview] [--session /ruta/bookos-session]\n\nSin --preview requiere Greetd y su variable GREETD_SOCK.\nLa vista previa no autentica ni inicia sesiones."
                );
                return Ok(());
            }
            _ => anyhow::bail!("Argumento desconocido: {argumento}"),
        }
    }
    let socket = if previa {
        None
    } else {
        let ruta = std::env::var_os("GREETD_SOCK").ok_or_else(|| {
            anyhow::anyhow!("Falta GREETD_SOCK; usa --preview para ver la interfaz")
        })?;
        anyhow::ensure!(
            std::path::Path::new(&sesion).is_file(),
            "No existe el programa de sesión: {sesion}"
        );
        Some(PathBuf::from(ruta))
    };
    iced::application(
        move || {
            let mut personas = usuarios::leer();
            if previa {
                // Con una sola cuenta la lente no tiene vecinas que mover.
                for nombre in ["Laura", "Álvaro"]
                    .iter()
                    .skip(personas.len().saturating_sub(1))
                {
                    personas.push(Persona::nueva(
                        &nombre.to_lowercase(),
                        nombre,
                        None,
                        &usuarios::raiz_publicada(),
                    ));
                }
            }
            Acceso::nuevo(socket.clone(), sesion.clone(), personas)
        },
        Acceso::actualizar,
        Acceso::vista,
    )
    .title("BookOS · Iniciar sesión")
    .theme(tema_iced)
    .subscription(Acceso::suscripcion)
    .window_size((1280.0, 800.0))
    .decorations(previa)
    .exit_on_close_request(previa)
    .centered()
    .run()?;
    Ok(())
}


// 0.1.1
fn tema_iced(_: &Acceso) -> Theme {
    Theme::custom(
        "BookOS",
        iced::theme::Palette {
            background: vista::FONDO,
            text: iced::Color::WHITE,
            primary: tema::acento(),
            success: tema::verde(),
            warning: tema::amarillo(),
            danger: tema::rojo(),
        },
    )
}

// 0.1.1
fn entrada(
    evento: iced::Event,
    estado: iced::event::Status,
    _: iced::window::Id,
) -> Option<Mensaje> {
    use iced::keyboard::{Event as Tecla, Key, key::Named};
    use iced::{Event, mouse};
    let libre = estado == iced::event::Status::Ignored;
    match evento {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Mensaje::Puntero(Some(position)))
        }
        Event::Mouse(mouse::Event::CursorLeft) => Some(Mensaje::Puntero(None)),
        Event::Keyboard(Tecla::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) => {
            if let Key::Named(Named::CapsLock) = key {
                return Some(Mensaje::AlternarMayusculas);
            }
            if modifiers.alt() && modifiers.shift() {
                return Some(Mensaje::SiguienteTeclado);
            }
            match key {
                // El campo de texto se queda con Esc para soltar el foco; aquí
                // tiene que cancelar igual, o haría falta pulsarlo dos veces.
                Key::Named(Named::Escape) => Some(Mensaje::Cancelar),
                Key::Named(Named::Tab) if libre && modifiers.shift() => {
                    Some(Mensaje::FocoAnterior)
                }
                Key::Named(Named::Tab) if libre => Some(Mensaje::FocoSiguiente),
                Key::Named(Named::Enter) if libre => Some(Mensaje::Continuar),
                Key::Named(Named::ArrowUp) if libre => Some(Mensaje::Mover(-1)),
                Key::Named(Named::ArrowDown) if libre => Some(Mensaje::Mover(1)),
                _ => text.map(|t| Mensaje::TeclaTexto(t.to_string(), modifiers.shift())),
            }
        }
        _ => None,
    }
}

/// Un aviso al empezar cada minuto. iced no trae temporizador con el ejecutor
/// de hilos, y despertar cada segundo para un reloj sin segundos sobra.
// 0.1.1
fn minutos() -> iced::futures::channel::mpsc::UnboundedReceiver<Mensaje> {
    let (envio, recepcion) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        loop {
            let segundos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() % 60);
            std::thread::sleep(Duration::from_secs(60 - segundos));
            if envio.unbounded_send(Mensaje::Tic).is_err() {
                return;
            }
        }
    });
    recepcion
}

/// Lo que deja `pam_bookos_huella` sobre la huella y lo que pregunta. Están
/// repetidos de `crates/bookos-pam-huella`: el módulo es una biblioteca de C
/// (`cdylib`) y no se puede usar como dependencia.
const ESTADO_HUELLA: &str = "/run/bookos-huella/estado";
pub(crate) const PREGUNTA_HUELLA: &str = "Contraseña o huella";

/// Vigila lo que el módulo PAM dice de la huella mientras greetd pregunta la
/// contraseña. Cada 100 ms y no con inotify: es un fichero que cambia dos o
/// tres veces por intento, y el chip ya tarda ~50 ms en decir que casa.
// añadido en la 0.2.0
fn huella() -> iced::futures::channel::mpsc::UnboundedReceiver<Mensaje> {
    let (envio, recepcion) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let mut visto = String::new();
        // Se acaba cuando iced suelta la suscripción y con ella el receptor.
        while !envio.is_closed() {
            if let Ok(ahora) = std::fs::read_to_string(ESTADO_HUELLA)
                && ahora != visto
            {
                visto = ahora;
                if envio
                    .unbounded_send(Mensaje::Huella(visto.clone()))
                    .is_err()
                {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    recepcion
}

#[derive(Clone, Copy, Debug)]
enum Operacion {
    Autenticar,
    Iniciar,
    Cancelar,
}

enum Estado {
    Usuario,
    Pregunta { texto: String, tipo: TipoEntrada },
    Aviso(String),
    Ocupado(Operacion),
}

#[derive(Clone, Copy)]
enum TipoEntrada {
    Visible,
    Secreta,
}

#[derive(Clone)]
enum Respuesta {
    Confirmado,
    Pregunta { texto: String, tipo: TipoEntrada },
    Aviso(String),
    Rechazo(String),
}

impl From<Response> for Respuesta {
    // 0.1.1
    fn from(respuesta: Response) -> Self {
        match respuesta {
            Response::Success => Self::Confirmado,
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret => Self::Pregunta {
                    texto: auth_message,
                    tipo: TipoEntrada::Secreta,
                },
                AuthMessageType::Visible => Self::Pregunta {
                    texto: auth_message,
                    tipo: TipoEntrada::Visible,
                },
                AuthMessageType::Info | AuthMessageType::Error => Self::Aviso(auth_message),
            },
            Response::Error {
                error_type,
                description,
            } => Self::Rechazo(match error_type {
                ErrorType::AuthError => {
                    "No se ha podido autenticar. Comprueba tus datos e inténtalo de nuevo.".into()
                }
                ErrorType::Error => format!("No se pudo iniciar la sesión: {description}"),
            }),
        }
    }
}

/// Qué tarjeta emergente de la esquina superior derecha está abierta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Idioma,
    Bateria,
    Red,
    Energia,
}

#[derive(Clone)]
enum Mensaje {
    /// Lo que dice `pam_bookos_huella` del lector: `coincide` o
    /// `no-coincide N`.
    Huella(String),
    Usuario(String),
    Respuesta(String),
    Continuar,
    Cancelar,
    FocoAnterior,
    FocoSiguiente,
    Recibido(Operacion, Result<Respuesta, String>),
    Elegir(usize),
    Mover(i32),
    Puntero(Option<Point>),
    Pantalla(Size),
    Fondo(usize, (u32, u32), Option<usuarios::Fondo>),
    Densidad(f32),
    AbrirPanel(Option<Panel>),
    ElegirTeclado(String),
    SiguienteTeclado,
    ElegirIdioma(Idioma),
    AlternarMostrarClave,
    AlternarMayusculas,
    TeclaTexto(String, bool),
    PerfilEnergia(String),
    BateriaActualizada(EstadoBateria),
    WifiEstado(EstadoWifi),
    WifiAlternar(bool),
    WifiRefrescar,
    WifiElegir(usize),
    WifiClave(String),
    WifiConectar,
    WifiCancelarClave,
    WifiDesconectar,
    Energia(Peticion),
    /// El error de systemctl, si lo hubo.
    EnergiaHecha(Option<String>),
    Tic,
    Fotograma,
}

#[cfg(test)]
mod tests;

impl std::fmt::Debug for Mensaje {
    // 0.1.1
    fn fmt(&self, formato: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Iced puede incluir los mensajes en diagnósticos; nunca volcamos la
        // respuesta a PAM ni la contraseña Wi-Fi, aunque se dibujen como puntos.
        formato.write_str(match self {
            Self::Usuario(_) => "Usuario",
            Self::Respuesta(_) => "Respuesta reservada",
            Self::WifiClave(_) => "Clave Wi-Fi reservada",
            Self::TeclaTexto(..) => "Tecla reservada",
            Self::Continuar => "Continuar",
            Self::Cancelar => "Cancelar",
            Self::FocoAnterior => "Foco anterior",
            Self::FocoSiguiente => "Foco siguiente",
            Self::Recibido(..) => "Respuesta de greetd",
            _ => "Interfaz",
        })
    }
}

/// Dónde está el centro de la lente de la lista.
#[derive(Clone, Copy)]
enum Foco {
    Ninguno,
    /// Altura del puntero, medida desde el borde de arriba de la lista.
    Puntero(f32),
    /// La fila elegida con las flechas.
    Fila(usize),
}

struct Acceso {
    cliente: Option<ipc::Cliente>,
    sesion: String,
    personas: Vec<Persona>,
    usuario: String,
    respuesta: String,
    mostrar_respuesta: bool,
    caps_lock: bool,
    estado: Estado,
    error: Option<String>,
    /// El nombre se escribe a mano en vez de elegirse de la lista.
    manual: bool,
    /// Ya se contestó a PAM en esta conversación. Un rechazo después de eso es
    /// una contraseña mala y se vuelve a preguntar; uno antes es que la cuenta
    /// no puede entrar, y repetirlo sería un bucle.
    respondido: bool,
    señalada: usize,
    /// De quién son el tema y el fondo que se ven.
    de: usize,
    foco: Foco,
    fuerza: Transicion,
    menu: Transicion,
    panel_abierto: Option<Panel>,
    panel_visible: Option<Panel>,
    hora: String,
    fecha: String,
    bateria: Option<(Icono, (f32, f32), String, iced::Color)>,
    bateria_detalle: EstadoBateria,
    red: Option<Network>,
    wifi_estado: EstadoWifi,
    wifi_cargando: bool,
    wifi_conectar_a: Option<String>,
    wifi_clave: String,
    wifi_error: Option<String>,
    idioma: Idioma,
    teclado_base: String,
    teclado_codigo: String,
    teclado: Option<String>,
    ultima_tecla: Option<(String, bool, Instant)>,
    /// Tamaño de la ventana en las unidades de iced: píxeles en cage.
    pantalla: Size,
    /// Por cuánto se multiplica cada medida de la vista.
    ///
    /// No es el `scale_factor` de iced a propósito: con él, tiny-skia pinta
    /// todo a través de una transformación y pierde el camino rápido de copia.
    /// Medido a 2880×1800, el fondo solo costaba 28 ms por fotograma.
    escala: f32,
    escala_fijada: bool,
    /// Píxeles por unidad de iced: 1 en cage, 2 en una ventana de vista previa
    /// dentro de una sesión a escala 2.
    densidad: f32,
    fondo: Option<usuarios::Fondo>,
    fondo_pedido: Option<(usize, (u32, u32))>,
    cambio_desde: Instant,
    fallo_desde: Option<Instant>,
    candado: Icono,
    flecha: Icono,
    wifi: Icono,
    encendido: Icono,
    tecla: Icono,
    ojo: Icono,
    ojo_tachado: Icono,
    comprobado: Icono,
}

impl Acceso {
    // 0.1.1
    fn nuevo(
        socket: Option<PathBuf>,
        sesion: String,
        personas: Vec<Persona>,
    ) -> (Self, Task<Mensaje>) {
        let ahora = Instant::now();
        let previa = socket.is_none();
        let idioma = Idioma::detectar();
        let teclado_base = sistema::teclado_inicial();
        let mut acceso = Self {
            cliente: socket.map(ipc::Cliente::nuevo),
            sesion,
            manual: personas.is_empty(),
            personas,
            usuario: String::new(),
            respuesta: String::new(),
            mostrar_respuesta: false,
            caps_lock: false,
            estado: Estado::Usuario,
            error: None,
            respondido: false,
            señalada: 0,
            de: 0,
            foco: Foco::Ninguno,
            fuerza: Transicion::nueva(0.0, tema::M_RAPIDO),
            menu: Transicion::nueva(0.0, tema::M_SUAVE),
            panel_abierto: None,
            panel_visible: None,
            hora: Shell::hora_bloqueo(),
            fecha: idioma.fecha_hoy(),
            bateria: None,
            bateria_detalle: EstadoBateria {
                dato: None,
                tope: None,
                perfil: "balanced".into(),
                perfiles_disponibles: vec![
                    "power-saver".into(),
                    "balanced".into(),
                    "performance".into(),
                ],
            },
            red: None,
            wifi_estado: if previa {
                EstadoWifi {
                    encendida: true,
                    cable: false,
                    ssid_actual: Some("BookOS-5G".into()),
                    redes: vec![
                        RedWifi {
                            ssid: "BookOS-5G".into(),
                            senal: 92,
                            activa: true,
                            segura: true,
                            guardada: true,
                        },
                        RedWifi {
                            ssid: "Invitados".into(),
                            senal: 68,
                            activa: false,
                            segura: true,
                            guardada: false,
                        },
                        RedWifi {
                            ssid: "Cafetería".into(),
                            senal: 45,
                            activa: false,
                            segura: false,
                            guardada: false,
                        },
                    ],
                    error: None,
                }
            } else {
                EstadoWifi::default()
            },
            wifi_cargando: false,
            wifi_conectar_a: None,
            wifi_clave: String::new(),
            wifi_error: None,
            idioma,
            teclado: Some(teclado_base.to_uppercase()),
            teclado_codigo: teclado_base.clone(),
            teclado_base,
            ultima_tecla: None,
            pantalla: Size::new(1280.0, 800.0),
            escala: 1.0,
            escala_fijada: previa,
            densidad: 1.0,
            fondo: None,
            fondo_pedido: None,
            cambio_desde: ahora,
            fallo_desde: None,
            candado: bookos_shell::icono::desde_svg(bookos_shell::bloqueo::CANDADO),
            flecha: bookos_shell::icono::desde_svg(bookos_shell::bloqueo::FLECHA),
            wifi: bookos_shell::icono::desde_svg(vista::WIFI),
            encendido: bookos_shell::icono::desde_svg(vista::ENCENDIDO),
            tecla: bookos_shell::icono::desde_svg(vista::TECLADO),
            ojo: bookos_shell::icono::propio("ojo")
                .unwrap_or_else(|| bookos_shell::icono::desde_svg(vista::TECLADO)),
            ojo_tachado: bookos_shell::icono::propio("ojo-tachado")
                .unwrap_or_else(|| bookos_shell::icono::desde_svg(vista::TECLADO)),
            comprobado: bookos_shell::icono::propio("comprobado")
                .unwrap_or_else(|| bookos_shell::icono::desde_svg(bookos_shell::bloqueo::FLECHA)),
        };
        acceso.leer_estado();
        acceso.vestir(0);
        let ventana = iced::window::latest().and_then(|id| {
            Task::batch([
                iced::window::size(id).map(Mensaje::Pantalla),
                iced::window::scale_factor(id).map(Mensaje::Densidad),
            ])
        });
        let inicio = if acceso.personas.len() == 1 && !previa {
            acceso.elegir(0)
        } else {
            iced::widget::operation::focus("usuario")
        };
        (acceso, Task::batch([ventana, inicio]))
    }

    // 0.1.1
    // modificado en la 0.2.0 -- vigila la huella mientras pregunta pam_bookos_huella
    fn suscripcion(&self) -> Subscription<Mensaje> {
        let mut todas = vec![
            iced::event::listen_with(entrada),
            iced::window::resize_events().map(|(_, tamano)| Mensaje::Pantalla(tamano)),
            Subscription::run(minutos),
        ];
        if self.cliente.is_some()
            && matches!(&self.estado, Estado::Pregunta { texto, .. } if texto == PREGUNTA_HUELLA)
        {
            todas.push(Subscription::run(huella));
        }
        if self.animando() {
            todas.push(iced::window::frames().map(|_| Mensaje::Fotograma));
        }
        Subscription::batch(todas)
    }

    // 0.1.1
    fn animando(&self) -> bool {
        self.fuerza.animando()
            || self.menu.animando()
            || self.cambio_desde.elapsed() < tema::D_PAGINA
            || self.fallo_desde.is_some_and(|d| d.elapsed() < SACUDIDA)
    }

    // 0.1.1
    fn en_lista(&self) -> bool {
        matches!(self.estado, Estado::Usuario) && !self.manual
    }

    // 0.1.1
    fn leer_estado(&mut self) {
        self.bateria = Battery::read(false)
            .as_ref()
            .map(bookos_shell::bateria_sobre_fondo);
        self.red = Network::read();
    }

    // 0.1.1
    fn fijar_teclado(&mut self, codigo: String) {
        self.teclado_codigo = codigo.trim().to_lowercase();
        self.teclado = (!self.teclado_codigo.is_empty()).then(|| self.teclado_codigo.to_uppercase());
    }

    /// Filas, factor de encogido y borde de arriba de la lista en reposo.
    // 0.1.1
    fn lista(&self) -> (usize, f32, f32) {
        let filas = self.personas.len() + 1;
        let reposo = lente::alto_en_reposo(filas, 1.0);
        let k = (self.pantalla.height * 0.45 / reposo).min(self.escala);
        let arriba = (self.pantalla.height * 0.80 - reposo * k / 2.0)
            .min(self.pantalla.height - 32.0 * self.escala - reposo * k);
        (filas, k, arriba)
    }

    /// Pone el tema y el acento de una persona. El fondo llega después.
    // 0.1.1
    fn vestir(&mut self, de: usize) {
        self.de = de;
        let Some(persona) = self.personas.get(de) else {
            return;
        };
        tema::aplicar(persona.apariencia.tema);
        tema::aplicar_acento(persona.apariencia.acento);
        tema::aplicar_efectos_reducidos(persona.apariencia.efectos_reducidos);
        if let Some(teclado) = &persona.apariencia.teclado {
            self.fijar_teclado(teclado.clone());
        }
    }

    // 0.1.1
    fn pedir_fondo(&mut self) -> Task<Mensaje> {
        let px = (
            (self.pantalla.width * self.densidad).round() as u32,
            (self.pantalla.height * self.densidad).round() as u32,
        );
        let pedido = (self.de, px);
        if self.fondo_pedido == Some(pedido) || px.0 == 0 || px.1 == 0 {
            return Task::none();
        }
        self.fondo_pedido = Some(pedido);
        let apariencia = self.personas.get(self.de).map(|p| &p.apariencia);
        let tema_de = apariencia.map_or_else(tema::actual, |a| a.tema);
        let automatico = apariencia.is_some_and(|a| a.acento == tema::Acento::Automatico);
        // El de serie va también detrás del publicado: un SVG o un fichero
        // roto no pueden dejar la pantalla de acceso en negro.
        let rutas = [
            apariencia.and_then(|a| a.fondo.clone()),
            usuarios::fondo_por_defecto(tema_de),
        ];
        let densidad = self.densidad;
        Task::perform(
            async move {
                rutas
                    .iter()
                    .flatten()
                    .find_map(|ruta| usuarios::cargar_fondo(ruta, px, densidad, automatico))
            },
            move |fondo| Mensaje::Fondo(pedido.0, pedido.1, fondo),
        )
    }

    // 0.1.1
    fn elegir(&mut self, i: usize) -> Task<Mensaje> {
        self.error = None;
        self.mostrar_respuesta = false;
        self.cambio_desde = Instant::now();
        let Some(persona) = self.personas.get(i) else {
            self.manual = true;
            self.usuario.clear();
            return iced::widget::operation::focus("usuario");
        };
        self.usuario = persona.cuenta.clone();
        self.señalada = i;
        self.vestir(i);
        let fondo = self.pedir_fondo();
        let acceso = if self.cliente.is_some() {
            self.autenticar()
        } else {
            self.estado = Estado::Pregunta {
                texto: self
                    .idioma
                    .t(
                        "Vista previa: no se comprueba la contraseña.",
                        "Preview mode: password is not checked.",
                    )
                    .into(),
                tipo: TipoEntrada::Secreta,
            };
            iced::widget::operation::focus("respuesta")
        };
        Task::batch([fondo, acceso])
    }

    // 0.1.1
    fn autenticar(&mut self) -> Task<Mensaje> {
        self.respondido = false;
        self.enviar(
            Operacion::Autenticar,
            Request::CreateSession {
                username: self.usuario.trim().into(),
            },
        )
    }

    // 0.1.1
    fn enviar(&mut self, operacion: Operacion, peticion: Request) -> Task<Mensaje> {
        let Some(cliente) = &self.cliente else {
            match operacion {
                Operacion::Cancelar => {
                    self.error = None;
                    self.volver();
                }
                Operacion::Autenticar | Operacion::Iniciar => {
                    self.error = Some(
                        self.idioma
                            .t(
                                "Vista previa: la autenticación está desactivada.",
                                "Preview mode: authentication is disabled.",
                            )
                            .into(),
                    );
                    self.fallo_desde = Some(Instant::now());
                }
            }
            return Task::none();
        };
        self.estado = Estado::Ocupado(operacion);
        self.error = None;
        Task::perform(cliente.enviar(peticion), move |resultado| {
            Mensaje::Recibido(operacion, resultado.map(Respuesta::from))
        })
    }

    // 0.1.1
    fn volver(&mut self) {
        self.estado = Estado::Usuario;
        self.mostrar_respuesta = false;
        self.cambio_desde = Instant::now();
    }

    // 0.1.1
    fn aplicar_traduccion_teclado(&mut self, valor: &mut String) {
        if self.teclado_base == self.teclado_codigo {
            return;
        }
        let Some((tecla, shift, cuando)) = self.ultima_tecla.take() else {
            return;
        };
        if cuando.elapsed() > Duration::from_millis(80) || !valor.ends_with(&tecla) {
            return;
        }
        if let Some(traducida) =
            sistema::traducir_caracter(&tecla, &self.teclado_base, &self.teclado_codigo, shift)
        {
            let nuevo_largo = valor.len().saturating_sub(tecla.len());
            valor.truncate(nuevo_largo);
            valor.push_str(&traducida);
        }
    }

    // 0.1.1
    fn abrir_o_cerrar_panel(&mut self, panel: Option<Panel>) -> Task<Mensaje> {
        let destino = if self.panel_abierto == panel {
            None
        } else {
            panel
        };
        self.panel_abierto = destino;
        if let Some(p) = destino {
            self.panel_visible = Some(p);
            self.menu.ir_a(1.0);
            match p {
                Panel::Bateria => {
                    return Task::perform(
                        async { sistema::leer_bateria(true) },
                        Mensaje::BateriaActualizada,
                    );
                }
                Panel::Red if self.cliente.is_some() => {
                    self.wifi_cargando = true;
                    return Task::perform(
                        async { sistema::leer_wifi(false) },
                        Mensaje::WifiEstado,
                    );
                }
                _ => {}
            }
        } else {
            self.wifi_conectar_a = None;
            self.wifi_clave.clear();
            self.wifi_error = None;
            self.menu.ir_a(0.0);
        }
        Task::none()
    }

    // 0.1.1
    fn actualizar(&mut self, mensaje: Mensaje) -> Task<Mensaje> {
        match mensaje {
            Mensaje::FocoAnterior => return iced::widget::operation::focus_previous(),
            Mensaje::FocoSiguiente => return iced::widget::operation::focus_next(),
            Mensaje::TeclaTexto(texto, shift) => {
                self.ultima_tecla = Some((texto, shift, Instant::now()));
            }
            // La huella casó: se contesta vacío solo para soltar la conversación.
            // Lo que abre la sesión es lo que vio el módulo en fprintd, no
            // esta respuesta.
            Mensaje::Huella(estado)
                if estado == "coincide" && matches!(self.estado, Estado::Pregunta { .. }) =>
            {
                self.respondido = true;
                self.respuesta.clear();
                self.mostrar_respuesta = false;
                return self.enviar(
                    Operacion::Autenticar,
                    Request::PostAuthMessageResponse {
                        response: Some(String::new()),
                    },
                );
            }
            Mensaje::Huella(estado) if estado.starts_with("no-coincide") => {
                self.error = Some(
                    self.idioma
                        .t("Huella no reconocida", "Fingerprint not recognized")
                        .into(),
                );
                self.fallo_desde = Some(Instant::now());
            }
            Mensaje::Huella(_) => {}
            Mensaje::AlternarMayusculas => {
                self.caps_lock = !self.caps_lock;
            }
            Mensaje::AlternarMostrarClave => {
                self.mostrar_respuesta = !self.mostrar_respuesta;
                return iced::widget::operation::focus("respuesta");
            }
            Mensaje::Usuario(mut valor) if matches!(self.estado, Estado::Usuario) => {
                self.aplicar_traduccion_teclado(&mut valor);
                self.usuario = valor;
            }
            Mensaje::Respuesta(mut valor) if matches!(self.estado, Estado::Pregunta { .. }) => {
                self.aplicar_traduccion_teclado(&mut valor);
                self.respuesta = valor;
            }
            Mensaje::Continuar => match &self.estado {
                Estado::Usuario if !self.manual => return self.elegir(self.señalada),
                Estado::Usuario if !self.usuario.trim().is_empty() => return self.autenticar(),
                Estado::Pregunta { .. } => {
                    self.respondido = true;
                    self.mostrar_respuesta = false;
                    let response = Some(std::mem::take(&mut self.respuesta));
                    return self.enviar(
                        Operacion::Autenticar,
                        Request::PostAuthMessageResponse { response },
                    );
                }
                Estado::Usuario | Estado::Aviso(_) | Estado::Ocupado(_) => {}
            },
            Mensaje::Cancelar if self.menu.destino() > 0.0 => {
                return self.abrir_o_cerrar_panel(None);
            }
            Mensaje::Cancelar
                if matches!(self.estado, Estado::Pregunta { .. } | Estado::Aviso(_)) =>
            {
                self.respuesta.clear();
                self.mostrar_respuesta = false;
                return self.enviar(Operacion::Cancelar, Request::CancelSession);
            }
            Mensaje::Cancelar
                if matches!(self.estado, Estado::Usuario)
                    && self.manual
                    && !self.personas.is_empty() =>
            {
                self.manual = false;
                self.error = None;
                self.cambio_desde = Instant::now();
            }
            Mensaje::Recibido(operacion, resultado) => {
                self.respuesta.clear();
                self.mostrar_respuesta = false;
                match resultado {
                    Ok(Respuesta::Confirmado) => match operacion {
                        Operacion::Autenticar => {
                            let peticion = ipc::iniciar_sesion(
                                &self.sesion,
                                &self.teclado_codigo,
                                self.idioma.locale(),
                            );
                            return self.enviar(Operacion::Iniciar, peticion);
                        }
                        Operacion::Iniciar => return iced::exit(),
                        Operacion::Cancelar => self.volver(),
                    },
                    Ok(Respuesta::Pregunta { texto, tipo }) => {
                        self.estado = Estado::Pregunta { texto, tipo };
                        return iced::widget::operation::focus("respuesta");
                    }
                    // greetd retiene la conversación de PAM hasta que se
                    // contesta un aviso. pam_fprintd avisa «pon el dedo» y no
                    // empieza a leer el lector hasta recibir respuesta: había
                    // que pulsar Intro para que la huella funcionase.
                    Ok(Respuesta::Aviso(aviso)) => {
                        let tarea = self.enviar(
                            Operacion::Autenticar,
                            Request::PostAuthMessageResponse { response: None },
                        );
                        self.estado = Estado::Aviso(aviso);
                        return tarea;
                    }
                    Ok(Respuesta::Rechazo(error)) if self.respondido => {
                        let tarea = self.autenticar();
                        self.error = Some(error);
                        self.fallo_desde = Some(Instant::now());
                        return tarea;
                    }
                    Ok(Respuesta::Rechazo(error)) | Err(error) => {
                        self.volver();
                        self.error = Some(error);
                    }
                }
                return iced::widget::operation::focus("usuario");
            }
            Mensaje::Elegir(i) if self.en_lista() => return self.elegir(i),
            Mensaje::Mover(paso) if self.en_lista() => {
                let ultima = self.personas.len() as i32;
                self.señalada = (self.señalada as i32 + paso).clamp(0, ultima) as usize;
                self.foco = Foco::Fila(self.señalada);
                self.fuerza
                    .ir_a(if tema::efectos_reducidos() { 0.0 } else { 1.0 });
            }
            Mensaje::Puntero(puntero) if self.en_lista() => {
                let (filas, k, arriba) = self.lista();
                let alcance = lente::ALTO * k;
                let dentro = puntero.filter(|p| {
                    (p.x - self.pantalla.width / 2.0).abs() <= vista::ANCHO_LISTA * k
                        && p.y >= arriba - alcance
                        && p.y <= arriba + lente::alto_en_reposo(filas, k) + alcance
                });
                match dentro {
                    Some(p) => {
                        self.foco = Foco::Puntero(p.y - arriba);
                        if let Some(i) = lente::fila_en(filas, p.y - arriba, k) {
                            self.señalada = i;
                        }
                        self.fuerza
                            .ir_a(if tema::efectos_reducidos() { 0.0 } else { 1.0 });
                    }
                    // Con el foco puesto por las flechas, un ratón que pasa
                    // lejos no tiene por qué apagar la lente.
                    None if matches!(self.foco, Foco::Fila(_)) => {}
                    None => {
                        self.fuerza.ir_a(0.0);
                    }
                }
            }
            Mensaje::Pantalla(tamano) => {
                self.pantalla = tamano;
                if !self.escala_fijada {
                    // cage no escala la salida: sin esto, en un panel de
                    // 2880×1800 todo sale a la mitad de su tamaño. Manda la
                    // escala con la que corre BookOS, que el compositor publica
                    // aunque esté en automático; si no hay ninguna —nadie ha
                    // entrado todavía—, 900 lógicos de alto, que en este panel
                    // da el 2,0 que el compositor deduce por DPI. Con 1050 salía
                    // 1,75 y la pantalla de acceso, más pequeña que el escritorio.
                    // Se recalcula en cada cambio: el primer tamaño puede ser
                    // el 1280×800 pedido antes de que cage ocupe la pantalla,
                    // y fijarla entonces dejaba la escala en 1 en un panel 2,8K.
                    self.escala = self
                        .personas
                        .first()
                        .and_then(|p| p.apariencia.escala)
                        .map_or(((tamano.height / 900.0) * 4.0).round() / 4.0, |e| e as f32)
                        .clamp(1.0, 3.0);
                }
                return self.pedir_fondo();
            }
            Mensaje::Densidad(densidad) => {
                self.densidad = densidad;
                return self.pedir_fondo();
            }
            Mensaje::Fondo(de, px, Some(fondo)) if self.fondo_pedido == Some((de, px)) => {
                self.fondo = Some(fondo);
            }
            Mensaje::AbrirPanel(panel) => {
                return self.abrir_o_cerrar_panel(panel);
            }
            Mensaje::ElegirTeclado(codigo) => {
                self.fijar_teclado(codigo.clone());
                let locale = self.idioma.locale();
                let aplicar_sistema = self.cliente.is_some();
                return Task::perform(
                    async move {
                        let _ = sistema::guardar_preferencias_greeter(&codigo, locale);
                        if aplicar_sistema {
                            sistema::aplicar_teclado_sistema(&codigo);
                        }
                    },
                    |_| Mensaje::Fotograma,
                );
            }
            Mensaje::SiguienteTeclado => {
                let actual = sistema::DISTRIBUCIONES
                    .iter()
                    .position(|(c, _, _)| *c == self.teclado_codigo)
                    .unwrap_or(0);
                let sig = (actual + 1) % sistema::DISTRIBUCIONES.len();
                let codigo = sistema::DISTRIBUCIONES[sig].0.to_string();
                return self.actualizar(Mensaje::ElegirTeclado(codigo));
            }
            Mensaje::ElegirIdioma(idioma) => {
                self.idioma = idioma;
                self.fecha = idioma.fecha_hoy();
                let codigo_teclado = self.teclado_codigo.clone();
                let locale = idioma.locale();
                let aplicar_sistema = self.cliente.is_some();
                return Task::perform(
                    async move {
                        let _ = sistema::guardar_preferencias_greeter(&codigo_teclado, locale);
                        if aplicar_sistema {
                            sistema::aplicar_idioma_sistema(locale);
                        }
                    },
                    |_| Mensaje::Fotograma,
                );
            }
            Mensaje::BateriaActualizada(detalle) => {
                if let Some(bat) = &detalle.dato {
                    self.bateria = Some(bookos_shell::bateria_sobre_fondo(bat));
                }
                self.bateria_detalle = detalle;
            }
            Mensaje::PerfilEnergia(perfil) => {
                self.bateria_detalle.perfil = perfil.clone();
                if self.cliente.is_none() {
                    return Task::none();
                }
                return Task::perform(
                    async move {
                        sistema::cambiar_perfil_energia(&perfil);
                        sistema::leer_bateria(true)
                    },
                    Mensaje::BateriaActualizada,
                );
            }
            Mensaje::WifiEstado(estado) => {
                self.wifi_cargando = false;
                if estado.cable {
                    self.red = Some(Network {
                        kind: Link::Cable,
                        up: true,
                    });
                } else {
                    self.red = Some(Network {
                        kind: Link::Wifi,
                        up: estado.encendida && estado.ssid_actual.is_some(),
                    });
                }
                self.wifi_estado = estado;
            }
            Mensaje::WifiAlternar(encender) => {
                self.wifi_estado.encendida = encender;
                self.wifi_conectar_a = None;
                self.wifi_clave.clear();
                self.wifi_error = None;
                if self.cliente.is_none() {
                    if !encender {
                        self.wifi_estado.ssid_actual = None;
                        for r in &mut self.wifi_estado.redes {
                            r.activa = false;
                        }
                    }
                    self.red = Some(Network {
                        kind: Link::Wifi,
                        up: encender && self.wifi_estado.ssid_actual.is_some(),
                    });
                    return Task::none();
                }
                self.wifi_cargando = true;
                return Task::perform(
                    async move {
                        match sistema::alternar_wifi(encender) {
                            Ok(estado) => estado,
                            Err(err) => EstadoWifi {
                                encendida: !encender,
                                error: Some(err),
                                ..sistema::leer_wifi(false)
                            },
                        }
                    },
                    Mensaje::WifiEstado,
                );
            }
            Mensaje::WifiRefrescar => {
                if self.cliente.is_none() {
                    return Task::none();
                }
                self.wifi_cargando = true;
                self.wifi_error = None;
                return Task::perform(
                    async { sistema::leer_wifi(true) },
                    Mensaje::WifiEstado,
                );
            }
            Mensaje::WifiElegir(i) => {
                let Some(red) = self.wifi_estado.redes.get(i).cloned() else {
                    return Task::none();
                };
                self.wifi_error = None;
                if red.activa {
                    return Task::none();
                }
                if red.segura && !red.guardada {
                    self.wifi_conectar_a = Some(red.ssid);
                    self.wifi_clave.clear();
                    return iced::widget::operation::focus("wifi_clave");
                }
                self.wifi_conectar_a = None;
                self.wifi_clave.clear();
                if self.cliente.is_none() {
                    for r in &mut self.wifi_estado.redes {
                        r.activa = r.ssid == red.ssid;
                    }
                    self.wifi_estado.ssid_actual = Some(red.ssid);
                    self.red = Some(Network {
                        kind: Link::Wifi,
                        up: true,
                    });
                    return Task::none();
                }
                self.wifi_cargando = true;
                let ssid = red.ssid;
                return Task::perform(
                    async move {
                        match sistema::conectar_wifi(&ssid, None) {
                            Ok(estado) => estado,
                            Err(err) => EstadoWifi {
                                error: Some(err),
                                ..sistema::leer_wifi(false)
                            },
                        }
                    },
                    Mensaje::WifiEstado,
                );
            }
            Mensaje::WifiClave(mut valor) => {
                self.aplicar_traduccion_teclado(&mut valor);
                self.wifi_clave = valor;
            }
            Mensaje::WifiCancelarClave => {
                self.wifi_conectar_a = None;
                self.wifi_clave.clear();
                self.wifi_error = None;
            }
            Mensaje::WifiConectar => {
                let Some(ssid) = self.wifi_conectar_a.clone() else {
                    return Task::none();
                };
                let clave = std::mem::take(&mut self.wifi_clave);
                self.wifi_conectar_a = None;
                self.wifi_error = None;
                if self.cliente.is_none() {
                    for r in &mut self.wifi_estado.redes {
                        r.activa = r.ssid == ssid;
                        if r.activa {
                            r.guardada = true;
                        }
                    }
                    self.wifi_estado.ssid_actual = Some(ssid);
                    self.red = Some(Network {
                        kind: Link::Wifi,
                        up: true,
                    });
                    return Task::none();
                }
                self.wifi_cargando = true;
                return Task::perform(
                    async move {
                        match sistema::conectar_wifi(&ssid, Some(&clave)) {
                            Ok(estado) => estado,
                            Err(err) => EstadoWifi {
                                error: Some(err),
                                ..sistema::leer_wifi(false)
                            },
                        }
                    },
                    Mensaje::WifiEstado,
                );
            }
            Mensaje::WifiDesconectar => {
                if self.cliente.is_none() {
                    for r in &mut self.wifi_estado.redes {
                        r.activa = false;
                    }
                    self.wifi_estado.ssid_actual = None;
                    self.red = Some(Network {
                        kind: Link::Wifi,
                        up: false,
                    });
                    return Task::none();
                }
                self.wifi_cargando = true;
                return Task::perform(
                    async {
                        sistema::desconectar_wifi().unwrap_or_else(|_| sistema::leer_wifi(false))
                    },
                    Mensaje::WifiEstado,
                );
            }
            Mensaje::Energia(peticion) => {
                self.panel_abierto = None;
                self.menu.ir_a(0.0);
                let orden = match peticion {
                    Peticion::Apagar => "poweroff",
                    Peticion::Reiniciar => "reboot",
                    Peticion::Suspender => "suspend",
                    Peticion::Hibernar => "hibernate",
                    Peticion::CerrarSesion => return Task::none(),
                };
                if self.cliente.is_none() {
                    self.error = Some(format!(
                        "{}: «systemctl {orden}».",
                        self.idioma
                            .t("Vista previa: no se ejecuta", "Preview mode: skipped")
                    ));
                    return Task::none();
                }
                // Se espera a systemctl para enseñar su error: con spawn() a secas
                // un rechazo de polkit acababa en el stderr de cage y el botón
                // parecía no hacer nada.
                return Task::perform(
                    async move {
                        match std::process::Command::new("systemctl")
                            .args(["--no-ask-password", orden])
                            .output()
                        {
                            Ok(salida) if salida.status.success() => None,
                            Ok(salida) => Some(format!(
                                "systemctl {orden}: {}",
                                String::from_utf8_lossy(&salida.stderr).trim()
                            )),
                            Err(error) => {
                                Some(format!("No se pudo ejecutar systemctl {orden}: {error}"))
                            }
                        }
                    },
                    Mensaje::EnergiaHecha,
                );
            }
            Mensaje::EnergiaHecha(fallo) => {
                self.error = fallo;
            }
            Mensaje::Tic => {
                self.hora = Shell::hora_bloqueo();
                self.fecha = self.idioma.fecha_hoy();
                self.leer_estado();
            }
            _ => {}
        }
        Task::none()
    }
}
