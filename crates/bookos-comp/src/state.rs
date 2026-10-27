//! Estado del compositor y su bucle de eventos.
//!
//! Dos decisiones de este fichero condicionan todo el consumo del escritorio:
//!
//! 1. **El bucle es calloop, no un bucle activo.** El ejemplo `minimal` de
//!    Smithay gira sin parar llamando a `dispatch_new_events`; eso mantiene un
//!    core despierto el 100% del tiempo. Aquí el proceso duerme en `epoll` y
//!    solo despierta cuando hay algo real que atender.
//! 2. **Redibujar es opt-in.** `needs_redraw` solo se marca cuando un cliente
//!    hace commit o cambia el layout. Sin clientes pidiendo frame callback, no
//!    se dibuja nada y la GPU no se toca.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Instant;

use smithay::desktop::{Space, Window};
use smithay::input::pointer::PointerHandle;
use smithay::input::{Seat, SeatState};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::IsAlive;
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState};
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;

/// Lo que se está escribiendo en la pantalla de bloqueo.
///
/// Vive en el compositor y **no** en el shell: el shell corre dentro de un
/// `catch_unwind` y su estado se descarta al primer panic, que es el último
/// sitio donde debe estar una contraseña. Aquí, además, es donde se puede
/// borrar de verdad al terminar.
#[derive(Default)]
pub struct Bloqueo {
    pub escrito: String,
    pub comprobando: Option<crate::autenticar::Comprobacion>,
    pub huella: Option<crate::autenticar::Comprobacion>,
    pub huella_reintentar: Option<Instant>,
    /// PAM ya dijo que no hay con qué leer una huella en esta máquina. No se
    /// vuelve a intentar mientras dure el bloqueo: cada intento son varios
    /// segundos de espera y un mensaje de error por algo que no va a cambiar.
    pub huella_sin_lector: bool,
    /// Cuántas veces ha vuelto el equipo de dormir con este bloqueo echado.
    /// La conversación con el lector compara el valor con el que tenía al
    /// lanzarse: si cambió, su «no disponible» lo provocó la suspensión.
    pub huella_reanudaciones: u32,
    /// El último intento falló y aún no se ha vuelto a escribir.
    pub fallo: bool,
    /// Fallos consecutivos y barrera local adicional a la que aplique PAM.
    /// Evita fuerza bruta incluso con una pila PAM sin `pam_faillock`.
    pub fallos: u32,
    pub reintentar_desde: Option<Instant>,
}

impl Bloqueo {
    /// Quita el último carácter y pone a ceros el hueco que deja.
    ///
    /// `String::pop()` a secas solo mueve la longitud: el byte del carácter
    /// borrado se queda tal cual en la capacidad reservada, legible hasta que
    /// algo lo pise. Con un `Backspace` a media contraseña y luego un cierre
    /// de sesión sin volver a escribir ahí, ese byte sobrevivía al `clear()`
    /// de más abajo porque nunca estuvo dentro de la longitud que se pone a
    /// cero.
    pub fn borrar_ultimo(&mut self) {
        self.escrito.pop();
        zerar_capacidad_no_usada(unsafe { self.escrito.as_mut_vec() });
    }

    /// Vacía la contraseña escrita, poniendo sus bytes a cero antes de
    /// soltarlos.
    ///
    /// `String::clear()` a secas no basta: pone la longitud a 0 y dejar los
    /// bytes tal cual en el montón. Es el mismo argumento que
    /// [`crate::autenticar::borrar`] para la copia que ve PAM, y por lo
    /// mismo: este compositor vive toda la sesión y puede acabar en un
    /// volcado, y lo que no se borre queda ahí legible. `write_volatile`
    /// impide que el optimizador se salte la escritura por creer que nadie
    /// vuelve a leerla, que es justo el caso.
    pub fn olvidar_escrito(&mut self) {
        // SAFETY: cero es un byte UTF-8 válido, así que la cadena sigue siendo
        // válida en todo momento mientras se pisa su contenido.
        let bytes = unsafe { self.escrito.as_mut_vec() };
        for b in bytes.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        zerar_capacidad_no_usada(bytes);
        self.escrito.clear();
    }
}

/// Pone a ceros la parte de la capacidad reservada que no cubre la longitud
/// actual: donde se queda un carácter recién borrado con `pop()`, o el resto
/// de una asignación más grande que la que hace falta ahora.
fn zerar_capacidad_no_usada(v: &mut Vec<u8>) {
    for hueco in v.spare_capacity_mut() {
        // SAFETY: escribir un byte inicializado sobre memoria reservada pero
        // no necesariamente inicializada es exactamente para lo que está
        // `MaybeUninit::as_mut_ptr` aquí.
        unsafe { std::ptr::write_volatile(hueco.as_mut_ptr(), 0) };
    }
}

#[cfg(test)]
mod tests_bloqueo {
    use super::Bloqueo;

    /// Mira el montón de verdad, no solo lo que `String` deja ver: la
    /// capacidad reservada por debajo de la longitud actual, que es donde se
    /// queda el byte que `String::clear()`/`pop()` no tocan.
    fn capacidad_a_ceros(s: &String) -> bool {
        // SAFETY: solo se lee, y `capacity()` es lo que ya reservó el String.
        let bytes = unsafe { std::slice::from_raw_parts(s.as_ptr(), s.capacity().max(s.len())) };
        bytes.iter().all(|&b| b == 0)
    }

    #[test]
    fn olvidar_escrito_pone_los_bytes_a_cero_y_no_solo_la_longitud() {
        let mut b = Bloqueo {
            escrito: String::with_capacity(32),
            ..Default::default()
        };
        b.escrito.push_str("hunter2");
        assert!(
            !capacidad_a_ceros(&b.escrito),
            "la prueba no prueba nada si ya estaba a cero"
        );

        b.olvidar_escrito();

        assert_eq!(b.escrito.len(), 0);
        assert!(
            capacidad_a_ceros(&b.escrito),
            "quedaron bytes de la contraseña en el montón tras olvidar_escrito"
        );
    }

    #[test]
    fn borrar_ultimo_no_deja_el_caracter_quitado_legible() {
        let mut b = Bloqueo {
            escrito: String::with_capacity(32),
            ..Default::default()
        };
        b.escrito.push_str("secreto");

        b.borrar_ultimo();

        assert_eq!(b.escrito, "secret");
        // La 'o' que se quitó vivía justo después de la longitud nueva: si
        // sigue ahí, el borrado de la cola no funcionó.
        let bytes = unsafe { std::slice::from_raw_parts(b.escrito.as_ptr(), b.escrito.capacity()) };
        assert!(
            bytes[b.escrito.len()..].iter().all(|&x| x == 0),
            "queda algo del carácter borrado en la capacidad reservada"
        );
    }
}

/// Correspondencia entre una ventana viva y el identificador estable que ven
/// barras de tareas y herramientas externas mediante foreign-toplevel-list.
pub struct ToplevelPublicado {
    pub window: Window,
    pub handle: ForeignToplevelHandle,
    pub titulo: String,
    pub app_id: String,
}

/// Un gesto en curso sobre el escritorio.
pub enum ArrastreEscritorio {
    /// La banda elástica, desde ese punto lógico. `previa` es la selección que
    /// había al empezar: con Ctrl la banda **suma** a lo que ya estaba
    /// marcado, y sin él va vacía.
    Banda {
        origen: smithay::utils::Point<f64, smithay::utils::Logical>,
        previa: Vec<bool>,
    },
    /// Iconos agarrados. Hasta que el ratón no se aleja del umbral no se mueve
    /// nada: sin eso, un clic con la mano poco firme recolocaría el icono.
    Iconos {
        origen: smithay::utils::Point<f64, smithay::utils::Logical>,
        movido: bool,
    },
}

/// El permiso de compartir pantalla que la tarjeta tiene abierto.
///
/// Los dos emisarios van al hilo del portal, que está esperándolos: primero la
/// respuesta del usuario y, si es que sí, el identificador del nodo que dará
/// PipeWire. **Soltar este `Consentimiento` sin contestar es la negativa**, así
/// que no hay camino por el que el portal se quede colgado.
pub struct Consentimiento {
    pub sesion: u32,
    pub ruta: zbus::zvariant::OwnedObjectPath,
    pub respuesta: crate::portal::Emisario<(u32, u32)>,
    pub nodo: crate::portal::Emisario<u32>,
    /// Las salidas que se le enseñaron, en el mismo orden que las celdas de la
    /// tarjeta: la respuesta viene como índice de esta lista.
    pub salidas: Vec<smithay::output::Output>,
}

/// Un rectángulo de la pantalla que hay que fotografiar, en lógicos.
#[derive(Debug, Clone, Copy)]
pub struct CapturaPedida {
    pub x: i32,
    pub y: i32,
    pub ancho: i32,
    pub alto: i32,
    /// A un fichero. Con `false` va al portapapeles.
    pub guardar: bool,
}

/// Apaga (`false`) o reactiva las superficies KMS.
pub type AplicarDpms = Box<dyn FnMut(bool) -> Result<(), String>>;

pub struct BookosComp {
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, BookosComp>,
    pub loop_signal: LoopSignal,
    pub start_time: Instant,
    pub socket_name: OsString,

    /// Marca de trabajo pendiente para el backend. El backend la consulta en
    /// su punto de dibujo y la limpia; nadie dibuja "por si acaso".
    pub needs_redraw: bool,
    pub brillo_poll_activo: bool,
    pub brillo_auto: crate::brillo_auto::Estado,
    pub atenuacion: crate::atenuacion::Estado,
    pub salvapantallas: crate::salvapantallas::Estado,
    /// Lo que se lleva escrito en la pantalla de bloqueo.
    pub bloqueo: Bloqueo,
    /// Cuándo llegó el último evento de entrada. Decide si el backend anidado
    /// sondea rápido o se relaja.
    pub last_input: Option<Instant>,
    /// Tiempo de inactividad configurado para el bloqueo automático.
    pub bloqueo_inactividad: std::time::Duration,
    /// Tiempo bloqueado antes de pedir a logind una suspensión automática.
    pub suspension_inactividad: std::time::Duration,
    /// Alarma de una sola ejecución para bloquear al vencer la inactividad.
    /// No se usa un tick periódico: cuando no hay entrada, el proceso duerme
    /// hasta la fecha exacta del bloqueo.
    pub tick_bloqueo: Option<smithay::reexports::calloop::RegistrationToken>,
    pub tick_suspension: Option<smithay::reexports::calloop::RegistrationToken>,
    /// El segundero del centro de control abierto. Ver `multimedia::vigilar_centro`.
    pub tick_centro: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Repetición de Retroceso sobre una capa del shell: ver `input::repetir_retroceso`.
    pub repeticion_retroceso: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Qué conexión del bus publicó cada actividad viva, por `app_id`. Ver
    /// `ajustes::vigilar_remitentes`.
    pub remitentes_actividad: std::collections::HashMap<String, String>,
    /// Alarma que apaga físicamente las salidas unos segundos después de
    /// echar el bloqueo.
    pub tick_dpms: Option<smithay::reexports::calloop::RegistrationToken>,
    pub dpms_encendido: bool,
    /// Hasta cuándo la entrada no enciende la pantalla tras apagarla para
    /// dormir: la suelta del clic o del Intro que pidió dormir llega después.
    pub dpms_sordo_hasta: Option<std::time::Instant>,
    /// Backend real: apagar o reactivar las superficies KMS. Anidado queda en
    /// `None`, porque la energía de la pantalla pertenece al anfitrión.
    pub aplicar_dpms: Option<AplicarDpms>,
    /// El ritmo de dibujo: qué cuesta cada fotograma y cuántos se pierden.
    pub metricas: crate::metricas::Metricas,
    /// Un despertar por segundo mientras el panel de diagnóstico esté puesto.
    /// Es la única fuente de sondeo del compositor y existe solo mientras se
    /// está mirando: sin esto los números se congelarían con el escritorio
    /// quieto, que es justo cuando hace falta ver que están a cero.
    pub tick_diagnostico: Option<smithay::reexports::calloop::RegistrationToken>,

    pub space: Space<Window>,
    /// Los menús y desplegables de los clientes (`xdg_popup`). Sin registrarlos
    /// aquí no reciben su primer `configure` y el protocolo no les deja
    /// mostrarse: ningún menú de ninguna aplicación llegaba a aparecer.
    pub popups: smithay::desktop::PopupManager,
    /// El menú abierto que aún no tiene el foco del teclado. Se le da con la
    /// primera tecla y no al abrirse: ver `XdgShellHandler::grab`.
    pub menu_sin_foco: Option<smithay::desktop::PopupGrab<BookosComp>>,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub toplevels_publicados: Vec<ToplevelPublicado>,
    pub seat: Seat<Self>,
    pub pointer: PointerHandle<Self>,
    /// Qué cursor toca pintar: el del tema, o una superficie del cliente.
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    pub cursor_theme: Option<crate::cursor::CursorTheme>,
    /// El fondo del escritorio, ya subido a textura. `None` si no se encontró
    /// ninguno: entonces se ve el color liso de siempre.
    pub fondo: Option<crate::fondo::Fondo>,
    /// El shader del fondo esmerilado y su textura de trabajo. `None` si el
    /// driver no compiló el shader: el escritorio sigue, con el panel opaco.
    pub cristal: Option<std::rc::Rc<std::cell::RefCell<crate::desenfoque::Cristal>>>,
    /// El contador de cambios del cristal. Sube en cada frame porque lo que
    /// desenfoca —el escritorio de debajo— puede haber cambiado.
    pub cristal_commit: smithay::backend::renderer::utils::CommitCounter,
    /// Posición del cursor en coordenadas lógicas del compositor.
    pub pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    /// El shell in-process. `None` hasta que el backend sabe el tamaño de la
    /// pantalla.
    pub shell: Option<crate::shell::ShellHost>,
    /// Los gestos de touchpad en curso. Ver [`crate::gestos`].
    pub gestos: crate::gestos::Gestos,
    /// Los escritorios virtuales. Ver [`crate::escritorios`].
    pub escritorios: crate::escritorios::Escritorios,
    /// La ventana a la que lleva cada celda del conmutador, en el mismo orden.
    ///
    /// Vive aquí y no en el shell porque el shell no ve ventanas: elige un
    /// índice y el compositor lo traduce. Es lo que permite que una celda sea
    /// una ventana concreta sin que el shell sepa qué es una ventana.
    pub menu_ventana_companera: Option<smithay::desktop::Window>,
    pub multitarea: crate::multitarea::Multitarea,
    pub conmutador_destinos: Vec<smithay::desktop::Window>,
    pub menu_ventana: Option<smithay::desktop::Window>,
    pub menu_ventana_suelta: bool,
    /// Modo del selector abierto. También identifica qué modificador debe
    /// soltarse para confirmar: Alt en aplicaciones, Meta en ventanas.
    pub conmutador_modo: Option<bookos_shell::conmutador::Modo>,
    /// Se ha soltado el modificador con el conmutador abierto.
    ///
    /// La acción no puede ejecutarse dentro del filtro de `kbd.input`: allí hay
    /// que devolver `Forward` para que la suelta de Alt llegue al cliente, y
    /// `Intercept` se la traga. Se marca aquí y se resuelve al volver.
    pub conmutador_resolver: bool,
    /// Un clic que empezó sobre el selector no debe entregar su liberación al
    /// cliente que acaba de recibir el foco.
    pub conmutador_clic: bool,
    /// Cuándo se pulsó Meta+Alt+Retroceso la primera vez: la segunda, poco
    /// después, es la que cierra.
    pub salida_armada: Option<std::time::Instant>,
    /// Meta se pulsó y desde entonces no ha pasado nada más.
    ///
    /// Es lo que hace que **Meta a secas** abra el launchpad sin robarle
    /// Meta+algo a nadie: la tecla sola solo significa algo al soltarla, y
    /// cualquier otra tecla o clic por el medio la deja en un modificador
    /// normal. Se decide al soltar y no al pulsar porque al pulsar todavía no se
    /// sabe si va a ser un atajo.
    pub meta_sola: bool,
    /// El dedo que está haciendo de ratón sobre el shell, si hay uno. Ver
    /// `input::toque_abajo`.
    pub toque_emulado: Option<smithay::backend::input::TouchSlot>,
    /// El teclado en pantalla: cuándo sale y qué tecla tiene apretada.
    pub teclado: crate::teclado::Estado,
    pub emojis: crate::emojis::Estado,
    /// Hay un launchpad pendiente de abrir por la suelta de Meta.
    pub abrir_launchpad: bool,
    /// Los remapeos de `teclas.conf`. Se releen con `ReloadConfig("teclas")`.
    pub teclas: crate::teclas::Remapeos,
    pub traductor: crate::teclas::Traductor,
    /// La app de teclas espera la próxima tecla (`CaptureKey`).
    pub captura_tecla: bool,
    /// El shader del «magic lamp», compilado al arrancar el backend. `None` si
    /// el driver no lo acepta: entonces el minimizar encoge sin deformar.
    pub genio: Option<crate::genio::Genio>,
    /// El shader del velo de la captura, compilado al arrancar el backend.
    /// `None` si el driver no lo acepta: el velo sale con esquinas rectas.
    pub velo_captura: Option<crate::captura::Velo>,
    /// La banda elástica redondeada. Ver [`crate::banda`].
    pub banda: Option<crate::banda::Banda>,
    /// Las ventanas congeladas que se están yendo al dock —o volviendo—, con la
    /// textura de la que sale su deformación.
    pub capturas: Vec<(smithay::desktop::Window, crate::genio::Captura)>,
    /// Las ventanas que se están cerrando, pintadas desde las texturas que
    /// dejaron. Ver [`crate::cierre`].
    pub cierres: Vec<crate::cierre::Cierre>,
    /// Una ventana cogida de una miniatura de la vista de escritorios.
    pub arrastre_vista: Option<crate::escritorios::ArrastreVista>,
    /// La pulsación cayó en el ✕ de una ventana de la vista: el soltar
    /// también es nuestro, o el shell lo tomaría por un clic suelto.
    pub cierre_en_vista: bool,
    /// El cierre rasterizado; el tema y el hover invalidan también la textura.
    pub boton_cerrar_vista: Option<(
        i32,
        bookos_shell::tema::Tema,
        bool,
        smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    )>,
    /// La foto del tema anterior mientras se desvanece. Ver [`crate::fundido`].
    pub fundido: Option<crate::fundido::Fundido>,
    pub transicion_vista: crate::escritorios::TransicionVista,
    /// Con qué contexto GL se subieron las texturas de las superficies. Hace
    /// falta para pedírselas a Smithay fuera del bucle de pintado.
    pub contexto_gl: Option<
        smithay::backend::renderer::ContextId<smithay::backend::renderer::gles::GlesTexture>,
    >,
    /// Sube en cada frame mientras haya alguna en marcha: el elemento cambia de
    /// forma sin cambiar de sitio, y sin esto el damage tracker lo daría por
    /// quieto y no lo repintaría.
    pub genio_commit: smithay::backend::renderer::utils::CommitCounter,
    /// El botón de barra de título bajo el cursor y el que se está pulsando.
    pub decoracion: crate::decoracion::Interaccion,
    /// Ventanas minimizadas, con la posición a la que vuelven.
    ///
    /// Igual que las de otro escritorio: fuera del `Space` pero vivas, con su
    /// buffer intacto. La lista es del compositor y no del `Escritorios` porque
    /// una minimizada no pertenece a ningún escritorio mientras está guardada:
    /// vuelve al que esté activo cuando se restaure.
    pub minimizadas: Vec<crate::escritorios::Apartada>,
    /// Las que se están yendo al dock y todavía se dibujan.
    pub minimizando: Vec<crate::escritorios::Apartada>,
    /// El conmutador se abrió con un gesto y **no** se cierra al soltar teclas.
    ///
    /// El de Alt+Tab vive mientras sostienes el modificador; la exposición de
    /// tres dedos no tiene ninguno que sostener, así que sin esto la primera
    /// tecla que se soltara la daría por resuelta y saltaría a una ventana.
    pub conmutador_pegado: bool,
    /// Cuándo se abrió la exposición de tres dedos, para que las ventanas
    /// viajen desde su sitio hasta su miniatura. Alt+Tab y Meta+Tab no lo
    /// ponen: se abren y se cierran en lo que dura una pulsación, y un viaje
    /// de 260 ms los haría ir por detrás del dedo.
    pub conmutador_entrada: Option<std::time::Instant>,
    /// Cuándo empezó a cerrarse la exposición: las ventanas vuelven a su sitio
    /// por el mismo camino por el que llegaron. Sin esto el gesto de bajar
    /// borraba la rejilla de golpe y las ventanas aparecían ya colocadas.
    pub conmutador_salida: Option<std::time::Instant>,
    /// La configuración del escritorio, leída una vez al arrancar.
    ///
    /// El backend la consulta para la escala **antes** de que exista el shell
    /// —hay que saberla para crear la salida— y luego se la entrega a él, que
    /// es quien la consume. Es `Option` justamente por ese traspaso.
    pub config: Option<bookos_shell::Config>,
    /// La escala que pide la configuración, si pide alguna.
    ///
    /// Es copia de lo que hay en `config`, y existe porque esa se la lleva el
    /// shell al construirse: el backend sigue necesitando el dato después, cada
    /// vez que la pantalla cambia de tamaño.
    pub escala_forzada: Option<f64>,
    /// Lo que hay que pedirle a libinput sobre cada dispositivo que aparezca.
    pub entrada: bookos_shell::Entrada,
    /// Si el touchpad está encendido. Se guarda aquí y no en el dispositivo
    /// porque tras un cambio de TTY libinput los vuelve a crear de cero: sin
    /// esto, el touchpad que habías apagado volvería encendido.
    pub touchpad_activo: bool,
    /// `sonido_volumen` de `panel.conf`; la config entera no se guarda.
    pub sonido_volumen: bool,
    /// `sonidos_interfaz` de `panel.conf`: bloquear, desbloquear, capturar e
    /// iniciar sesión. Ver `multimedia::sonar_interfaz`.
    pub sonidos_interfaz: bool,
    /// `genio` de `panel.conf`: el shader sigue compilado, esto solo decide si se usa.
    pub genio_activo: bool,
    /// Si estaba enchufado en el último aviso de `power_supply`, para sonar
    /// solo cuando cambia. Ver `multimedia::cargador`.
    pub enchufado: Option<bool>,
    /// Cómo encender y apagar los touchpads que ya están abiertos. Lo rellena
    /// el backend de sesión real; anidado se queda en `None` porque ahí la
    /// entrada la da el compositor de debajo.
    pub aplicar_touchpad: Option<Box<dyn Fn(bool)>>,
    /// Reaplica velocidad, toque y scroll a todos los dispositivos abiertos.
    /// Sin esto los ajustes de entrada solo llegaban a los que aparecían
    /// después del cambio, o sea, al volver a iniciar sesión.
    pub aplicar_entrada: Option<Box<dyn Fn(&bookos_shell::Entrada, bool)>>,
    /// Distribución con la que se compiló el teclado ahora mismo; evita
    /// recompilar el mapa (y reenviarlo a los clientes) en recargas que no la
    /// tocan.
    pub teclado_activo: Option<String>,
    /// `(retardo ms, repeticiones/s)` puestos en el teclado del seat. smithay
    /// no deja leerlos de vuelta y el Retroceso del launchpad los necesita.
    pub teclado_repeticion: (u32, u32),
    /// Cuánto mueven las teclas de volumen y brillo, de `panel.conf`.
    pub pasos: bookos_shell::Pasos,
    /// La repetición de volumen o brillo con la tecla mantenida.
    pub repeticion_multimedia: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Si el sonido del volumen espera a que se suelte su tecla.
    pub chasquido: crate::multimedia::Chasquido,
    /// El último volumen de salida visto, para enseñar el aviso cuando lo
    /// cambia otro (el gesto de unos auriculares, otra aplicación).
    pub volumen_visto: Option<(u8, bool)>,
    /// Cómo encender las luces de Bloq Mayús, Bloq Num y Bloq Despl en los
    /// teclados abiertos. Con libinput las luces no van solas: si el
    /// compositor no las manda, Bloq Mayús funciona pero no se enciende. Lo
    /// rellena el backend de sesión real, igual que `aplicar_touchpad`.
    pub aplicar_leds: Option<Box<dyn Fn(smithay::input::keyboard::LedState)>>,
    /// Las últimas luces que pidió el teclado, para un teclado que se conecte
    /// después con Bloq Mayús ya puesto.
    pub leds: smithay::input::keyboard::LedState,
    /// Rutas del fondo que pide la configuración: la de siempre y, si las hay,
    /// una por tema. Copiadas por lo mismo que la escala: la `Config` se la
    /// lleva el shell al construirse.
    ///
    /// Se guardan las **tres** y se elige al recargar, no al arrancar, porque
    /// el tema cambia en caliente y el fondo tiene que irse con él.
    pub fondo_config: crate::fondo::Eleccion,
    /// Hay que volver a cargar el fondo, porque cambió el tema o la
    /// configuración.
    ///
    /// Es una marca y no una llamada directa por lo mismo que `needs_redraw`:
    /// decodificar la imagen y subirla —la del fondo y la mipmapeada del
    /// cristal— necesita el `GlesRenderer`, que vive dentro del backend y no
    /// se ve desde aquí. El backend la consulta en su punto de dibujo, donde
    /// sí lo tiene.
    pub recargar_fondo: bool,
    /// Despertar del próximo fotograma WebP. No existe con fondos estáticos
    /// ni con movimiento reducido.
    pub tick_fondo: Option<smithay::reexports::calloop::RegistrationToken>,
    /// El RGBA cambió y la textura mipmapeada del cristal sigue siendo la del
    /// fotograma anterior. Se consume una vez al construir la escena.
    pub fondo_cristal_pendiente: bool,
    /// El fondo que se está yendo y cuándo empezó a irse, mientras dura el
    /// fundido con el que entra. `None` fuera de la transición.
    ///
    /// Son otros veinte megas de textura viva durante esos milisegundos, y por
    /// eso se suelta en cuanto acaba en vez de guardarse por si acaso.
    pub fondo_saliente: Option<(crate::fondo::Fondo, Instant)>,
    /// Lo que el usuario eligió en Apariencia: claro, oscuro o que siga la hora.
    ///
    /// El tema **efectivo** vive en `bookos_shell::tema`, que es global del
    /// proceso; esto es lo otro, lo elegido, y de los dos sale el que se pinta.
    pub modo_tema: bookos_shell::tema::ModoTema,
    /// Las dos horas del modo automático.
    pub horas_tema: (
        bookos_shell::tema::HoraDelDia,
        bookos_shell::tema::HoraDelDia,
    ),
    /// El despertar del próximo cambio automático. `None` con un modo fijo: sin
    /// nada que esperar no se deja ningún temporizador puesto.
    pub tick_tema: Option<smithay::reexports::calloop::RegistrationToken>,
    /// La luz nocturna: lo configurado, lo que hay puesto y sus despertares.
    pub luz_nocturna: crate::luz_nocturna::Estado,
    /// Cómo poner la curva de gamma en las pantallas. Solo la rellena el
    /// backend real: anidado no hay curva que tocar.
    pub aplicar_gamma: Option<crate::luz_nocturna::AplicarGamma>,
    /// Tamaño lógico del cursor. Copia de la configuración por lo mismo que
    /// `escala_forzada`: el tema se vuelve a cargar cada vez que cambia la
    /// escala, y para entonces la `Config` ya se la llevó el shell.
    pub cursor_nominal: u32,
    /// Cuándo y dónde fue la última pulsación del botón izquierdo, para poder
    /// reconocer el doble clic. Se guarda el punto además del instante porque
    /// dos clics seguidos en sitios distintos son dos clics, no uno doble.
    pub ultimo_clic: Option<(Instant, smithay::utils::Point<f64, smithay::utils::Logical>)>,
    /// La ventana que el usuario está moviendo o redimensionando ahora mismo.
    /// Mientras dura, los eventos del puntero son del compositor y no llegan al
    /// cliente: si no, la terminal que arrastras cree que estás seleccionando
    /// texto.
    pub arrastre: Option<crate::ventanas::Arrastre>,
    /// Lo que se está arrastrando **sobre el escritorio**: la banda elástica o
    /// un puñado de iconos. Va aparte de `arrastre`, que es de ventanas: aquí
    /// no hay ningún cliente de por medio y el gesto no puede acabar encajando
    /// nada contra un borde.
    pub arrastre_escritorio: Option<ArrastreEscritorio>,

    /// Cómo saltar a otro terminal virtual. Lo rellena el backend de sesión
    /// real; anidado se queda en `None` porque el TTY no es nuestro.
    ///
    /// Es un cierre y no la `LibSeatSession` para que este fichero no dependa
    /// de libseat: con la característica `udev` apagada el tipo ni existe.
    pub cambiar_vt: Option<Box<dyn Fn(i32)>>,
    /// Programas lanzados desde la sesión. Se guardan solo para recogerlos y
    /// que no queden zombis colgando del compositor.
    pub hijos: Vec<std::process::Child>,
    /// Escritorio completo, o una sola aplicación y nada más.
    pub modo: Modo,

    /// El gestor de ventanas X11, cuando XWayland ya está en pie. `None`
    /// mientras arranca, y para siempre si `Xwayland` no está instalado.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    /// El número de pantalla de XWayland, para el `DISPLAY` de los hijos.
    pub display_x11: Option<u32>,
    /// El cliente Wayland que es XWayland, para fijarle la escala.
    pub cliente_x11: Option<smithay::reexports::wayland_server::Client>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    /// `xdg-decoration`: por él se reclama dibujar la barra de título. Nadie lo
    /// lee después de crearlo, pero soltarlo retiraría el global y los clientes
    /// volverían a decorarse solos.
    #[allow(dead_code)]
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub shm_state: ShmState,
    /// Nadie lo lee, pero tiene que seguir vivo: al soltarse se retiran los
    /// globales `wl_output` y `xdg_output_manager` y los clientes dejan de ver
    /// las pantallas.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    /// La selección primaria: lo que se copia con solo seleccionar y se pega
    /// con el botón central. Es de X11 de toda la vida y en Linux se usa a
    /// diario; sin ella, el botón central no pega nada.
    pub primary_selection_state: PrimarySelectionState,
    /// `wlr-data-control`: deja que un programa **lea y escriba** el
    /// portapapeles sin tener ventana ni foco. Es lo que necesitan los
    /// gestores de historial (`clipman`, `cliphist`) y `wl-copy`/`wl-paste`.
    pub data_control_state: DataControlState,
    /// La conexión al bus de sesión donde vive el servidor de notificaciones.
    /// Hay que guardarla: al soltarla se cierra el bus y se pierde el nombre.
    /// `None` si no había bus con quien hablar, o si el nombre ya estaba cogido
    /// por otro escritorio —lo normal desarrollando anidado dentro de Plasma—.
    pub bus_notificaciones: Option<zbus::blocking::Connection>,
    /// Interfaz privada de la sesión (`org.bookos.Desktop`) para que BookOS
    /// Settings recargue la configuración y reconfigure las pantallas. Hay que
    /// guardarla por lo mismo que la de notificaciones —al soltarla se pierde
    /// el nombre en el bus— y además es por donde sale `OutputsChanged`.
    pub bus_ajustes: Option<zbus::blocking::Connection>,
    /// El censo de pantallas y lo último que se aplicó. Ver [`crate::pantallas`].
    pub pantallas: crate::pantallas::Estado,
    /// Cómo reconfigurar las pantallas. Lo rellena el backend; es el único
    /// camino desde D-Bus hasta KMS, y pasa siempre por este hilo.
    pub aplicar_pantallas: Option<crate::pantallas::Aplicador>,
    /// Cómo volver a mirar qué hay enchufado, sin cambiar nada.
    pub censar_pantallas: Option<crate::pantallas::Censador>,
    /// Entrega al bucle la lectura MPRIS hecha después de echar el bloqueo.
    /// Lleva una generación para que una consulta lenta de un bloqueo anterior
    /// no aparezca en el siguiente.
    pub medios_bloqueo:
        smithay::reexports::calloop::channel::Sender<(u64, Option<bookos_shell::medios::Sonando>)>,
    pub bloqueo_generacion: u64,
    /// `wp_fractional_scale` + `wp_viewporter`: la pareja que permite a un
    /// cliente dibujar a 1,75× y decirle al compositor "recórtame a este
    /// tamaño lógico". Sin viewporter, la escala fraccional no sirve de nada.
    #[allow(dead_code)]
    pub fractional_scale_state: FractionalScaleManagerState,
    /// `wp_presentation`: los clientes reciben el vblank real y la frecuencia
    /// de la salida, necesario para animar a 120/144 Hz sin asumir 60 Hz.
    pub _presentation_state: PresentationState,
    /// `zwp_linux_dmabuf_v1`: el cliente dibuja en la GPU y nos pasa el
    /// **descriptor** del buffer, no sus píxeles.
    ///
    /// Sin este global, Mesa no encuentra forma de hacer EGL acelerado sobre
    /// Wayland y todo cliente cae a software: el navegador rasteriza por CPU y
    /// entrega el resultado por `wl_shm`, que aquí obliga a una copia CPU→GPU
    /// por ventana y por fotograma. Y como un buffer de memoria compartida
    /// nunca se puede exportar a un plano DRM, el direct scanout que promete
    /// la cabecera de `backend::udev` no podía ocurrir para ningún cliente.
    /// `zwlr_screencopy_v1`: copiar una salida a un buffer del cliente. Es lo
    /// que da capturas de pantalla y compartir pantalla. Ver [`crate::captura`].
    #[allow(dead_code)]
    pub captura_state: crate::captura::CapturaState,
    /// Las copias pedidas y todavía sin servir. Se sirven tras el próximo
    /// fotograma de su salida, que es de donde sale la imagen.
    pub capturas_pantalla: Vec<crate::captura::Pendiente>,
    /// Una captura pedida desde la capa del escritorio y todavía sin hacer.
    ///
    /// Se sirve al componer el siguiente fotograma, igual que las de
    /// `zwlr_screencopy`: la capa acaba de cerrarse y hay que dejar que el
    /// escritorio se dibuje sin ella antes de fotografiarlo.
    pub captura_pedida: Option<CapturaPedida>,
    /// El portal de escritorio (`org.freedesktop.impl.portal.*`). Nadie la lee:
    /// se guarda porque al soltarla se cierra el bus y se pierde el nombre, y
    /// entonces `xdg-desktop-portal` dejaría de encontrar el backend. Ver
    /// [`crate::portal`].
    #[allow(dead_code)]
    pub bus_portal: Option<zbus::blocking::Connection>,
    /// Las pantallas que se están compartiendo ahora mismo, y el hilo de
    /// PipeWire que las sirve. Ver [`crate::emision`].
    pub emisiones: crate::emision::Emisiones,
    /// El permiso de compartir pantalla que está en la tarjeta ahora mismo.
    ///
    /// Vive aquí y no en el shell porque el que espera es el hilo del portal:
    /// pase lo que pase con la tarjeta —se contesta, se cierra con un clic
    /// fuera, o el shell se cae— hay que mandarle **una** respuesta, y este es
    /// el sitio desde el que se ve todo eso.
    pub consentimiento: Option<Consentimiento>,
    /// Quién espera la ruta del PNG de la captura en curso, si la pidió el
    /// portal en vez de la tecla Impr.
    pub captura_portal: Option<crate::portal::Emisario<std::path::PathBuf>>,
    pub dmabuf_state: DmabufState,
    /// El global vivo. Al soltarlo desaparece y los clientes vuelven a SHM, así
    /// que hay que guardarlo aunque nadie lo lea. `None` hasta que el backend
    /// arranca: los formatos salen del contexto EGL, que todavía no existe.
    #[allow(dead_code)]
    pub dmabuf_global: Option<DmabufGlobal>,
    /// La conexión EGL del backend, solo para **validar** una importación.
    ///
    /// El `GlesRenderer` vive dentro del backend y el estado no lo ve; sacarlo
    /// hasta aquí sería repartir de nuevo la propiedad del renderer para
    /// contestar sí o no a una pregunta que `EGLDisplay` ya sabe contestar:
    /// si sabe hacer la `EGLImage`, el import del fotograma también podrá.
    pub egl_display: Option<smithay::backend::egl::EGLDisplay>,
    /// `xdg_activation_v1`: cómo una aplicación pide el foco sin robarlo.
    ///
    /// Sin esto, una ventana recién mapeada o se lleva el foco siempre —y te lo
    /// quita de donde estabas escribiendo— o no se lo lleva nunca, y lanzar
    /// algo desde el dock te deja mirando una ventana que no escribe. Con el
    /// protocolo hay una tercera vía: el compositor **le da un vale** a quien
    /// lanza, y la ventana que llega con ese vale es la que se enfoca.
    pub activacion_state: smithay::wayland::xdg_activation::XdgActivationState,
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    /// `wp_cursor_shape_v1`: el cliente dice **qué forma** quiere —"texto",
    /// "mano", "redimensionar"— y la dibuja el compositor con su tema, en vez
    /// de mandar él una superficie con la imagen ya pintada.
    ///
    /// Sin esto, cada toolkit elige el tema por su cuenta: medido, konsole
    /// dentro de esta sesión mandaba `Surface` con el cursor de Breeze que le
    /// da la integración de KDE, y el escritorio enseñaba dos cursores
    /// distintos según sobre qué estuviera el puntero. Con el protocolo pasa a
    /// pedir `Named(Text)` y lo dibujamos nosotros.
    #[allow(dead_code)]
    pub cursor_shape_state: smithay::wayland::cursor_shape::CursorShapeManagerState,
    /// `zwp_idle_inhibit_v1`: superficies de vídeo/presentación que piden que
    /// no se apague la pantalla mientras están activas.
    #[allow(dead_code)]
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub idle_inhibidores: Vec<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    /// `ext_idle_notifier_v1`: permite que salvapantallas y agentes de sesión
    /// conozcan el idle sin sondear dispositivos ni despertar el proceso.
    pub idle_notifier_state: IdleNotifierState<Self>,
    /// Protocolos que necesitan juegos y aplicaciones 3D para ocultar o
    /// confinar el puntero y recibir movimiento relativo.
    #[allow(dead_code)]
    pub pointer_constraints_state: PointerConstraintsState,
    #[allow(dead_code)]
    pub relative_pointer_state: RelativePointerManagerState,
    /// `zwp_pointer_gestures_v1`: pellizcos y «mantener» del touchpad para las
    /// ventanas. Sin él, el zoom con dos dedos de Figma o de un navegador no
    /// llegaba nunca: el compositor se quedaba todos los gestos.
    #[allow(dead_code)]
    pub pointer_gestures_state: smithay::wayland::pointer_gestures::PointerGesturesState,
    /// Entrada de texto v3 y canal para métodos de entrada (IBus/Fcitx).
    #[allow(dead_code)]
    pub text_input_state: TextInputManagerState,
    #[allow(dead_code)]
    pub input_method_state: InputMethodManagerState,
    /// Paneles, fondos y overlays externos compatibles con wlroots.
    pub layer_shell_state: WlrLayerShellState,
    /// El globales `xwayland_shell_v1`, por el que XWayland asocia cada ventana
    /// X11 con su `wl_surface`. Sin él las ventanas X11 nunca se emparejan con
    /// un buffer y se ven como huecos negros.
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
}

/// Para qué se ha arrancado el compositor.
///
/// El kiosko existe para el asistente de primer arranque (bookos-welcome): una
/// sola aplicación, como root y antes del gestor de acceso. No hay shell,
/// atajos ni gestos —un Meta+Return ahí abriría un terminal de root—, ni los
/// servicios de sesión, que sin bus de sesión fallarían igual. Cuando la
/// aplicación termina, el compositor termina con su mismo código, que es lo
/// que deja seguir al arranque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Escritorio,
    Kiosko {
        /// El código de salida del programa, cuando ya ha terminado.
        salida: Option<i32>,
    },
}

impl Modo {
    pub fn kiosko(&self) -> bool {
        matches!(self, Modo::Kiosko { .. })
    }
}

/// Qué distribución de teclado usar.
///
/// `XkbConfig::default()` es **us**, y eso es lo que salía: un teclado español
/// escribiendo en inglés. La configuración de teclado de esta máquina está
/// dicha en tres sitios a la vez —`XKB_DEFAULT_LAYOUT`, `localectl` y
/// `/etc/vconsole.conf`— y ninguno lo lee xkb por su cuenta.
///
/// Se mira, por orden: lo que diga `panel.conf`, la variable de entorno —que
/// es el estándar de facto y la que ponen las sesiones de Wayland— y el
/// `KEYMAP` de vconsole, que es lo que queda en un TTY pelado.
///
/// El `Box::leak` es deliberado: `XkbConfig` presta `&str` y esto se lee una
/// vez al arrancar y vive lo que dure la sesión. Un `String` en el estado
/// obligaría a arrastrar el préstamo por medio compositor para ahorrar treinta
/// bytes.
pub(crate) fn distribucion_teclado(
    config: Option<&str>,
) -> smithay::input::keyboard::XkbConfig<'static> {
    let layout = config
        .map(str::to_string)
        .or_else(|| std::env::var("XKB_DEFAULT_LAYOUT").ok())
        .or_else(keymap_de_vconsole)
        .filter(|s| !s.trim().is_empty());
    let variant = std::env::var("XKB_DEFAULT_VARIANT").ok();
    let options = std::env::var("XKB_DEFAULT_OPTIONS").ok();
    tracing::info!(?layout, ?variant, "distribución de teclado");
    smithay::input::keyboard::XkbConfig {
        layout: layout
            .map(|s| &*Box::leak(s.into_boxed_str()))
            .unwrap_or(""),
        variant: variant
            .map(|s| &*Box::leak(s.into_boxed_str()))
            .unwrap_or(""),
        options: options.map(|s| Box::leak(s.into_boxed_str()).to_string()),
        ..Default::default()
    }
}

/// `KEYMAP="es"` de `/etc/vconsole.conf`, que es lo que queda cuando no hay
/// sesión gráfica que ponga las variables.
fn keymap_de_vconsole() -> Option<String> {
    let texto = std::fs::read_to_string("/etc/vconsole.conf").ok()?;
    texto.lines().find_map(|l| {
        let valor = l.trim().strip_prefix("KEYMAP=")?;
        Some(valor.trim_matches('"').to_string())
    })
}

impl BookosComp {
    /// ¿Hay alguna animación en marcha?
    ///
    /// El backend lo usa para no relajarse mientras algo se mueve. Sin esto, el
    /// ritmo se decide **después** de dibujar, cuando `needs_redraw` ya se ha
    /// limpiado, y una animación de 180 ms se dibujaba a 33 ms por fotograma:
    /// medido, cuatro fotogramas para abrir el launchpad en vez de veintidós.
    pub fn hay_animacion(&self) -> bool {
        // El aviso solo cuenta como animación **mientras se está yendo**: los
        // 1,2 s que está quieto no cambian un píxel, y repintarlos sería
        // despertar ciento cuarenta veces para enseñar lo mismo.
        let osd_saliendo = self
            .shell
            .as_ref()
            .and_then(|s| s.osd_queda())
            .is_some_and(|q| q <= bookos_shell::osd::SALIDA);
        let toast_saliendo = self.shell.as_ref().is_some_and(|s| {
            s.toasts_quedan()
                .iter()
                .any(|q| *q <= bookos_shell::toast::SALIDA)
        });
        osd_saliendo
            || toast_saliendo
            // El fundido entre dos fondos: sin esto el bucle se dormiría a
            // mitad y el fondo nuevo se quedaría a medio aparecer.
            || self.fondo_saliente.is_some()
            // El recuadro de la captura se mueve con el ratón: sin esto el
            // bucle se dormiría a mitad del arrastre.
            || self.shell.as_ref().is_some_and(|s| s.captura_animando())
            || self
                .shell
                .as_ref()
                .is_some_and(|s| s.animando() || s.barras_animando())
            || self.space.elements().any(crate::ventanas::animando)
            || self.arrastre.as_ref().is_some_and(|a| a.animando())
            // El deslizamiento entre escritorios mueve ventanas en el `Space`,
            // no dentro de un buffer del shell, así que ninguna de las de
            // arriba lo ve. Sin esto el bucle se relajaba a mitad de la
            // transición y el cambio de escritorio salía a trompicones.
            || self.escritorios.deslizando()
            || self
                .conmutador_entrada
                .is_some_and(|desde| desde.elapsed() < bookos_shell::tema::D_VENTANA)
            || self.conmutador_salida.is_some()
    }

    pub fn new(
        event_loop: &mut EventLoop<'static, BookosComp>,
        display: Display<BookosComp>,
        modo: Modo,
    ) -> anyhow::Result<Self> {
        let dh = display.handle();
        let loop_handle = event_loop.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state =
            smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        // Se le pasa la selección primaria para que un cliente de data-control
        // pueda leer también esa, no solo el portapapeles: `wl-paste
        // --primary` es justo para lo que existe el protocolo.
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(&dh);
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<Self>(&dh);
        // Linux CLOCK_MONOTONIC. DRM usa este mismo reloj cuando el driver
        // anuncia timestamps monotónicos para los page-flips.
        let presentation_state = PresentationState::new::<Self>(&dh, 1);
        // El global se crea en el backend, cuando ya hay contexto EGL del que
        // sacar los formatos: anunciar una lista vacía sería peor que no
        // anunciar nada.
        let dmabuf_state = DmabufState::new();
        let captura_state = crate::captura::CapturaState::new(&dh);
        let activacion_state =
            smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let cursor_shape_state =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&dh);
        let idle_notifier_state = IdleNotifierState::new(&dh, loop_handle.clone());
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let pointer_gestures_state =
            smithay::wayland::pointer_gestures::PointerGesturesState::new::<Self>(&dh);
        let text_input_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_| true);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let xwayland_shell_state =
            smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);

        // La configuración se lee antes que el teclado: la distribución sale de
        // ahí si el fichero la dice. El kiosko corre como root antes de que
        // exista ninguna cuenta: el `panel.conf` que encontrara sería el de
        // /root o, al probarlo, el de quien lo lanza, y ninguno es del sistema.
        let mut config = match modo {
            Modo::Escritorio => bookos_shell::Config::cargar(),
            Modo::Kiosko { .. } => bookos_shell::Config::default(),
        };
        let teclado_config = config.teclado.clone();
        let locale = modo.kiosko().then(crate::locale1::actual).flatten();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "bookos");
        let teclado_repeticion = (config.teclado_retardo, config.teclado_velocidad);
        let pasos = config.pasos;
        seat.add_keyboard(
            match &locale {
                Some(distribucion) => distribucion.xkb(),
                None => distribucion_teclado(teclado_config.as_deref()),
            },
            teclado_repeticion.0 as i32,
            teclado_repeticion.1 as i32,
        )?;
        let pointer = seat.add_pointer();
        seat.add_touch();

        let socket_name = Self::init_socket(&loop_handle, display)?;

        let escala_forzada = config.escala;
        // Se saca de la `Config` antes de que el shell se la lleve entera: los
        // dispositivos no se configuran al arrancar sino cada vez que libinput
        // anuncia uno, y eso vuelve a pasar tras cada cambio de TTY.
        // El servidor de notificaciones: un canal hacia este bucle, y zbus
        // atendiendo el bus en su propio hilo. Va aquí y no en `init_wayland`
        // porque no tiene nada que ver con el socket de Wayland: son dos
        // conversaciones distintas con dos mundos distintos.
        let (avisos, fuente) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(fuente, |evento, _, state| {
                use smithay::reexports::calloop::channel::Event;
                if let Event::Msg(aviso) = evento {
                    crate::notificaciones::recibir(state, aviso);
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(notificaciones): {err}"))?;
        // En el kiosko no se arranca ninguno de los servicios de sesión: corre
        // como root sin bus de sesión, y bookos-system reintentaría cada 3 s
        // despertando el bucle para nada.
        let bus_notificaciones = match modo {
            Modo::Escritorio => crate::notificaciones::arrancar(avisos),
            Modo::Kiosko { .. } => None,
        };

        let (system_events, system_source) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(system_source, |event, _, state| {
                if let smithay::reexports::calloop::channel::Event::Msg(()) = event {
                    if let Some(shell) = state.shell.as_mut() {
                        shell.refresh();
                    }
                    crate::multimedia::recibir_sistema(state);
                    state.needs_redraw = true;
                }
            })
            .map_err(|e| anyhow::anyhow!("system events: {e}"))?;
        if !modo.kiosko() {
            bookos_system::start(move || {
                let _ = system_events.send(());
            });
        }

        // Settings escribe el fichero y solo manda una orden pequeña por
        // D-Bus. La lectura y aplicación se hacen aquí, en el hilo dueño del
        // shell, para no compartir estado gráfico entre hilos.
        let (ajustes, fuente_ajustes) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(fuente_ajustes, |evento, _, state| {
                use smithay::reexports::calloop::channel::Event;
                if let Event::Msg(aviso) = evento {
                    crate::ajustes::recibir(state, aviso);
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(ajustes): {err}"))?;
        let pantallas = crate::pantallas::Estado::default();
        let bus_ajustes = match modo {
            Modo::Escritorio => crate::ajustes::arrancar(ajustes, pantallas.compartido.clone()),
            Modo::Kiosko { .. } => None,
        };

        // El portal de escritorio, por el mismo camino: su hilo pide, este
        // bucle decide. Ver la cabecera de `crate::portal`.
        let (portal, fuente_portal) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(fuente_portal, |evento, _, state| {
                use smithay::reexports::calloop::channel::Event;
                if let Event::Msg(aviso) = evento {
                    crate::portal::recibir(state, aviso);
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(portal): {err}"))?;
        let bus_portal = match modo {
            Modo::Escritorio => crate::portal::arrancar(portal, &socket_name),
            Modo::Kiosko { .. } => None,
        };
        if modo.kiosko() {
            crate::locale1::escuchar(&loop_handle);
        }
        crate::ajustes::vigilar_nombres(
            [
                (bus_notificaciones.clone(), crate::notificaciones::INTERFAZ),
                (bus_ajustes.clone(), crate::ajustes::NOMBRE),
                (bus_portal.clone(), crate::portal::NOMBRE),
            ]
            .into_iter()
            .filter_map(|(c, n)| c.map(|c| (c, n)))
            .collect(),
        );

        // MPRIS puede lanzar varios procesos `busctl`; se consulta después de
        // que el bloqueo ya esté visible y en un hilo corto. El resultado
        // vuelve por calloop, igual que las notificaciones, sin bloquear frames.
        let (medios_bloqueo, fuente_medios) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(fuente_medios, |evento, _, state| {
                use smithay::reexports::calloop::channel::Event;
                if let Event::Msg((generacion, sonando)) = evento
                    && generacion == state.bloqueo_generacion
                    && let Some(shell) = state.shell.as_mut()
                {
                    shell.bloqueo_medio(sonando);
                    state.needs_redraw = true;
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(medios bloqueo): {err}"))?;

        let brillo_auto =
            crate::brillo_auto::Estado::arrancar(&loop_handle, config.brillo_automatico)?;

        let entrada = std::mem::take(&mut config.entrada);
        let cursor_nominal = config.cursor;
        let bloqueo_inactividad = std::time::Duration::from_secs(config.bloqueo_inactividad);
        let suspension_inactividad = std::time::Duration::from_secs(config.suspension_inactividad);
        let modo_tema = config.modo_tema;
        let horas_tema = (config.tema_claro_desde, config.tema_oscuro_desde);
        let luz_nocturna = crate::luz_nocturna::Estado::nuevo(config.luz_nocturna);
        let fondo_config = crate::fondo::Eleccion {
            ambos: config.fondo.clone(),
            claro: config.fondo_claro.clone(),
            oscuro: config.fondo_oscuro.clone(),
        };

        Ok(Self {
            display_handle: dh,
            loop_handle,
            loop_signal: event_loop.get_signal(),
            start_time: Instant::now(),
            socket_name,
            needs_redraw: true,
            brillo_poll_activo: false,
            brillo_auto,
            bloqueo: Bloqueo::default(),
            last_input: None,
            bloqueo_inactividad,
            suspension_inactividad,
            tick_bloqueo: None,
            atenuacion: crate::atenuacion::Estado::nuevo(config.atenuacion_inactividad),
            salvapantallas: crate::salvapantallas::Estado::nuevo(
                config.salvapantallas_inactividad,
                config.apagado_inactividad,
                config.salvapantallas_estilo,
            ),
            tick_suspension: None,
            tick_centro: None,
            repeticion_retroceso: None,
            remitentes_actividad: Default::default(),
            tick_dpms: None,
            dpms_encendido: true,
            dpms_sordo_hasta: None,
            aplicar_dpms: None,
            metricas: crate::metricas::Metricas::new(),
            tick_diagnostico: None,
            space: Space::default(),
            popups: smithay::desktop::PopupManager::default(),
            menu_sin_foco: None,
            foreign_toplevel_list_state,
            toplevels_publicados: Vec::new(),
            seat,
            pointer,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor_theme: None,
            fondo: None,
            cristal: None,
            cristal_commit: Default::default(),
            pointer_location: (0.0, 0.0).into(),
            shell: None,
            gestos: crate::gestos::Gestos::default(),
            escritorios: crate::escritorios::Escritorios::new(
                config.escritorios,
                config.nombres_escritorios.clone(),
            ),
            menu_ventana_companera: None,
            multitarea: crate::multitarea::Multitarea::default(),
            conmutador_destinos: Vec::new(),
            menu_ventana: None,
            menu_ventana_suelta: false,
            conmutador_modo: None,
            conmutador_resolver: false,
            conmutador_clic: false,
            salida_armada: None,
            meta_sola: false,
            toque_emulado: None,
            teclado: Default::default(),
            emojis: Default::default(),
            abrir_launchpad: false,
            // Los remapeos de `teclas.conf` pueden lanzar órdenes: en el
            // kiosko serían órdenes de root.
            teclas: match modo {
                Modo::Escritorio => crate::teclas::cargar(),
                Modo::Kiosko { .. } => Default::default(),
            },
            traductor: Default::default(),
            captura_tecla: false,
            conmutador_pegado: false,
            conmutador_entrada: None,
            conmutador_salida: None,
            genio: None,
            velo_captura: None,
            banda: None,
            capturas: Vec::new(),
            cierres: Vec::new(),
            arrastre_vista: None,
            cierre_en_vista: false,
            boton_cerrar_vista: None,
            fundido: None,
            transicion_vista: Default::default(),
            contexto_gl: None,
            genio_commit: Default::default(),
            decoracion: crate::decoracion::Interaccion::default(),
            minimizadas: Vec::new(),
            minimizando: Vec::new(),
            sonido_volumen: config.sonido_volumen,
            sonidos_interfaz: config.sonidos_interfaz,
            genio_activo: config.genio,
            // Leído al arrancar: entrar en la sesión con el cargador puesto no
            // es «conectarlo».
            enchufado: bookos_shell::cargador().map(|(e, _)| e),
            config: Some(config),
            escala_forzada,
            entrada,
            touchpad_activo: true,
            aplicar_touchpad: None,
            aplicar_entrada: None,
            teclado_activo: teclado_config.clone(),
            teclado_repeticion,
            pasos,
            repeticion_multimedia: None,
            chasquido: crate::multimedia::Chasquido::Libre,
            volumen_visto: None,
            aplicar_leds: None,
            leds: Default::default(),
            cursor_nominal,
            fondo_config,
            recargar_fondo: false,
            tick_fondo: None,
            fondo_cristal_pendiente: false,
            fondo_saliente: None,
            modo_tema,
            horas_tema,
            tick_tema: None,
            luz_nocturna,
            aplicar_gamma: None,
            arrastre: None,
            arrastre_escritorio: None,
            ultimo_clic: None,
            cambiar_vt: None,
            hijos: Vec::new(),
            modo,
            xwm: None,
            display_x11: None,
            cliente_x11: None,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            bus_notificaciones,
            bus_ajustes,
            pantallas,
            aplicar_pantallas: None,
            censar_pantallas: None,
            medios_bloqueo,
            bloqueo_generacion: 0,
            fractional_scale_state,
            _presentation_state: presentation_state,
            captura_state,
            capturas_pantalla: Vec::new(),
            captura_pedida: None,
            bus_portal,
            emisiones: crate::emision::Emisiones::new(),
            consentimiento: None,
            captura_portal: None,
            dmabuf_state,
            dmabuf_global: None,
            egl_display: None,
            activacion_state,
            viewporter_state,
            cursor_shape_state,
            idle_inhibit_state,
            idle_inhibidores: Vec::new(),
            idle_notifier_state,
            pointer_constraints_state,
            relative_pointer_state,
            pointer_gestures_state,
            text_input_state,
            input_method_state,
            layer_shell_state,
            xwayland_shell_state,
        })
    }

    /// Publica el socket Wayland y engancha tanto las conexiones nuevas como el
    /// tráfico de las existentes al bucle de eventos.
    fn init_socket(
        loop_handle: &LoopHandle<'static, BookosComp>,
        display: Display<BookosComp>,
    ) -> anyhow::Result<OsString> {
        use smithay::reexports::wayland_server::ListeningSocket;

        let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
        let socket_name = listener
            .socket_name()
            .ok_or_else(|| anyhow::anyhow!("el socket no tiene nombre"))?
            .to_os_string();

        // ListeningSocket no es una fuente de calloop por sí misma; se vigila su
        // fd y se acepta a mano.
        let mut dh = display.handle();
        loop_handle
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                move |_, listener, _state| {
                    while let Some(stream) = listener.accept()? {
                        if let Err(err) = dh.insert_client(stream, Arc::new(ClientState::default()))
                        {
                            tracing::warn!("no se pudo aceptar el cliente: {err}");
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow::anyhow!("insert_source(listener): {err}"))?;

        // Tráfico de los clientes ya conectados. El fd del display se vigila con
        // epoll: el proceso duerme mientras nadie hable.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // SAFETY: calloop da acceso exclusivo al Display en el callback.
                    let r = unsafe { display.get_mut().dispatch_clients(state) };
                    // El error **no** se propaga a propósito. Un `?` aquí sale
                    // del callback, calloop lo trata como fallo del bucle y
                    // `run` termina: la sesión entera se cae porque un solo
                    // cliente mandó algo que Smithay no supo despachar. El
                    // cliente ofensor ya queda desconectado por su cuenta; el
                    // compositor tiene que seguir de pie.
                    if let Err(err) = r {
                        tracing::warn!("fallo despachando a un cliente: {err}");
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow::anyhow!("insert_source(display): {err}"))?;

        // El supervisor recibe nuestro socket exacto; no debe adivinarlo a
        // partir de los sockets de otras sesiones del mismo usuario.
        if let Some(path) = std::env::var_os("BOOKOS_SESSION_SOCKET_FILE") {
            std::fs::write(path, socket_name.as_encoded_bytes())?;
        }
        Ok(socket_name)
    }

    /// La ventana bajo un punto y dónde se dibuja, como `Space::element_under`,
    /// pero sin las que se están yendo al dock o volviendo de él. Esas siguen
    /// en el `Space` con su tamaño entero durante los 280 ms de la animación,
    /// y se quedaban los clics y el foco en un rectángulo donde ya solo se
    /// veía la ventana de debajo.
    pub fn ventana_bajo(
        &self,
        punto: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<(
        smithay::desktop::Window,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    )> {
        use smithay::desktop::space::SpaceElement;
        self.space
            .elements()
            .rev()
            .filter(|w| {
                crate::ventanas::encogido(w).is_none() && crate::multitarea::visible(self, w)
            })
            .find_map(|w| {
                if !self.space.element_bbox(w)?.to_f64().contains(punto) {
                    return None;
                }
                let dibujada = self.space.element_location(w)? - w.geometry().loc;
                w.is_in_input_region(&(punto - dibujada.to_f64()))
                    .then(|| (w.clone(), dibujada))
            })
    }

    /// ¿Es del shell este punto? Con la pantalla completa que haya en la
    /// salida bajo el punto, la misma que decide qué se dibuja.
    pub fn shell_contiene(
        &self,
        point: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let completa = self
            .space
            .output_under(point)
            .next()
            .is_some_and(|output| self.hay_pantalla_completa_en(output));
        self.shell
            .as_ref()
            .is_some_and(|s| s.contiene(point.x, point.y, completa))
    }

    /// Qué superficie hay bajo un punto, y en qué posición está dibujada.
    ///
    /// Devuelve la coordenada *relativa a la superficie*, que es lo que espera
    /// el puntero de Smithay: el cliente razona en su propio sistema, no en el
    /// del escritorio.
    pub fn surface_under(
        &self,
        point: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )> {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
        use smithay::wayland::shell::wlr_layer::Layer;

        // Overlay y paneles layer-shell reciben entrada según el mismo orden
        // en que se dibujan. Durante el bloqueo no se consulta ninguna capa de
        // cliente: además de ocultarlas, hay que cortarles también la entrada.
        let bloqueado = self.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
        if !bloqueado {
            for output in self.space.output_under(point) {
                let Some(output_geo) = self.space.output_geometry(output) else {
                    continue;
                };
                let local = point - output_geo.loc.to_f64();
                let map = layer_map_for_output(output);
                for capa in [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background] {
                    let Some(layer) = map.layer_under(capa, local) else {
                        continue;
                    };
                    let Some(geo) = map.layer_geometry(layer) else {
                        continue;
                    };
                    if let Some((surface, offset)) =
                        layer.surface_under(local - geo.loc.to_f64(), WindowSurfaceType::ALL)
                    {
                        return Some((surface, (output_geo.loc + geo.loc + offset).to_f64()));
                    }
                }
            }
        }
        // Las ventanas siguen en el espacio debajo del candado: sin cortar aquí
        // reciben el hover a ciegas y la de debajo impone su cursor —el de
        // texto de una terminal— sobre la pantalla de bloqueo.
        if bloqueado {
            return None;
        }
        // El panel y el dock se dibujan encima de las ventanas: mientras el
        // puntero esté sobre ellos, para los clientes no hay nada bajo el
        // cursor. Sin esto la ventana de debajo se cree señalada y cambia la
        // forma del cursor al pasar por encima del dock.
        if self.shell_contiene(point) {
            return None;
        }
        let (window, location) = self.ventana_bajo(point)?;
        window
            .surface_under(point - location.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, offset)| (surface, (location + offset).to_f64()))
    }

    /// Todo lo que hay que hacer justo antes de volver a dormir en `epoll`.
    /// Si el próximo dibujo va a componer **además** en un buffer aparte.
    ///
    /// Lo hacen las copias de `zwlr_screencopy`, la captura de la tecla Impr y
    /// las pantallas compartidas: las tres recomponen la escena en su propio
    /// offscreen después de presentar. Ver [`crate::backend::servir_capturas`].
    pub fn hay_offscreen_pendiente(&self) -> bool {
        !self.capturas_pantalla.is_empty() || self.captura_pedida.is_some() || self.emisiones.hay()
    }

    pub fn post_dispatch(&mut self) {
        self.hijos
            .retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));
        // Se espera a que la ventana se haya ido y a su animación de cierre: si
        // no, el último fotograma sería la ventana entera cortada de golpe.
        if let Modo::Kiosko {
            salida: Some(codigo),
        } = self.modo
            && self.cierres.is_empty()
            && self.space.elements().next().is_none()
        {
            tracing::info!(codigo, "el programa del kiosko ha terminado");
            self.loop_signal.stop();
        }
        self.actualizar_foreign_toplevels();
        self.popups.cleanup();
        // Retira layer-surfaces cuyo cliente desapareció sin dejar residuos
        // en la zona exclusiva ni en el recorrido de render.
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            smithay::desktop::layer_map_for_output(&output).cleanup();
        }
        // Si la tarjeta del permiso se cerró sin contestar —un clic fuera—, el
        // hilo del portal sigue bloqueado. Se le manda la negativa.
        crate::portal::denegar_si_se_cerro(self);
        crate::teclado::revisar(self);

        // Una emergente entrando pide fotogramas aunque no pase nada más. Es la
        // única vez que este compositor dibuja sin que haya ocurrido un evento,
        // y dura lo que dura la animación: 280 ms.
        if !self.cierres.is_empty()
            || self.fundido.is_some()
            || self.transicion_vista.animando()
            || self
                .shell
                .as_ref()
                .is_some_and(|s| s.animando() || s.barras_animando())
        {
            self.needs_redraw = true;
        } else if let Some(shell) = self.shell.as_mut() {
            // Ya no se mueve nada: se sueltan los buffers de lo que se cerró.
            shell.fin_animacion();
            // `fin_animacion` puede abrir la tarjeta que quedó en cola al
            // cambiar de widget. Esa entrada necesita su primer frame ahora;
            // sin marcarlo, el bucle dormiría justo después de crearla.
            if shell.animando() {
                self.needs_redraw = true;
            }
        }
        // Y lo mismo con las ventanas: una que está apareciendo o llegando a su
        // sitio pide fotogramas aunque el cliente no haga commit. `any` corta en
        // cuanto encuentra una, y con el escritorio quieto es un recorrido de
        // punteros sobre una lista de tres elementos.
        if self.space.elements().any(crate::ventanas::animando)
            || self.arrastre.as_ref().is_some_and(|a| a.animando())
        {
            self.needs_redraw = true;
        }
        // Cierra la ventana de medida si toca y se la pasa al panel, que solo
        // repinta si los números cambiaron.
        if self.metricas.toca_cerrar() {
            let vivas: Vec<String> = self.space.outputs().map(|o| o.name()).collect();
            self.metricas.retener(&vivas);
            if self.metricas.cerrar_ventana() {
                let datos = self.metricas.datos().clone();
                if self
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.diagnostico_datos(datos))
                {
                    self.needs_redraw = true;
                }
            }
        }
        self.space.refresh();
        crate::multitarea::purgar(self);
        self.display_handle.flush_clients().ok();
    }

    /// Publica altas, cambios de metadatos y cierres sin sondeo: se llama al
    /// final de la misma vuelta que procesó el commit o la destrucción.
    fn actualizar_foreign_toplevels(&mut self) {
        let vivas: Vec<_> = self
            .space
            .elements()
            .filter(|window| window.alive())
            .cloned()
            .collect();

        let mut i = 0;
        while i < self.toplevels_publicados.len() {
            if vivas
                .iter()
                .any(|window| *window == self.toplevels_publicados[i].window)
            {
                i += 1;
            } else {
                let publicado = self.toplevels_publicados.remove(i);
                self.foreign_toplevel_list_state
                    .remove_toplevel(&publicado.handle);
            }
        }

        for window in vivas {
            let titulo = crate::handlers::titulo(&window).unwrap_or_default();
            let app_id = crate::handlers::app_id(&window).unwrap_or_default();
            if let Some(publicado) = self
                .toplevels_publicados
                .iter_mut()
                .find(|publicado| publicado.window == window)
            {
                let cambio = publicado.titulo != titulo || publicado.app_id != app_id;
                if publicado.titulo != titulo {
                    publicado.handle.send_title(&titulo);
                    publicado.titulo = titulo;
                }
                if publicado.app_id != app_id {
                    publicado.handle.send_app_id(&app_id);
                    publicado.app_id = app_id;
                }
                if cambio {
                    publicado.handle.send_done();
                }
            } else {
                let handle = self
                    .foreign_toplevel_list_state
                    .new_toplevel::<Self>(&titulo, &app_id);
                self.toplevels_publicados.push(ToplevelPublicado {
                    window,
                    handle,
                    titulo,
                    app_id,
                });
            }
        }
        self.foreign_toplevel_list_state.cleanup_closed_handles();
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, id: ClientId) {
        tracing::debug!(?id, "cliente conectado");
    }

    fn disconnected(&self, id: ClientId, reason: DisconnectReason) {
        tracing::debug!(?id, ?reason, "cliente desconectado");
    }
}
