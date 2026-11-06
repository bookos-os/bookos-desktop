//! Las superficies que aparecen y desaparecen: el menú, el calendario y el
//! launchpad.
//!
//! Son la tercera clase de superficie del shell. El panel y el dock están
//! siempre y tienen tamaño fijo; estas nacen al pulsar algo, se colocan
//! respecto a quien las abrió y se van al pulsar fuera o al pulsar Esc.
//!
//! # Por qué un enum y no otro trait
//!
//! Son tres, se conocen todas en tiempo de compilación y cada una tiene una
//! forma distinta de posicionarse y de responder al teclado. Un trait aquí
//! sería una capa de indirección para tres casos cerrados; el `match` dice lo
//! que hay sin esconderlo. El trait está donde sí hace falta —los widgets del
//! panel, que sí se añaden desde fuera— y aquí no.
//!
//! # Anclaje
//!
//! Cada emergente dice **dónde** quiere salir con [`Ancla`], en píxeles
//! lógicos. El compositor la traduce a la pantalla; el shell no sabe dónde
//! empieza la pantalla ni cuánto mide, solo su relación con quien lo abrió.

mod acerca;
mod apagar;
mod apariencia;
mod atajos;
mod bluetooth;
mod brillo;
mod buscador;
mod calendario;
mod centro;
mod compartir;
pub(crate) mod control;
mod emojis;
mod energia;
mod escritorios;
pub mod launchpad;
pub(crate) mod lista;
mod menu;
mod menu_dock;
mod notificaciones;
mod color_carpeta;
mod proyeccion;
mod red;
mod respuesta;
mod sonido;

use std::sync::LazyLock;

use crate::Accion;
use crate::view::PanelElement;

/// Español si la sesión va en español, inglés en cualquier otro idioma: la
/// misma regla que `apps.rs` con el `Name[es]` de los `.desktop`.
static ES: LazyLock<bool> =
    LazyLock::new(|| std::env::var("LANG").is_ok_and(|l| l.starts_with("es")));

// 0.1.0
// modificado en la 0.2.0 -- pública: el compositor la usa para sus propios avisos
pub fn t(es: &'static str, en: &'static str) -> &'static str {
    if *ES { es } else { en }
}

/// Dónde se coloca una superficie emergente.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ancla {
    /// Colgando del panel, con su borde izquierdo en esa `x` lógica. Es lo que
    /// quiere el menú: sale justo debajo del logo.
    BajoElPanel { x: f32 },
    /// Colgando del panel, centrada bajo el widget que la abre. Es lo que
    /// quieren los estados de la derecha: el emergente del volumen sale bajo
    /// su icono, no en un sitio fijo. El shell resuelve el nombre con las
    /// zonas del panel, que son las mismas que deciden a quién va el clic.
    BajoWidget(&'static str),
    /// Justo encima del dock, centrada en esa `x` **relativa al dock**. Es el
    /// menú del clic derecho: sale sobre el icono que lo abrió.
    SobreElDock { x: f32 },
    /// En el centro de la pantalla. El launchpad.
    Centrada,
    /// Con la esquina superior izquierda en ese punto lógico, sin salirse de
    /// la pantalla. Es el menú del clic derecho sobre el escritorio: tiene
    /// que salir junto al cursor, no en el centro.
    Punto { x: f32, y: f32 },
    /// Franja de gestión pegada al borde superior de la pantalla.
    Arriba,
}

/// Qué hacer con una tecla que llega a una emergente.
pub enum Tecla {
    /// La emergente se cierra.
    Cerrar,
    /// La ha consumido y hay que repintar.
    Consumida,
    /// No le interesa; que siga su camino.
    Ignorada,
    /// La ha consumido y además pide algo al compositor. Es Intro sobre una
    /// aplicación del launchpad: consumir la tecla y lanzar son lo mismo.
    Hacer(Accion),
}

pub use compartir::Pantalla as PantallaCompartible;
pub use escritorios::RADIO_MINIATURA as RADIO_MINIATURA_ESCRITORIO;
pub use menu_dock::Objetivo;
pub use proyeccion::Modo as ModoProyeccion;

/// Recorta un texto al ancho que hay, con puntos suspensivos.
///
/// Vive en las listas de conectividad y se saca aquí porque el toast también lo
/// necesita: dos recortes distintos en el mismo escritorio cortan por sitios
/// distintos.
pub use lista::recortar as recortar_texto;

/// La tarjeta de notificaciones con el silencio que ya hubiera puesto.
// 0.1.0
pub(crate) fn notificaciones_con(
    silencio: Option<(std::time::Instant, Option<std::time::Duration>)>,
) -> notificaciones::Notificaciones {
    notificaciones::Notificaciones::con_silencio(silencio)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transicion {
    /// Se ha entrado en una carpeta: el panel crece y la rejilla se aparta.
    AbrirCarpeta,
    /// Se ha salido: al revés y sin muelle, que rebotar al salir se lee como
    /// que la carpeta no quería cerrarse.
    CerrarCarpeta,
    /// Lo de dentro ha cambiado sin cambiar de vista —el color de la carpeta,
    /// una aplicación que se quita, el modo edición—: fundido y nada más.
    Fundido,
    AbrirDetalle(iced_core::Rectangle),
    CerrarDetalle(iced_core::Rectangle),
}

pub use centro::{alternar_mantener_despierto, mantener_despierto};

pub enum Emergente {
    Menu(menu::Menu),
    Apariencia(apariencia::Apariencia),
    Apagar(apagar::Apagar),
    Calendario(calendario::Calendario),
    Sonido(sonido::Sonido),
    Brillo(brillo::Brillo),
    Energia(energia::Energia),
    MenuDock(menu_dock::MenuDock),
    Acerca(acerca::Acerca),
    /// La hoja de atajos de teclado y gestos.
    Atajos(atajos::Atajos),
    Centro(centro::Centro),
    Notificaciones(notificaciones::Notificaciones),
    Red(red::Red),
    Bluetooth(bluetooth::Bluetooth),
    Launchpad(launchpad::Launchpad),
    Buscador(buscador::Buscador),
    Emojis(emojis::Emojis),
    Respuesta(respuesta::Respuesta),
    Escritorios(escritorios::Escritorios),
    Proyeccion(proyeccion::Proyeccion),
    ColorCarpeta(color_carpeta::ColorCarpeta),
    Compartir(compartir::Compartir),
}

impl Emergente {
    // 0.1.0
    pub fn menu_ventana(opciones: Vec<(String, Accion)>) -> Self {
        Self::MenuDock(menu_dock::MenuDock::ventana(opciones))
    }

    /// El mismo menú, pero anclado a un punto de la pantalla en vez de
    /// centrado: lo que quiere el clic derecho sobre el escritorio.
    // 0.1.1
    pub fn menu_escritorio(
        opciones: Vec<Option<(String, &'static str, Accion)>>,
        punto: (f32, f32),
    ) -> Self {
        Self::MenuDock(menu_dock::MenuDock::escritorio(opciones, punto))
    }
    // 0.1.0
    pub fn menu() -> Self {
        Self::Menu(menu::Menu::new())
    }

    // 0.1.0
    pub fn apariencia() -> Self {
        Self::Apariencia(apariencia::Apariencia::new(crate::tema::modo_actual()))
    }

    /// El diálogo del botón de encendido.
    // 0.1.0
    pub fn apagar() -> Self {
        Self::Apagar(apagar::Apagar::new())
    }

    // 0.1.0
    pub fn confirmar_energia(accion: crate::confirmacion::Energia) -> Self {
        Self::Apagar(apagar::Apagar::confirmar(accion))
    }

    // 0.1.0
    pub fn calendario() -> Self {
        Self::Calendario(calendario::Calendario::new())
    }

    // 0.1.0
    pub fn sonido() -> Self {
        Self::Sonido(sonido::Sonido::new())
    }

    // 0.1.0
    pub fn brillo() -> Self {
        Self::Brillo(brillo::Brillo::new())
    }

    // 0.1.0
    pub fn energia() -> Self {
        Self::Energia(energia::Energia::new())
    }

    // 0.1.0
    pub fn acerca() -> Self {
        Self::Acerca(acerca::Acerca::new())
    }

    // añadido en la 0.2.0
    pub fn atajos() -> Self {
        Self::Atajos(atajos::Atajos::new())
    }

    // 0.1.0
    pub fn centro() -> Self {
        Self::Centro(centro::Centro::new())
    }

    // 0.1.0
    pub fn notificaciones() -> Self {
        Self::Notificaciones(notificaciones::Notificaciones::new())
    }

    // 0.1.0
    pub fn red() -> Self {
        Self::Red(red::Red::new())
    }

    // 0.1.0
    pub fn bluetooth() -> Self {
        Self::Bluetooth(bluetooth::Bluetooth::new())
    }

    // 0.1.0
    pub fn menu_dock(objetivo: menu_dock::Objetivo) -> Self {
        Self::MenuDock(menu_dock::MenuDock::new(objetivo))
    }

    /// `pantalla` es el tamaño **lógico** de la pantalla: el launchpad la
    /// ocupa entera.
    // 0.1.0
    pub fn launchpad(pantalla: (f32, f32)) -> Self {
        Self::Launchpad(launchpad::Launchpad::new(pantalla))
    }

    /// El buscador de Meta+Espacio con un tamaño de referencia para clientes
    /// del shell que no conocen la pantalla. Se conserva para tests y para
    /// integraciones externas.
    // 0.1.0
    // modified 0.1.1 -- pasa la lista de abiertas
    pub fn buscador() -> Self {
        Self::buscador_en((1920.0, 1080.0), Vec::new())
    }

    // 0.1.1
    pub fn emojis(x: f32, y: f32, recientes: &[String]) -> Self {
        Self::Emojis(emojis::Emojis::new(x, y, recientes))
    }

    /// El buscador de Meta+Espacio. `pantalla` está en píxeles lógicos para
    /// que el ancho sea cómodo tanto en un portátil como en un monitor 4K/8K.
    /// `abiertas` son los `(exec, app_id)` con ventana: elegir una de esas
    /// aplicaciones la trae en vez de abrir otra.
    // 0.1.0
    // modified 0.1.1 -- pasa la lista de abiertas
    pub fn buscador_en(pantalla: (f32, f32), abiertas: Vec<(String, String)>) -> Self {
        Self::Buscador(buscador::Buscador::new(pantalla, abiertas))
    }

    /// La caja para responder a la notificación `id` sin abrir su aplicación.
    // 0.1.0
    pub fn respuesta(id: u32, app: &str, resumen: &str) -> Self {
        Self::Respuesta(respuesta::Respuesta::new(id, app, resumen))
    }

    // 0.1.0
    pub fn escritorios(pantalla: (f32, f32), activo: usize, nombres: Vec<String>) -> Self {
        Self::Escritorios(escritorios::Escritorios::new(pantalla, activo, nombres))
    }

    // 0.1.0
    pub fn color_carpeta(carpetas: &[(&str, Option<&str>)]) -> Self {
        Self::ColorCarpeta(color_carpeta::ColorCarpeta::new(carpetas))
    }
    // 0.1.0
    pub fn proyeccion(conectadas: usize) -> Self {
        Self::Proyeccion(proyeccion::Proyeccion::new(conectadas))
    }

    /// El permiso de compartir pantalla que pide el portal de escritorio.
    // 0.1.0
    pub fn compartir(sesion: u32, app: String, pantallas: Vec<compartir::Pantalla>) -> Self {
        Self::Compartir(compartir::Compartir::new(sesion, app, pantallas))
    }

    /// Nombre para las trazas.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn nombre(&self) -> &'static str {
        match self {
            Self::Menu(_) => "menu",
            Self::Apariencia(_) => "apariencia",
            Self::Apagar(_) => "apagar",
            Self::Calendario(_) => "calendario",
            Self::Sonido(_) => "sonido",
            Self::Brillo(_) => "brillo",
            Self::Energia(_) => "energia",
            Self::MenuDock(m) => {
                if m.es_ventana() {
                    "menu-ventana"
                } else {
                    "menu-dock"
                }
            }
            Self::Acerca(_) => "acerca",
            Self::Atajos(_) => "atajos",
            Self::Centro(_) => "centro",
            Self::Notificaciones(_) => "notificaciones",
            Self::Red(_) => "red",
            Self::Bluetooth(_) => "bluetooth",
            Self::Launchpad(_) => "launchpad",
            Self::Buscador(_) => "buscador",
            Self::Emojis(_) => "emojis",
            Self::Respuesta(_) => "respuesta",
            Self::Escritorios(_) => "escritorios",
            Self::Proyeccion(_) => "proyeccion",
            Self::ColorCarpeta(_) => "color_carpeta",
            Self::Compartir(_) => "compartir",
        }
    }

    /// Tamaño lógico.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn size(&self) -> (f32, f32) {
        match self {
            Self::Menu(m) => m.size(),
            Self::Apariencia(a) => a.size(),
            Self::Apagar(a) => a.size(),
            Self::Calendario(c) => c.size(),
            Self::Sonido(s) => s.size(),
            Self::Brillo(b) => b.size(),
            Self::Energia(e) => e.size(),
            Self::MenuDock(m) => m.size(),
            Self::Acerca(a) => a.size(),
            Self::Atajos(a) => a.size(),
            Self::Centro(c) => c.size(),
            Self::Notificaciones(n) => n.size(),
            Self::Red(r) => r.size(),
            Self::Bluetooth(b) => b.size(),
            Self::Launchpad(l) => l.size(),
            Self::Buscador(b) => b.size(),
            Self::Emojis(e) => e.size(),
            Self::Respuesta(r) => r.size(),
            Self::Escritorios(e) => e.size(),
            Self::Proyeccion(p) => p.size(),
            Self::ColorCarpeta(c) => c.size(),
            Self::Compartir(c) => c.size(),
        }
    }

    /// El alto con el que hay que **colocarla**, cuando no es el que mide
    /// ahora mismo.
    ///
    /// Solo el buscador lo tiene: su alto cambia con cada tecla —cada resultado
    /// que entra o sale es media fila— y centrarlo por el alto de ahora lo hace
    /// saltar mientras escribes. Se coloca por el alto estable de su campo: el
    /// campo queda centrado y la lista crece hacia abajo. Las demás devuelven
    /// `None` y se colocan por lo que miden, que es lo que siempre han hecho.
    // 0.1.0
    pub fn alto_estable(&self) -> Option<f32> {
        match self {
            Self::Buscador(_) => Some(buscador::Buscador::alto_colocacion()),
            _ => None,
        }
    }

    /// Lo que ocupa de verdad dentro de su buffer, si es menos. Solo el
    /// buscador: crece dentro de un buffer del alto máximo.
    // añadido en la 0.2.0
    pub fn alto_visible(&self) -> Option<f32> {
        match self {
            Self::Buscador(b) => Some(b.alto_visible()),
            _ => None,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn ancla(&self) -> Ancla {
        match self {
            Self::Menu(m) => m.ancla(),
            Self::Apariencia(a) => a.ancla(),
            Self::Apagar(a) => a.ancla(),
            Self::Calendario(c) => c.ancla(),
            Self::Sonido(s) => s.ancla(),
            Self::Brillo(b) => b.ancla(),
            Self::Energia(e) => e.ancla(),
            Self::MenuDock(m) => m.ancla(),
            Self::Acerca(a) => a.ancla(),
            Self::Atajos(a) => a.ancla(),
            Self::Centro(c) => c.ancla(),
            Self::Notificaciones(n) => n.ancla(),
            Self::Red(r) => r.ancla(),
            Self::Bluetooth(b) => b.ancla(),
            Self::Launchpad(l) => l.ancla(),
            Self::Buscador(b) => b.ancla(),
            Self::Emojis(e) => e.ancla(),
            Self::Respuesta(r) => r.ancla(),
            Self::Escritorios(e) => e.ancla(),
            Self::Proyeccion(p) => p.ancla(),
            Self::ColorCarpeta(c) => c.ancla(),
            Self::Compartir(c) => c.ancla(),
        }
    }

    /// Relee lo suyo. `true` si lo que enseña ha cambiado.
    ///
    /// Solo las tarjetas que dependen de algo de fuera —las redes a la vista,
    /// los dispositivos emparejados— y solo mientras están abiertas: cerrada,
    /// una emergente no existe, así que esto no es un sondeo en reposo.
    // 0.1.0
    pub fn refrescar(&mut self) -> bool {
        match self {
            Self::Red(r) => r.refrescar(),
            Self::Bluetooth(b) => b.refrescar(),
            Self::Sonido(s) => s.refrescar(),
            Self::Centro(c) => c.refrescar(),
            Self::Brillo(b) => b.refrescar(),
            Self::Energia(e) => e.refrescar(),
            _ => false,
        }
    }

    /// ¿Tiene una animación propia en marcha dentro de su buffer?
    ///
    /// No es la de entrada y salida, que la lleva el compositor moviendo la
    /// superficie: es el contenido cambiando —la rejilla del launchpad
    /// pasando de página— y obliga a repintar el buffer en cada fotograma.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn animando(&self) -> bool {
        match self {
            Self::Launchpad(l) => l.animando(),
            Self::Buscador(b) => b.animando(),
            Self::Emojis(e) => e.animando(),
            Self::Respuesta(r) => r.animando(),
            Self::Apariencia(a) => a.animando(),
            Self::Apagar(a) => a.animando(),
            Self::Menu(m) => m.animando(),
            Self::MenuDock(m) => m.animando(),
            Self::Calendario(c) => c.animando(),
            Self::Sonido(s) => s.animando(),
            Self::Energia(e) => e.animando(),
            Self::Centro(c) => c.animando(),
            Self::Notificaciones(n) => n.animando(),
            Self::Red(r) => r.animando(),
            Self::Bluetooth(b) => b.animando(),
            Self::Acerca(a) => a.animando(),
            Self::Atajos(_) => false,
            Self::Escritorios(e) => e.animando(),
            Self::Proyeccion(p) => p.animando(),
            Self::ColorCarpeta(c) => c.animando(),
            Self::Compartir(c) => c.animando(),
            Self::Brillo(b) => b.animando(),
        }
    }

    /// El compositor anima el cambio conservando la superficie de antes.
    // 0.1.0
    pub fn tomar_transicion(&mut self) -> Option<Transicion> {
        match self {
            Self::Centro(c) => c.tomar_transicion(),
            Self::Launchpad(l) => l.tomar_transicion(),
            _ => None,
        }
    }

    /// El launchpad y la vista general. El panel no se dibuja por encima: son
    /// modos temporales que gobiernan toda la pantalla aunque su chrome ocupe
    /// solo la zona con contenido.
    // 0.1.0
    pub fn tapa_la_pantalla(&self) -> bool {
        matches!(self, Self::Launchpad(_) | Self::Escritorios(_))
    }

    /// El color con que se limpia el buffer antes de dibujar la vista, para
    /// la emergente que va entera sobre un fondo liso. Ver
    /// `Shell::paint_con_fondo`.
    // 0.1.0
    pub fn fondo(&self) -> Option<iced_core::Color> {
        match self {
            Self::Escritorios(e) => Some(e.fondo()),
            _ => None,
        }
    }

    /// El velo a pantalla completa que va detrás, si la emergente lo quiere.
    /// Lo pinta el compositor con un color sólido, que en la GPU es gratis.
    // 0.1.0
    pub fn velo(&self) -> Option<iced_core::Color> {
        match self {
            Self::Launchpad(l) => Some(l.velo()),
            // La vista de escritorios **no** lo lleva: es una franja que se
            // asoma sobre el escritorio, no algo que haya que atender, y apagar
            // lo de debajo la convertiría en un diálogo.
            Self::Buscador(b) => Some(b.velo()),
            Self::Respuesta(r) => Some(r.velo()),
            Self::Acerca(a) => Some(a.velo()),
            Self::Atajos(a) => Some(a.velo()),
            // Es un diálogo: hay que contestarle antes de seguir, y el velo es
            // lo que lo dice sin escribirlo.
            Self::Apagar(a) => Some(a.velo()),
            // También es un diálogo, y de los que hay que mirar dos veces:
            // alguien está pidiendo ver la pantalla entera.
            Self::Compartir(c) => Some(c.velo()),
            _ => None,
        }
    }

    /// ¿Lleva cristal esmerilado debajo de su propia superficie?
    ///
    /// Ninguna tarjeta opaca lo necesita. Ponerlo bajo el buscador dejaba
    /// asomar el color del fondo en el píxel antialiasado de sus esquinas, como
    /// un segundo borde que cambiaba con el wallpaper.
    // 0.1.0
    pub fn usa_cristal(&self) -> bool {
        false
    }

    /// El realce de lo señalado: rectángulo **relativo a la emergente** y si
    /// lleva marco de acento. Va en su propia superficie para que mover el
    /// ratón no cueste un repintado.
    // 0.1.0
    pub fn realce(&self) -> Option<(iced_core::Rectangle, bool)> {
        match self {
            Self::Launchpad(l) => l.realce(),
            _ => None,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn view(&self) -> PanelElement<'_> {
        match self {
            Self::Menu(m) => m.view(),
            Self::Apariencia(a) => a.view(),
            Self::Apagar(a) => a.view(),
            Self::Calendario(c) => c.view(),
            Self::Sonido(s) => s.view(),
            Self::Brillo(b) => b.view(),
            Self::Energia(e) => e.view(),
            Self::MenuDock(m) => m.view(),
            Self::Acerca(a) => a.view(),
            Self::Atajos(a) => a.view(),
            Self::Centro(c) => c.view(),
            Self::Notificaciones(n) => n.view(),
            Self::Red(r) => r.view(),
            Self::Bluetooth(b) => b.view(),
            Self::Launchpad(l) => l.view(),
            Self::Buscador(b) => b.view(),
            Self::Emojis(e) => e.view(),
            Self::Respuesta(r) => r.view(),
            Self::Escritorios(e) => e.view(),
            Self::Proyeccion(p) => p.view(),
            Self::ColorCarpeta(c) => c.view(),
            Self::Compartir(c) => c.view(),
        }
    }

    /// ¿Dibuja lo señalado en una superficie aparte?
    ///
    /// Si es que sí, mover el ratón por encima no repinta su buffer.
    // 0.1.0
    pub fn usa_realce(&self) -> bool {
        matches!(self, Self::Launchpad(_))
    }

    /// Mueve el puntero por encima, en coordenadas lógicas relativas a la
    /// esquina superior izquierda de la emergente. `None` = está fuera.
    /// Devuelve `true` si hay que repintar.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        match self {
            Self::Menu(m) => m.puntero(punto),
            Self::Apariencia(a) => a.puntero(punto),
            Self::Apagar(a) => a.puntero(punto),
            Self::Calendario(c) => c.puntero(punto),
            Self::Sonido(s) => s.puntero(punto),
            Self::Brillo(b) => b.puntero(punto),
            Self::Energia(e) => e.puntero(punto),
            Self::MenuDock(m) => m.puntero(punto),
            Self::Acerca(a) => a.puntero(punto),
            Self::Atajos(_) => false,
            Self::Centro(c) => c.puntero(punto),
            Self::Notificaciones(n) => n.puntero(punto),
            Self::Red(r) => r.puntero(punto),
            Self::Bluetooth(b) => b.puntero(punto),
            Self::Launchpad(l) => l.puntero(punto),
            Self::Buscador(b) => b.puntero(punto),
            Self::Emojis(e) => e.puntero(punto),
            // Un campo de texto: no hay nada que señalar con el ratón.
            Self::Respuesta(_) => false,
            Self::Escritorios(e) => e.puntero(punto),
            Self::Proyeccion(p) => p.puntero(punto),
            Self::ColorCarpeta(c) => c.puntero(punto),
            Self::Compartir(c) => c.puntero(punto),
        }
    }

    /// Pulsación, en las mismas coordenadas. `None` si ahí no hay nada que
    /// hacer.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        match self {
            Self::Menu(m) => m.pulsar(x, y),
            Self::Apariencia(a) => a.pulsar(x, y),
            Self::Apagar(a) => a.pulsar(x, y),
            Self::Calendario(c) => c.pulsar(x, y),
            Self::Sonido(s) => s.pulsar(x, y),
            Self::Brillo(b) => b.pulsar(x, y),
            Self::Energia(e) => e.pulsar(x, y),
            Self::MenuDock(m) => m.pulsar(x, y),
            Self::Acerca(a) => a.pulsar(x, y),
            Self::Atajos(_) => None,
            Self::Centro(c) => c.pulsar(x, y),
            Self::Notificaciones(n) => n.pulsar(x, y),
            Self::Red(r) => r.pulsar(x, y),
            Self::Bluetooth(b) => b.pulsar(x, y),
            Self::Launchpad(l) => l.pulsar(x, y),
            Self::Buscador(b) => b.pulsar(x, y),
            Self::Emojis(e) => e.pulsar(x, y),
            Self::Respuesta(_) => None,
            Self::Escritorios(e) => e.pulsar(x, y),
            Self::Proyeccion(p) => p.pulsar(x, y),
            Self::ColorCarpeta(c) => c.pulsar(x, y),
            Self::Compartir(c) => c.pulsar(x, y),
        }
    }

    /// Desplazamiento de rueda o touchpad, en píxeles lógicos. `true` si ha
    /// cambiado algo y hay que repintar.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn desplazar(&mut self, dx: f32, dy: f32) -> bool {
        match self {
            Self::Launchpad(l) => l.desplazar(dx, dy),
            Self::Emojis(e) => e.desplazar(dy),
            Self::Sonido(s) => s.desplazar(dx, dy),
            Self::Brillo(b) => b.desplazar(dx, dy),
            Self::Red(r) => r.desplazar(dy),
            Self::Bluetooth(b) => b.desplazar(dy),
            Self::Centro(c) => c.desplazar(dy),
            // El menú y el calendario caben enteros; no hay nada que desplazar.
            _ => false,
        }
    }

    /// El botón se ha soltado. `true` si hay que repintar.
    ///
    /// Solo le importa a lo que se arrastra: hasta el deslizador del volumen,
    /// ninguna emergente necesitaba saber cuándo acababa una pulsación.
    /// Devuelve si hay que repintar y, si soltar completa una interacción, qué
    /// hacer: en el launchpad, soltar un icono sin haberlo arrastrado **es** la
    /// pulsación, y por eso la acción no puede salir de `pulsar`.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        match self {
            Self::Sonido(s) => (s.soltar(), None),
            Self::Brillo(b) => (b.soltar(), None),
            Self::Centro(c) => c.soltar(),
            Self::Emojis(e) => (e.soltar(), None),
            Self::Launchpad(l) => l.soltar(),
            Self::ColorCarpeta(c) => (c.soltar(), None),
            _ => (false, None),
        }
    }

    /// ¿Tiene algo agarrado? Mientras lo tenga, el puntero le sigue llegando
    /// aunque se salga de la tarjeta: al llevar el volumen al máximo de un
    /// tirón, la mano se sale y el deslizador tiene que seguir.
    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn agarrado(&self) -> bool {
        match self {
            Self::Sonido(s) => s.agarrado(),
            Self::Brillo(b) => b.agarrado(),
            Self::Centro(c) => c.agarrado(),
            Self::Emojis(e) => e.agarrado(),
            Self::Launchpad(l) => l.agarrado(),
            Self::ColorCarpeta(c) => c.agarrado(),
            _ => false,
        }
    }

    // añadido en la 0.2.0 -- el reconocimiento táctil consulta la geometría sin ejecutar la acción.
    pub fn arrastre_en(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Sonido(s) => s.arrastre_en(x, y),
            Self::Brillo(b) => b.arrastre_en(x, y),
            Self::Centro(c) => c.arrastre_en(x, y),
            Self::Emojis(e) => e.arrastre_en(x, y),
            Self::Launchpad(l) => l.arrastre_en(x, y),
            Self::ColorCarpeta(c) => c.arrastre_en(x, y),
            _ => false,
        }
    }

    // 0.1.0
    // modified 0.1.1 -- selector de emojis
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match self {
            Self::Menu(m) => m.tecla(tecla),
            Self::Apariencia(a) => a.tecla(tecla),
            Self::Apagar(a) => a.tecla(tecla),
            Self::Calendario(c) => c.tecla(tecla),
            Self::Sonido(s) => s.tecla(tecla),
            Self::Brillo(b) => b.tecla(tecla),
            Self::Energia(e) => e.tecla(tecla),
            Self::MenuDock(m) => m.tecla(tecla),
            Self::Acerca(a) => a.tecla(tecla),
            Self::Atajos(a) => a.tecla(tecla),
            Self::Centro(c) => c.tecla(tecla),
            Self::Notificaciones(n) => n.tecla(tecla),
            Self::Red(r) => r.tecla(tecla),
            Self::Bluetooth(b) => b.tecla(tecla),
            Self::Launchpad(l) => l.tecla(tecla),
            Self::Buscador(b) => b.tecla(tecla),
            Self::Emojis(e) => e.tecla(tecla),
            Self::Respuesta(r) => r.tecla(tecla),
            Self::Escritorios(e) => e.tecla(tecla),
            Self::Proyeccion(p) => p.tecla(tecla),
            Self::ColorCarpeta(c) => c.tecla(tecla),
            Self::Compartir(c) => c.tecla(tecla),
        }
    }

    /// La respuesta que hay que mandar si esto se cierra sin contestarlo.
    ///
    /// Solo el permiso de compartir pantalla la tiene: hay alguien bloqueado al
    /// otro lado del portal, y un clic fuera de la tarjeta tiene que llegarle
    /// como una negativa y no como silencio.
    // 0.1.0
    pub fn respuesta_pendiente(&self) -> Option<Accion> {
        match self {
            Self::Compartir(c) => Some(c.denegar()),
            _ => None,
        }
    }

    // 0.1.0
    pub fn miniaturas_escritorios(&self) -> Vec<iced_core::Rectangle> {
        match self {
            Self::Escritorios(e) => e.miniaturas(),
            _ => Vec::new(),
        }
    }

    // 0.1.0
    pub fn actualizar_escritorios(&mut self, activo: usize, nombres: Vec<String>) -> bool {
        match self {
            Self::Escritorios(e) => e.actualizar(activo, nombres),
            _ => false,
        }
    }
}
