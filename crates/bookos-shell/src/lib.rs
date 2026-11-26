//! Shell de BookOS: los widgets del escritorio, dibujados **dentro** del
//! proceso del compositor.
//!
//! Por qué in-process, y no un `plasmashell` aparte:
//!
//! - No hay IPC ni un segundo proceso que arrancar. El panel y el dock pueden
//!   estar pintados en el mismo primer frame que el compositor, que es lo que
//!   hace que no haya pantalla de carga al iniciar sesión.
//! - No se cargan dos veces las librerías de dibujo ni se abren dos contextos.
//!
//! El precio es que un panic aquí se lleva la sesión por delante. Por eso este
//! crate **no puede hacer panic hacia fuera**: el compositor envuelve las
//! llamadas en `catch_unwind` y, si algo revienta, se queda sin shell pero con
//! las ventanas vivas.
//!
//! El rasterizado es `iced_tiny_skia`, en CPU, sobre buffers que el compositor
//! sube como texturas. Un panel repinta una franja diminuta y muy de vez en
//! cuando (el reloj, una vez por minuto), así que no compensa abrir un segundo
//! backend GPU con `wgpu` solo para eso.

use iced_core::mouse::Cursor;
use iced_core::renderer::Style;
use iced_core::{Color, Font, Pixels, Rectangle, Size, Theme};
use iced_graphics::Viewport;
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::Renderer;

pub mod accesible;
pub mod actividad;
mod apps;
pub mod bloqueo;
pub mod carga_sesion;
pub mod captura;
mod config;
pub mod confirmacion;
/// El conmutador de Alt+Tab. Público porque el compositor le da la lista de
/// aplicaciones por orden de uso: el shell no ve el foco.
pub mod conmutador;
/// La barra de título que el escritorio dibuja por las ventanas que la aceptan.
/// Pública porque el compositor decide quién la lleva y qué hace cada botón.
pub mod decoracion;
pub mod diagnostico;
mod dock;
mod emergente;
/// Los iconos del escritorio. Público porque el compositor los coloca sobre el
/// fondo, por debajo de las ventanas, y le da los clics que no son de nadie.
pub mod escritorio;
pub mod fondos;
pub mod icono;
/// El logo, incrustado en el binario. Público porque el "Acerca de" del menú
/// lo pinta grande.
pub mod marca;
pub mod medios;
/// La cola de notificaciones. Pública porque quien las recibe es el compositor:
/// el shell no habla D-Bus.
pub mod notificaciones;
pub mod osd;
mod recorte;
mod state;
/// El teclado en pantalla.
pub mod teclado;
/// Los tokens del sistema de diseño. Público porque el compositor anima con
/// las mismas curvas y duraciones que el shell: dos tablas de movimiento en el
/// mismo escritorio se separan a la primera.
pub mod tema;
/// El aviso de una notificación recién llegada.
pub mod toast;
mod view;
mod widget;
mod widgets;

/// Qué enseña el widget de batería del panel. Lo llama el compositor al
/// recargar `panel.conf`.
// añadido en la 0.2.2
pub fn bateria_mostrar(porcentaje: bool, tiempo: bool, colores: bool) {
    widgets::bateria_mostrar(porcentaje, tiempo, colores);
}

/// El perfil de energía cambió: lo avisa el compositor al notificarlo sysfs.
// añadido en la 0.2.2
pub fn bateria_perfil_cambiado() {
    widgets::bateria_perfil_cambiado();
}

/// Qué hacer con una tecla que llega a una capa del shell. Lo necesita el
/// compositor para la capa de captura, que gobierna él.
pub use emergente::Tecla;

/// Las dos superficies de la capa de captura. Van separadas porque cambian a
/// ritmos muy distintos: la pastilla con cada píxel del arrastre y la barra
/// solo al señalar o pulsar una celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiezaCaptura {
    Barra,
    Pastilla,
    Opciones,
}
pub use emergente::RADIO_MINIATURA_ESCRITORIO;
/// Un cambio de vista del launchpad que anima el compositor. Público porque es
/// él quien tiene las dos superficies que hay que cruzar.
pub use emergente::Transicion as TransicionLaunchpad;
/// Cuánto hay que mover el ratón para que un clic pase a ser un arrastre. Lo
/// comparten el launchpad y el dock: el gesto es el mismo y con dos umbrales
/// distintos sacar un icono del dock respondería antes o después que moverlo
/// por la rejilla.
pub use emergente::launchpad::UMBRAL_ARRASTRE;
pub use emergente::mantener_despierto;
pub use emergente::recortar_texto;
pub use emergente::t;
mod menus;
pub use apps::{LanzadorMime, poner_al_iniciar, visor_imagenes};
pub use emergente::{
    Ancla, ContenidoPortapapeles, ElementoPortapapeles, Emergente, EntradaMenu, FilaMenu,
    IdentidadPolkit, MarcaMenu, MiniaturaPortapapeles, ModoProyeccion, OpcionAcceso,
    PantallaCompartible, PeticionAcceso, PeticionElegirApp, PeticionPolkit, RapidaMenu,
    miniatura_de_imagen,
};
/// El icono ya cargado de una aplicación. Sale al exterior porque el conmutador
/// lo arma el compositor, que conoce las ventanas pero no el tema de iconos.
pub use icono::Icono;

/// Busca el icono de una aplicación por su nombre, en el tema del sistema.
// 0.1.0
pub fn icono_de_app(nombre: &str) -> Option<Icono> {
    icono::cargar(nombre)
}

/// El nombre de la aplicación de ese `app_id`, según su `.desktop`.
// 0.1.1
pub fn nombre_de_app(app_id: &str) -> Option<String> {
    apps::por_app_id(app_id).map(|a| a.nombre)
}

/// Una celda del conmutador, a partir de la ventana que representa.
///
/// El `app_id` casi nunca sirve como nombre ni como icono —`org.kde.konsole` no
/// es «Konsole» ni `utilities-terminal`—, así que se resuelve su `.desktop`,
/// que es lo mismo que hace el dock.
///
/// El rótulo es el **título** de la ventana, que es lo único que distingue dos
/// ventanas del mismo programa. Cuando viene vacío —hay clientes que tardan en
/// ponerlo— se cae al nombre de la aplicación: una celda sin renglón descoloca
/// la tarjeta entera.
// 0.1.0
pub fn entrada_de_ventana(app_id: &str, titulo: &str) -> conmutador::Entrada {
    let app = apps::por_app_id(app_id);
    let nombre = match (titulo.trim(), &app) {
        ("", Some(app)) => app.nombre.clone(),
        ("", None) => app_id.to_string(),
        (titulo, _) => titulo.to_string(),
    };
    conmutador::Entrada {
        nombre,
        app: app.as_ref().map_or_else(|| app_id.to_string(), |a| a.nombre.clone()),
        // Sin `.desktop` se prueba con el propio `app_id`: hay programas cuyo
        // icono se llama igual que ellos, y un hueco es peor que intentarlo.
        icono: icono::cargar(app.as_ref().map_or(app_id, |a| a.icono.as_str())),
    }
}

pub use config::{
    Actividades as ConfigActividades, AperturaVentanas, Bloqueo as ConfigBloqueo,
    BrilloAutomatico, Config, Efectos, EncajeFondo, Entrada, EstiloSalvapantallas, FuenteReloj,
    LuzNocturna, MAXIMO_ESCRITORIOS, ModoLuzNocturna, Pasos, RETARDOS_CAPTURA, TamanoPanel,
    apps_encima, guardar_apariencia, guardar_brillo_automatico, guardar_captura,
    guardar_configuracion, guardar_dock, guardar_efectos, guardar_escritorios, guardar_esquiva,
    guardar_fondo, guardar_iconos_escritorio, guardar_luz_nocturna,
    guardar_notificaciones_silenciadas, guardar_tamano_iconos, luz_nocturna_encendida,
    marcar_app_encima, marcar_luz_nocturna, olvidar_fondo, ruta as ruta_config, sumar_uso,
};
pub use dock::{
    Dock, DockItem, ICON as DOCK_ICON, MARGIN as DOCK_MARGIN, PAD as DOCK_PAD,
    icon_path as icon_debug,
};
/// Donde cada sesión deja la apariencia que la pantalla de acceso puede leer
/// antes de que nadie haya entrado: `<login>/panel.conf`, los fondos y el
/// avatar. No vale `~/.config`: el greeter es otro usuario y no llega.
pub const APARIENCIA_PUBLICA: &str = "/var/lib/bookos/apariencia";
pub use state::PanelData;
/// La hora local de ahora, en `(hora, minuto)`. La necesita el compositor para
/// el temporizador del tema automático.
pub use state::hora_local_ahora;
pub use state::{Battery, Fecha, Link, Network};
pub use widgets::Privacidad;
pub use widget::Widget;
pub use widgets::NOMBRES as WIDGETS_DISPONIBLES;
pub use widgets::bateria_sobre_fondo;

/// Alto del panel en píxeles lógicos.
pub const PANEL_HEIGHT: u32 = 32;

/// Lo que el shell le pide al compositor tras una interacción.
///
/// El shell no lanza el programa él mismo aunque podría: el entorno correcto
/// —`WAYLAND_DISPLAY` de esta sesión, `DISPLAY` fuera— lo sabe el compositor, y
/// duplicar eso aquí es la forma segura de que una de las dos copias se quede
/// atrás.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    EnergiaConfirmada(confirmacion::Energia),
    CerrarEmergente,
    InsertarEmoji(String),
    PortapapelesPegar(u64),
    PortapapelesFijar(u64),
    PortapapelesEliminar(u64),
    PortapapelesVaciar,
    /// Texto al portapapeles: la ficha de «Acerca de este equipo».
    // añadido en la 0.2.2
    CopiarTexto(String),
    VentanaEncima,
    VentanaUnir(bool),
    VentanaDisposicion(bool),
    VentanaSeparar,
    VentanaEscritorio(usize),
    VentanaMonitor(String),
    Lanzar(String),
    /// Traer al frente lo que ya está abierto, en vez de abrirlo otra vez.
    ///
    /// Lleva el `exec` además del `app_id` porque el emparejamiento por `app_id`
    /// puede fallar —un cliente que no lo declara, o que usa otro distinto del
    /// que dice su `.desktop`— y en ese caso lanzar es mejor que no hacer nada:
    /// desde fuera, un icono que no responde parece el dock roto.
    Activar {
        app_id: String,
        exec: String,
    },
    /// Lo mismo, pero sin alternar: si ya está delante se queda. Es lo que
    /// pide el buscador, donde elegir una aplicación abierta es querer verla,
    /// no esconderla como al pulsar su icono del dock por segunda vez.
    Mostrar {
        app_id: String,
        exec: String,
    },
    /// Abrir o cerrar el launchpad. No es un programa que lanzar: vive dentro
    /// del shell, así que el dock no puede pedirlo con un `exec`.
    Launchpad,
    /// Fijar la aplicación al dock o soltarla. La escribe el compositor en la
    /// configuración: el shell no toca el disco.
    Anclar {
        app_id: String,
        exec: String,
        icono: String,
        /// `None` alterna, que es lo que quiere el menú del clic derecho: una
        /// sola entrada que dice «Fijar» o «Soltar» según esté.
        ///
        /// Arrastrando **no** vale alternar: soltar sobre el dock algo que ya
        /// estaba anclado lo desanclaría, que es lo contrario de lo que acaba
        /// de hacer la mano. Por eso el arrastre manda `Some(true)` o
        /// `Some(false)` y dice lo que quiere.
        fijar: Option<bool>,
    },
    /// El orden ya se ve durante el arrastre; se persiste una vez al soltar.
    GuardarDock(Vec<String>),
    /// Abrir «Acerca de este PC», que es una superficie del propio shell.
    Acerca,
    /// Abrir la hoja de atajos de teclado.
    Atajos,
    /// Poner el brillo de la pantalla en este tanto por ciento.
    Brillo(u8),
    /// Poner el volumen de salida en este tanto por ciento.
    Volumen(u8),
    /// Cambiar el tema y el acento. Los dos juntos y no una acción por cada uno
    /// porque la tarjeta de Apariencia manda **su estado entero**: así el
    /// compositor escribe las dos claves de una vez y no puede quedarse con un
    /// fichero a medio cambiar si algo falla entre medias.
    Apariencia {
        /// Lo **elegido**, que puede ser «automático». El tema concreto que
        /// toca lo resuelve el compositor con el reloj: la tarjeta no tiene por
        /// qué saber la hora.
        modo: tema::ModoTema,
        acento: tema::Acento,
    },
    /// Abrir otra tarjeta del shell por su nombre de widget. Es lo que hace el
    /// centro de control al pulsar «Wi-Fi»: la lista de redes ya existe como
    /// emergente y no tiene sentido dibujarla dos veces.
    Emergente(&'static str),
    /// Capturar ese rectángulo de la pantalla, en lógicos. Lo pide la capa de
    /// captura al soltar el arrastre o al pulsar «Pantalla».
    Capturar {
        x: i32,
        y: i32,
        ancho: i32,
        alto: i32,
        /// A un fichero. Con `false` va al portapapeles, que es lo normal: la
        /// mayoría de las capturas se pegan en un chat y no se vuelven a mirar.
        guardar: bool,
        /// Si el cursor sale en la foto.
        // añadido en la 0.2.0
        cursor: bool,
        /// Segundos de espera antes de hacerla; 0 es al instante.
        // añadido en la 0.2.0
        retardo: u8,
    },
    /// Recordar las opciones de la barra de captura (cursor y espera). La capa
    /// ya las tiene puestas; esto es para que el compositor las escriba.
    // añadido en la 0.2.0
    CapturaOpciones {
        cursor: bool,
        retardo: u8,
    },
    /// Cerrar la capa de captura desde el botón «×» de la barra.
    CapturaCerrar,
    /// Empezar una grabación de pantalla en ese rectángulo lógico.
    GrabarIniciar {
        x: i32,
        y: i32,
        ancho: i32,
        alto: i32,
        cursor: bool,
        retardo: u8,
        audio: captura::AudioGrabacion,
    },
    /// Detener la grabación activa desde el pill flotante.
    GrabarDetener,
    /// Pausar o reanudar la grabación activa desde el pill flotante.
    GrabarPausar(bool),
    /// Silenciar o reactivar el micrófono desde el pill flotante.
    GrabarSilenciar(bool),
    /// Poner una familia de fondos: la imagen clara y la oscura de golpe.
    ///
    /// Las **dos** y no la que toca ahora, que es lo que permite que el cambio
    /// de tema se lleve el fondo con él. Es lo mismo que hace Apariencia con el
    /// tema y el acento: la tarjeta manda su estado entero.
    Fondo {
        claro: std::path::PathBuf,
        oscuro: std::path::PathBuf,
    },
    /// Elegir cómo se reparten dos pantallas desde el selector de Fn+F4.
    Proyeccion(ModoProyeccion),
    /// Contestar al permiso de compartir pantalla del portal de escritorio.
    ///
    /// `pantalla` es el índice dentro de la lista que se le pasó a la tarjeta,
    /// y `None` es la negativa. Lleva la sesión porque puede haber más de una
    /// petición en vuelo y la respuesta tiene que llegar a la suya.
    Compartir {
        sesion: u32,
        pantalla: Option<usize>,
    },
    /// Retirar esa notificación. La lista vive en el shell, pero avisar a la
    /// aplicación de que se cerró es D-Bus, y eso lo hace el compositor.
    CerrarNotificacion(u32),
    /// Retirar varias a la vez: la ✕ de una pila o de un grupo de la misma app.
    // añadido en la 0.2.1
    CerrarNotificaciones(Vec<u32>),
    /// Activar un botón ofrecido por una notificación.
    NotificacionAccion {
        id: u32,
        clave: String,
    },
    /// La respuesta escrita en la caja de «Responder» de una notificación. El
    /// compositor la entrega a la aplicación por `NotificationReplied`.
    NotificacionRespuesta {
        id: u32,
        texto: String,
    },
    /// Vaciar la cola entera.
    BorrarNotificaciones,
    /// Silenciar o reactivar los avisos no críticos de una aplicación concreta.
    // añadido en la 0.2.4
    SilenciarAppNotificaciones {
        app: String,
        silenciar: bool,
    },
    /// Echar la pantalla de bloqueo. La del propio compositor, no la de KDE.
    Bloquear,
    /// Terminar la sesión, o sea el compositor. Lo hace él: es su bucle.
    CerrarSesion,
    /// Cerrar las ventanas de esa aplicación. Quien las conoce es el
    /// compositor, que tiene el `Space`.
    Cerrar {
        app_id: String,
    },
    /// Ir a ese escritorio. La pide el indicador del panel al pulsar un punto;
    /// quien los gobierna es el compositor.
    Escritorio(usize),
    /// Abrir la vista general de escritorios desde el indicador del panel.
    VistaEscritorios,
    CrearEscritorio,
    EliminarEscritorio(usize),
    RenombrarEscritorio {
        indice: usize,
        nombre: String,
    },
    /// Acción de una actividad viva; el compositor la devuelve a la aplicación
    /// propietaria por la señal privada de BookOS.
    Actividad(actividad::Accion),
    /// Traer al frente la app que publica una actividad, o lanzarla si no
    /// tiene ventana. Aparte de `Mostrar` porque el `app_id` de la actividad
    /// (`com.bookos.player`) no es el de su ventana (`bookos-player`): se
    /// comparan con [`es_de_la_app`].
    // añadido en la 0.2.2
    AbrirAppDeActividad { app_id: String, exec: String },
    /// Poner o quitar los efectos reducidos. El compositor la escribe en la
    /// configuración, como el resto de lo que se elige desde el shell.
    AlternarEfectos,
    /// Qué luces siguen al sensor, con el estado entero como `Apariencia`.
    BrilloAutomatico(config::BrilloAutomatico),
    /// Encender (`true`) o apagar la luz nocturna. Con horario vale hasta el
    /// próximo borde; ver `bookos-comp/src/luz_nocturna.rs`.
    LuzNocturna(bool),
    /// Poner (`true`) o quitar «No molestar» sin duración, desde la píldora del
    /// centro de control. Lo guarda el shell (`poner_no_molestar`), pero el
    /// compositor es quien lo sabe aplicar a la isla y a los avisos.
    // añadido en la 0.2.0
    NoMolestar(bool),
    /// Reprogramar las alarmas de inactividad al cambiar el inhibidor del shell.
    MantenerDespierto,
    PerfilEnergia(&'static str),
    AbrirCaptura,
    /// Crear una carpeta o un archivo en el escritorio, del submenú «Crear» del
    /// menú de clic derecho.
    EscritorioNuevo(escritorio::NuevoElemento),
    /// Mandar lo seleccionado del escritorio a la papelera, del mismo menú.
    EscritorioEliminarSeleccion,
    /// Enseñar (`true`) u ocultar los iconos del escritorio.
    EscritorioIconos(bool),
    /// Color de las carpetas seleccionadas: `#RRGGBB`, o `None` para volver
    /// al automático.
    EscritorioColor(Option<String>),
    /// Abrir el diálogo de color de las carpetas seleccionadas.
    EscritorioElegirColor,
    // añadido en la 0.2.3 -- lo del menú contextual nuevo del escritorio.
    /// Copiar lo seleccionado como archivos al portapapeles del sistema
    /// (`text/uri-list`), para pegarlo en el escritorio o en un explorador.
    EscritorioCopiar,
    /// Pegar los archivos del portapapeles. El compositor lee la selección y,
    /// si son copias, pregunta si se quiere un acceso directo.
    EscritorioPegar,
    /// La respuesta del diálogo de pegar: `true` crea accesos directos.
    EscritorioPegarComo(bool),
    /// La ruta de lo seleccionado, como texto.
    EscritorioCopiarRuta,
    /// Un `.zip` con lo seleccionado, al lado.
    EscritorioComprimir,
    /// Una terminal en la carpeta seleccionada, o en la del escritorio.
    EscritorioTerminal,
    /// Mover lo seleccionado a una carpeta nueva y dejarla lista para nombrar.
    EscritorioAgrupar,
    /// Abrir el diálogo de renombrar, y el nombre que se elige en él.
    EscritorioRenombrar,
    EscritorioRenombrarA(String),
    /// La ficha de información de lo seleccionado.
    EscritorioInformacion,
    /// Poner o quitar el permiso de ejecución del script seleccionado.
    EscritorioEjecutable(bool),
    /// Abrir lo seleccionado con esa orden (`Exec` de un `.desktop`).
    EscritorioAbrirCon(String),
    /// Abrir la carpeta que contiene el original de un acceso directo.
    EscritorioMostrarOriginal,
    /// Recolocar los iconos ordenados, o solo encajarlos en la rejilla.
    EscritorioOrdenar(escritorio::Orden),
    EscritorioAlinear,
    /// Tamaño de los iconos del escritorio.
    EscritorioTamano(escritorio::Tamano),
    /// Vista rápida del archivo seleccionado (Espacio).
    EscritorioVistaRapida,
    // añadido en la 0.2.3 -- lo del menú contextual nuevo del dock.
    /// Traer al frente la ventana `n` (en el orden del compositor) de una app.
    VentanaDeApp { app_id: String, n: usize },
    /// Que la app se abra al iniciar sesión (`~/.config/autostart`).
    AbrirAlIniciar { app_id: String, abrir: bool },
    /// Envía la contraseña o respuesta escrita en el diálogo de Polkit.
    PolkitEnviar { id: u64 },
    /// Cancela la petición de Polkit en curso.
    PolkitCancelar { id: u64 },
    /// Cambia la cuenta activa en el diálogo de Polkit.
    PolkitCambiarIdentidad { id: u64, indice: usize },
    /// Respuesta del diálogo de permisos `org.freedesktop.impl.portal.Access`.
    PortalAcceso {
        id: u32,
        permitido: bool,
        opciones: Vec<(String, String)>,
    },
    /// Respuesta del diálogo «Abrir con…» (`org.freedesktop.impl.portal.AppChooser`).
    PortalElegirApp { id: u32, eleccion: Option<String> },
}

impl Accion {
    /// Las operaciones de administración actualizan la vista en el sitio; elegir
    /// un escritorio o lanzar algo sí completa la interacción y la cierra.
    // 0.1.0
    // modified 0.1.1 -- guardar el dock no cierra la emergente
    fn conserva_emergente(&self) -> bool {
        matches!(
            self,
            Self::CrearEscritorio
                | Self::EliminarEscritorio(_)
                | Self::RenombrarEscritorio { .. }
                // Cerrar una notificación no cierra la tarjeta: se leen varias
                // seguidas, y que desapareciera al despachar la primera
                // obligaría a volver a abrirla cada vez.
                | Self::CerrarNotificacion(_)
                | Self::CerrarNotificaciones(_)
                | Self::NotificacionAccion { .. }
                | Self::BorrarNotificaciones
                | Self::SilenciarAppNotificaciones { .. }
                // Elegir un color es probar: la tarjeta se queda abierta para
                // ver el resultado y poder cambiar de idea.
                | Self::Apariencia { .. }
                // El botón «A» se pulsa para ver cómo queda: cerrar la tarjeta
                // escondería justo el nivel que acaba de cambiar.
                | Self::BrilloAutomatico(_)
                // Lo mismo: se enciende para ver cómo queda.
                | Self::LuzNocturna(_)
                // Copiar la ficha de «Acerca de» deja la ventana a la vista: la
                // fila dice «Ficha copiada» y se sigue leyendo.
                | Self::CopiarTexto(_)
                // añadido en la 0.2.3 -- el interruptor de la ficha de
                // información se mira y se sigue leyendo la ficha; renombrar
                // cierra él mismo si sale bien, y si no se queda con el error.
                | Self::EscritorioEjecutable(_)
                | Self::EscritorioRenombrarA(_)
                | Self::MantenerDespierto
                // Un conmutador del centro: se mira el estado y se sigue.
                | Self::NoMolestar(_)
                | Self::PerfilEnergia(_)
                | Self::Actividad(_)
                // Anclar arrastrando **no** cierra el launchpad: la mano está
                // ahí para poner varias, y cerrarse tras la primera obligaría a
                // reabrirlo por cada icono.
                | Self::Anclar { .. }
                | Self::GuardarDock(_)
                // El diálogo de Polkit se queda abierto mientras PAM autentica,
                // enseña un error o cambia de usuario; solo se cierra al cancelar
                // o cuando el compositor recibe `SUCCESS`.
                | Self::PolkitEnviar { .. }
                | Self::PolkitCambiarIdentidad { .. }
                | Self::PortapapelesFijar(_)
                | Self::PortapapelesEliminar(_)
                | Self::PortapapelesVaciar
        )
    }

    /// [`Self::conserva_emergente`] en la emergente `e`. El buscador se cierra
    /// siempre: lo que pides en él es lo último que haces, y que se quedara
    /// abierto tras «Encender luz nocturna» o «Nuevo escritorio» parecía que
    /// no había pasado nada.
    // añadido en la 0.2.0
    fn conserva_en(&self, e: &Emergente) -> bool {
        match e {
            // Con la confirmación en marcha se queda y se cierra sola al vencer.
            Emergente::Buscador(b) => b.confirmando(),
            _ => self.conserva_emergente(),
        }
    }
}

/// Qué emergente abre cada widget del panel al pulsarlo.
///
/// `None` para los que todavía no tienen una: se pulsan y no pasa nada, que es
/// lo que hace hoy la red.
// 0.1.0
fn emergente_de(nombre: &str) -> Option<fn() -> Emergente> {
    match nombre {
        "reloj" => Some(Emergente::calendario),
        "volumen" => Some(Emergente::sonido),
        "brillo" => Some(Emergente::brillo),
        "bateria" => Some(Emergente::energia),
        "red" => Some(Emergente::red),
        "bluetooth" => Some(Emergente::bluetooth),
        "usuarios" => Some(Emergente::usuarios),
        "notificaciones" => Some(Emergente::notificaciones),
        "control" => Some(Emergente::centro),
        // No es un widget del panel: es la tarjeta que abre el menú de BookOS.
        // Entra por aquí porque `Accion::Emergente` ya es el camino de «abre
        // esa otra tarjeta del shell» y no hacía falta un segundo.
        "apariencia" => Some(Emergente::apariencia),
        "apagar" => Some(Emergente::apagar),
        // El logo del panel lo abre por su zona fija; esto es para el atajo.
        "menu" => Some(Emergente::menu),
        _ => None,
    }
}

/// Nombre interno de la tarjeta que corresponde a una zona del panel.
///
/// El widget y su emergente no siempre se llaman igual (`bateria`/`energia`,
/// `reloj`/`calendario`). El compositor necesita ambos nombres: el primero para
/// construirla después de una transición y el segundo para saber si el segundo
/// clic debe cerrarla.
// 0.1.0
fn nombre_emergente_de(nombre: &str) -> Option<&'static str> {
    match nombre {
        "reloj" => Some("calendario"),
        "volumen" => Some("sonido"),
        "brillo" => Some("brillo"),
        "bateria" => Some("energia"),
        "red" => Some("red"),
        "bluetooth" => Some("bluetooth"),
        "usuarios" => Some("usuarios"),
        "notificaciones" => Some("notificaciones"),
        "control" => Some("centro"),
        "apariencia" => Some("apariencia"),
        "apagar" => Some("apagar"),
        _ => None,
    }
}

/// El brillo de la pantalla ahora mismo, en tanto por ciento.
// 0.1.0
pub fn brillo_actual() -> Option<u8> {
    state::read_brightness()
}

/// El brillo actual sin redondear a enteros, para arrancar rampas suaves
/// desde el valor real del hardware sin salto inicial.
// añadido en la 0.2.3
pub fn brillo_exacto() -> Option<f64> {
    state::read_brightness_exact()
}

/// Precalienta en segundo plano catálogos `.desktop`, iconos e historial de
/// usos para que la primera apertura de Centro de control y Buscador entre en
/// un solo fotograma. Seguro desde un hilo secundario (solo toca cachés
/// protegidas por `OnceLock`/`Mutex`).
pub fn precalentar() {
    let apps = apps::leer();
    for nombre in [
        "wifi",
        "bluetooth",
        "noche",
        "no-molestar",
        "micro-silencio",
        "captura",
        "mantener",
        "brillo",
        "volumen-alto",
        "volumen-medio",
        "volumen-bajo",
        "volumen-silencio",
        "buscar",
        "terminal",
        "pantalla",
        "control",
        "apariencia",
        "bateria-100",
        "usuarios",
    ] {
        let _ = icono::propio(nombre);
    }
    for nombre in [
        "wifi",
        "bluetooth",
        "pantalla",
        "fondo",
        "control",
        "captura",
        "bloquear",
        "apagar",
        "monitor",
    ] {
        let _ = icono::insignia(nombre);
    }
    for app in apps.iter().take(12) {
        if !app.icono.is_empty() {
            let _ = icono::cargar(&app.icono);
        }
    }
    let _ = config::cargar_usos();
}

/// Si está enchufado y el porcentaje, para avisar al conectar el cargador.
/// `None` en un equipo sin batería.
// 0.1.0
pub fn cargador() -> Option<(bool, u8)> {
    state::Battery::read(false).map(|b| (b.plugged, b.percent))
}

/// El pictograma de la batería del panel, para el aviso del cargador.
// 0.1.0
pub fn icono_cargador() -> Option<icono::Icono> {
    state::Battery::read(false).map(|b| widgets::icono_aviso_bateria(&b))
}

pub mod retroiluminacion;

/// Nombre, nivel actual y máximo de la luz de teclado detectada.
// 0.1.0
pub fn brillo_teclado_actual() -> Option<(String, u32, u32)> {
    state::teclado_actual()
}

/// El dispositivo de retroiluminación y su valor crudo máximo.
// 0.1.0
pub fn backlight() -> Option<(String, u32)> {
    let d = state::backlight_device()?;
    let max = state::backlight_max(&d)?;
    Some((d, max))
}

/// Decodifica una imagen a RGBA sin premultiplicar.
///
/// Vive aquí y no en el compositor porque `image` ya es dependencia de este
/// crate —la usa el dock para los iconos que no son SVG— y no hacía falta
/// repetirla al otro lado.
// 0.1.0
pub fn decodificar_rgba(ruta: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::ImageReader::open(ruta).ok()?.decode().ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

/// Variante para portadas que llegan por IPC como bytes PNG/JPEG.
// 0.1.0
pub fn decodificar_imagen(datos: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(datos).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

/// Cómo guarda el alfa un búfer RGBA.
#[derive(Clone, Copy, Debug)]
pub enum Alfa {
    /// El de iced y `image`: el color va entero y el alfa aparte.
    Recto,
    /// El que mezcla el compositor (`ONE, ONE_MINUS_SRC_ALPHA`): el color ya
    /// va multiplicado por el alfa. Tocar solo el alfa aquí deja un halo claro
    /// en el borde.
    Premultiplicado,
}

/// Cuánto de un píxel cae dentro de un rectángulo redondeado de radio `r`: 1
/// dentro, 0 fuera, y el paso de uno a otro en un píxel para que la curva no
/// salga en escalera.
// 0.1.0
fn cobertura(x: usize, y: usize, ancho: usize, alto: usize, r: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = px.clamp(r, ancho as f32 - r);
    let cy = py.clamp(r, alto as f32 - r);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}

/// Recorta las esquinas de una imagen a un radio, sobre los píxeles.
///
/// Hace falta porque el renderizador de CPU de iced **ignora**
/// `Image::border_radius` —`iced_tiny_skia::Engine::draw_image` no lo lee— y
/// `clip` recorta en rectángulo: una imagen dentro de un marco redondeado
/// asoma por las cuatro esquinas. En el compositor pasa lo mismo con un
/// `MemoryRenderBuffer`.
// 0.1.0
pub fn redondear_esquinas(rgba: &mut [u8], ancho: u32, alto: u32, radio: f32, alfa: Alfa) {
    let (w, h) = (ancho as usize, alto as usize);
    let r = radio.clamp(0.0, w.min(h) as f32 / 2.0);
    for (i, px) in rgba
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .take(w * h)
        .enumerate()
    {
        let c = cobertura(i % w, i / w, w, h, r);
        if c >= 1.0 {
            continue;
        }
        let canales = match alfa {
            Alfa::Recto => &mut px[3..],
            Alfa::Premultiplicado => &mut px[..],
        };
        canales
            .iter_mut()
            .for_each(|v| *v = (*v as f32 * c).round() as u8);
    }
}

/// ¿El `app_id` de un lanzador y el de una ventana son del mismo programa?
///
/// La comparación no es exacta porque casi nunca coinciden: el lanzador dice
/// `konsole` y la ventana se presenta como `org.kde.konsole`. Se acepta el
/// sufijo tras un punto, que cubre la convención de nombre inverso de dominio
/// sin casar cosas que solo se parecen — `konsole-profile` no cuenta. Un
/// lanzador que necesite otra cosa puede decir su `app_id` en la configuración.
///
/// Es pública porque el compositor tiene que emparejar exactamente igual que el
/// dock al buscar qué ventana traer al frente: dos copias de esta regla se
/// desincronizan y el icono deja de responder solo para algunos programas.
// 0.1.0
pub fn mismo_programa(lanzador: &str, ventana: &str) -> bool {
    ventana.eq_ignore_ascii_case(lanzador)
        || ventana
            .rsplit('.')
            .next()
            .is_some_and(|ultimo| ultimo.eq_ignore_ascii_case(lanzador))
}

/// ¿La actividad publicada con ese `app_id` es de la app que tiene esa ventana?
///
/// Las apps publican con el identificador de Tauri (`com.bookos.calendar`) y su
/// ventana sale con el nombre del binario (`bookos-calendar`), o con el mismo
/// identificador según cómo la abra GTK. Se comparan por lo que queda al quitar
/// `com.bookos.` / `bookos-` y los guiones, que también junta
/// `systemmonitor` con `bookos-system-monitor`.
/// Zenith no es una actividad que se pueda apagar como el reproductor: la isla
/// es su única forma de verse, y con las actividades apagadas Meta+Alt+A
/// escuchaba sin enseñar nada.
// añadido en la 0.2.0
fn es_asistente(estado: &actividad::Estado) -> bool {
    matches!(estado.clase, actividad::Clase::Asistente(_))
}

// 0.1.0
pub fn es_de_la_app(actividad: &str, ventana: &str) -> bool {
    let nombre = |id: &str| {
        id.strip_prefix("com.bookos.")
            .or_else(|| id.strip_prefix("bookos-"))
            .unwrap_or(id)
            .replace('-', "")
            .to_ascii_lowercase()
    };
    mismo_programa(actividad, ventana) || nombre(actividad) == nombre(ventana)
}

/// El binario de una app sin `.desktop` (el Monitor del sistema publica como
/// `com.bookos.systemmonitor` y se instala como `bookos-system-monitor` sin
/// fichero de lanzador). Se busca en el `PATH` con la misma comparación que la
/// ventana. Solo corre al pulsar «Abrir», no al pintar.
// añadido en la 0.2.2
fn programa_de_actividad(app_id: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    programa_en(std::env::split_paths(&path), app_id)
}

// añadido en la 0.2.2
fn programa_en(
    dirs: impl Iterator<Item = std::path::PathBuf>,
    app_id: &str,
) -> Option<String> {
    dirs.filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .find(|nombre| nombre.starts_with("bookos-") && es_de_la_app(app_id, nombre))
}

/// Las teclas que le importan al shell.
///
/// Es un enum propio y diminuto en vez del keysym de xkb porque el shell no
/// tiene por qué saber de xkb: el compositor traduce, y aquí solo llegan las
/// cuatro teclas que una superficie emergente necesita. `Caracter` es para el
/// campo de búsqueda del launchpad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeclaPulsada {
    Escape,
    Intro,
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
    /// Inicio y Fin: al principio y al final de lo que se esté viendo.
    Inicio,
    Fin,
    /// RePág y AvPág: una página entera, que en el launchpad es la unidad de
    /// verdad —las flechas se mueven de icono en icono—.
    PaginaArriba,
    PaginaAbajo,
    /// El tabulador. Lo usa el launchpad para el selector de color de una
    /// carpeta: es la única tecla libre ahí dentro, porque las letras buscan y
    /// las flechas recorren la rejilla.
    Tabulador,
    Retroceso,
    /// Supr (`Delete`): borra el carácter que sigue al cursor.
    Suprimir,
    /// Ctrl+Retroceso. El launchpad vacía con ella el campo entero; el
    /// buscador borra la última palabra.
    BorrarTodo,
    /// Ctrl+A. Solo la entiende el buscador; el resto la ignora.
    SeleccionarTodo,
    /// Ctrl+V. Pega texto plano en el buscador o respuesta.
    Pegar(String),
    Caracter(char),
}

/// Una zona repintada, en píxeles físicos.
///
/// Es un tipo propio y no el `Rectangle` de iced a propósito: así el
/// compositor no necesita conocer iced para hablar con el shell, y cambiar de
/// toolkit no obliga a tocar el compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Una superficie del shell: su caché de layout y su geometría.
///
/// El `Renderer` **no** vive aquí: es uno solo para todo el shell. Cada
/// `Renderer` de iced arrastra su propio motor de texto (cosmic-text) con su
/// FontSystem, así que tener uno por superficie cargaría las fuentes del
/// sistema dos veces dentro del proceso del compositor.
/// Una barra de título con su buffer y lo último que se pintó en él.
struct Barra {
    estado: decoracion::Estado,
    canvas: Canvas,
}

struct Canvas {
    cache: Option<Cache>,
    size: Size<f32>,
    scale: f32,
    painted_once: bool,
}

/// Ajusta un tamaño lógico para que en píxeles físicos salga **entero**.
///
/// El compositor le dice a Smithay el tamaño lógico de la superficie y Smithay
/// lo multiplica por la escala de la salida para saber dónde pintarla. Si el
/// resultado no coincide con los píxeles que de verdad tiene el buffer, la
/// textura se reescala y el filtrado bilineal emborrona **todo** el contenido,
/// por poco que sea el desajuste.
///
/// Medido a escala 1,75: una tarjeta de 335 lógicos da un buffer de 586 px y el
/// compositor pide 586,25 —un cuarto de píxel—, y con eso el texto y los iconos
/// salen borrosos. Con 336 lógicos el físico es 588 exacto y se ve nítido. El
/// panel no lo sufría porque su ancho es el de la pantalla, que siempre encaja.
///
/// Se sube como mucho unos pocos píxeles lógicos: para una escala de la forma
/// `n/d` basta llegar al siguiente múltiplo de `d`, y las que se usan de verdad
/// —1,25, 1,5, 1,75, 2— tienen denominadores de 4 o menos.
// 0.1.0
pub fn a_pixel_entero(logico: f32, escala: f32) -> f32 {
    if escala <= 0.0 {
        return logico;
    }
    let base = logico.ceil();
    for extra in 0..16 {
        let candidato = base + extra as f32;
        let fisico = candidato * escala;
        if (fisico - fisico.round()).abs() < 0.001 {
            return candidato;
        }
    }
    base
}

/// El alto del panel en lógicos: el del tamaño elegido, ajustado al alto de
/// la pantalla y cuadrado a píxel entero.
///
/// La pantalla cuenta porque 32 lógicos son mucho en un portátil de 720 de
/// alto y poco en un monitor de 1440. La referencia son 900, los del Book5 Pro
/// a escala 2, que así se queda en los 32 de siempre con el mediano. El
/// margen es corto (±10 %) para que el elegido siga notándose.
/// El panel con los widgets que dice la configuración.
///
/// Un nombre que no existe se ignora y se avisa: el panel se queda sin ese
/// widget, no sin panel.
// añadido en la 0.2.0
fn panel_de(centro: Option<&str>, derecha: &[String]) -> widget::Panel {
    let construir = |nombre: &str| match widgets::por_nombre(nombre) {
        Some(w) => Some(w),
        None => {
            tracing::warn!(nombre, "widget desconocido en la configuración");
            None
        }
    };
    let mut lista = derecha.to_vec();
    config::asegurar_privacidad(centro, &mut lista);
    widget::Panel::new(
        centro.and_then(construir),
        lista
            .iter()
            .map(String::as_str)
            .filter_map(construir)
            .collect(),
    )
}

// añadido en la 0.2.0
fn alto_del_panel(tamano: config::TamanoPanel, pantalla_alto: f32, escala: f32) -> f32 {
    let factor = if pantalla_alto > 0.0 {
        (pantalla_alto / 900.0).clamp(0.9, 1.1)
    } else {
        1.0
    };
    a_pixel_entero((tamano.alto_base() * factor).round(), escala)
}

/// El lienzo del panel: la maqueta de [`PANEL_HEIGHT`] pintada con zoom para
/// que ocupe `ancho × alto` lógicos.
///
/// No pasa por [`Canvas::new`]: cuadraría el ancho de maqueta a píxel entero
/// **a la escala con zoom** y el buffer saldría unos píxeles más ancho que la
/// pantalla, que el compositor reescalaría emborronando el panel entero.
// añadido en la 0.2.0
fn canvas_panel(ancho: f32, alto: f32, escala: f32) -> Canvas {
    let zoom = alto / PANEL_HEIGHT as f32;
    Canvas {
        cache: Some(Cache::default()),
        size: Size::new(ancho / zoom, PANEL_HEIGHT as f32),
        scale: escala * zoom,
        painted_once: false,
    }
}

impl Canvas {
    // 0.1.0
    fn new(size: Size<f32>, scale: f32) -> Self {
        // El tamaño se cuadra a píxel entero aquí, que es por donde pasan
        // todas las superficies del shell: el panel, el dock, las emergentes,
        // el aviso y el bloqueo.
        let size = Size::new(
            a_pixel_entero(size.width, scale),
            a_pixel_entero(size.height, scale),
        );
        Self {
            cache: Some(Cache::default()),
            size,
            scale,
            painted_once: false,
        }
    }

    /// Tamaño del buffer en píxeles físicos.
    // 0.1.0
    fn buffer_size(&self) -> (u32, u32) {
        (
            (self.size.width * self.scale).round().max(1.0) as u32,
            (self.size.height * self.scale).round().max(1.0) as u32,
        )
    }
}

pub struct Shell {
    theme: Theme,

    /// **Tres renderizadores, no uno**, porque dentro de cada uno vive la caché
    /// de rasterizado de iced —los SVG ya parseados y los pixmaps ya pintados—
    /// y esa caché **se purga al final de cada `draw`**: `iced_tiny_skia` tira
    /// todo lo que no haya salido en el último dibujo (`Engine::trim`).
    ///
    /// Con uno solo, el panel y el dock se pisaban mutuamente: dibujar el panel
    /// tiraba los iconos del dock y viceversa, así que en cada fotograma se
    /// reparseaban y re-rasterizaban los SVG de los dos. Medido en release a
    /// 2881×1801 y escala 1,75: el panel solo cuesta 1,19 ms y el dock solo
    /// 1,39 ms, pero **alternándolos —que es lo que hace el compositor de
    /// verdad— costaban 7,61 ms**, casi tres veces la suma. Separándolos, 2,84.
    ///
    /// El tercero lo comparte todo lo demás —emergentes, conmutador, bloqueo,
    /// barras de título— y no pasa nada: de esas se dibuja **una por
    /// fotograma**, así que no hay a quién pisar. Van juntas y no cada una con
    /// el suyo porque nacen y mueren, y un renderizador por superficie efímera
    /// se llevaría la caché con él: el conmutador recrea su lienzo en cada
    /// Alt+Tab, y con caché fría cada apertura costaba 64 ms en vez de 15.
    ///
    /// Crear uno es barato —tres mapas vacíos— y el sistema de fuentes es un
    /// estático compartido que no se duplica.
    renderer_panel: Renderer,
    renderer_dock: Renderer,
    /// El de las superficies modales grandes (emergente y bloqueo), aislado de
    /// `renderer` para que las ondas de la isla de actividades o una barra de
    /// título en segundo plano no purguen los iconos del launchpad a mitad de
    /// animación, y para poder soltar sus bitmaps al cerrar la modal.
    renderer_modal: Renderer,
    renderer: Renderer,
    /// Máscara de recorte reutilizada entre llamadas a `paint_con_fondo` para
    /// no reservar y liberar varios MB por fotograma.
    mask: Option<tiny_skia::Mask>,

    /// El panel se maqueta siempre a [`PANEL_HEIGHT`] y se pinta con zoom
    /// —escala × `panel_alto / PANEL_HEIGHT`—: así su tamaño crece entero,
    /// iconos, texto y huecos a la vez, sin tocar cada medida de `view::panel`.
    /// Por eso su `size` está en unidades de maqueta y su `scale` no es la de
    /// la pantalla; la de la pantalla es [`Shell::escala`].
    panel: Canvas,
    widgets: widget::Panel,
    /// Lo que pide la configuración para el centro y la derecha: sirve para no
    /// rehacer el panel —y con él los procesos que lanzan algunos widgets— si
    /// Ajustes vuelve a mandar la misma lista.
    // añadido en la 0.2.0
    widgets_pedidos: (Option<String>, Vec<String>),
    /// El último escritorio activo y cuántos hay, que el compositor manda al
    /// cambiar y no vuelve a mandar: un panel rehecho en vivo los necesita.
    // añadido en la 0.2.0
    escritorios_vistos: (usize, usize),
    /// Último estado de privacidad recibido del compositor, para conservarlo al
    /// reordenar widgets del panel.
    privacidad_vista: Privacidad,
    /// El control del panel o del dock que tiene el foco de teclado, si se
    /// llegó a ellos con Meta+Alt+P. Ver [`accesible::Foco`].
    // añadido en la 0.2.2
    foco_barra: Option<accesible::Foco>,
    /// Escala de la pantalla, la de todas las superficies menos el panel.
    escala: f32,
    /// Ancho del panel en lógicos de pantalla.
    panel_ancho: f32,
    /// Alto del panel en lógicos de pantalla. Ver [`alto_del_panel`].
    panel_alto: f32,
    panel_tamano: config::TamanoPanel,
    /// Aplicaciones silenciadas en las notificaciones (solo avisos no críticos).
    // añadido en la 0.2.4
    apps_silenciadas: Vec<String>,
    /// Modo de bajo consumo unificado (configurado o por perfil `power-saver`).
    // añadido en la 0.2.4
    bajo_consumo: bool,
    // añadido en la 0.2.4
    perfil_ahorro: bool,
    /// Alto lógico de la pantalla; 0 hasta que lo diga el compositor.
    pantalla_alto: f32,

    dock: Canvas,
    dock_items: Dock,
    /// La foto de perfil que dice la configuración, si la dice. El bloqueo la
    /// prefiere a las de siempre (`~/.face`, AccountsService).
    avatar: Option<String>,
    /// Disposición y elementos visibles del bloqueo. Se guarda aparte del
    /// `Config` porque el resto se consume al construir panel y dock.
    /// Lo que el usuario ha elegido. El valor **efectivo** puede ser otro: la
    /// batería baja fuerza los efectos reducidos sin tocar esta preferencia.
    efectos_config: config::Efectos,
    /// ¿El dock se queda a la vista con el launchpad abierto?
    launchpad_dock: bool,
    bloqueo_config: config::Bloqueo,
    actividades_config: config::Actividades,

    /// La pantalla de bloqueo, cuando está echada.
    ///
    /// Va con su propio `Canvas` y no dentro de `emergente` porque no es una
    /// emergente: no cuelga de nada, ocupa la pantalla entera y no se cierra
    /// pulsando fuera. Comparte el renderer con el resto del shell, que es lo
    /// que hace que el texto se dibuje: un `Renderer` recién creado mide el
    /// texto pero no llega a rasterizarlo hasta la siguiente vuelta, y el
    /// reloj salía en blanco.
    bloqueo: Option<(bloqueo::Bloqueo, Canvas)>,
    // añadido en la 0.2.0 -- solo las capas visibles necesitan un lienzo rasterizado.
    bloqueo_capas: Vec<(bloqueo::Capa, Canvas)>,
    // añadido en la 0.2.0 -- las previsualizaciones pueden seguir pidiendo un dock de una sola pieza.
    dock_en_capas: bool,
    dock_revision: u64,
    /// La capa de captura de pantalla, mientras está abierta: la barra de modos,
    /// la pastilla de las medidas y el popover de opciones.
    captura: Option<(captura::Captura, Canvas, Canvas, Canvas)>,
    /// Pill flotante durante una grabación de pantalla (excluido del vídeo).
    grabacion_pill: Option<(captura::GrabacionPill, Canvas, u64)>,

    /// El aviso de volumen, brillo y demás, mientras dura.
    osd: Option<(osd::Osd, Canvas)>,
    /// El teclado en pantalla, mientras se ve o se está yendo.
    teclado: Option<(teclado::Teclado, Canvas)>,
    /// El panel de diagnóstico, mientras esté puesto. Va con su propio lienzo
    /// por lo mismo que el aviso: es una superficie aparte, con su tamaño, que
    /// aparece y desaparece sin tocar al panel.
    diagnostico: Option<(diagnostico::Hud, Canvas)>,
    /// El conmutador de Alt+Tab, mientras el modificador siga pulsado.
    conmutador: Option<(conmutador::Conmutador, Canvas)>,

    /// «No molestar»: desde cuándo y hasta cuánto. Vive aquí y no en la
    /// tarjeta porque la tarjeta se destruye al cerrarla, y el silencio tiene
    /// que seguir puesto: silenciar y perder el silencio al cerrar la ventana
    /// donde lo pusiste es peor que no poder silenciar.
    silencio: Option<(std::time::Instant, Option<std::time::Duration>)>,

    /// El aviso de la última notificación, mientras dura.
    /// Los avisos a la vista, el más nuevo primero.
    toasts: Vec<(toast::Toast, Canvas)>,

    /// La tarea viva publicada por una aplicación de sistema. Es distinta de
    /// `toast`: no caduca y sus controles devuelven acciones a quien la creó.
    actividad: Option<(actividad::Actividad, Canvas)>,
    /// Todas las tareas publicadas. La isla enseña una por prioridad, pero una
    /// grabación no debe borrar la música: al parar vuelve la que seguía viva.
    actividades: std::collections::HashMap<String, actividad::Estado>,
    /// La actividad que el usuario pasó a la píldora pulsando su burbuja. Sin
    /// elección, o si la elegida terminó, manda la prioridad.
    actividad_principal: Option<String>,
    /// La que se pidió «mostrar primero» desde el menú ⋯: encabeza el orden
    /// mientras viva.
    // añadido en la 0.2.2
    actividad_primera: Option<String>,
    /// Las que se ocultaron de la isla desde el menú ⋯. Siguen vivas en su app;
    /// vuelven al publicarse de nuevo tras cerrarse.
    // añadido en la 0.2.2
    actividades_ocultas: std::collections::HashSet<String>,
    /// El `app_id` de la ventana con el foco, que pone el compositor: el shell
    /// no ve el foco. Sirve para no enseñar en la isla lo que ya se está viendo.
    app_enfocada: Option<String>,

    /// Las notificaciones que han llegado. Las recibe el compositor por D-Bus
    /// y las deja aquí; el panel y su tarjeta las leen de un solo sitio.
    notificaciones: notificaciones::Registro,

    /// Las barras de título vivas, por ventana. La clave la pone el
    /// compositor, que es quien sabe qué ventana es cuál; aquí solo se guarda
    /// el buffer y lo último que se pintó en él, para no repintar una barra que
    /// no ha cambiado —hay una por ventana, y repintarlas todas en cada frame
    /// sería el trabajo del panel multiplicado por las ventanas abiertas.
    barras: std::collections::HashMap<u64, Barra>,

    /// La superficie emergente abierta, si hay alguna. Solo puede haber una:
    /// abrir el calendario con el menú desplegado cierra el menú, que es lo que
    /// hace cualquier barra de menús.
    emergente: Option<(Emergente, Canvas)>,

    /// Los iconos del escritorio y el lienzo con el que se pinta **cada** uno.
    ///
    /// El lienzo es uno solo y no uno por icono porque todas las celdas miden
    /// lo mismo: lo único que guarda es el tamaño, la escala y la caché de
    /// disposición de iced, y esa se reconstruye igual entre celdas —el árbol
    /// de widgets de todas es el mismo—. El buffer no está aquí: lo pone el
    /// compositor, que es quien tiene una superficie por icono.
    escritorio: escritorio::Escritorio,
    escritorio_canvas: Canvas,
}

impl Shell {
    // 0.1.0
    pub fn new(width: u32, scale: f32) -> Self {
        // Aquí y no en `con_config`: el canal sale de ficheros de la máquina,
        // igual que `panel.conf`, y los tests no deben depender de ellos.
        marca::aplicar_canal(marca::Canal::leer());
        Self::con_config(width, scale, Config::cargar())
    }

    /// Como [`Shell::new`] pero con una configuración dada.
    ///
    /// `new` lee `~/.config/bookos/panel.conf`, y eso ata cualquier prueba a lo
    /// que tenga el usuario en su casa: el test del panel empezó a fallar en
    /// cuanto hubo un fichero de configuración de verdad en la máquina.
    // 0.1.0
    // modified 0.1.1 -- renderer aparte para lo modal, máscara reutilizable, teclado en pantalla e iconos del escritorio según config
    pub fn con_config(width: u32, scale: f32, config: Config) -> Self {
        // Lo primero de todo: los widgets preguntan por los colores mientras se
        // construyen, y con el tema puesto después el primer frame saldría con
        // la paleta anterior.
        tema::aplicar(config.tema);
        tema::aplicar_modo(config.modo_tema);
        tema::aplicar_acento(config.acento);
        tema::aplicar_alto_contraste(config.alto_contraste);
        widgets::bateria_mostrar(
            config.bateria_porcentaje,
            config.bateria_tiempo,
            config.bateria_colores,
        );
        // Cuál de las familias instaladas está puesta, para que la tarjeta de
        // Apariencia pueda marcarla. Se mira la del tema que toca y, si esa
        // clave no está, el `fondo` común; sin ninguna no hay nada que marcar y
        // manda la que elija `fondos::por_defecto`.
        let puesto = if config.tema == tema::Tema::Claro {
            config.fondo_claro.as_deref()
        } else {
            config.fondo_oscuro.as_deref()
        }
        .or(config.fondo.as_deref());
        fondos::poner_elegida(
            puesto
                .map(std::path::Path::new)
                .and_then(fondos::familia_de)
                .or_else(|| {
                    fondos::por_defecto(config.tema == tema::Tema::Claro).map(|f| f.nombre)
                }),
        );
        tema::aplicar_efectos_reducidos(config.efectos == config::Efectos::Reducidos);
        let widgets = panel_de(config.centro.as_deref(), &config.derecha);
        let mut dock_items = Dock::from_config(&config.dock);
        dock_items.tamano(config.dock_tamano);
        let (dw, dh) = dock_items.size();
        let alto_panel = alto_del_panel(config.panel_tamano, 0.0, scale);
        Self {
            // Font::DEFAULT resuelve contra las fuentes del sistema vía fontdb.
            renderer_panel: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            renderer_dock: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            renderer_modal: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            renderer: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            mask: None,
            theme: view::theme(),
            panel: canvas_panel(width as f32, alto_panel, scale),
            widgets,
            widgets_pedidos: (config.centro.clone(), config.derecha.clone()),
            escritorios_vistos: (0, 0),
            privacidad_vista: Privacidad::default(),
            foco_barra: None,
            escala: scale,
            panel_ancho: width as f32,
            panel_alto: alto_panel,
            panel_tamano: config.panel_tamano,
            apps_silenciadas: config.notificaciones_silenciadas.clone(),
            bajo_consumo: config.bajo_consumo,
            perfil_ahorro: false,
            pantalla_alto: 0.0,
            dock: Canvas::new(Size::new(dw, dh), scale),
            dock_items,
            avatar: config.avatar.clone(),
            efectos_config: config.efectos,
            launchpad_dock: config.launchpad_dock,
            bloqueo_config: config.bloqueo,
            actividades_config: config.actividades,
            emergente: None,
            bloqueo: None,
            bloqueo_capas: Vec::new(),
            dock_en_capas: false,
            dock_revision: 0,
            captura: None,
            grabacion_pill: None,
            osd: None,
            teclado: None,
            diagnostico: None,
            conmutador: None,
            notificaciones: notificaciones::Registro::default(),
            silencio: None,
            toasts: Vec::new(),
            actividad: None,
            actividades: std::collections::HashMap::new(),
            actividad_principal: None,
            actividad_primera: None,
            actividades_ocultas: std::collections::HashSet::new(),
            app_enfocada: None,
            barras: std::collections::HashMap::new(),
            // Sin alto todavía: lo sabe el compositor y llega por
            // `escritorio_pantalla` antes del primer frame.
            escritorio: escritorio::Escritorio::new(
                alto_panel,
                (width as f32, 0.0),
                config.iconos_escritorio,
                config.tamano_iconos,
            ),
            escritorio_canvas: Canvas::new(
                Size::new(
                    escritorio::CELDA.0 * config.tamano_iconos.escala(),
                    escritorio::CELDA.1 * config.tamano_iconos.escala(),
                ),
                scale,
            ),
        }
    }

    // 0.1.0
    // modificado en la 0.2.0 -- el panel se rehace con su zoom
    pub fn resize(&mut self, width: u32, scale: f32) {
        if self.panel_ancho != width as f32 || self.escala != scale {
            self.panel_ancho = width as f32;
            self.escala = scale;
            self.rehacer_panel();
            self.dock.scale = scale;
            self.dock.painted_once = false;
            if let Some((_, canvas)) = self.actividad.as_mut() {
                canvas.scale = scale;
                canvas.painted_once = false;
            }
        }
    }

    /// Tamaño **lógico** del panel: el que ve el escritorio, independiente de
    /// la escala. No confundir con `panel_buffer_size`, que son píxeles reales.
    // 0.1.0
    // modificado en la 0.2.0 -- en lógicos de pantalla, no de maqueta
    pub fn panel_logical_size(&self) -> (i32, i32) {
        (self.panel_ancho as i32, self.panel_alto as i32)
    }

    /// Tamaño **lógico** del dock.
    // 0.1.0
    pub fn dock_logical_size(&self) -> (i32, i32) {
        (self.dock.size.width as i32, self.dock.size.height as i32)
    }

    // 0.1.0
    pub fn panel_buffer_size(&self) -> (u32, u32) {
        self.panel.buffer_size()
    }

    // 0.1.0
    pub fn dock_buffer_size(&self) -> (u32, u32) {
        self.dock.buffer_size()
    }

    /// Alto del panel en píxeles **lógicos**. Es el que se usa para colocar
    /// ventanas: el `Space` de Smithay razona en lógico, no en físico.
    // 0.1.0
    // modificado en la 0.2.0 -- según el tamaño elegido y la pantalla
    pub fn panel_height(&self) -> i32 {
        self.panel_alto as i32
    }

    /// Cuánto se amplía la maqueta del panel para pintarla.
    // añadido en la 0.2.0
    fn panel_zoom(&self) -> f32 {
        self.panel_alto / PANEL_HEIGHT as f32
    }

    /// El compositor dice el alto lógico de la pantalla. `true` si el panel
    /// cambió de alto y hay que rehacer su superficie.
    // añadido en la 0.2.0
    pub fn panel_pantalla(&mut self, alto: f32) -> bool {
        self.pantalla_alto = alto;
        self.rehacer_panel()
    }

    /// El tamaño elegido en Ajustes. `true` si el panel cambió de alto.
    // añadido en la 0.2.0
    pub fn panel_tamano(&mut self, tamano: config::TamanoPanel) -> bool {
        self.panel_tamano = tamano;
        self.rehacer_panel()
    }

    /// Los widgets elegidos en Ajustes. `true` si el panel cambió.
    ///
    /// Reconstruye el panel entero en vez de insertar o quitar uno: los
    /// widgets son pocos y baratos de crear, y así el orden que se ve es
    /// siempre exactamente el de la lista. Lo que el compositor solo manda al
    /// cambiar —avisos, No molestar, escritorio— se vuelve a repartir, porque
    /// si no un panel recién hecho enseñaría la campana sin su contador.
    // añadido en la 0.2.0
    pub fn widgets_panel(&mut self, centro: Option<&str>, derecha: &[String]) -> bool {
        if self.widgets_pedidos.0.as_deref() == centro && self.widgets_pedidos.1 == derecha {
            return false;
        }
        self.widgets_pedidos = (centro.map(str::to_owned), derecha.to_vec());
        self.widgets = panel_de(centro, derecha);
        self.widgets.no_molestar(self.notificaciones_silenciadas());
        self.widgets.notificaciones(self.notificaciones.cuantas());
        let (activo, cuantos) = self.escritorios_vistos;
        // Antes de la primera llamada del compositor no hay nada que repartir.
        if cuantos > 0 {
            self.widgets.escritorios(activo, cuantos);
        }
        self.widgets.privacidad(self.privacidad_vista);
        self.panel.painted_once = false;
        true
    }

    // añadido en la 0.2.0
    fn rehacer_panel(&mut self) -> bool {
        let alto = alto_del_panel(self.panel_tamano, self.pantalla_alto, self.escala);
        let cambio = alto != self.panel_alto;
        self.panel_alto = alto;
        self.panel = canvas_panel(self.panel_ancho, alto, self.escala);
        cambio
    }

    // 0.1.0
    pub fn scale(&self) -> f32 {
        self.escala
    }

    // 0.1.0
    pub fn dock(&self) -> &Dock {
        &self.dock_items
    }

    // --- Iconos del escritorio ---------------------------------------------

    /// Alto lógico del panel, que es donde empieza la rejilla del escritorio.
    // añadido en la 0.2.3
    pub fn panel_alto(&self) -> f32 {
        self.panel_alto
    }

    // 0.1.0
    pub fn escritorio(&self) -> &escritorio::Escritorio {
        &self.escritorio
    }

    // 0.1.0
    pub fn escritorio_mut(&mut self) -> &mut escritorio::Escritorio {
        &mut self.escritorio
    }

    /// Le dice al escritorio el tamaño **lógico** de la pantalla, que el shell
    /// no conoce: solo se le pasa el ancho al construirlo.
    // 0.1.0
    pub fn escritorio_pantalla(&mut self, pantalla: (f32, f32), scale: f32) {
        self.escritorio.recolocar(self.panel_alto, pantalla);
        // modificado en la 0.2.3 -- también cuando cambia el tamaño de icono.
        let (w, h) = self.escritorio.celda();
        if self.escritorio_canvas.scale != scale || self.escritorio_canvas.size != Size::new(w, h) {
            self.escritorio_canvas = Canvas::new(Size::new(w, h), scale);
        }
    }

    /// Vuelve a mirar la carpeta del escritorio. `true` si la lista cambió.
    // 0.1.0
    pub fn escritorio_releer(&mut self, pantalla: (f32, f32)) -> bool {
        self.escritorio.releer(self.panel_alto, pantalla)
    }

    /// Crea una carpeta o archivo del submenú «Crear».
    // 0.2.0
    pub fn escritorio_nuevo(
        &mut self,
        tipo: escritorio::NuevoElemento,
        pantalla: (f32, f32),
    ) -> bool {
        self.escritorio.crear_nuevo(tipo, self.panel_alto, pantalla)
    }

    /// Manda lo seleccionado a la papelera, del mismo menú.
    // 0.1.0
    pub fn escritorio_eliminar_seleccion(&mut self, pantalla: (f32, f32)) -> bool {
        self.escritorio
            .eliminar_seleccionados(self.panel_alto, pantalla)
    }

    /// Enseña u oculta los iconos. `true` si cambió.
    // 0.1.1
    pub fn escritorio_mostrar(&mut self, visible: bool, pantalla: (f32, f32)) -> bool {
        self.escritorio.mostrar(visible, self.panel_alto, pantalla)
    }

    /// Tamaño en píxeles físicos del buffer de **una** celda. Son todas
    /// iguales, así que el compositor reserva el mismo para cada icono.
    // 0.1.0
    pub fn escritorio_buffer_size(&self) -> (u32, u32) {
        self.escritorio_canvas.buffer_size()
    }

    // 0.1.0
    pub fn escritorio_logical_size(&self) -> (f32, f32) {
        (
            self.escritorio_canvas.size.width,
            self.escritorio_canvas.size.height,
        )
    }

    /// Pinta la celda `i` en `buf`, del tamaño de [`Shell::escritorio_buffer_size`].
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_escritorio(&mut self, i: usize, buf: &mut [u8]) -> Vec<Damage> {
        let view = self.escritorio.ver(i);
        let Self {
            renderer,
            theme,
            escritorio_canvas,
            mask,
            ..
        } = self;
        Self::paint(escritorio_canvas, renderer, mask, theme, view, buf)
    }

    /// Relee los estados. Devuelve `true` si hay que repintar el panel.
    ///
    /// Esta es la puerta que mantiene el shell callado: si nada de lo que se
    /// enseña ha cambiado, nadie dibuja nada. Se llama tanto en el cambio de
    /// minuto como cuando el kernel avisa de un cambio de hardware, así que
    /// tiene que ser barata y no dar por hecho cada cuánto la llaman.
    // 0.1.0
    pub fn refresh(&mut self) -> bool {
        let mut cambio = self.widgets.refrescar();
        // «No molestar» caduca solo y nadie avisa de que ha caducado: se mira
        // aquí, que se llama al menos una vez por minuto con el reloj.
        cambio |= self.widgets.no_molestar(self.notificaciones_silenciadas());
        // Por el mismo motivo, las agujas del reloj y el día del calendario en
        // el dock: la app reescribe el icono en disco y nadie avisa.
        if self.dock_items.refrescar_dinamicos() {
            self.dock.painted_once = false;
        }
        let cambio_emergente = self.refresh_emergente();
        cambio || cambio_emergente || !self.panel.painted_once
    }

    // 0.1.0
    pub fn panel_animando(&self) -> bool {
        self.widgets.animando()
    }

    // 0.1.0
    pub fn panel_avanzar(&mut self) {
        self.widgets.avanzar();
    }

    /// Relee solo la tarjeta abierta; no consulta batería ni carpetas.
    // 0.1.0
    pub fn refresh_emergente(&mut self) -> bool {
        if matches!(&self.emergente, Some((Emergente::Buscador(b), _)) if b.confirmacion_vencida()) {
            self.cerrar_emergente();
            return true;
        }
        let mut cambio_emergente = false;
        // La emergente abierta también relee lo suyo: la lista de redes cambia
        // mientras la tienes delante, y sin esto se quedaría con la foto del
        // instante en que se abrió.
        if let Some((e, canvas)) = self.emergente.as_mut()
            && e.refrescar()
        {
            cambio_emergente = true;
            Self::ajustar(e, canvas);
            canvas.painted_once = false;
        }
        cambio_emergente
    }

    // --- Notificaciones ----------------------------------------------------

    // --- Actividades vivas -------------------------------------------------

    // 0.1.0
    // modified 0.1.1 -- rehace la carátula cuando llega una nueva de la misma app
    // modificado en la 0.2.0 -- Zenith se ve aunque las actividades estén apagadas
    pub fn publicar_actividad(&mut self, mut estado: actividad::Estado) {
        if !self.actividades_config.habilitadas && !es_asistente(&estado) {
            return;
        }
        let portada_nueva = estado.portada.is_some();
        // La portada pesa cientos de KB. El Player solo la reenvía al cambiar
        // de canción y las actualizaciones de posición llegan vacías.
        //
        // Las de la cola se heredan igual. Sin eso solo vivían en la tarjeta ya
        // construida, y cada vez que la isla se rehace —la app recupera el foco
        // y lo suelta, otra actividad le quita la píldora— la cola salía sin
        // carátulas hasta el siguiente reenvío del Player, hasta 30 s después.
        //
        // Se mueven en vez de clonarse: el estado anterior se va a sustituir.
        if let Some(mut anterior) = self.actividades.remove(&estado.app_id) {
            if estado.portada.is_none() {
                estado.portada = anterior.portada.take();
            } else if let Some((actual, _)) = self.actividad.as_mut()
                && actual.app_id() == estado.app_id
            {
                actual.invalidar_arte();
            }
            for item in estado.cola.iter_mut().filter(|i| i.portada.is_none()) {
                item.portada = anterior
                    .cola
                    .iter_mut()
                    .find(|a| a.id == item.id)
                    .and_then(|a| a.portada.take());
            }
        } else if portada_nueva
            && let Some((actual, _)) = self.actividad.as_mut()
            && actual.app_id() == estado.app_id
        {
            actual.invalidar_arte();
        }
        self.actividades.insert(estado.app_id.clone(), estado);
        self.sincronizar_actividad();
    }

    /// Dice qué aplicación tiene el foco. Su actividad se esconde de la isla
    /// mientras esté delante y vuelve al salir de ella.
    // 0.1.0
    pub fn poner_app_enfocada(&mut self, app_id: Option<String>) {
        if self.app_enfocada == app_id {
            return;
        }
        self.app_enfocada = app_id;
        self.sincronizar_actividad();
    }

    // 0.1.0
    // modified 0.1.1 -- no clona carátulas ni miniaturas ya preparadas en cada tick
    fn sincronizar_actividad(&mut self) {
        let mut vivas = vivas_ordenadas(
            &self.actividades,
            self.app_enfocada.as_deref(),
            &self.actividades_config,
            self.actividad_primera.as_deref(),
            &self.actividades_ocultas,
        );
        // Al asistente se le está hablando: manda aunque se hubiese elegido
        // otra burbuja como principal, o la respuesta quedaría en un círculo.
        let principal = vivas
            .iter()
            .position(|e| {
                matches!(e.clase, actividad::Clase::Asistente(f) if f != actividad::Fase::Minimizado)
            })
            .or_else(|| {
                vivas
                    .iter()
                    .position(|e| Some(&e.app_id) == self.actividad_principal.as_ref())
            })
            .unwrap_or(0);
        if vivas.is_empty() {
            self.actividad = None;
            return;
        }
        // Todas, la principal incluida y marcada: la isla las enseña en su
        // columna en este mismo orden, que es el que recorre la rueda.
        let burbujas = vivas
            .iter()
            .enumerate()
            .map(|(i, e)| actividad::Burbuja {
                app_id: e.app_id.clone(),
                clase: e.clase,
                color: actividad::color_burbuja(e),
                activa: e.activo && !e.pausado,
                actual: i == principal,
            })
            .collect();
        let elegido = vivas.remove(principal);
        let escala = self.escala;
        match self.actividad.as_mut() {
            Some((actual, canvas)) if actual.app_id() == elegido.app_id => {
                // La carátula y las miniaturas ya están recortadas en `actual`:
                // clonar varios MB de RGBA cada segundo solo para mover la
                // posición de reproducción desperdiciaría RAM y CPU.
                let mut estado = actividad::Estado {
                    app_id: elegido.app_id.clone(),
                    clase: elegido.clase,
                    titulo: elegido.titulo.clone(),
                    subtitulo: elegido.subtitulo.clone(),
                    posicion_ms: elegido.posicion_ms,
                    duracion_ms: elegido.duracion_ms,
                    restante_ms: elegido.restante_ms,
                    pausado: elegido.pausado,
                    activo: elegido.activo,
                    nivel: elegido.nivel,
                    volumen: elegido.volumen,
                    aleatorio: elegido.aleatorio,
                    repetir: elegido.repetir,
                    anc: elegido.anc,
                    portada: None,
                    cola: elegido
                        .cola
                        .iter()
                        .map(|item| actividad::ItemCola {
                            id: item.id.clone(),
                            titulo: item.titulo.clone(),
                            artista: item.artista.clone(),
                            duracion_ms: item.duracion_ms,
                            favorita: item.favorita,
                            actual: item.actual,
                            portada: if actual.tiene_miniatura(&item.id) {
                                None
                            } else {
                                item.portada.clone()
                            },
                        })
                        .collect(),
                    metricas: elegido.metricas.clone(),
                };
                if !actual.tiene_arte() {
                    estado.portada = elegido.portada.clone();
                }
                actual.actualizar(estado);
                actual.poner_burbujas(burbujas);
                let (w, h) = actual.size();
                let objetivo = Size::new(w, h);
                if canvas.size != objetivo {
                    *canvas = Canvas::new(objetivo, escala);
                } else {
                    canvas.painted_once = false;
                }
            }
            _ => {
                let mut actividad = actividad::Actividad::nueva(
                    elegido.clone(),
                    self.actividades_config.animaciones,
                );
                actividad.poner_burbujas(burbujas);
                // Pasar de una a otra es la misma isla: hereda cómo iba y
                // funde el contenido mientras la caja cambia de forma.
                if let Some((anterior, _)) = self.actividad.take() {
                    actividad.continuar_de(anterior);
                }
                let (w, h) = actividad.size();
                self.actividad = Some((actividad, Canvas::new(Size::new(w, h), escala)));
            }
        }
    }

    // 0.1.0
    pub fn cerrar_actividad(&mut self, app_id: &str) -> bool {
        // Al terminar se olvida la elección: la próxima vez que publique es
        // otra actividad y vuelve a verse.
        self.actividades_ocultas.remove(app_id);
        if self.actividad_primera.as_deref() == Some(app_id) {
            self.actividad_primera = None;
        }
        if self.actividades.remove(app_id).is_none() {
            return false;
        }
        if let Some((actual, _)) = self.actividad.as_mut()
            && actual.app_id() == app_id
            && actual.salir()
        {
            return true;
        }
        self.sincronizar_actividad();
        true
    }

    /// Quita la isla que se estaba yendo cuando acaba su fundido.
    // añadido en la 0.2.0
    pub fn terminar_salida_actividad(&mut self) -> bool {
        if !self
            .actividad
            .as_ref()
            .is_some_and(|(a, _)| a.salida_terminada())
        {
            return false;
        }
        self.sincronizar_actividad();
        true
    }

    /// Clic fuera o Esc con Zenith delante. Ver `Actividad::despedir_asistente`.
    // añadido en la 0.2.0
    pub fn despedir_asistente(&mut self, cancelar: bool) -> Option<actividad::Accion> {
        self.actividad
            .as_mut()
            .and_then(|(a, _)| a.despedir_asistente(cancelar))
    }

    // 0.1.0
    // modificado en la 0.2.0 -- apagar las actividades no se lleva a Zenith
    pub fn aplicar_actividades_config(&mut self, config: config::Actividades) {
        self.actividades_config = config;
        if !config.habilitadas {
            self.actividades.retain(|_, e| es_asistente(e));
        }
        if let Some((actividad, canvas)) = self.actividad.as_mut() {
            actividad.set_animaciones(config.animaciones);
            canvas.painted_once = false;
        }
        self.sincronizar_actividad();
    }

    // 0.1.0
    pub fn actividad_app_id(&self) -> Option<&str> {
        self.actividad.as_ref().map(|(a, _)| a.app_id())
    }

    // 0.1.0
    pub fn actividad_clase(&self) -> Option<actividad::Clase> {
        self.actividad.as_ref().map(|(a, _)| a.clase())
    }

    /// La voz que oye Zenith, ya suavizada (0..1). Para el brillo de borde.
    // añadido en la 0.2.0
    pub fn actividad_nivel(&self) -> f32 {
        self.actividad
            .as_ref()
            .map_or(0.0, |(a, _)| a.nivel_suave())
    }

    // 0.1.0
    pub fn actividad_buffer_size(&self) -> Option<(u32, u32)> {
        self.actividad.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn actividad_logical_size(&self) -> Option<(f32, f32)> {
        self.actividad
            .as_ref()
            .map(|(_, c)| (c.size.width, c.size.height))
    }

    // 0.1.0
    pub fn actividad_needs_paint(&self) -> bool {
        self.actividad
            .as_ref()
            .is_some_and(|(a, c)| !c.painted_once || a.ondas_pendientes() || a.animando_relevo())
    }

    /// Cierra el relevo de la isla cuando termina y devuelve el lienzo a su
    /// tamaño. `true` si hay que repintar. Lo llama el compositor en cada
    /// fotograma de animación de la isla.
    // añadido en la 0.2.3
    pub fn avanzar_relevo_actividad(&mut self) -> bool {
        let escala = self.escala;
        let Some((a, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if !a.terminar_relevo() {
            return false;
        }
        let (w, h) = a.size();
        *canvas = Canvas::new(Size::new(w, h), escala);
        true
    }

    // 0.1.0
    pub fn actividad_animando(&self) -> bool {
        self.actividad.as_ref().is_some_and(|(a, _)| a.animando())
    }

    // 0.1.0
    // modificado en la 0.2.0 -- también el desplazamiento horizontal
    pub fn actividad_entrada(&self) -> (f32, f32, f32, f32) {
        self.actividad
            .as_ref()
            .map_or((0.0, 1.0, 0.0, 0.0), |(a, _)| a.entrada())
    }

    /// La parte visible de la isla como `(x, ancho)` dentro de la tarjeta.
    // 0.1.0
    pub fn actividad_zona(&self) -> Option<(f32, f32)> {
        self.actividad.as_ref().map(|(a, _)| a.zona())
    }

    /// Pasa a la actividad siguiente (`paso` > 0) o anterior en el orden de
    /// [`vivas_ordenadas`], dando la vuelta. Es lo que hace la rueda sobre la
    /// isla: una actividad a la vez, y la elegida queda como principal hasta
    /// que termine o se elija otra. `false` si no hay a cuál pasar.
    // añadido en la 0.2.0
    pub fn actividad_pasar(&mut self, paso: i32) -> bool {
        let orden: Vec<String> =
            vivas_ordenadas(
            &self.actividades,
            self.app_enfocada.as_deref(),
            &self.actividades_config,
            self.actividad_primera.as_deref(),
            &self.actividades_ocultas,
        )
                .iter()
                .map(|e| e.app_id.clone())
                .collect();
        if orden.len() < 2 {
            return false;
        }
        let actual = self
            .actividad_app_id()
            .and_then(|id| orden.iter().position(|o| o == id))
            .unwrap_or(0) as i32;
        let n = orden.len() as i32;
        let siguiente = (actual + paso).rem_euclid(n) as usize;
        let desplegada = self.actividad.as_ref().is_some_and(|(a, _)| a.desplegada());
        self.actividad_principal = Some(orden[siguiente].clone());
        self.sincronizar_actividad();
        if desplegada && let Some((a, canvas)) = self.actividad.as_mut() {
            a.seguir_desplegada();
            let (w, h) = a.size();
            *canvas = Canvas::new(Size::new(w, h), self.escala);
        }
        true
    }

    // 0.1.0
    pub fn actividad_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let elegida = self
            .actividad
            .as_ref()
            .and_then(|(a, _)| a.burbuja_en(x, y))
            .map(str::to_string);
        if let Some(app_id) = elegida {
            self.actividad_principal = Some(app_id);
            self.sincronizar_actividad();
            return None;
        }
        let (accion, size) = {
            let (actividad, _) = self.actividad.as_mut()?;
            let accion = actividad.pulsar(x, y);
            (accion, actividad.size())
        };
        let escala = self.escala;
        if let Some((_, canvas)) = self.actividad.as_mut() {
            let objetivo = Size::new(size.0, size.1);
            if canvas.size != objetivo {
                *canvas = Canvas::new(objetivo, escala);
            } else {
                canvas.painted_once = false;
            }
        }
        self.accion_de_isla(accion?)
    }

    /// Las órdenes del menú ⋯ y de la cabecera las resuelve el shell; el resto
    /// van a la app.
    // añadido en la 0.2.2
    fn accion_de_isla(&mut self, a: actividad::Accion) -> Option<Accion> {
        match a.nombre.as_str() {
            actividad::ISLA_ABRIR => {
                // La app pasa al frente: la isla vuelve a píldora para no taparla.
                self.recoger_actividad();
                let cola = a.app_id.rsplit('.').next().unwrap_or(&a.app_id);
                let exec = apps::por_app_id(&a.app_id)
                    .or_else(|| apps::por_app_id(&format!("bookos-{cola}")))
                    .map(|app| app.exec)
                    .or_else(|| programa_de_actividad(&a.app_id))
                    .unwrap_or_default();
                Some(Accion::AbrirAppDeActividad {
                    app_id: a.app_id,
                    exec,
                })
            }
            actividad::ISLA_PRIMERO => {
                self.actividad_primera = Some(a.app_id.clone());
                self.actividad_principal = Some(a.app_id);
                self.sincronizar_actividad();
                None
            }
            actividad::ISLA_AJUSTES => {
                self.recoger_actividad();
                Some(Accion::Lanzar("bookos-settings --page avanzadas".into()))
            }
            actividad::ISLA_OCULTAR => {
                self.actividades_ocultas.insert(a.app_id);
                self.actividad_principal = None;
                self.sincronizar_actividad();
                None
            }
            _ => Some(Accion::Actividad(a)),
        }
    }

    /// El clic derecho sobre la isla abre su menú ⋯.
    // añadido en la 0.2.2
    pub fn actividad_menu(&mut self) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if !actividad.desplegada() || !actividad.alternar_menu() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.escala);
        true
    }

    /// Si la tarjeta abierta puede recogerse sola al apartar el puntero.
    // añadido en la 0.2.2
    pub fn actividad_recogible(&self) -> bool {
        self.actividad.as_ref().is_some_and(|(a, _)| a.desplegada() && a.recogible())
    }

    // 0.1.0
    pub fn abrir_actividad_previsualizacion(&mut self, app_id: &str) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if actividad.app_id() != app_id || !actividad.abrir_previsualizacion() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.escala);
        true
    }

    // 0.1.0
    pub fn actividad_abierta(&self) -> bool {
        self.actividad.as_ref().is_some_and(|(a, _)| a.desplegada())
    }

    // 0.1.0
    pub fn recoger_actividad(&mut self) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if !actividad.recoger() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.escala);
        true
    }

    // 0.1.0
    pub fn alternar_actividad(&mut self) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if !actividad.alternar_vista() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.escala);
        true
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_actividad(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return Vec::new();
        };
        // La isla toma el tono de control del tema (ver `actividad::isla`). En
        // claro los grises y el acento se ajustan a su fondo mientras dura este
        // pintado: los estilos de iced se evalúan al pintar y todo ocurre en el
        // hilo del compositor.
        tema::pintando_isla_clara(tema::es_claro());
        let vista = actividad.view();
        let damage = Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        );
        tema::pintando_isla_clara(false);
        actividad.marcar_ondas_pintadas();
        damage
    }

    /// Guarda una notificación recién llegada y saca su aviso. `true` si hay
    /// que repintar.
    ///
    /// Con «No molestar» puesto **no** se saca el aviso y la notificación se
    /// queda solo en la lista, que es justo lo que promete la fila de la
    /// tarjeta («las notificaciones solo se guardan aquí»). Las críticas sí
    /// salen: para eso son críticas.
    ///
    /// `caducidad` es el `expire_timeout` de D-Bus, tal cual llega.
    // modified 0.1.1 -- otra de la misma app sustituye a la de delante en vez de apilarse
    pub fn notificar(
        &mut self,
        notificacion: notificaciones::Notificacion,
        caducidad: i32,
    ) -> bool {
        // Con una tarjeta abierta tampoco: saldría encima de ella. Llega a la
        // lista y al contador de la campana igual.
        let callado = self.notificaciones_silenciadas() || self.emergente.is_some();
        if !callado || notificacion.critica {
            let aviso = toast::Toast::new(notificacion.clone(), caducidad);
            let (w, h) = aviso.size();
            let objetivo = Size::new(w, h);
            // Otra de la misma app que la de delante la sustituye en vez de
            // apilarse: una conversación de chat son diez avisos seguidos, y
            // cada uno entraba arriba empujando la pila. La anterior sigue en
            // la campana. Las críticas no: esas no pueden quedar tapadas.
            let misma_app = || {
                self.toasts
                    .iter()
                    .position(|(t, _)| !t.saliendo())
                    .filter(|&i| {
                        let delante = &self.toasts[i].0;
                        !notificacion.critica
                            && !delante.critica()
                            && delante.app() == notificacion.app
                    })
            };
            let sitio = self
                .toasts
                .iter()
                .position(|(t, _)| t.id() == notificacion.id)
                .or_else(misma_app);
            match sitio.map(|i| &mut self.toasts[i]) {
                // `replaces_id`: la misma notificación actualizada —un progreso
                // que avanza— se queda en su sitio de la pila en vez de entrar
                // otra vez arriba.
                Some((viejo, canvas)) => {
                    *viejo = aviso;
                    if canvas.size != objetivo {
                        *canvas = Canvas::new(objetivo, self.escala);
                    }
                    canvas.painted_once = false;
                }
                None => {
                    let canvas = Canvas::new(objetivo, self.escala);
                    self.toasts.insert(0, (aviso, canvas));
                }
            }
            // La que no cabe no desaparece de golpe: se le da la salida, y la
            // pila se cierra mientras se va.
            if let Some((sobra, _)) = self
                .toasts
                .iter_mut()
                .filter(|(t, _)| !t.saliendo())
                .nth(toast::MAXIMO_A_LA_VISTA)
            {
                sobra.descartar();
            }
            self.ordenar_pila();
        }
        self.archivar_notificacion(notificacion)
    }

    /// A la lista y al contador de la campana, sin aviso flotante.
    // 0.1.1
    pub fn archivar_notificacion(&mut self, notificacion: notificaciones::Notificacion) -> bool {
        self.notificaciones.añadir(notificacion);
        self.sincronizar_notificaciones();
        true
    }

    // --- El aviso de una notificación ---------------------------------------

    /// Retira los avisos a los que se les acabó el tiempo. `true` si queda
    /// alguno.
    // 0.1.0
    pub fn toasts_vivos(&mut self) -> bool {
        self.toasts.retain(|(t, _)| t.queda().is_some());
        self.toasts.iter_mut().for_each(|(t, _)| t.avanzar());
        self.ordenar_pila();
        !self.toasts.is_empty()
    }

    /// Lo que le queda a cada uno, para programar un despertar por aviso en
    /// vez de repintar sesenta veces por segundo enseñando lo mismo.
    // 0.1.0
    pub fn toasts_quedan(&self) -> Vec<std::time::Duration> {
        self.toasts.iter().filter_map(|(t, _)| t.queda()).collect()
    }

    /// Los identificadores de los avisos, el más nuevo primero, que es el
    /// orden de la pila de arriba abajo.
    // 0.1.0
    pub fn toasts_ids(&self) -> Vec<u32> {
        self.toasts.iter().map(|(t, _)| t.id()).collect()
    }

    // 0.1.0
    fn toast(&self, id: u32) -> Option<&(toast::Toast, Canvas)> {
        self.toasts.iter().find(|(t, _)| t.id() == id)
    }

    /// Alfa, escala y cuánto baja el aviso `id`, en lógicos, por su sitio en la
    /// pila.
    // 0.1.0
    pub fn toast_estado(&self, id: u32) -> Option<(f32, f32, f32)> {
        self.toast(id)
            .map(|(t, _)| (t.alfa(), t.escala(), t.desplazamiento()))
    }

    // 0.1.1
    pub fn toast_desplazamiento_horizontal(&self, id: u32) -> Option<f32> {
        self.toast(id).map(|(t, _)| t.desplazamiento_horizontal())
    }

    /// Los avisos en el orden en que hay que componerlos: el de delante
    /// primero, porque en la lista de elementos de Smithay el primero queda
    /// encima. Los que se están yendo van antes que los demás de su nivel: se
    /// desvanecen por encima del que sube a ocupar su sitio.
    // 0.1.0
    pub fn toasts_para_componer(&self) -> Vec<u32> {
        let mut orden: Vec<_> = self.toasts.iter().map(|(t, _)| t).collect();
        orden.sort_by_key(|t| (t.profundidad(), !t.saliendo()));
        orden.into_iter().map(|t| t.id()).collect()
    }

    /// El aviso de delante, el único que se pulsa.
    // 0.1.0
    pub fn toast_de_delante(&self) -> Option<u32> {
        self.toasts
            .iter()
            .map(|(t, _)| t)
            .find(|t| t.profundidad() == 0 && !t.saliendo())
            .map(|t| t.id())
    }

    // 0.1.0
    pub fn toasts_animando(&self) -> bool {
        self.toasts.iter().any(|(t, _)| t.animando())
    }

    // 0.1.0
    // modificado en la 0.2.2 -- también mientras se despliega
    pub fn toast_needs_paint(&self, id: u32) -> bool {
        self.toast(id)
            .is_some_and(|(t, c)| !c.painted_once || t.redibujando())
    }

    /// El puntero entra en el aviso `id` o sale de él. `true` si hay que
    /// repintar.
    // añadido en la 0.2.2
    pub fn toast_señalar(&mut self, id: u32, encima: bool) -> bool {
        self.toasts
            .iter_mut()
            .find(|(t, _)| t.id() == id)
            .is_some_and(|(t, _)| t.señalar(encima))
    }

    /// El puntero se mueve dentro del aviso `id`; coordenadas de la tarjeta.
    // añadido en la 0.2.2
    pub fn toast_señalar_en(&mut self, id: u32, x: f32, y: f32) -> bool {
        self.toasts
            .iter_mut()
            .find(|(t, _)| t.id() == id)
            .is_some_and(|(t, _)| t.señalar_en(x, y))
    }

    /// «Responder» en un aviso: abre su campo.
    // añadido en la 0.2.2
    pub fn toast_responder(&mut self, id: u32) {
        if let Some((t, canvas)) = self.toasts.iter_mut().find(|(t, _)| t.id() == id) {
            t.empezar_respuesta();
            canvas.painted_once = false;
        }
        self.ordenar_pila();
    }

    /// El aviso que espera una respuesta escrita, si hay alguno.
    // añadido en la 0.2.2
    pub fn toast_respondiendo(&self) -> Option<u32> {
        self.toasts
            .iter()
            .find(|(t, _)| t.respondiendo())
            .map(|(t, _)| t.id())
    }

    /// Una tecla para el campo del aviso `id`.
    // añadido en la 0.2.2
    pub fn toast_tecla(&mut self, id: u32, tecla: TeclaPulsada) -> Option<toast::TeclaAviso> {
        let (t, canvas) = self.toasts.iter_mut().find(|(t, _)| t.id() == id)?;
        let resultado = t.tecla(tecla);
        // Lo escrito se ve: sin esto el aviso solo se repinta al animarse.
        canvas.painted_once = false;
        // Quitar el campo (Esc) cambia el alto de la tarjeta.
        self.ordenar_pila();
        resultado
    }

    /// El aviso `id` sigue al puntero mientras se arrastra.
    // añadido en la 0.2.2
    pub fn toast_arrastrar(&mut self, id: u32, dx: f32, dy: f32) {
        if let Some((t, _)) = self.toasts.iter_mut().find(|(t, _)| t.id() == id) {
            t.arrastrar(dx, dy);
        }
    }

    /// Suelta el arrastre del aviso `id`; `None` si no llegó al umbral y vuelve.
    // añadido en la 0.2.2
    pub fn toast_soltar_arrastre(&mut self, id: u32, vx: f32, vy: f32) -> Option<toast::Soltado> {
        let soltado = self
            .toasts
            .iter_mut()
            .find(|(t, _)| t.id() == id)
            .and_then(|(t, _)| t.soltar_arrastre(vx, vy));
        self.ordenar_pila();
        soltado
    }

    // 0.1.0
    pub fn toast_accion_relativa(&self, id: u32, x: f32, y: f32) -> Option<toast::Pulsado> {
        self.toast(id).and_then(|(t, _)| t.accion_en(x, y))
    }

    /// La acción `default` del aviso, que es lo que hace pulsar su cuerpo.
    // 0.1.0
    pub fn toast_accion_por_defecto(&self, id: u32) -> Option<String> {
        self.toast(id).and_then(|(t, _)| t.accion_por_defecto())
    }

    // 0.1.0
    pub fn toast_buffer_size(&self, id: u32) -> Option<(u32, u32)> {
        self.toast(id).map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn toast_logical_size(&self, id: u32) -> Option<(f32, f32)> {
        self.toast(id).map(|(_, c)| (c.size.width, c.size.height))
    }

    /// Manda el aviso a irse: es lo que hace pulsarlo. La notificación se queda
    /// en la lista —descartar el aviso no es haberla atendido—.
    // 0.1.0
    pub fn descartar_toast(&mut self, id: u32) -> bool {
        match self.toasts.iter_mut().find(|(t, _)| t.id() == id) {
            Some((t, _)) => {
                t.descartar();
                self.ordenar_pila();
                true
            }
            None => false,
        }
    }

    /// El aviso `id` pasa a la confirmación de un botón. `false` si ya no está.
    // añadido en la 0.2.2
    pub fn confirmar_toast_boton(&mut self, id: u32, clave: &str) -> bool {
        self.confirmar_toast(id, |t| t.confirmar_boton(clave))
    }

    /// El aviso `id` pasa a la confirmación de una respuesta enviada.
    // añadido en la 0.2.2
    pub fn confirmar_toast_respuesta(&mut self, id: u32, texto: &str) -> bool {
        self.confirmar_toast(id, |t| t.confirmar_respuesta(texto))
    }

    /// La confirmación cambia el alto del aviso: hay que rehacer su lienzo y
    /// el alto que heredan los de atrás.
    // añadido en la 0.2.2
    fn confirmar_toast(&mut self, id: u32, cambio: impl FnOnce(&mut toast::Toast)) -> bool {
        let Some((t, canvas)) = self.toasts.iter_mut().find(|(t, _)| t.id() == id) else {
            return false;
        };
        cambio(t);
        let (w, h) = t.size();
        let objetivo = Size::new(w, h);
        if canvas.size != objetivo {
            *canvas = Canvas::new(objetivo, self.escala);
        }
        canvas.painted_once = false;
        self.ordenar_pila();
        true
    }

    /// Reparte los niveles de la pila: el primero que no se está yendo va
    /// delante y cada uno de los siguientes un nivel más al fondo. Los de atrás
    /// toman el alto del de delante, y un aviso que cambia de nivel o de alto
    /// se repinta —y se recrea su lienzo si el tamaño ya no es el mismo—.
    // 0.1.0
    fn ordenar_pila(&mut self) {
        let alto_cabeza = self
            .toasts
            .iter()
            .find(|(t, _)| !t.saliendo())
            .map(|(t, _)| t.alto_propio());
        let mut nivel = 0;
        for (t, canvas) in &mut self.toasts {
            let (profundidad, fijo) = if t.saliendo() {
                (0, None)
            } else {
                nivel += 1;
                (nivel - 1, alto_cabeza.filter(|_| nivel > 1))
            };
            if !t.colocar(profundidad, fijo) {
                continue;
            }
            let (w, h) = t.size();
            let objetivo = Size::new(w, h);
            if canvas.size != objetivo {
                *canvas = Canvas::new(objetivo, self.escala);
            }
            canvas.painted_once = false;
        }
    }

    // --- Las barras de título ----------------------------------------------

    /// Prepara la barra de la ventana `id` para el ancho y el estado que se le
    /// pasan. Devuelve `true` si hay que volver a pintarla.
    ///
    /// El compositor llama a esto en cada frame, así que lo normal es que no
    /// haya nada que hacer: solo se marca para repintar cuando de verdad cambia
    /// algo que se ve —el ancho, el título, el foco o el botón señalado—.
    // 0.1.0
    pub fn barra_preparar(&mut self, id: u64, ancho: f32, estado: decoracion::Estado) -> bool {
        let escala = self.escala;
        match self.barras.get_mut(&id) {
            Some(barra) if barra.canvas.size.width == ancho && barra.canvas.scale == escala => {
                if barra.estado == estado {
                    return !barra.canvas.painted_once;
                }
                barra.estado = estado;
                barra.canvas.painted_once = false;
            }
            _ => {
                let canvas = Canvas::new(Size::new(ancho, decoracion::ALTO), escala);
                self.barras.insert(id, Barra { estado, canvas });
            }
        }
        true
    }

    // 0.1.0
    pub fn barra_buffer_size(&self, id: u64) -> Option<(u32, u32)> {
        self.barras.get(&id).map(|b| b.canvas.buffer_size())
    }

    /// Tamaño **lógico** de la barra ya cuadrado a píxel entero, que es el que
    /// el compositor tiene que darle a Smithay para que la textura no se
    /// reescale.
    // 0.1.0
    pub fn barra_logical_size(&self, id: u64) -> Option<(f32, f32)> {
        self.barras
            .get(&id)
            .map(|b| (b.canvas.size.width, b.canvas.size.height))
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_barra(&mut self, id: u64, buf: &mut [u8]) -> Vec<Damage> {
        let Some(barra) = self.barras.get_mut(&id) else {
            return Vec::new();
        };
        let vista = decoracion::vista(&barra.estado, barra.canvas.size.width);
        Self::paint(
            &mut barra.canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Tira las barras de las ventanas que ya no están. Sin esto, cada ventana
    /// cerrada dejaría su buffer —del ancho de la ventana— en el mapa para
    /// siempre.
    // 0.1.0
    pub fn barras_retener(&mut self, vivas: &[u64]) {
        self.barras.retain(|id, _| vivas.contains(id));
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_toast(&mut self, id: u32, buf: &mut [u8]) -> Vec<Damage> {
        let Some((t, canvas)) = self.toasts.iter_mut().find(|(t, _)| t.id() == id) else {
            return Vec::new();
        };
        let vista = t.view();
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Retira la del `id`, la haya cerrado quien la haya cerrado.
    // 0.1.0
    pub fn cerrar_notificacion(&mut self, id: u32) -> bool {
        if !self.notificaciones.cerrar(id) {
            return false;
        }
        self.sincronizar_notificaciones();
        true
    }

    /// Vacía la cola y devuelve los identificadores, que hay que cerrar por
    /// D-Bus uno a uno.
    // 0.1.0
    pub fn borrar_notificaciones(&mut self) -> Vec<u32> {
        let ids = self.notificaciones.vaciar();
        if !ids.is_empty() {
            self.sincronizar_notificaciones();
        }
        ids
    }

    /// Marca como leídas las notificaciones al abrir su historial.
    // 0.1.0
    pub fn marcar_notificaciones_leidas(&mut self) -> bool {
        if !self.notificaciones.marcar_leidas() {
            return false;
        }
        self.sincronizar_notificaciones();
        true
    }

    /// Abre la caja de respuesta de la notificación `id`. `false` si ya no está
    /// en la lista: la pudo cerrar su aplicación mientras se pulsaba el botón.
    // 0.1.0
    pub fn abrir_respuesta(&mut self, id: u32) -> bool {
        let Some(n) = self.notificaciones.lista().iter().find(|n| n.id == id) else {
            return false;
        };
        let emergente = Emergente::respuesta(id, &n.app, &n.resumen);
        self.abrir(emergente);
        true
    }

    // 0.1.0
    pub fn notificaciones(&self) -> &[notificaciones::Notificacion] {
        self.notificaciones.lista()
    }

    /// ¿Está «No molestar» puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque **caduca solo**: un temporizador
    /// que despierte al shell a la hora en punto para apagar un booleano es
    /// justo el despertar de más que este compositor evita.
    // 0.1.0
    pub fn poner_no_molestar(&mut self, enabled: bool) {
        self.silencio = enabled.then(|| (std::time::Instant::now(), None));
        if self.widgets.no_molestar(enabled) {
            self.panel.painted_once = false;
        }
        if let Some((Emergente::Notificaciones(n), canvas)) = self.emergente.as_mut() {
            n.poner_silencio(self.silencio);
            n.actualizar(self.notificaciones.lista().to_vec());
            canvas.painted_once = false;
        }
    }

    // 0.1.0
    pub fn notificaciones_silenciadas(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// Actualiza la lista de aplicaciones silenciadas en las notificaciones.
    // añadido en la 0.2.4
    pub fn poner_apps_silenciadas(&mut self, apps: Vec<String>) {
        self.apps_silenciadas = apps;
        if let Some((Emergente::Notificaciones(n), canvas)) = self.emergente.as_mut() {
            n.poner_silenciadas(self.apps_silenciadas.clone());
            canvas.painted_once = false;
        }
    }

    /// ¿Está silenciada esta aplicación? Comparación sin distinguir mayúsculas.
    // añadido en la 0.2.4
    pub fn app_silenciada(&self, app: &str) -> bool {
        self.apps_silenciadas
            .iter()
            .any(|s| s.eq_ignore_ascii_case(app))
    }

    /// Reparte la cola: el contador al widget del panel y la lista a la tarjeta
    /// si está abierta.
    // 0.1.0
    fn sincronizar_notificaciones(&mut self) {
        let cuantas = self.notificaciones.cuantas();
        if self.widgets.notificaciones(cuantas) {
            self.panel.painted_once = false;
        }
        let lista = self.notificaciones.lista().to_vec();
        if let Some((Emergente::Notificaciones(n), canvas)) = self.emergente.as_mut() {
            n.actualizar(lista);
            canvas.painted_once = false;
        }
    }

    /// Cambia el tema y el acento en caliente, y manda repintarlo todo.
    ///
    /// Los colores son un global del proceso y lo ya dibujado no se entera:
    /// hay que invalidar **todas** las superficies, no solo la que se está
    /// mirando. El dock y la pantalla de bloqueo se quedaban con la paleta
    /// anterior hasta que algo más los tocara.
    // 0.1.0
    // modified 0.2.3 -- no vacía la caché de iconos de aplicación al cambiar solo de paleta
    pub fn aplicar_apariencia(&mut self, tema_nuevo: tema::Tema, acento: tema::Acento) {
        let cambia_tema = tema::actual() != tema_nuevo;
        tema::aplicar(tema_nuevo);
        tema::aplicar_acento(acento);
        self.theme = view::theme();
        if cambia_tema {
            icono::refrescar_segun_tema();
            self.recargar_iconos_internos();
        }
        self.escritorio_canvas.painted_once = false;
        self.escritorio_canvas.cache = None;
        for (_, canvas) in &mut self.toasts {
            canvas.painted_once = false;
            canvas.cache = None;
        }
        for canvas in [
            Some(&mut self.panel),
            Some(&mut self.dock),
            self.emergente.as_mut().map(|(_, c)| c),
            self.bloqueo.as_mut().map(|(_, c)| c),
            self.osd.as_mut().map(|(_, c)| c),
            self.conmutador.as_mut().map(|(_, c)| c),
            self.actividad.as_mut().map(|(_, c)| c),
        ]
        .into_iter()
        .flatten()
        {
            canvas.painted_once = false;
            // Y la caché del layout de iced con ellos: guarda los elementos ya
            // construidos, con el color de antes dentro.
            canvas.cache = None;
        }
    }

    fn recargar_iconos_internos(&mut self) {
        self.dock_items.recargar_iconos();
        self.dock_revision = self.dock_revision.wrapping_add(1);
        self.escritorio.recargar_iconos();
        self.escritorio_canvas.painted_once = false;
        self.escritorio_canvas.cache = None;
        self.notificaciones.recargar_iconos();
        for (toast, canvas) in &mut self.toasts {
            toast.recargar_icono();
            canvas.painted_once = false;
            canvas.cache = None;
        }
        if let Some((emergente, canvas)) = self.emergente.as_mut() {
            emergente.recargar_iconos(self.notificaciones.lista());
            canvas.painted_once = false;
            canvas.cache = None;
        }
    }

    /// Recarga el tema de iconos del sistema y los lanzadores del dock cuando
    /// Ajustes cambia el estilo de iconos.
    // added 0.2.3
    pub fn recargar_iconos(&mut self) {
        icono::recargar_tema();
        self.recargar_iconos_internos();
        self.dock.painted_once = false;
        self.dock.cache = None;
        self.panel.painted_once = false;
        self.panel.cache = None;
    }

    /// Marca sucia únicamente la tarjeta de Apariencia cuando cambia la paleta
    /// extraída del fondo y el acento activo es fijo (solo cambia la muestra de
    /// franjas del acento automático dentro de la tarjeta).
    // added 0.2.3
    pub fn invalidar_apariencia(&mut self) {
        if let Some((Emergente::Apariencia(_), canvas)) = self.emergente.as_mut() {
            canvas.painted_once = false;
        }
    }

    /// Cambia el color del logo del panel al del canal de actualizaciones.
    // 0.1.0
    pub fn aplicar_canal(&mut self, canal: marca::Canal) {
        if canal == marca::canal_actual() {
            return;
        }
        marca::aplicar_canal(canal);
        self.panel.painted_once = false;
        self.panel.cache = None;
    }

    /// Cuánto falta para que algún widget tenga algo nuevo que enseñar.
    ///
    /// Es el mínimo de las alarmas de los widgets vivos. Antes esto era una
    /// función suelta que daba por hecho que el único con reloj era el reloj;
    /// ahora un widget nuevo con su propia cadencia entra sin tocar nada.
    // 0.1.0
    pub fn next_tick(&self) -> Option<std::time::Duration> {
        let widget = self.widgets.proxima_alarma();
        let emergente = self.emergente.as_ref().and_then(|(e, _)| e.proxima_alarma());
        match (widget, emergente) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }

    /// Si la emergente abierta ha pedido un despertar programado.
    pub fn emergente_tiene_alarma(&self) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.proxima_alarma().is_some())
    }

    /// Subsistemas de udev que hay que vigilar para este panel.
    // 0.1.0
    pub fn subsistemas(&self) -> Vec<&'static str> {
        let mut subsistemas = self.widgets.subsistemas();
        // También los usan las tarjetas y las teclas, aunque el widget no esté fijado.
        subsistemas.extend(["backlight", "leds"]);
        // El monitor de udev se monta una vez al arrancar, y Ajustes puede
        // añadir el widget de red con la sesión abierta: sin esto su icono no
        // se enteraría de una conexión hasta el siguiente cambio de minuto.
        subsistemas.push("net");
        subsistemas.sort_unstable();
        subsistemas.dedup();
        subsistemas
    }

    /// El resalte necesita fotogramas aunque deje de moverse el puntero;
    /// los cambios de contenido y de orden invalidan el lienzo directamente.
    // 0.1.0
    pub fn dock_needs_paint(&self) -> bool {
        !self.dock.painted_once || (!self.dock_en_capas && self.dock_items.animando())
    }

    /// ¿Se está moviendo algo dentro del dock?
    // 0.1.0
    pub fn dock_animando(&self) -> bool {
        self.dock_items.animando()
    }

    /// Ver [`Dock::lanzando`].
    // 0.1.1
    pub fn dock_lanzando(&mut self, exec: &str) -> bool {
        self.dock_items.lanzando(exec)
    }

    /// Señala el icono que haya en `punto`, en coordenadas **lógicas relativas
    /// al dock**. `None` = el puntero se ha ido del dock.
    ///
    /// Devuelve `true` si hay que repintar. El que llama tiene que acotar esto
    /// al rectángulo del dock: si cada movimiento del ratón por la pantalla
    /// llegara aquí, se pagaría el recorrido por nada.
    // 0.1.0
    pub fn dock_hover(&mut self, punto: Option<(f32, f32)>) -> bool {
        let item = punto.and_then(|(x, y)| self.dock_items.item_en(x, y));
        if !self.dock_items.set_hover(item) {
            return false;
        }
        if !self.dock_en_capas {
            self.dock.painted_once = false;
        }
        true
    }

    // --- Superficies emergentes --------------------------------------------

    /// Suelta la máscara de pantalla completa al cerrar la última modal.
    ///
    /// `renderer_modal` **no** se reinicia: medido con el ejemplo del
    /// Launchpad a 1440×900 escala 2, reiniciarlo ahorraba 4,1 MB pero cada
    /// apertura volvía a rasterizar glifos e iconos, con un primer dibujo de
    /// ~107 ms en vez de ~6. Su caché tampoco crece: iced descarta en cada
    /// `draw` lo que ese dibujo no usó.
    // 0.1.1
    fn limpiar_cache_modal(&mut self) {
        if self.emergente.is_none() && self.bloqueo.is_none() {
            self.mask = None;
        }
    }

    /// Abre una emergente, cerrando la que hubiera.
    // 0.1.0
    pub fn abrir(&mut self, mut emergente: Emergente) {
        // La tarjeta de notificaciones nace con la cola dentro. Va aquí y no
        // en quien la pide porque se abre por tres caminos —el atajo, el clic
        // en el panel y el paso de una tarjeta a otra— y solo el primero la
        // rellenaba: con el clic salía «No hay notificaciones» con cuatro
        // guardadas y el punto rojo encendido.
        if let Emergente::Notificaciones(n) = &mut emergente {
            self.marcar_notificaciones_leidas();
            // El silencio lo guarda el shell: la tarjeta se destruye al
            // cerrarla y volvería a nacer sin él.
            *n = emergente::notificaciones_con(self.silencio);
            n.poner_silenciadas(self.apps_silenciadas.clone());
            n.actualizar(self.notificaciones.lista().to_vec());
        }
        if let Emergente::Centro(c) = &mut emergente {
            c.poner_no_molestar(self.notificaciones_silenciadas());
        }
        let (w, h) = emergente.size();
        let canvas = Canvas::new(Size::new(w, h), self.escala);
        tracing::debug!(que = emergente.nombre(), w, h, "abriendo emergente");
        self.emergente = Some((emergente, canvas));
        // Los avisos caen en la misma esquina que las tarjetas del panel, van
        // delante y ganan el clic: con la de notificaciones abierta tapaban
        // justo el interruptor de «No molestar». Se van —con su salida— y lo
        // que decían sigue en la lista.
        for (t, _) in &mut self.toasts {
            if !t.saliendo() {
                t.descartar();
            }
        }
    }

    /// Abre la tarjeta de un widget del panel por su nombre. No hace nada si
    /// ese widget no tiene ninguna.
    // 0.1.0
    pub fn abrir_de_widget(&mut self, widget: &str) {
        let Some(constructor) = emergente_de(widget) else {
            return;
        };
        self.abrir(constructor());
    }

    /// Cierra la emergente abierta. `true` si había alguna.
    // 0.1.0
    // modified 0.1.1 -- suelta la caché del renderer modal al cerrar
    pub fn cerrar_emergente(&mut self) -> bool {
        let cerrada = self.emergente.take().is_some();
        if cerrada {
            self.limpiar_cache_modal();
        }
        cerrada
    }

    /// Extrae la contraseña actual del diálogo de Polkit si coincide su `id`.
    pub fn tomar_polkit_clave(&mut self, id: u64) -> Option<Vec<u8>> {
        let (Emergente::Polkit(p), canvas) = self.emergente.as_mut()? else {
            return None;
        };
        if p.id() != id {
            return None;
        }
        let bytes = p.tomar_clave();
        canvas.painted_once = false;
        Some(bytes)
    }

    /// Muestra o retira un mensaje informativo en el diálogo de Polkit (por ejemplo, huella).
    pub fn polkit_info(&mut self, id: u64, texto: Option<String>) -> bool {
        let Some((Emergente::Polkit(p), canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if p.id() != id || !p.poner_info(texto) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Muestra un error de autenticación en Polkit y devuelve el control para reintentar.
    pub fn polkit_error(&mut self, id: u64, texto: String) -> bool {
        let Some((Emergente::Polkit(p), canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if p.id() != id || !p.poner_error(texto) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Actualiza la etiqueta y el modo de eco del campo en Polkit según pida PAM.
    pub fn polkit_prompt(&mut self, id: u64, prompt: Option<String>, oculto: bool) -> bool {
        let Some((Emergente::Polkit(p), canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if p.id() != id || !p.poner_prompt(prompt, oculto) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Actualiza el aviso de Bloq Mayús si el diálogo de Polkit está abierto.
    pub fn polkit_caps_lock(&mut self, activo: bool) -> bool {
        let Some((Emergente::Polkit(p), canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if !p.poner_caps_lock(activo) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Cierra el diálogo de Polkit solo si el abierto corresponde a `id`.
    pub fn cerrar_polkit_si(&mut self, id: u64) -> bool {
        if let Some((Emergente::Polkit(p), _)) = self.emergente.as_ref()
            && p.id() == id
        {
            return self.cerrar_emergente();
        }
        false
    }

    /// Identificador del diálogo de Polkit abierto, si hay uno en pantalla.
    pub fn polkit_id(&self) -> Option<u64> {
        match self.emergente.as_ref() {
            Some((Emergente::Polkit(p), _)) => Some(p.id()),
            _ => None,
        }
    }

    /// Identificador del diálogo de permisos `Access` abierto, si corresponde.
    pub fn acceso_id(&self) -> Option<u32> {
        match self.emergente.as_ref() {
            Some((Emergente::Acceso(a), _)) => Some(a.id()),
            _ => None,
        }
    }

    /// Identificador del diálogo «Abrir con…» abierto, si corresponde.
    pub fn elegir_app_id(&self) -> Option<u32> {
        match self.emergente.as_ref() {
            Some((Emergente::ElegirApp(e), _)) => Some(e.id()),
            _ => None,
        }
    }

    /// Actualiza la lista de aplicaciones sugeridas del diálogo «Abrir con…».
    pub fn elegir_app_actualizar(&mut self, id: u32, opciones: Vec<String>) -> bool {
        let Some((Emergente::ElegirApp(e), canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if e.id() != id || !e.actualizar_opciones(opciones) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// El nombre de la emergente abierta, para las trazas y las pruebas.
    // 0.1.0
    pub fn emergente_nombre(&self) -> Option<&'static str> {
        self.emergente.as_ref().map(|(e, _)| e.nombre())
    }

    // 0.1.0
    pub fn hay_emergente(&self) -> bool {
        self.emergente.is_some()
    }

    /// Tamaño **lógico** de la emergente abierta, y dónde quiere colocarse.
    /// El tercer campo es el alto con el que hay que **colocarla** cuando no es
    /// el que mide: ver [`Emergente::alto_estable`].
    // 0.1.0
    pub fn emergente_geometria(&self) -> Option<((i32, i32), Ancla, Option<i32>)> {
        let (e, canvas) = self.emergente.as_ref()?;
        Some((
            (canvas.size.width as i32, canvas.size.height as i32),
            e.ancla(),
            e.alto_estable().map(|h| h as i32),
        ))
    }

    /// Rehace el lienzo si la tarjeta ha cambiado de alto sola, a mitad de una
    /// animación (el bloque de calidad del centro crece con su muelle). Hasta
    /// ahora solo se ajustaba al pulsar, y lo que crecía fotograma a fotograma
    /// se pintaba fuera del búfer. Sin cambio de tamaño no hace nada.
    // añadido en la 0.2.0
    pub fn ajustar_emergente(&mut self) {
        // añadido en la 0.2.2 -- el buscador se cierra solo al acabar su confirmación.
        if matches!(&self.emergente, Some((Emergente::Buscador(b), _)) if b.confirmacion_vencida()) {
            self.cerrar_emergente();
            return;
        }
        if let Some((e, canvas)) = self.emergente.as_mut() {
            Self::ajustar(e, canvas);
        }
    }

    // añadido en la 0.2.0
    pub fn emergente_alto_visible(&self) -> Option<f32> {
        self.emergente.as_ref()?.0.alto_visible()
    }

    // 0.1.0
    pub fn emergente_buffer_size(&self) -> Option<(u32, u32)> {
        self.emergente.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn emergente_needs_paint(&self) -> bool {
        self.emergente
            .as_ref()
            // Con una animación de contenido en marcha hay que repintar en cada
            // fotograma: lo que se mueve está **dentro** del buffer, así que no
            // basta con que el compositor recoloque la superficie.
            .is_some_and(|(e, c)| !c.painted_once || e.animando())
    }

    /// ¿Está la emergente animando su contenido? Lo consulta el compositor para
    /// no dormirse a mitad de la transición.
    // 0.1.0
    pub fn emergente_animando(&self) -> bool {
        self.emergente.as_ref().is_some_and(|(e, _)| e.animando())
    }

    /// Mueve el puntero sobre la emergente. `punto` es relativo a su esquina;
    /// `None` cuando está fuera. Devuelve `true` si hay que repintar.
    // 0.1.0
    pub fn emergente_puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        let pagina = |e: &Emergente| match e {
            Emergente::Launchpad(l) => Some(l.pagina()),
            _ => None,
        };
        let antes = pagina(e);
        if !e.puntero(punto) {
            return false;
        }
        // Una emergente con realce propio resuelve el hover moviendo esa
        // superficie, sin tocar su buffer. Las demás son pequeñas y se repintan
        // enteras, que sale más barato que llevar la cuenta. La excepción es el
        // launchpad pasando de página con un icono arrastrado contra el borde:
        // ahí cambia la rejilla entera.
        if !e.usa_realce() || pagina(e) != antes {
            canvas.painted_once = false;
        }
        true
    }

    /// Pulsación sobre la emergente, en coordenadas relativas a su esquina.
    // 0.1.0
    // modified 0.1.1 -- suelta la caché del renderer modal al cerrar
    // modificado en la 0.2.0 -- el buscador se cierra tras cualquier acción
    pub fn emergente_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (e, canvas) = self.emergente.as_mut()?;
        let accion = e.pulsar(x, y);
        // El silencio se lo queda el shell en cuanto se toca: es lo único de la
        // tarjeta que tiene que sobrevivirla. Se recoge aquí, con `e` todavía a
        // mano, y se guarda al final: `self` está prestado hasta entonces.
        let silencio = match e {
            Emergente::Notificaciones(n) => Some(n.silencio()),
            _ => None,
        };
        // Una pulsación también puede cambiar estado sin producir una acción
        // (el segundo clic sobre un nombre entra en edición).
        canvas.painted_once = false;
        // Hay tarjetas que crecen con lo que se pulsa: «No molestar» despliega
        // sus cuatro duraciones. Sin esto el contenido nuevo se pintaba fuera
        // del buffer y desaparecía —el canvas se hizo al abrir y nadie lo
        // volvía a mirar.
        Self::ajustar(e, canvas);
        // Elegir algo del menú lo cierra, como cualquier menú. Pulsar en un
        // hueco no: ahí el usuario ha fallado la puntería, no ha decidido nada.
        if accion.as_ref().is_some_and(|a| !a.conserva_en(e)) {
            self.emergente = None;
            self.limpiar_cache_modal();
        } else if accion.is_some() {
            canvas.painted_once = false;
        }
        if let Some(silencio) = silencio {
            self.silencio = silencio;
        }
        accion
    }

    // añadido en la 0.2.0 -- preguntar no altera selección, volumen ni configuración.
    pub fn emergente_arrastre_en(&self, x: f32, y: f32) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.arrastre_en(x, y))
    }

    // añadido en la 0.2.0 -- una cancelación no debe producir la acción de soltar un icono.
    pub fn cancelar_arrastre_launchpad(&mut self) {
        if let Some((Emergente::Launchpad(l), _)) = self.emergente.as_mut() {
            l.cancelar_arrastre();
        }
    }

    /// Desplazamiento sobre la emergente abierta. `true` si hay que repintar.
    // 0.1.0
    pub fn emergente_desplazar(&mut self, dx: f32, dy: f32) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if !e.desplazar(dx, dy) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Rehace el buffer si la emergente ha cambiado de tamaño.
    ///
    /// Hay tarjetas que crecen con lo que se hace dentro: «No molestar»
    /// despliega sus cuatro duraciones al pulsarla, y el buscador cambia de
    /// alto con cada tecla según cuántos resultados haya. Sin esto el contenido
    /// nuevo se pinta fuera del buffer y desaparece —el canvas se hizo al abrir
    /// y nadie lo volvía a mirar.
    // 0.1.0
    fn ajustar(e: &Emergente, canvas: &mut Canvas) {
        let (w, h) = e.size();
        // Cuadrado a píxel entero antes de comparar: `Canvas::new` lo hace por
        // dentro, y sin cuadrarlo aquí el tamaño pedido nunca coincidiría con
        // el guardado y se tiraría la caché de iced en cada interacción.
        let objetivo = Size::new(
            a_pixel_entero(w, canvas.scale),
            a_pixel_entero(h, canvas.scale),
        );
        if canvas.size != objetivo {
            *canvas = Canvas::new(objetivo, canvas.scale);
        }
    }

    /// Una tecla para la emergente. Devuelve si la ha consumido y, si toca,
    /// qué hay que ejecutar.
    // 0.1.0
    // modified 0.1.1 -- suelta la caché del renderer modal al cerrar
    // modificado en la 0.2.0 -- el buscador se cierra tras cualquier acción
    pub fn emergente_tecla(&mut self, tecla: TeclaPulsada) -> (bool, Option<Accion>) {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return (false, None);
        };
        match e.tecla(tecla) {
            emergente::Tecla::Cerrar => {
                self.emergente = None;
                self.limpiar_cache_modal();
                (true, None)
            }
            emergente::Tecla::Consumida => {
                Self::ajustar(e, canvas);
                canvas.painted_once = false;
                (true, None)
            }
            // Lanzar algo cierra la emergente, igual que elegirlo con el ratón.
            emergente::Tecla::Hacer(accion) => {
                if accion.conserva_en(e) {
                    canvas.painted_once = false;
                } else {
                    self.emergente = None;
                    self.limpiar_cache_modal();
                }
                (true, Some(accion))
            }
            emergente::Tecla::Ignorada => (false, None),
        }
    }

    /// El velo a pantalla completa que pide la emergente abierta, en RGBA.
    // 0.1.0
    pub fn emergente_velo(&self) -> Option<[f32; 4]> {
        let (e, _) = self.emergente.as_ref()?;
        let c = e.velo()?;
        Some([c.r, c.g, c.b, c.a])
    }

    /// ¿Tapa la emergente abierta la pantalla entera?
    /// ¿La emergente abierta quiere cristal esmerilado debajo?
    // 0.1.0
    pub fn emergente_usa_cristal(&self) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.usa_cristal())
    }

    /// El compositor recortó una emergente anclada a un punto para que no se
    /// saliera de la pantalla, y aquí se apunta dónde quedó. Al arrastrar el
    /// selector de emojis contra un borde el ancla seguía sumando lo que el
    /// puntero se pasaba, y al volver había que deshacer ese exceso invisible
    /// antes de que la tarjeta se moviera: parecía pegada al borde.
    /// `true` si la emergente se lo apuntó: solo la que se arrastra lo hace.
    // 0.1.1
    pub fn emergente_recortada(&mut self, x: f32, y: f32) -> bool {
        let Some((emergente, _)) = self.emergente.as_mut() else {
            return false;
        };
        match emergente {
            emergente::Emergente::Emojis(e) => {
                e.recortar_ancla(x, y);
                true
            }
            emergente::Emergente::Portapapeles(p) => {
                p.recortar_ancla(x, y);
                true
            }
            _ => false,
        }
    }

    pub fn actualizar_portapapeles(&mut self, elementos: Vec<ElementoPortapapeles>) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        let cambio = e.actualizar_portapapeles(elementos);
        if cambio {
            canvas.painted_once = false;
        }
        cambio
    }

    // 0.1.0
    pub fn emergente_tapa_la_pantalla(&self) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.tapa_la_pantalla())
    }

    /// El realce de lo señalado dentro de la emergente: `(x, y, ancho, alto)`
    /// lógicos relativos a su esquina, y si lleva marco de acento.
    // 0.1.0
    pub fn emergente_realce(&self) -> Option<(f32, f32, f32, f32, bool)> {
        let (e, _) = self.emergente.as_ref()?;
        let (r, marco) = e.realce()?;
        Some((r.x, r.y, r.width, r.height, marco))
    }

    /// Rectángulos relativos al buffer reservados para las previsualizaciones.
    // 0.1.0
    pub fn escritorios_miniaturas(&self) -> Vec<Rectangle> {
        self.emergente
            .as_ref()
            .map(|(e, _)| e.miniaturas_escritorios())
            .unwrap_or_default()
    }

    /// Actualiza la vista general tras crear, borrar, renombrar o cambiar.
    // 0.1.0
    pub fn actualizar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        let cambio = e.actualizar_escritorios(activo, nombres);
        if cambio {
            canvas.painted_once = false;
        }
        cambio
    }

    /// El compositor conserva la superficie anterior para animar el cambio.
    // 0.1.0
    pub fn launchpad_transicion(&mut self) -> Option<emergente::Transicion> {
        self.emergente.as_mut()?.0.tomar_transicion()
    }

    /// ¿El realce tiene que ir **por delante** del buffer de la emergente?
    ///
    /// Solo dentro de una carpeta del launchpad: su panel es opaco —es un
    /// diálogo— y detrás de él el realce no se ve. Por delante se cuela un 6 %
    /// de tinta sobre el icono señalado, que no se nota; detrás, no hay
    /// realce que valga.
    // 0.1.0
    pub fn realce_delante(&self) -> bool {
        matches!(
            self.emergente.as_ref(),
            Some((Emergente::Launchpad(l), _)) if l.en_carpeta()
        )
    }

    /// Pinta el realce en `buf`, que debe medir lo que diga
    /// [`Shell::emergente_realce`].
    ///
    /// Es una superficie propia y diminuta: repintarla cuesta lo que una celda,
    /// no lo que la rejilla entera, y mover el ratón solo cambia su posición.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_realce(&mut self, buf: &mut [u8], ancho: f32, alto: f32, marco: bool) {
        use iced_core::{Border, Length};
        use iced_widget::{Space, container};

        let vista: view::PanelElement<'_> = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(
                    if marco {
                        Color {
                            a: 0.18,
                            ..tema::acento()
                        }
                    } else {
                        tema::hover()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: if marco { 2.0 } else { 0.0 },
                    color: Color {
                        a: 0.7,
                        ..tema::acento()
                    },
                },
                ..Default::default()
            })
            .into();

        let escala = self.escala;
        let mut canvas = Canvas::new(Size::new(ancho, alto), escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        );
    }

    /// Pinta la emergente. `buf` debe tener el tamaño de
    /// [`Shell::emergente_buffer_size`].
    // 0.1.0
    // modified 0.1.1 -- pinta con el renderer modal y la máscara reutilizable
    pub fn draw_emergente(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return Vec::new();
        };
        // añadido en la 0.2.2 -- los archivos del buscador llegan de un hilo.
        if let Emergente::Buscador(b) = e {
            b.recoger();
        }
        let fondo = e.fondo().unwrap_or(Color::TRANSPARENT);
        let view = e.view();
        Self::paint_con_fondo(
            canvas,
            &mut self.renderer_modal,
            &mut self.mask,
            &self.theme,
            view,
            buf,
            fondo,
        )
    }

    /// Qué pasa al pulsar en el panel, en coordenadas lógicas de pantalla.
    ///
    /// Las zonas son aritméticas, como en el dock: el nombre a la izquierda
    /// abre el menú y el reloj del centro abrirá el calendario. Son las mismas
    /// que dibuja [`view::panel`], y por eso los anchos viven ahí.
    // 0.1.0
    pub fn panel_pulsado(&mut self, x: f32, _y: f32) -> Option<Accion> {
        // El indicador de escritorios no abre ninguna tarjeta: el punto que se
        // pulsa es el escritorio al que se va, así que sale por otro camino
        // —una acción para el compositor— antes de mirar las emergentes.
        if let Some((nombre, x0, _)) = self
            .zonas_panel()
            .into_iter()
            .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
            && let Some(accion) = self.widgets.pulsar(nombre, (x - x0) / self.panel_zoom())
        {
            return Some(accion);
        }
        // El logo no es un widget: es zona fija a la izquierda del todo.
        let que: Option<fn() -> Emergente> = if x <= view::ANCHO_LOGO * self.panel_zoom() {
            Some(Emergente::menu)
        } else {
            // Y el resto sale de dónde dice cada widget que se ha dibujado, en
            // vez de compararse con dos posiciones fijas: así responden todos y
            // no solo el reloj, y siguen respondiendo si cambian de orden en la
            // configuración.
            let nombre = self
                .zonas_panel()
                .into_iter()
                .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
                .map(|(nombre, _, _)| nombre);
            nombre.and_then(emergente_de)
        };
        // Pulsar el mismo sitio con su emergente ya abierta la cierra, como
        // cualquier barra de menús. Pulsar *otro* sitio cambia de emergente.
        if let Some(que) = que {
            let abierta = self.emergente.as_ref().map(|(e, _)| e.nombre());
            let nueva = que();
            if abierta == Some(nueva.nombre()) {
                self.cerrar_emergente();
            } else {
                self.abrir(nueva);
            }
        }
        None
    }

    /// Nombre de la tarjeta que corresponde a una coordenada del panel.
    /// Permite al compositor encadenar la salida de una tarjeta con la entrada
    /// de otra sin construir la nueva encima de la textura anterior.
    // 0.1.0
    pub fn objetivo_emergente_panel(&self, x: f32) -> Option<(&'static str, &'static str)> {
        if x <= view::ANCHO_LOGO * self.panel_zoom() {
            return Some(("menu", "menu"));
        }
        self.zonas_panel()
            .into_iter()
            .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
            .and_then(|(widget, _, _)| {
                nombre_emergente_de(widget).map(|emergente| (widget, emergente))
            })
    }

    // 0.1.0
    pub fn abrir_emergente_nombre(&mut self, nombre: &'static str) -> bool {
        let nueva = if nombre == "menu" {
            Some(Emergente::menu())
        } else {
            emergente_de(nombre).map(|f| f())
        };
        if let Some(nueva) = nueva {
            self.abrir(nueva);
            true
        } else {
            false
        }
    }

    /// Lo que el compositor sabe de los escritorios y el panel no puede saber.
    ///
    /// `true` si hay que repintar. Se llama en cada cambio de escritorio, no en
    /// cada frame: el widget compara y calla si no ha cambiado nada.
    // 0.1.0
    pub fn escritorios(&mut self, activo: usize, cuantos: usize) -> bool {
        self.escritorios_vistos = (activo, cuantos);
        let cambio = self.widgets.escritorios(activo, cuantos);
        if cambio {
            self.panel.painted_once = false;
        }
        cambio
    }

    /// El puntero se mueve por el panel, con `x` en lógicos, o se ha ido de él.
    /// `true` si hay que repintar el panel.
    // 0.1.0
    pub fn panel_puntero(&mut self, x: Option<f32>) -> bool {
        let nombre = x.and_then(|x| {
            self.zonas_panel()
                .into_iter()
                .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
                .map(|(nombre, _, _)| nombre)
        });
        let cambio = self.widgets.señalar(nombre);
        if cambio {
            self.panel.painted_once = false;
        }
        cambio
    }

    /// Resalta en el panel el widget cuya tarjeta está abierta. `true` si hay
    /// que repintar el panel.
    // 0.1.0
    pub fn panel_resalte(&mut self) -> bool {
        let widget = self.emergente_nombre().and_then(|abierta| {
            self.zonas_panel()
                .into_iter()
                .map(|(nombre, _, _)| nombre)
                .find(|nombre| nombre_emergente_de(nombre) == Some(abierta))
        });
        let cambio = self.widgets.abrir(widget);
        if cambio {
            self.panel.painted_once = false;
        }
        cambio
    }

    /// Dónde cae cada widget del panel, en lógicos: `(nombre, x0, x1)`.
    ///
    /// Es lo que convierte un clic en el panel en "has pulsado el volumen".
    // 0.1.0
    // modificado en la 0.2.0 -- de maqueta a lógicos de pantalla, con el zoom del panel
    pub fn zonas_panel(&self) -> Vec<(&'static str, f32, f32)> {
        let zoom = self.panel_zoom();
        self.widgets
            .zonas_derecha(self.panel.size.width, view::MARGEN_PANEL, view::HUECO)
            .into_iter()
            .map(|(nombre, x0, x1)| (nombre, x0 * zoom, x1 * zoom))
            .collect()
    }

    /// El botón se ha soltado. `true` si hay que repintar.
    // 0.1.0
    // modified 0.1.1 -- suelta la caché del renderer modal al cerrar
    // modificado en la 0.2.0 -- el buscador se cierra tras cualquier acción
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return (false, None);
        };
        let (repintar, accion) = e.soltar();
        if repintar {
            canvas.painted_once = false;
        }
        // Lanzar algo cierra el launchpad, igual que hacía cuando la acción
        // salía de `pulsar`.
        if accion.as_ref().is_some_and(|a| !a.conserva_en(e)) {
            self.emergente = None;
            self.limpiar_cache_modal();
        }
        (repintar, accion)
    }

    /// ¿Se enseña el dock con el launchpad abierto? Lo decide la configuración.
    // 0.1.0
    pub fn dock_en_launchpad(&self) -> bool {
        self.launchpad_dock
    }

    /// Le dice al launchpad dónde está el dock, en coordenadas de pantalla.
    ///
    /// `None` mientras no se vea: sin esto, con el dock oculto se seguiría
    /// anclando al soltar un icono en la franja de abajo, donde no hay nada.
    // 0.1.0
    pub fn launchpad_zona_dock(&mut self, zona: Option<(f32, f32, f32, f32)>) {
        if let Some((Emergente::Launchpad(l), _)) = self.emergente.as_mut() {
            l.poner_zona_dock(zona.map(|(x, y, w, h)| iced_core::Rectangle {
                x,
                y,
                width: w,
                height: h,
            }));
        }
    }

    /// Los primeros elementos visibles del launchpad. Para el autotest.
    // 0.1.0
    pub fn launchpad_primeros(&self, n: usize) -> Vec<String> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.primeros(n),
            _ => Vec::new(),
        }
    }

    /// El icono que se arrastra por el launchpad: `(x, y, lado)` lógicos
    /// relativos a su esquina. `None` si no hay ninguno en el aire.
    // 0.1.0
    pub fn launchpad_agarrado(&self) -> Option<(f32, f32, f32)> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.rect_agarrado().map(|r| (r.x, r.y, r.width)),
            _ => None,
        }
    }

    /// Pinta el icono agarrado. `buf` debe tener el tamaño que corresponde a
    /// `lado` con la escala del shell.
    // 0.1.0
    // modified 0.1.1 -- pinta también el icono arrastrado desde el dock
    pub fn draw_agarrado(&mut self, buf: &mut [u8], lado: f32) {
        let vista = if let Some(vista) = self.dock_items.vista_agarrado() {
            vista
        } else {
            let Some((Emergente::Launchpad(l), _)) = self.emergente.as_ref() else {
                return;
            };
            let Some(vista) = l.vista_agarrado() else {
                return;
            };
            vista
        };
        let escala = self.escala;
        let mut canvas = Canvas::new(Size::new(lado, lado), escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        );
    }

    /// El centro de una celda del launchpad, en coordenadas de pantalla. Para
    /// el autotest.
    // 0.1.0
    pub fn launchpad_celda(&self, i: usize) -> Option<(f32, f32)> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.centro_de_celda(i),
            _ => None,
        }
    }

    /// El chip del selector de color de la carpeta abierta y un tono de su
    /// tira, en coordenadas del launchpad. Para el autotest.
    // 0.1.0
    pub fn launchpad_selector(&self, grados: f32) -> Option<((f32, f32), (f32, f32))> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => {
                Some((l.centro_selector(), l.centro_tono(grados)))
            }
            _ => None,
        }
    }

    /// Alterna el modo edición del launchpad, si es lo que está abierto.
    // 0.1.0
    pub fn launchpad_editar(&mut self) -> bool {
        match self.emergente.as_mut() {
            Some((Emergente::Launchpad(l), canvas)) => {
                canvas.painted_once = false;
                l.alternar_edicion()
            }
            _ => false,
        }
    }

    /// ¿La emergente abierta tiene algo agarrado?
    // 0.1.0
    pub fn emergente_agarrada(&self) -> bool {
        self.emergente.as_ref().is_some_and(|(e, _)| e.agarrado())
    }

    /// La hora del bloqueo: solo `H:mm`, sin fecha. En el panel la fecha ayuda
    /// —es donde se mira de reojo—, pero en una pantalla que se lee de lejos
    /// sobra todo lo que no sea la hora.
    // 0.1.0
    pub fn hora_bloqueo() -> String {
        state::local_hhmm()
    }

    /// La fecha larga que va bajo el reloj del bloqueo: «lunes, 17 de agosto».
    ///
    /// Aquí sí, al contrario que en el reloj del panel: la pantalla de bloqueo
    /// se mira al volver a un equipo que llevaba horas parado, y es el momento
    /// del día en que uno menos sabe qué día es.
    // 0.1.0
    pub fn fecha_bloqueo() -> String {
        let hoy = state::Fecha::hoy();
        if emergente::t("es", "en") == "es" {
            const DIAS: [&str; 7] = [
                "lunes",
                "martes",
                "miércoles",
                "jueves",
                "viernes",
                "sábado",
                "domingo",
            ];
            const MESES: [&str; 12] = [
                "enero",
                "febrero",
                "marzo",
                "abril",
                "mayo",
                "junio",
                "julio",
                "agosto",
                "septiembre",
                "octubre",
                "noviembre",
                "diciembre",
            ];
            format!(
                "{}, {} de {}",
                DIAS[hoy.dia_semana() as usize],
                hoy.dia,
                MESES[(hoy.mes - 1) as usize]
            )
        } else {
            const DAYS: [&str; 7] = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            format!(
                "{}, {} {}",
                DAYS[hoy.dia_semana() as usize],
                MONTHS[(hoy.mes - 1) as usize],
                hoy.dia
            )
        }
    }

    /// Enseña un aviso: el icono, su nivel y, si no lo tiene, un texto.
    ///
    /// Sustituye al que hubiera: subir el volumen dos veces seguidas no apila
    /// dos tarjetas, reinicia la misma.
    /// Enseña el aviso. Si ya hay uno de la misma clase a la vista, lo
    /// reaprovecha: mueve su barra y le reinicia el tiempo en vez de apilar
    /// otra cápsula encima.
    // 0.1.0
    pub fn mostrar_osd(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) {
        if let Some((o, canvas)) = self.osd.as_mut() {
            // El ancho depende del texto, así que un aviso reaprovechado puede
            // pedir otro tamaño de buffer; si cambia, se hace uno nuevo.
            if o.actualizar(icono, nivel, texto.clone()) {
                let (w, h) = o.size();
                let objetivo = Size::new(
                    a_pixel_entero(w, canvas.scale),
                    a_pixel_entero(h, canvas.scale),
                );
                if canvas.size != objetivo {
                    *canvas = Canvas::new(Size::new(w, h), canvas.scale);
                } else {
                    canvas.painted_once = false;
                }
                return;
            }
        }
        let osd = osd::Osd::new(icono, nivel, texto);
        let (w, h) = osd.size();
        let canvas = Canvas::new(Size::new(w, h), self.escala);
        self.osd = Some((osd, canvas));
    }

    /// La escala con la que se compone el aviso: entra creciendo.
    // 0.1.0
    pub fn osd_escala(&self) -> f32 {
        self.osd.as_ref().map_or(1.0, |(o, _)| o.escala())
    }

    /// ¿Se está moviendo algo del aviso? La desaparición no cuenta: es alfa, y
    /// va con un solo despertar programado.
    // 0.1.0
    pub fn osd_animando(&self) -> bool {
        self.osd.as_ref().is_some_and(|(o, _)| o.animando())
    }

    /// ¿Hace falta repintar el buffer del aviso? Mientras la barra se mueve, sí.
    // 0.1.0
    pub fn osd_needs_paint(&self) -> bool {
        self.osd
            .as_ref()
            .is_some_and(|(o, c)| !c.painted_once || o.animando())
    }

    /// El aviso, si sigue a la vista. Se retira solo al agotarse su tiempo.
    // 0.1.0
    pub fn osd_vivo(&mut self) -> bool {
        if self.osd.as_ref().is_some_and(|(o, _)| o.queda().is_none()) {
            self.osd = None;
        }
        self.osd.is_some()
    }

    /// Cuánto falta para que el aviso desaparezca, para programar el despertar.
    // 0.1.0
    pub fn osd_queda(&self) -> Option<std::time::Duration> {
        self.osd.as_ref().and_then(|(o, _)| o.queda())
    }

    // 0.1.0
    pub fn osd_alfa(&self) -> f32 {
        self.osd.as_ref().map(|(o, _)| o.alfa()).unwrap_or(0.0)
    }

    // 0.1.0
    pub fn osd_buffer_size(&self) -> Option<(u32, u32)> {
        self.osd.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn osd_logical_size(&self) -> Option<(f32, f32)> {
        self.osd
            .as_ref()
            .map(|(_, c)| (c.size.width, c.size.height))
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_osd(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((osd, canvas)) = self.osd.as_mut() else {
            return Vec::new();
        };
        let vista = osd.view();
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Saca el teclado, o lo hace volver si se estaba yendo. `ancho` es el de
    /// la pantalla en lógicos: va de lado a lado.
    // 0.1.1
    pub fn mostrar_teclado(&mut self, ancho: f32, capa: teclado::Capa) {
        if let Some((t, c)) = self.teclado.as_mut()
            && t.size().0 == ancho
        {
            if t.mostrar(capa) {
                c.painted_once = false;
            }
            return;
        }
        let t = teclado::Teclado::new(ancho, capa);
        let (w, h) = t.size();
        let canvas = Canvas::new(Size::new(w, h), self.escala);
        self.teclado = Some((t, canvas));
    }

    // 0.1.1
    pub fn ocultar_teclado(&mut self) {
        if let Some((t, _)) = self.teclado.as_mut() {
            t.ocultar();
        }
    }

    /// Hay teclado y no se está yendo.
    // 0.1.1
    pub fn teclado_abierto(&self) -> bool {
        self.teclado.as_ref().is_some_and(|(t, _)| !t.saliendo())
    }

    /// El teclado, si sigue a la vista. Lo tira cuando acaba de irse.
    // 0.1.1
    pub fn teclado_vivo(&mut self) -> bool {
        if self.teclado.as_ref().is_some_and(|(t, _)| t.terminado()) {
            self.teclado = None;
        }
        self.teclado.is_some()
    }

    // 0.1.1
    pub fn teclado_animando(&self) -> bool {
        self.teclado.as_ref().is_some_and(|(t, _)| t.animando())
    }

    /// Lo que tapa ahora mismo del borde de abajo, en lógicos.
    // 0.1.1
    pub fn teclado_asoma(&self) -> f32 {
        self.teclado.as_ref().map_or(0.0, |(t, _)| t.asoma())
    }

    // 0.1.1
    pub fn teclado_needs_paint(&self) -> bool {
        self.teclado.as_ref().is_some_and(|(_, c)| !c.painted_once)
    }

    /// Un dedo sobre el teclado, en lógicos relativos a él.
    // 0.1.1
    pub fn teclado_pulsar(&mut self, x: f32, y: f32) -> Option<teclado::Accion> {
        let (t, c) = self.teclado.as_mut()?;
        c.painted_once = false;
        t.pulsar(x, y)
    }

    // 0.1.1
    pub fn teclado_soltar(&mut self) -> Option<teclado::Accion> {
        let (t, c) = self.teclado.as_mut()?;
        c.painted_once = false;
        t.soltar()
    }

    // 0.1.1
    pub fn teclado_buffer_size(&self) -> Option<(u32, u32)> {
        self.teclado.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.1
    pub fn teclado_logical_size(&self) -> Option<(f32, f32)> {
        self.teclado.as_ref().map(|(t, _)| t.size())
    }

    /// Hay que repintarlo entero al cambiar de tema o de acento: sus colores
    /// salen de los dos.
    // 0.1.1
    pub fn teclado_invalidar(&mut self) {
        if let Some((_, c)) = self.teclado.as_mut() {
            c.painted_once = false;
        }
    }

    // 0.1.1
    pub fn draw_teclado(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((t, canvas)) = self.teclado.as_mut() else {
            return Vec::new();
        };
        // Se limpia con el fondo opaco del teclado en vez de pintarlo como un
        // contenedor: ver `paint_con_fondo`.
        let fondo = t.fondo();
        let vista = t.view();
        Self::paint_con_fondo(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
            fondo,
        )
    }

    /// Cambia la preferencia de efectos y la aplica. Devuelve la elegida, que
    /// es lo que el compositor tiene que escribir en el fichero.
    ///
    /// Lo elegido y lo efectivo no son lo mismo: con la batería baja el
    /// escritorio ya está en reducidos, y volver a «completos» desde aquí
    /// guarda la preferencia aunque no se vea hasta que haya corriente.
    // 0.1.0
    pub fn alternar_efectos(&mut self) -> config::Efectos {
        self.efectos_config = match self.efectos_config {
            config::Efectos::Completos => config::Efectos::Reducidos,
            config::Efectos::Reducidos => config::Efectos::Completos,
        };
        self.revisar_efectos();
        self.efectos_config
    }

    // 0.1.0
    pub fn aplicar_efectos_config(&mut self, efectos: config::Efectos) -> bool {
        self.efectos_config = efectos;
        self.revisar_efectos()
    }

    /// Recalcula si tocan efectos reducidos y lo aplica. Devuelve si cambió,
    /// que es cuando hay que repintar.
    ///
    /// El umbral es el mismo 20 % con el que el widget de batería pide el perfil
    /// de ahorro: dos umbrales distintos para «va justo de batería» harían que
    /// el escritorio cambiara de aspecto en un momento y de perfil en otro.
    /// Leer la batería aquí cuesta 0,02 ms —`capacity` y `status`, sin
    /// `current_now`— y se hace solo cuando el panel ya se estaba refrescando.
    // 0.1.0
    // modified 0.2.4 -- bajo consumo unificado y perfil power-saver también reducen efectos
    pub fn revisar_efectos(&self) -> bool {
        let bateria_baja =
            state::Battery::read(false).is_some_and(|b| !b.plugged && b.percent <= 20);
        tema::aplicar_efectos_reducidos(
            self.efectos_config == config::Efectos::Reducidos
                || bateria_baja
                || self.bajo_consumo
                || self.perfil_ahorro,
        )
    }

    /// ¿Está activo el modo de bajo consumo unificado (manual, por perfil
    /// `power-saver` o por batería ≤ 20 % sin cargador)?
    // añadido en la 0.2.4
    pub fn bajo_consumo_efectivo(&self) -> bool {
        self.bajo_consumo
            || self.perfil_ahorro
            || state::Battery::read(false).is_some_and(|b| !b.plugged && b.percent <= 20)
    }

    // añadido en la 0.2.4
    pub fn poner_bajo_consumo(&mut self, activo: bool) -> bool {
        if self.bajo_consumo == activo {
            return false;
        }
        self.bajo_consumo = activo;
        self.revisar_efectos();
        self.panel.painted_once = false;
        true
    }

    // añadido en la 0.2.4
    pub fn poner_perfil_ahorro(&mut self, activo: bool) -> bool {
        if self.perfil_ahorro == activo {
            return false;
        }
        self.perfil_ahorro = activo;
        self.revisar_efectos();
        self.panel.painted_once = false;
        true
    }

    /// Actualiza los puntos de privacidad en el panel.
    // añadido en la 0.2.4
    pub fn poner_privacidad(&mut self, estado: Privacidad) -> bool {
        self.privacidad_vista = estado;
        if self.widgets.privacidad(estado) {
            self.panel.painted_once = false;
            true
        } else {
            false
        }
    }

    // --- Panel de diagnóstico -----------------------------------------------

    // 0.1.0
    pub fn diagnostico_visible(&self) -> bool {
        self.diagnostico.is_some()
    }

    /// Pone o quita el panel. Devuelve si quedó puesto.
    // 0.1.0
    pub fn alternar_diagnostico(&mut self) -> bool {
        match self.diagnostico.take() {
            Some(_) => false,
            None => {
                let hud = diagnostico::Hud::new();
                let (w, h) = hud.size();
                let canvas = Canvas::new(Size::new(w, h), self.escala);
                self.diagnostico = Some((hud, canvas));
                true
            }
        }
    }

    /// Entrega la medida de la última ventana. Solo repinta si algo cambió: con
    /// el escritorio quieto, los números son los mismos y despertar para
    /// redibujar lo mismo es exactamente lo que este panel existe para detectar.
    // 0.1.0
    pub fn diagnostico_datos(&mut self, datos: diagnostico::Datos) -> bool {
        let Some((hud, canvas)) = self.diagnostico.as_mut() else {
            return false;
        };
        if hud.datos == datos {
            return false;
        }
        // El número de salidas cambia el alto, y con él el buffer.
        let alto_antes = hud.size().1;
        hud.datos = datos;
        let (w, h) = hud.size();
        if h != alto_antes {
            *canvas = Canvas::new(Size::new(w, h), canvas.scale);
        } else {
            canvas.painted_once = false;
        }
        true
    }

    // 0.1.0
    pub fn diagnostico_buffer_size(&self) -> Option<(u32, u32)> {
        self.diagnostico.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn diagnostico_logical_size(&self) -> Option<(f32, f32)> {
        self.diagnostico.as_ref().map(|(h, _)| h.size())
    }

    // 0.1.0
    pub fn diagnostico_needs_paint(&self) -> bool {
        self.diagnostico
            .as_ref()
            .is_some_and(|(_, c)| !c.painted_once)
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_diagnostico(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((hud, canvas)) = self.diagnostico.as_mut() else {
            return Vec::new();
        };
        let vista = hud.view();
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    // --- Conmutador de aplicaciones ----------------------------------------

    /// Abre el conmutador con las celdas por orden de uso reciente.
    ///
    /// La lista la arma el compositor: el shell no ve el foco ni las ventanas.
    /// Sin **celdas** no se abre y el compositor lo sabe porque esto devuelve
    /// `false`. Con una sola sí: sirve para ver qué hay abierto, aunque no haya
    /// a dónde ir.
    // 0.1.0
    pub fn abrir_conmutador(
        &mut self,
        modo: conmutador::Modo,
        apps: Vec<conmutador::Entrada>,
        pantalla: (f32, f32),
    ) -> bool {
        if apps.is_empty() {
            return false;
        }
        let c = conmutador::Conmutador::new(modo, apps, pantalla);
        let (w, h) = c.size();
        let canvas = Canvas::new(Size::new(w, h), self.escala);
        self.conmutador = Some((c, canvas));
        true
    }

    /// Mueve la selección. `1` adelante, `-1` atrás.
    // 0.1.0
    pub fn conmutador_mover(&mut self, pasos: i32) {
        if let Some((c, canvas)) = self.conmutador.as_mut() {
            c.mover(pasos);
            canvas.painted_once = false;
        }
    }

    // 0.1.0
    pub fn conmutador_elegir(&mut self, i: usize) -> bool {
        if let Some((c, canvas)) = self.conmutador.as_mut()
            && c.elegir(i)
        {
            canvas.painted_once = false;
            return true;
        }
        false
    }

    // 0.1.0
    pub fn conmutador_en(&self, x: f32, y: f32) -> Option<usize> {
        self.conmutador.as_ref()?.0.en(x, y)
    }

    // 0.1.0
    pub fn conmutador_miniaturas(&self) -> Vec<Rectangle> {
        self.conmutador
            .as_ref()
            .map(|(c, _)| c.miniaturas())
            .unwrap_or_default()
    }

    /// Qué celda está elegida, sin cerrarlo.
    // añadido en la 0.2.0
    pub fn conmutador_elegida(&self) -> Option<usize> {
        self.conmutador.as_ref()?.0.elegida()
    }

    /// Cierra el conmutador y dice **qué celda** quedó elegida.
    ///
    /// Un índice y no un `app_id`: quien sabe a qué ventana lleva cada celda es
    /// el compositor, que las tiene.
    // 0.1.0
    pub fn cerrar_conmutador(&mut self) -> Option<usize> {
        let (c, _) = self.conmutador.take()?;
        c.elegida()
    }

    /// Lo abandona sin elegir nada. Es lo que hace Escape.
    // 0.1.0
    pub fn cancelar_conmutador(&mut self) -> bool {
        self.conmutador.take().is_some()
    }

    // 0.1.0
    pub fn hay_conmutador(&self) -> bool {
        self.conmutador.is_some()
    }

    // 0.1.0
    pub fn conmutador_buffer_size(&self) -> Option<(u32, u32)> {
        self.conmutador.as_ref().map(|(_, c)| c.buffer_size())
    }

    // 0.1.0
    pub fn conmutador_logical_size(&self) -> Option<(f32, f32)> {
        self.conmutador.as_ref().map(|(c, _)| c.size())
    }

    // 0.1.0
    pub fn conmutador_needs_paint(&self) -> bool {
        self.conmutador
            .as_ref()
            .is_some_and(|(c, canvas)| !canvas.painted_once || c.animando())
    }

    /// ¿Se está moviendo el recuadro del conmutador?
    // 0.1.0
    pub fn conmutador_animando(&self) -> bool {
        self.conmutador.as_ref().is_some_and(|(c, _)| c.animando())
    }

    /// La celda elegida del conmutador de ventanas, en lógicos de su tarjeta.
    // 0.1.0
    pub fn conmutador_realce(&self) -> Option<Rectangle> {
        self.conmutador.as_ref()?.0.realce()
    }

    /// El renglón del rótulo del conmutador de ventanas, en lógicos de su tarjeta.
    // 0.1.0
    pub fn conmutador_rotulo(&self) -> Option<Rectangle> {
        self.conmutador.as_ref()?.0.rotulo()
    }

    /// Pinta el recuadro de la elegida en `buf`, de `ancho` × `alto` lógicos.
    /// Se pinta una vez al abrir: moverlo de celda es cosa del compositor.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_conmutador_realce(&mut self, buf: &mut [u8], ancho: f32, alto: f32) {
        let mut canvas = Canvas::new(Size::new(ancho, alto), self.escala);
        let vista = conmutador::Conmutador::view_realce();
        Self::paint(
            &mut canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        );
    }

    pub fn dibujar_etiqueta_tactil(
        &mut self,
        buf: &mut [u8],
        tam: (f32, f32),
        entradas: &[conmutador::Entrada],
        foco: bool,
        escala: f32,
    ) -> Vec<Damage> {
        let mut canvas = Canvas::new(Size::new(tam.0, tam.1), escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            conmutador::etiqueta_tactil(entradas, tam.0, foco),
            buf,
        )
    }

    /// Pinta el rótulo de la elegida en `buf`, de `ancho` × `alto` lógicos. Es
    /// lo único que se repinta al cambiar de celda, y mide un renglón.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_conmutador_rotulo(&mut self, buf: &mut [u8], ancho: f32, alto: f32) {
        let Some((c, _)) = self.conmutador.as_ref() else {
            return;
        };
        let mut canvas = Canvas::new(Size::new(ancho, alto), self.escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            c.view_rotulo(),
            buf,
        );
    }

    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_conmutador(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((c, canvas)) = self.conmutador.as_mut() else {
            return Vec::new();
        };
        let vista = c.view();
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Echa el bloqueo, con la hora ya formateada.
    ///
    /// `pantalla` es el tamaño **lógico**: el bloqueo la ocupa entera.
    // 0.1.0
    pub fn bloquear(&mut self, hora: String, fecha: String, pantalla: (f32, f32)) {
        let canvas = Canvas::new(Size::new(pantalla.0, pantalla.1), self.escala);
        let bloqueo =
            bloqueo::Bloqueo::new(hora, fecha, self.avatar.as_deref(), self.bloqueo_config);
        self.bloqueo_capas.clear();
        self.bloqueo = Some((bloqueo, canvas));
    }

    /// Abre la capa de captura. `pantalla` es el tamaño **lógico** de la
    /// pantalla que tapa.
    // 0.1.0
    pub fn abrir_captura(&mut self, pantalla: (f32, f32)) {
        let opciones = Config::cargar();
        let c = captura::Captura::new(pantalla)
            .con_opciones(opciones.captura_cursor, opciones.captura_retardo);
        let (bw, bh) = c.tamano_barra();
        let barra = Canvas::new(Size::new(bw, bh), self.escala);
        let pastilla = Canvas::new(
            Size::new(captura::PASTILLA_ANCHO, captura::PASTILLA_ALTO),
            self.escala,
        );
        let (ow, oh) = c.tamano_opciones();
        let popover = Canvas::new(Size::new(ow, oh), self.escala);
        self.captura = Some((c, barra, pastilla, popover));
    }

    // 0.1.0
    pub fn cerrar_captura(&mut self) -> bool {
        self.captura.take().is_some()
    }

    // 0.1.0
    pub fn hay_captura(&self) -> bool {
        self.captura.is_some()
    }

    pub fn captura_establecer_ventana(&mut self, rect: Option<captura::Recuadro>) {
        if let Some((c, _, _, _)) = self.captura.as_mut() {
            c.establecer_ventana(rect);
        }
    }

    // 0.1.0
    fn lienzo_captura(&self, pieza: PiezaCaptura) -> Option<&Canvas> {
        let (c, barra, pastilla, popover) = self.captura.as_ref()?;
        match pieza {
            PiezaCaptura::Barra => Some(barra),
            PiezaCaptura::Pastilla => Some(pastilla),
            PiezaCaptura::Opciones => c.popover_abierto().then_some(popover),
        }
    }

    // 0.1.0
    pub fn captura_buffer_size(&self, pieza: PiezaCaptura) -> Option<(u32, u32)> {
        self.lienzo_captura(pieza).map(Canvas::buffer_size)
    }

    /// Dónde va la pieza, en lógicos de la pantalla.
    // 0.1.0
    pub fn captura_origen(&self, pieza: PiezaCaptura) -> Option<(f32, f32)> {
        let (c, _, _, _) = self.captura.as_ref()?;
        match pieza {
            PiezaCaptura::Barra => Some(c.origen_barra()),
            PiezaCaptura::Pastilla => c.origen_pastilla(),
            PiezaCaptura::Opciones => c.origen_opciones(),
        }
    }

    // 0.1.0
    pub fn captura_forma(&self, x: f32, y: f32) -> Option<captura::Forma> {
        self.captura.as_ref().map(|(c, _, _, _)| c.forma(x, y))
    }

    // 0.1.0
    pub fn captura_marcado(&self) -> Option<captura::Recuadro> {
        self.captura.as_ref().and_then(|(c, _, _, _)| c.marcado())
    }

    // 0.1.0
    pub fn captura_animando(&self) -> bool {
        self.captura.as_ref().is_some_and(|(c, _, _, _)| c.animando())
    }

    // 0.1.0
    pub fn captura_puntero(&mut self, x: f32, y: f32) {
        let Some((c, barra, pastilla, popover)) = self.captura.as_mut() else {
            return;
        };
        let medidas = c.medidas();
        if c.puntero(x, y) {
            barra.painted_once = false;
            popover.painted_once = false;
        }
        if c.medidas() != medidas {
            pastilla.painted_once = false;
        }
    }

    // 0.1.0
    fn captura_repintar_todo(&mut self) {
        if let Some((_, barra, pastilla, popover)) = self.captura.as_mut() {
            barra.painted_once = false;
            pastilla.painted_once = false;
            popover.painted_once = false;
        }
    }

    // 0.1.0
    pub fn captura_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let accion = self.captura.as_mut()?.0.pulsar(x, y);
        self.captura_repintar_todo();
        accion
    }

    // 0.1.0
    pub fn captura_soltar(&mut self) {
        let Some((c, _, _, _)) = self.captura.as_mut() else {
            return;
        };
        c.soltar();
        self.captura_repintar_todo();
    }

    // 0.1.0
    pub fn captura_tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        let Some((c, _, _, _)) = self.captura.as_mut() else {
            return Tecla::Ignorada;
        };
        let resultado = c.tecla(tecla);
        self.captura_repintar_todo();
        resultado
    }

    // 0.1.0
    pub fn captura_needs_paint(&self, pieza: PiezaCaptura) -> bool {
        self.lienzo_captura(pieza).is_some_and(|c| !c.painted_once)
    }

    // 0.1.0
    pub fn draw_captura(&mut self, pieza: PiezaCaptura, buf: &mut [u8]) -> Vec<Damage> {
        let Some((c, barra, pastilla, popover)) = self.captura.as_mut() else {
            return Vec::new();
        };
        let (vista, canvas) = match pieza {
            PiezaCaptura::Barra => (c.vista_barra(), barra),
            PiezaCaptura::Pastilla => (c.vista_pastilla(), pastilla),
            PiezaCaptura::Opciones => (c.vista_opciones(), popover),
        };
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    pub fn abrir_grabacion_pill(&mut self, pantalla: (f32, f32), tiene_mic: bool) {
        let pill = captura::GrabacionPill::new(pantalla, tiene_mic);
        let (pw, ph) = pill.tamano();
        let canvas = Canvas::new(Size::new(pw, ph), self.escala);
        let segs = pill.segundos();
        self.grabacion_pill = Some((pill, canvas, segs));
    }

    pub fn cerrar_grabacion_pill(&mut self) -> bool {
        self.grabacion_pill.take().is_some()
    }

    pub fn hay_grabacion_pill(&self) -> bool {
        self.grabacion_pill.is_some()
    }

    pub fn grabacion_pill_origen(&self) -> Option<(f32, f32)> {
        self.grabacion_pill.as_ref().map(|(p, _, _)| p.origen())
    }

    pub fn grabacion_pill_buffer_size(&self) -> Option<(u32, u32)> {
        self.grabacion_pill
            .as_ref()
            .map(|(_, c, _)| c.buffer_size())
    }

    pub fn grabacion_pill_needs_paint(&mut self) -> bool {
        let Some((p, c, ult_seg)) = self.grabacion_pill.as_mut() else {
            return false;
        };
        let segs = p.segundos();
        if segs != *ult_seg {
            *ult_seg = segs;
            c.painted_once = false;
        }
        !c.painted_once
    }

    pub fn draw_grabacion_pill(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((p, canvas, _)) = self.grabacion_pill.as_mut() else {
            return Vec::new();
        };
        let vista = p.vista();
        Self::paint(
            canvas,
            &mut self.renderer,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    pub fn grabacion_pill_contiene(&self, x: f32, y: f32) -> bool {
        self.grabacion_pill
            .as_ref()
            .is_some_and(|(p, _, _)| p.contiene(x, y))
    }

    pub fn grabacion_pill_puntero(&mut self, x: f32, y: f32) -> bool {
        let Some((p, c, _)) = self.grabacion_pill.as_mut() else {
            return false;
        };
        if p.puntero(x, y) {
            c.painted_once = false;
            true
        } else {
            false
        }
    }

    pub fn grabacion_pill_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (p, c, _) = self.grabacion_pill.as_mut()?;
        let acc = p.pulsar(x, y);
        c.painted_once = false;
        acc
    }

    pub fn grabacion_pill_soltar(&mut self) {
        if let Some((p, _, _)) = self.grabacion_pill.as_mut() {
            p.soltar();
        }
    }

    // 0.1.0
    // modified 0.1.1 -- suelta la caché del renderer modal al desbloquear
    pub fn desbloquear(&mut self) {
        self.bloqueo = None;
        self.bloqueo_capas.clear();
        self.limpiar_cache_modal();
    }

    // 0.1.0
    pub fn esta_bloqueado(&self) -> bool {
        self.bloqueo.is_some()
    }

    /// El bloqueo, para cambiarle lo que enseña.
    // 0.1.0
    pub fn bloqueo_mut(&mut self) -> Option<&mut bloqueo::Bloqueo> {
        self.bloqueo.as_mut().map(|(b, _)| b)
    }

    /// Pone cuántos caracteres lleva escritos la contraseña y en qué estado
    /// está la comprobación. `true` si hay que repintar.
    ///
    /// La contraseña **no entra aquí**: el shell solo sabe cuántos puntos
    /// dibujar. Quien la guarda es el compositor, que es quien habla con el
    /// ayudante de PAM.
    /// ¿Se está moviendo algo del bloqueo? Es lo que mantiene al compositor
    /// dibujando mientras el menú de apagado entra o sale.
    // 0.1.0
    pub fn bloqueo_animando(&self) -> bool {
        self.bloqueo.as_ref().is_some_and(|(b, _)| b.animando())
    }

    /// Un clic sobre la pantalla de bloqueo, en lógicos. Devuelve lo que haya
    /// que hacer —apagar, reiniciar, suspender— y si hay que repintar.
    // 0.1.0
    pub fn bloqueo_pulsado(&mut self, x: f32, y: f32) -> (Option<bloqueo::Peticion>, bool) {
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return (None, false);
        };
        let pantalla = (canvas.size.width, canvas.size.height);
        let menu_antes = bloqueo.menu;
        let (peticion, cambio_interno) = bloqueo.pulsar(x, y, pantalla);
        (
            peticion,
            cambio_interno || peticion.is_some() || bloqueo.menu != menu_antes,
        )
    }

    /// El puntero se mueve sobre el bloqueo. `true` si cambia la barra de la
    /// música: agarrada la arrastra, y bajo el cursor engorda.
    // añadido en la 0.2.3
    pub fn bloqueo_mover(&mut self, x: f32, y: f32) -> bool {
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        bloqueo.mover(x, y, (canvas.size.width, canvas.size.height))
    }

    /// Ver [`bloqueo::Bloqueo::toque_directo`].
    // added 0.2.3
    pub fn bloqueo_toque_directo(&self, x: f32, y: f32) -> bool {
        self.bloqueo
            .as_ref()
            .is_some_and(|(b, c)| b.toque_directo(x, y, (c.size.width, c.size.height)))
    }

    /// Se suelta el botón: si se arrastraba la barra, salta ahí.
    // añadido en la 0.2.3
    pub fn bloqueo_soltar(&mut self) -> bool {
        self.bloqueo.as_mut().is_some_and(|(b, _)| b.soltar())
    }

    /// Cancela el toque o arrastre en curso sobre la tarjeta de medios del
    /// bloqueo sin disparar la acción ni buscar posición.
    pub fn bloqueo_cancelar(&mut self) -> bool {
        self.bloqueo
            .as_mut()
            .is_some_and(|(b, _)| b.cancelar_interaccion())
    }

    // 0.1.0
    pub fn bloqueo_estado(&mut self, escritos: usize, estado: bloqueo::Estado) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if !b.actualizar_estado(escritos, estado) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    // 0.1.0
    pub fn bloqueo_confirmar_tecla(
        &mut self,
        tecla: TeclaPulsada,
    ) -> (bool, Option<bloqueo::Peticion>) {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return (false, None);
        };
        let resultado = b.confirmar_tecla(tecla);
        if resultado.0 {
            canvas.painted_once = false;
        }
        resultado
    }

    // 0.1.0
    pub fn bloqueo_huella(&mut self, huella: bloqueo::Huella) {
        if let Some((b, canvas)) = self.bloqueo.as_mut()
            && b.actualizar_huella(huella)
        {
            canvas.painted_once = false;
        }
    }

    // 0.1.0
    pub fn bloqueo_caps_lock(&mut self, activo: bool) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if !b.actualizar_caps_lock(activo) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Entrega al bloqueo la lectura de MPRIS hecha fuera del hilo de dibujo.
    // 0.1.0
    // modificado en la 0.2.3 -- también aleatorio y repetición (solo los da BookOS Player)
    pub fn bloqueo_medio(
        &mut self,
        sonando: Option<medios::Sonando>,
        modos: (Option<bool>, Option<medios::Repetir>),
    ) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        // Los dos, sin cortocircuito: si cambian los modos y no la canción
        // también hay que repintar.
        let cambio = b.poner_medio(sonando) | b.poner_modos(modos);
        if !cambio {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Sustituye la composición del bloqueo sin reiniciar la sesión.
    ///
    /// Siempre se conserva para el próximo bloqueo; si ya está visible se
    /// invalida también su canvas para que BookOS Settings funcione como un
    /// editor en vivo.
    // 0.1.0
    pub fn aplicar_bloqueo_config(&mut self, config: ConfigBloqueo) -> bool {
        self.bloqueo_config = config;
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        bloqueo.aplicar_config(config);
        canvas.painted_once = false;
        true
    }

    // 0.1.0
    pub fn bloqueo_buffer_size(&self) -> Option<(u32, u32)> {
        self.bloqueo.as_ref().map(|(_, c)| c.buffer_size())
    }

    // añadido en la 0.2.0
    pub fn bloqueo_capas(&self) -> Vec<(bloqueo::Capa, (f32, f32, f32, f32))> {
        let Some((bloqueo, canvas)) = &self.bloqueo else {
            return Vec::new();
        };
        let pantalla = (canvas.size.width, canvas.size.height);
        bloqueo::Capa::TODAS
            .into_iter()
            .filter_map(|capa| bloqueo.geometria_capa(capa, pantalla).map(|g| (capa, g)))
            .collect()
    }

    // añadido en la 0.2.0
    pub fn bloqueo_entradas(&self) -> [f32; 4] {
        self.bloqueo
            .as_ref()
            .map_or([1.0; 4], |(b, _)| b.entradas())
    }

    // añadido en la 0.2.0
    pub fn bloqueo_avanzar_entrada(&mut self, ahora: std::time::Instant) -> bool {
        self.bloqueo
            .as_mut()
            .is_some_and(|(b, _)| b.avanzar_entrada(ahora))
    }

    // añadido en la 0.2.0
    pub fn bloqueo_contenido_animando(&self) -> bool {
        self.bloqueo
            .as_ref()
            .is_some_and(|(b, _)| b.contenido_animando())
    }

    /// La tarjeta de música del bloqueo moviéndose: la barra o un mando.
    // added 0.2.3
    pub fn bloqueo_tarjeta_animando(&self) -> bool {
        self.bloqueo.as_ref().is_some_and(|(b, _)| b.tarjeta_animando())
    }

    // añadido en la 0.2.0
    pub fn bloqueo_medios_avance(&self) -> f32 {
        self.bloqueo.as_ref().map_or(1.0, |(b, _)| {
            if b.medios.is_empty() {
                1.0
            } else {
                b.medio_avance()
            }
        })
    }

    // añadido en la 0.2.0 -- los cambios de contenido reutilizan el lienzo de la capa, nunca el de pantalla completa.
    pub fn draw_bloqueo_capa(&mut self, capa: bloqueo::Capa, buf: &mut [u8]) -> Vec<Damage> {
        let Some((bloqueo, pantalla)) = &self.bloqueo else {
            return Vec::new();
        };
        let pantalla = (pantalla.size.width, pantalla.size.height);
        let Some((_, _, w, h)) = bloqueo.geometria_capa(capa, pantalla) else {
            return Vec::new();
        };
        let indice = match self.bloqueo_capas.iter().position(|(c, _)| *c == capa) {
            Some(i) => i,
            None => {
                self.bloqueo_capas
                    .push((capa, Canvas::new(Size::new(w, h), self.escala)));
                self.bloqueo_capas.len() - 1
            }
        };
        let canvas = &mut self.bloqueo_capas[indice].1;
        if canvas.size != Size::new(w, h) {
            *canvas = Canvas::new(Size::new(w, h), self.escala);
        }
        let vista = bloqueo.view_capa(pantalla, capa);
        Self::paint(
            canvas,
            &mut self.renderer_modal,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Pinta el bloqueo. El buffer sale **transparente** donde no hay nada: el
    /// fondo lo pone el compositor con la imagen del escritorio.
    // 0.1.0
    // modified 0.1.1 -- pinta con el renderer modal y la máscara reutilizable
    pub fn draw_bloqueo(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return Vec::new();
        };
        // modificado en la 0.2.0 -- compatibilidad de los ejemplos que piden una imagen completa animada.
        bloqueo.avanzar_entrada(std::time::Instant::now());
        let vista = bloqueo.view((canvas.size.width, canvas.size.height));
        Self::paint(
            canvas,
            &mut self.renderer_modal,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Qué aplicación hay en ese punto del dock: `(app_id, exec, icono)`.
    // 0.1.0
    // modified 0.1.1 -- el launchpad también se puede agarrar
    pub fn dock_item_en(&self, x: f32, y: f32) -> Option<(String, String, String)> {
        let i = self.dock_items.item_en(x, y)?;
        let item = &self.dock_items.items()[i];
        Some((
            item.app_id().to_string(),
            item.exec().to_string(),
            item.icono_nombre().to_string(),
        ))
    }

    // 0.1.1
    pub fn dock_arrastrar(&mut self, app_id: Option<&str>) {
        self.dock_items.arrastrar(app_id);
        self.dock.painted_once = false;
    }

    // 0.1.1
    pub fn dock_mover(&mut self, app_id: &str, x: f32) -> bool {
        let cambio = self.dock_items.mover(app_id, x);
        if cambio {
            self.dock.painted_once = false;
        }
        cambio
    }

    /// Abre el menú contextual del icono que haya en ese punto del dock, en
    /// coordenadas lógicas relativas al dock. `true` si se abrió.
    // 0.1.0
    // modified 0.2.3 -- con sus ventanas, opciones y recientes
    pub fn dock_menu(&mut self, x: f32, y: f32, ventanas: &[(String, String, bool)]) -> bool {
        let Some(i) = self.dock_items.item_en(x, y) else {
            return false;
        };
        let item = &self.dock_items.items()[i];
        // El launchpad no es una aplicación: no se abre en otra ventana, ni se
        // suelta del dock, ni se cierra.
        if item.exec() == config::LAUNCHPAD {
            return false;
        }
        let suyas: Vec<(String, bool)> = ventanas
            .iter()
            .filter(|(app_id, ..)| mismo_programa(item.app_id(), app_id))
            .map(|(_, titulo, activa)| (titulo.clone(), *activa))
            .collect();
        let entradas = menus::dock(&menus::AppDock {
            app_id: item.app_id(),
            exec: item.exec(),
            icono: item.icono_nombre(),
            anclada: item.anclada(),
            ventanas: &suyas,
        });
        let x = self.dock_items.centro_de(i);
        self.abrir(Emergente::menu_dock(entradas, x));
        true
    }

    // 0.1.0
    pub fn menu_ventana(&mut self, opciones: Vec<(String, Accion)>) {
        self.abrir(Emergente::menu_ventana(opciones));
    }

    /// El menú de ventana anclado a un punto lógico de la pantalla: lo que
    /// pide el clic derecho sobre el escritorio, para salir junto al cursor.
    // 0.1.1
    // modified 0.2.0 -- el color va antes de la papelera, con una sola raya
    // modified 0.2.3 -- el menú lo arma el escritorio según la selección
    pub fn menu_escritorio(&mut self, punto: (f32, f32), pegable: bool) {
        let entradas = menus::escritorio(&self.escritorio, pegable);
        self.abrir(Emergente::menu_escritorio(entradas, punto));
    }

    /// El diálogo de pegar: acceso directo o copia de `rutas`.
    // añadido en la 0.2.3
    pub fn abrir_pegar(&mut self, rutas: &[std::path::PathBuf]) {
        let nombres: Vec<String> = rutas
            .iter()
            .filter_map(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let origen = rutas
            .first()
            .and_then(|r| r.parent())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let peso = rutas.iter().map(|r| escritorio::peso(r)).sum();
        self.abrir(Emergente::Dialogo(emergente::dialogo::Dialogo::pegar(
            &nombres, &origen, peso,
        )));
    }

    /// El diálogo de renombrar lo seleccionado (uno solo).
    // añadido en la 0.2.3
    pub fn abrir_renombrar(&mut self) {
        let Some(nombre) = self.escritorio.seleccionados().next().map(|e| e.nombre.clone()) else {
            return;
        };
        if self.escritorio.seleccionados().count() != 1 {
            return;
        }
        self.abrir(Emergente::Dialogo(emergente::dialogo::Dialogo::renombrar(&nombre)));
    }

    /// Renombra; si falla, el diálogo sigue abierto y dice por qué.
    /// `true` si se renombró.
    // añadido en la 0.2.3
    pub fn renombrar_a(&mut self, nombre: &str) -> bool {
        match self.escritorio.renombrar(nombre) {
            Ok(()) => {
                self.cerrar_emergente();
                true
            }
            Err(motivo) => {
                if let Some((Emergente::Dialogo(d), canvas)) = self.emergente.as_mut() {
                    d.error(motivo);
                    canvas.painted_once = false;
                }
                false
            }
        }
    }

    /// La ficha de información de lo seleccionado (uno solo).
    // añadido en la 0.2.3
    pub fn abrir_informacion(&mut self) {
        let Some(d) = menus::informacion(&self.escritorio) else {
            return;
        };
        self.abrir(Emergente::Dialogo(d));
    }

    /// La vista rápida de lo seleccionado (uno solo).
    // añadido en la 0.2.3
    pub fn abrir_vista_rapida(&mut self) {
        let Some(d) = menus::vista_rapida(&self.escritorio) else {
            return;
        };
        self.abrir(Emergente::Dialogo(d));
    }

    // añadido en la 0.2.3
    pub fn escritorio_seleccionar_nombre(&mut self, nombre: &str) {
        self.escritorio.seleccionar_nombre(nombre);
    }

    /// El diálogo de color de lo seleccionado en el escritorio.
    // añadido en la 0.2.0
    pub fn abrir_color_carpeta(&mut self) {
        let emergente = Emergente::color_carpeta(&self.escritorio.carpetas_seleccionadas());
        self.abrir(emergente);
    }

    /// Ancla una aplicación al dock, o la desancla si ya estaba.
    ///
    /// Devuelve la lista de anclados que hay que guardar, para que el
    /// compositor la escriba en `panel.conf`: el shell no toca el disco.
    // 0.1.0
    pub fn anclar(
        &mut self,
        app_id: &str,
        exec: &str,
        icono: &str,
        fijar: Option<bool>,
    ) -> Vec<String> {
        match fijar {
            None => self.dock_items.alternar_anclado(app_id, exec, icono),
            Some(true) => {
                self.dock_items.anclar(app_id, exec, icono);
            }
            Some(false) => {
                self.dock_items.desanclar(app_id);
            }
        }
        self.dock.painted_once = false;
        let (dw, dh) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(dw, dh), self.dock.scale);
        self.dock_items.como_configuracion()
    }

    /// Le dice al dock si está apoyado en el borde de la pantalla, para que se
    /// dibuje como parte del marco y no como algo que flota. `true` si hay que
    /// repintar.
    // 0.1.0
    pub fn dock_pegado(&mut self, pegado: bool) -> bool {
        if !self.dock_items.set_pegado(pegado) {
            return false;
        }
        self.dock.painted_once = false;
        true
    }

    // 0.1.0
    pub fn dock_tamano(&mut self, tamano: u32) {
        self.dock_items.tamano(tamano);
        let (w, h) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(w, h), self.dock.scale);
    }

    /// ¿Está esta aplicación anclada al dock?
    // 0.1.0
    pub fn esta_anclada(&self, app_id: &str) -> bool {
        self.dock_items.esta_anclada(app_id)
    }

    /// Marca en el dock qué aplicaciones tienen ventana, a partir de los
    /// `app_id` que hay en el escritorio. Devuelve `true` si hay que repintar.
    // 0.1.0
    pub fn dock_ventanas(&mut self, app_ids: &[String]) -> bool {
        if !self.dock_items.set_abiertas(app_ids) {
            return false;
        }
        // El dock cambia de ancho al entrar o salir una aplicación abierta, así
        // que no basta con repintar: hay que rehacer el lienzo.
        let (dw, dh) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(dw, dh), self.dock.scale);
        true
    }

    /// Qué hacer al pulsar en `punto`, en coordenadas **lógicas relativas al
    /// dock**.
    // 0.1.0
    pub fn dock_pulsar(&self, x: f32, y: f32) -> Option<Accion> {
        let item = self.dock_items.pulsado(x, y)?;
        if item.exec() == config::LAUNCHPAD {
            return Some(Accion::Launchpad);
        }
        // Sin `exec` y sin ventana no hay nada que hacer: es un icono que solo
        // existe mientras su ventana viva, porque su `app_id` no vale como
        // orden. Ver el respaldo de `Dock::set_abiertas`. Con ventana sí
        // responde, que para eso está: lo que no hará es lanzar nada.
        if item.exec().is_empty() && !item.abierta() {
            return None;
        }
        Some(if item.abierta() {
            Accion::Activar {
                app_id: item.app_id().to_string(),
                exec: item.exec().to_string(),
            }
        } else {
            Accion::Lanzar(item.exec().to_string())
        })
    }

    /// Pinta el panel. Ver [`Shell::draw_dock`] para el formato de `buf`.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_panel(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        // Escotilla para comprobar que el aislamiento de pánicos del compositor
        // funciona de verdad. Sin esto, "el shell no puede tumbar la sesión" es
        // una afirmación sin probar.
        //
        // Con el nombre de un widget (`=reloj`) revienta solo ese widget y el
        // panel sigue: son dos redes distintas y se prueban por separado.
        if std::env::var("BOOKOS_SHELL_PANIC_TEST").is_ok_and(|v| v == "shell") {
            panic!("panic de prueba del shell");
        }
        let view = view::panel(&self.widgets);
        let Self {
            renderer_panel,
            theme,
            panel,
            mask,
            ..
        } = self;
        Self::paint(panel, renderer_panel, mask, theme, view, buf)
    }

    // añadido en la 0.2.0
    pub fn dock_usar_capas(&mut self) {
        self.dock_en_capas = true;
        self.dock.painted_once = false;
    }

    // añadido en la 0.2.0
    pub fn dock_revision(&self) -> u64 {
        self.dock_revision
    }

    // añadido en la 0.2.0
    pub fn dock_capas(&self) -> Vec<(f32, f32, f32, f32, f32)> {
        self.dock_items.capas()
    }

    // añadido en la 0.2.0
    pub fn dock_realces(&self) -> Vec<(usize, f32)> {
        self.dock_items.realces()
    }

    // añadido en la 0.2.0
    pub fn draw_dock_icono(&mut self, indice: usize, buf: &mut [u8]) -> Vec<Damage> {
        let tamano = self.dock_items.capas()[indice];
        let mut canvas = Canvas::new(Size::new(tamano.2, tamano.3), self.escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer_dock,
            &mut self.mask,
            &self.theme,
            self.dock_items.view_icono(indice),
            buf,
        )
    }

    // añadido en la 0.2.0 -- una placa se reutiliza en todos los iconos y su intensidad la pone el compositor.
    pub fn draw_dock_realce(&mut self, buf: &mut [u8], lado: f32) -> Vec<Damage> {
        use iced_core::{Border, Length};
        use iced_widget::{Space, container};
        let vista = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| iced_widget::container::Style {
                background: Some(tema::alfa(tema::tinta(), 0.18).into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into();
        let mut canvas = Canvas::new(Size::new(lado, lado), self.escala);
        Self::paint(
            &mut canvas,
            &mut self.renderer_dock,
            &mut self.mask,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Pinta el dock en `buf`, que debe tener el tamaño de
    /// [`Shell::dock_buffer_size`].
    ///
    /// El formato de salida es premultiplicado y con los bytes en orden
    /// **B,G,R,A** — o sea `Fourcc::Argb8888` en little-endian. No es una
    /// elección nuestra sino lo que deja iced, y conviene no fiarse de la
    /// intuición aquí: el compositor tuvo puesto Abgr8888 por lo que parecía
    /// evidente y lo que se veía era el azul del acento en naranja.
    ///
    /// Devuelve las zonas dañadas en píxeles físicos. De momento se repinta
    /// entero: son pocos KB y ocurre una vez, no compensa afinar por widget
    /// hasta que el dock tenga contenido que cambie.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara de recorte
    pub fn draw_dock(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        // modificado en la 0.2.0 -- cambiar el contenido invalida las texturas de iconos una sola vez.
        self.dock_revision = self.dock_revision.wrapping_add(1);
        let view = if self.dock_en_capas {
            self.dock_items.view_fondo()
        } else {
            self.dock_items.view()
        };
        let Self {
            renderer_dock,
            theme,
            dock,
            mask,
            ..
        } = self;
        Self::paint(dock, renderer_dock, mask, theme, view, buf)
    }

    // 0.1.0
    // modified 0.1.1 -- pasa la máscara reutilizable
    fn paint(
        canvas: &mut Canvas,
        renderer: &mut Renderer,
        mask: &mut Option<tiny_skia::Mask>,
        theme: &Theme,
        view: view::PanelElement<'_>,
        buf: &mut [u8],
    ) -> Vec<Damage> {
        Self::paint_con_fondo(canvas, renderer, mask, theme, view, buf, Color::TRANSPARENT)
    }

    /// Como [`Self::paint`], pero el buffer se limpia con `fondo` en vez de
    /// con transparente. Iced limpia **sustituyendo** los píxeles
    /// (`BlendMode::Source`), mientras que un contenedor con ese mismo fondo
    /// translúcido se mezcla píxel a píxel: en la franja de la vista de
    /// escritorios (2240×490 a escala 1,75) eso eran 20 de los 31 ms de cada
    /// repintado, y el hover se repinta en cada fotograma de su animación.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la máscara si mide lo mismo en vez de reservarla en cada pintado
    fn paint_con_fondo(
        canvas: &mut Canvas,
        renderer: &mut Renderer,
        mask: &mut Option<tiny_skia::Mask>,
        theme: &Theme,
        view: view::PanelElement<'_>,
        buf: &mut [u8],
        fondo: Color,
    ) -> Vec<Damage> {
        let (w, h) = canvas.buffer_size();
        let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(buf, w, h) else {
            tracing::error!(
                w,
                h,
                len = buf.len(),
                "buffer del shell con tamaño incoherente"
            );
            return Vec::new();
        };

        let viewport = Viewport::with_physical_size(Size::new(w, h), canvas.scale);
        let logical = viewport.logical_size();

        let mut ui = UserInterface::build(
            view,
            logical,
            canvas.cache.take().unwrap_or_default(),
            renderer,
        );
        ui.draw(
            renderer,
            theme,
            &Style {
                text_color: view::TEXT(),
            },
            Cursor::Unavailable,
        );
        canvas.cache = Some(ui.into_cache());

        // Se repinta entero, y **el damage parcial no ayuda**: se probó pasando
        // aquí solo los rectángulos de las celdas que cambiaban, y señalar una
        // celda del launchpad seguía costando 23 ms de los 52 del repintado
        // completo. La razón está en `iced_tiny_skia::engine::adjust_clip_mask`,
        // que hace `clip_mask.clear()` —o sea, borra la máscara del tamaño del
        // buffer entero— **por cada capa y por cada zona dañada**. El coste va
        // con el tamaño del buffer, no con lo que cambia. Medido, con el mismo
        // contenido: buffer de 2881x1801, 23 ms; de 1440x900, 10,8 ms; de
        // 720x450, 3,6 ms.
        //
        // Por eso la palanca es el **tamaño**: el velo del launchpad lo pinta
        // el compositor y su realce va en una superficie de una celda.
        let zonas = [Rectangle {
            x: 0.0,
            y: 0.0,
            width: logical.width,
            height: logical.height,
        }];

        let mask_ref = match mask {
            Some(m) if m.width() == w && m.height() == h => m,
            _ => mask.insert(tiny_skia::Mask::new(w, h).expect("máscara del tamaño del pixmap")),
        };
        renderer.draw(&mut pixmap, mask_ref, &viewport, &zonas, fondo);

        canvas.painted_once = true;
        vec![Damage {
            x: 0,
            y: 0,
            width: w as i32,
            height: h as i32,
        }]
    }
}

/// Cabe cualquier `Length` en el panel; se reexporta para la vista.
pub(crate) const FILL: iced_core::Length = iced_core::Length::Fill;

/// Las actividades que pueden salir en la isla, de más a menos prioridad y por
/// `app_id` a igualdad, para que el orden no baile entre publicaciones. La usan
/// la elección de la principal y el cambio con la rueda, que recorre este orden.
// añadido en la 0.2.0 -- sacada de sincronizar_actividad para la rueda
fn vivas_ordenadas<'a>(
    actividades: &'a std::collections::HashMap<String, actividad::Estado>,
    enfocada: Option<&str>,
    config: &config::Actividades,
    primera: Option<&str>,
    ocultas: &std::collections::HashSet<String>,
) -> Vec<&'a actividad::Estado> {
        let prioridad = |e: &actividad::Estado| match e.clase {
            // Se le está hablando: es lo que se mira ahora. Minimizado cede,
            // porque se pidió apartarlo.
            actividad::Clase::Asistente(actividad::Fase::Minimizado) => 8,
            actividad::Clase::Asistente(_) => 50,
            actividad::Clase::Recorder => 40,
            actividad::Clase::Timer if e.restante_ms < 0 => 30,
            // Un aviso de clase pide atención como un temporizador vencido, pero
            // la grabadora en marcha manda más: no se puede perder un audio.
            actividad::Clase::Calendar => 35,
            actividad::Clase::Timer => 20,
            actividad::Clase::Transfer => 15,
            // Dura unos segundos al sacarlos: por encima de la música, que
            // sigue ahí cuando se cierre.
            actividad::Clase::Auriculares => 12,
            actividad::Clase::Player => 10,
            // Ambiental: nunca le quita la píldora a algo que esté pasando.
            actividad::Clase::Monitor => 5,
        };
        let mut vivas: Vec<&actividad::Estado> = actividades
            .values()
            // Zenith no se oculta: la isla es su única forma de verse.
            .filter(|e| !ocultas.contains(&e.app_id) || es_asistente(e))
            .filter(|e| {
                !enfocada
                    .is_some_and(|ventana| es_de_la_app(&e.app_id, ventana))
            })
            .filter(|e| {
                e.clase != actividad::Clase::Timer
                    || config.temporizador_siempre
                    || e.restante_ms <= 60_000
            })
            .collect();
        // De más a menos prioridad, y por app_id a igualdad para que las
        // burbujas no bailen entre publicaciones.
        vivas.sort_by(|a, b| {
            let fija = |e: &actividad::Estado| primera == Some(e.app_id.as_str());
            fija(b)
                .cmp(&fija(a))
                .then(prioridad(b).cmp(&prioridad(a)))
                .then(a.app_id.cmp(&b.app_id))
        });
        vivas
}

#[cfg(test)]
mod pruebas_esquinas {
    use super::*;

    /// La esquina queda transparente y el borde recto y el centro intactos.
    #[test]
    fn la_esquina_se_va_y_el_resto_queda() {
        let (w, h) = (40u32, 30u32);
        let mut rgba = vec![200u8; (w * h * 4) as usize];
        redondear_esquinas(&mut rgba, w, h, 10.0, Alfa::Premultiplicado);
        let px = |x: u32, y: u32| &rgba[((y * w + x) * 4) as usize..][..4];
        assert_eq!(px(0, 0), [0, 0, 0, 0], "la esquina tiene que desaparecer");
        assert_eq!(px(20, 0), [200; 4], "el borde recto no se toca");
        assert_eq!(px(20, 15), [200; 4], "el centro no se toca");
    }

    /// En alfa recto el color se queda: solo baja el alfa.
    #[test]
    fn con_alfa_recto_el_color_no_se_oscurece() {
        let mut rgba = vec![200u8; 4 * 4 * 4];
        redondear_esquinas(&mut rgba, 4, 4, 2.0, Alfa::Recto);
        assert_eq!(&rgba[..3], [200, 200, 200]);
        assert!(rgba[3] < 200);
    }

    /// La isla enseña lo de las demás apps, no lo de la que tienes delante.
    #[test]
    fn la_actividad_se_empareja_con_la_ventana_de_su_app() {
        assert!(es_de_la_app("com.bookos.calendar", "bookos-calendar"));
        assert!(es_de_la_app("com.bookos.calendar", "com.bookos.calendar"));
        assert!(es_de_la_app(
            "com.bookos.systemmonitor",
            "bookos-system-monitor"
        ));
        assert!(!es_de_la_app("com.bookos.calendar", "bookos-clock"));
        assert!(!es_de_la_app("com.bookos.calendar", "konsole"));
    }
}

#[cfg(test)]
mod pruebas_actividades {
    #[test]
    fn el_monitor_sin_desktop_se_encuentra_por_su_binario() {
        let dir = std::env::temp_dir().join(format!("bookos-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("carpeta temporal");
        std::fs::write(dir.join("bookos-system-monitor"), "").expect("binario falso");
        std::fs::write(dir.join("otro-monitor"), "").expect("binario falso");
        let hallado = super::programa_en(std::iter::once(dir.clone()), "com.bookos.systemmonitor");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(hallado.as_deref(), Some("bookos-system-monitor"));
    }

    use super::*;
    use actividad::{Clase, Estado, ItemCola, Portada};

    fn estado(portada_cola: Option<Portada>) -> Estado {
        Estado {
            app_id: "com.bookos.player".into(),
            clase: Clase::Player,
            activo: true,
            pausado: false,
            titulo: "Pista".into(),
            subtitulo: "Artista".into(),
            posicion_ms: 0,
            duracion_ms: 60_000,
            restante_ms: 0,
            volumen: 50,
            nivel: 0.0,
            aleatorio: false,
            repetir: false,
            anc: None,
            portada: None,
            cola: vec![ItemCola {
                id: "/pista.flac".into(),
                titulo: "Pista".into(),
                artista: "Artista".into(),
                duracion_ms: 0,
                favorita: false,
                actual: true,
                portada: portada_cola,
            }],
            metricas: Vec::new(),
        }
    }

    /// La isla se rehace desde el estado guardado cada vez que la app suelta
    /// el foco: si el latido sin carátulas las borrara de ahí, la cola saldría
    /// vacía hasta el siguiente reenvío del reproductor.
    #[test]
    fn un_latido_sin_caratulas_no_borra_las_de_la_cola() {
        let mut shell = Shell::con_config(1280, 1.0, Config::default());
        shell.publicar_actividad(estado(Some(Portada {
            rgba: vec![90; 32 * 32 * 4],
            width: 32,
            height: 32,
        })));
        shell.publicar_actividad(estado(None));
        let guardado = &shell.actividades["com.bookos.player"];
        assert!(guardado.cola[0].portada.is_some());
    }

    /// Pasar de una actividad a otra con la rueda deja la isla como estaba:
    /// antes la nueva nacía recogida y había que volver a abrirla.
    #[test]
    fn pasar_con_la_isla_abierta_la_deja_abierta() {
        let mut shell = Shell::con_config(1280, 1.0, Config::default());
        shell.publicar_actividad(estado(None));
        shell.publicar_actividad(Estado {
            app_id: "com.bookos.clock".into(),
            clase: Clase::Timer,
            ..estado(None)
        });
        assert!(shell.alternar_actividad());
        let antes = shell.actividad_app_id().map(str::to_string);
        assert!(shell.actividad_pasar(1));
        assert_ne!(shell.actividad_app_id().map(str::to_string), antes);
        assert!(shell.actividad_abierta(), "sigue desplegada");
        // Recogida, pasar no la abre.
        assert!(shell.alternar_actividad());
        assert!(shell.actividad_pasar(1));
        assert!(!shell.actividad_abierta());
    }

    #[test]
    fn zenith_sale_con_las_actividades_apagadas() {
        let mut shell = Shell::con_config(1280, 1.0, Config::default());
        let zenith = Estado {
            app_id: "com.bookos.assistant".into(),
            clase: Clase::Asistente(actividad::Fase::Escuchando),
            ..estado(None)
        };
        shell.publicar_actividad(estado(None));
        shell.publicar_actividad(zenith.clone());
        shell.aplicar_actividades_config(config::Actividades {
            habilitadas: false,
            ..Default::default()
        });
        // Apagar se lleva al reproductor y deja a Zenith.
        assert_eq!(shell.actividad_app_id(), Some("com.bookos.assistant"));
        assert!(!shell.actividades.contains_key("com.bookos.player"));
        shell.publicar_actividad(estado(None));
        assert!(!shell.actividades.contains_key("com.bookos.player"));
        // Y aparece aunque llegue con ellas ya apagadas.
        shell.actividades.clear();
        shell.actividad = None;
        shell.publicar_actividad(zenith);
        assert_eq!(shell.actividad_app_id(), Some("com.bookos.assistant"));
    }
}
