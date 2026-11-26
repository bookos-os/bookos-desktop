//! El emergente de las notificaciones.
//!
//! La lista viene de fuera: quien atiende `org.freedesktop.Notifications` es el
//! compositor —el shell no habla D-Bus, para poder estar pintado en el primer
//! frame— y la cola vive en [`crate::notificaciones::Registro`]. Aquí solo se
//! guarda una copia de lo que hay que dibujar, que se refresca cuando cambia.
//!
//! «No molestar» sí es estado de esta tarjeta, con su caducidad.
//!
//! ## Cómo se abre una app
//!
//! Los avisos de una misma app se agrupan en una **pila**: pulsarla la abre,
//! con la app como botón en la cabecera del grupo. **Pulsar un aviso hace su
//! acción por defecto** (la `default` de la especificación: abrir la app o la
//! conversación) y lo descarta; si no la tiene, lo despliega o, si no hay nada
//! que desplegar, lo cierra. El chevron que sale al pasar el puntero despliega
//! sin abrir, y el menú ⋯ ofrece abrir, ajustes y borrar. Los urgentes salen de
//! las pilas y van siempre arriba.
//!
//! Como en `bluetooth.rs` y `red.rs`, las piezas de la lista no miden todas lo
//! mismo, y la geometría (ventana, ratón, teclado) sale de
//! [`Notificaciones::colocar`] y no de `n × FILA`.

use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::Weight;
use iced_core::{Border, Color, Length, Padding, Point, Rectangle};
use iced_widget::{Space, column, container, row, stack, text};

use crate::Accion;
use crate::notificaciones::Notificacion;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista::{self, Colocada, interruptor};
use super::{Ancla, Tecla};

/// Ancho de una tarjeta: el interior de la emergente, sin grupo gris de por
/// medio.
const ANCHO_TARJETA: f32 = lista::ANCHO - lista::MARGEN * 2.0;
/// Relleno interior de una tarjeta y lado del icono de la app.
const PAD: f32 = 12.0;
const ICONO_LADO: f32 = 36.0;
/// Lo que le queda al texto: la tarjeta menos su relleno, el icono y su hueco.
const ANCHO_TEXTO: f32 = ANCHO_TARJETA - PAD * 2.0 - ICONO_LADO - 12.0;
/// La línea de arriba (app y hora), el título y cada renglón del cuerpo. Fijos
/// para que el alto de una tarjeta se calcule sin medir texto.
const CAB: f32 = 16.0;
const TIT: f32 = 20.0;
const LINEA: f32 = 16.0;
/// Renglones del cuerpo cerrado y desplegado.
const LINEAS_CERRADA: usize = 2;
const LINEAS_ABIERTA: usize = 8;
/// La barra de progreso con su aire de arriba.
const PROGRESO: f32 = 13.0;
/// La fila de botones desplegada, con su aire de arriba, y cada botón.
const ACCIONES: f32 = 40.0;
const BOTON: f32 = 30.0;
/// Lo mínimo que mide una tarjeta: el icono con su relleno.
const MIN_TARJETA: f32 = ICONO_LADO + PAD * 2.0;
/// Lo que asoman por debajo las capas de una pila.
const PILA_EXTRA: f32 = 12.0;
/// La cabecera de un grupo abierto y la fila de «Mostrar N más».
const GRUPO_CAB: f32 = 30.0;
const MAS: f32 = 36.0;
/// El menú ⋯ en línea: su relleno y cada opción.
const PAD_MENU: f32 = 4.0;
const ITEM_MENU: f32 = 40.0;
/// Lo más alto que se hace la lista; con más, se desplaza. La tarjeta no
/// crece hasta el dock.
const MAX_LISTA: f32 = 390.0;
/// La lista vacía sigue midiendo algo: es donde va el «No hay notificaciones».
const VACIO: f32 = 120.0;
/// Cuántas enseña un grupo abierto antes del «Mostrar N más».
const POR_GRUPO: usize = 4;
/// Aire entre la lista y la fila de «No molestar», con el divisor en medio.
const SOBRE_BLOQUE: f32 = 14.0;
/// La fila de «No molestar».
const BLOQUE: f32 = 60.0;
/// Alto de cada botón de duración, y hueco entre las piezas.
const DURACION: f32 = 36.0;
const HUECO: f32 = 6.0;
/// Lo que ocupan las duraciones con su nota: aire, dos filas de botones y la
/// línea que dice que las urgentes siguen sonando.
const DURACIONES_ALTO: f32 = 2.0 + DURACION * 2.0 + HUECO + 6.0 + 18.0;
/// Lo que le queda al texto de «No molestar»: la fila menos su relleno, el disco,
/// su hueco y el interruptor con su aire. Sin fijarlo, iced encoge el
/// interruptor cuando el estado es largo.
const ANCHO_ESTADO: f32 = ANCHO_TARJETA - 8.0 - ICONO_LADO - 12.0 - lista::INTERRUPTOR_ANCHO - 8.0;
/// Cada botón de la esquina de una tarjeta (chevron, ⋯ y ✕).
const BTN: f32 = 28.0;
/// El hueco que reservan a la derecha de la línea de arriba.
const ESQUINA: f32 = BTN * 3.0;
/// El botón de ajustes de la cabecera.
const AJUSTES: f32 = 32.0;
/// Margen lateral de la cabecera: deja el título a 20 del borde.
const SANGRIA: f32 = 4.0;

/// Cuánto dura el silencio, en la rejilla de dos por dos del diseño.
const DURACIONES: &[(&str, &str, Option<Duration>)] = &[
    ("1 hora", "1 hour", Some(Duration::from_secs(3600))),
    ("4 horas", "4 hours", Some(Duration::from_secs(4 * 3600))),
    ("8 horas", "8 hours", Some(Duration::from_secs(8 * 3600))),
    ("Hasta desactivarlo", "Until turned off", None),
];

/// De qué aviso o pila está abierto el menú ⋯.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Menu {
    Aviso(u32),
    Pila(String),
}

/// Las opciones del menú ⋯.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opcion {
    Abrir,
    Silenciar,
    Ajustes,
    Borrar,
}

/// Lo que hay a cada altura de la lista. Los índices son de `lista` para los
/// avisos y del orden de [`Notificaciones::grupos`] para los grupos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fila {
    Aviso(usize),
    Pila(usize),
    Grupo(usize),
    Mas(usize),
    Menu,
}

/// Lo que hay bajo un punto de la lista.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Zona {
    /// El cuerpo de un aviso: su acción por defecto, o desplegarlo.
    Aviso(usize),
    /// El cuerpo de una pila: abrirla.
    Pila(usize),
    Expandir(usize),
    Cerrar(usize),
    Opciones(usize),
    CerrarPila(usize),
    OpcionesPila(usize),
    Boton(usize, String),
    AbrirApp(usize),
    Plegar,
    BorrarGrupo(usize),
    Mostrar(usize),
    Item(usize),
}

/// Los avisos de una misma app, de más nuevo a más viejo, por índice de `lista`.
struct Grupo {
    app: String,
    idxs: Vec<usize>,
}

pub struct Notificaciones {
    /// Cuándo se activó el silencio y hasta cuándo. `None` = no está puesto.
    silencio: Option<(Instant, Option<Duration>)>,
    /// Qué duración está elegida, para marcarla.
    duracion: usize,
    señalada: tema::Realce,
    /// El puntero sobre el interruptor.
    sobre_interruptor: bool,
    /// El recorrido de la bolita, siguiendo al silencio.
    interruptor: tema::Transicion,
    /// Lo que hay que enseñar, copiado de la cola del shell. Copia y no
    /// préstamo porque la tarjeta se dibuja desde `view(&self)` y el `Shell`
    /// que tiene la cola es quien la llama: prestársela sería prestarse a sí
    /// mismo.
    lista: Vec<Notificacion>,
    /// La tarjeta señalada, por índice de `lista`, para el realce y la esquina.
    fila: tema::Realce,
    /// Los botones de la cabecera: 0 ajustes, 1 «Borrar».
    boton: tema::Realce,
    /// El aviso activo para quien navega sin ratón.
    seleccionada: Option<usize>,
    /// La notificación desplegada, por id: una sola a la vez. Por id y no por
    /// posición porque la lista se reordena al llegar avisos.
    desplegada: Option<u32>,
    /// El grupo abierto, por app, y los que ya enseñan todos sus avisos.
    grupo: Option<String>,
    todos: Vec<String>,
    menu: Option<Menu>,
    /// Lo que se ha subido la lista, en píxeles.
    desplazamiento: f32,
    /// Aplicaciones cuyos avisos no críticos están silenciados.
    // añadido en la 0.2.4
    silenciadas: Vec<String>,
}

impl Notificaciones {
    // 0.1.0
    pub fn new() -> Self {
        Self {
            silencio: None,
            // «Hasta desactivarlo» por defecto: es lo que espera quien pulsa el
            // interruptor sin mirar las opciones.
            duracion: 3,
            señalada: tema::Realce::nuevo(),
            sobre_interruptor: false,
            interruptor: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            lista: Vec::new(),
            fila: tema::Realce::nuevo(),
            boton: tema::Realce::nuevo(),
            seleccionada: None,
            desplegada: None,
            grupo: None,
            todos: Vec::new(),
            menu: None,
            desplazamiento: 0.0,
            silenciadas: Vec::new(),
        }
    }

    /// Actualiza la lista de apps silenciadas que enseña el menú ⋯.
    // añadido en la 0.2.4
    pub fn poner_silenciadas(&mut self, apps: Vec<String>) {
        self.silenciadas = apps;
    }

    // añadido en la 0.2.4
    fn app_silenciada(&self, app: &str) -> bool {
        self.silenciadas
            .iter()
            .any(|s| s.eq_ignore_ascii_case(app))
    }

    /// La tarjeta con el silencio que ya hubiera puesto el shell.
    // 0.1.0
    pub fn con_silencio(silencio: Option<(Instant, Option<Duration>)>) -> Self {
        let mut n = Self::new();
        n.poner_silencio(silencio);
        n
    }

    // 0.1.0
    pub fn poner_silencio(&mut self, silencio: Option<(Instant, Option<Duration>)>) {
        self.silencio = silencio;
        self.interruptor
            .fijar(f32::from(u8::from(self.silenciado())));
    }

    // 0.1.0
    pub fn silencio(&self) -> Option<(Instant, Option<Duration>)> {
        self.silencio
    }

    /// ¿Está el silencio puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque caduca solo: un temporizador que
    /// despierte al shell a la hora en punto para apagar un booleano es
    /// exactamente el despertar de más que este compositor evita.
    // 0.1.0
    pub fn silenciado(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// Cambia lo que se enseña. Lo llama el shell cuando la cola cambia.
    // 0.1.0
    // modified 0.2.1 -- también se olvidan el grupo, el menú y el aviso activo que ya no existan
    pub fn actualizar(&mut self, nueva: Vec<Notificacion>) {
        self.lista = nueva;
        // Lo señalado se apaga: la tarjeta que había bajo el puntero puede haber
        // desaparecido, y dejar el realce puesto marcaría a la de al lado.
        self.fila.señalar(None);
        self.seleccionada = self.seleccionada.filter(|i| *i < self.lista.len());
        if !self.lista.iter().any(|n| Some(n.id) == self.desplegada) {
            self.desplegada = None;
        }
        if let Some(app) = &self.grupo
            && self.lista.iter().filter(|n| &n.app == app).count() < 2
        {
            self.grupo = None;
        }
        if let Some(m) = &self.menu {
            let sigue = match m {
                Menu::Aviso(id) => self.lista.iter().any(|n| n.id == *id),
                Menu::Pila(app) => self.lista.iter().any(|n| &n.app == app),
            };
            if !sigue {
                self.menu = None;
            }
        }
        self.todos
            .retain(|app| self.lista.iter().any(|n| &n.app == app));
        self.desplazamiento = self.desplazamiento.min(self.maximo());
    }

    // --- Lo que hay en cada aviso -------------------------------------------

    /// El cuerpo partido en renglones del ancho del texto.
    // 0.1.0
    fn lineas(notif: &Notificacion, abierta: bool) -> Vec<String> {
        crate::escritorio::lineas(
            &notif.cuerpo,
            ANCHO_TEXTO,
            12.0,
            if abierta {
                LINEAS_ABIERTA
            } else {
                LINEAS_CERRADA
            },
        )
    }

    /// Lo que se ofrece como botón al desplegar: las acciones de la aplicación
    /// y, si la tiene, «Abrir» por la acción `default`.
    // 0.1.0
    fn botones(notif: &Notificacion) -> Vec<(String, String)> {
        let mut botones: Vec<_> = Vec::new();
        if notif.acciones.iter().any(|a| a.clave == "default") {
            botones.push(("default".into(), super::t("Abrir", "Open").into()));
        }
        botones.extend(
            notif
                .acciones
                .iter()
                .filter(|a| a.clave != "default")
                .map(|a| (a.clave.clone(), a.etiqueta.clone())),
        );
        botones
    }

    fn tiene_default(notif: &Notificacion) -> bool {
        notif.acciones.iter().any(|a| a.clave == "default")
    }

    /// ¿Tiene algo más que enseñar al desplegarse? Si no, la tarjeta se
    /// comporta como siempre: un toque ejecuta su acción o la cierra.
    // 0.1.0
    fn desplegable(notif: &Notificacion) -> bool {
        !Self::botones(notif).is_empty() || Self::lineas(notif, true).len() > LINEAS_CERRADA
    }

    // 0.1.0
    fn esta_desplegada(&self, notif: &Notificacion) -> bool {
        self.desplegada == Some(notif.id) && Self::desplegable(notif)
    }

    fn con_progreso(notif: &Notificacion) -> bool {
        notif.progreso.is_some() || notif.progreso_indeterminado
    }

    /// Alto de una tarjeta: el que pide su cuerpo, su barra y, desplegada, sus
    /// botones.
    // 0.1.0
    // modified 0.2.1 -- cabecera, título y renglones de alto fijo
    fn alto_aviso(&self, notif: &Notificacion, abierta: bool) -> f32 {
        let botones = if abierta && !Self::botones(notif).is_empty() {
            ACCIONES
        } else {
            0.0
        };
        let progreso = if Self::con_progreso(notif) {
            PROGRESO
        } else {
            0.0
        };
        let lineas = Self::lineas(notif, abierta).len() as f32;
        (PAD * 2.0 + CAB + TIT + lineas * LINEA + progreso + botones).max(MIN_TARJETA)
    }

    // --- Grupos y geometría ---------------------------------------------------

    /// Los avisos urgentes, que van sueltos y arriba, y los demás agrupados por
    /// app en el orden en que aparece cada una.
    // añadido en la 0.2.1
    fn grupos(&self) -> (Vec<usize>, Vec<Grupo>) {
        let urgentes = (0..self.lista.len())
            .filter(|i| self.lista[*i].critica)
            .collect();
        let mut grupos: Vec<Grupo> = Vec::new();
        for (i, n) in self.lista.iter().enumerate().filter(|(_, n)| !n.critica) {
            match grupos.iter_mut().find(|g| g.app == n.app) {
                Some(g) => g.idxs.push(i),
                None => grupos.push(Grupo {
                    app: n.app.clone(),
                    idxs: vec![i],
                }),
            }
        }
        (urgentes, grupos)
    }

    /// Todo lo que hay en la lista, con su altura y su `y` dentro de ella.
    // añadido en la 0.2.1
    fn colocar(&self) -> Vec<Colocada<Fila>> {
        let (urgentes, grupos) = self.grupos();
        let mut v = Vec::new();
        let mut y = 0.0;
        let menu_de = |m: &Menu| self.menu.as_ref() == Some(m);
        let alto_menu = |m: &Menu| self.items_menu(m).len() as f32 * ITEM_MENU + PAD_MENU * 2.0;
        for &i in &urgentes {
            let n = &self.lista[i];
            lista::poner(
                &mut v,
                &mut y,
                Fila::Aviso(i),
                self.alto_aviso(n, self.esta_desplegada(n)),
            );
            if menu_de(&Menu::Aviso(n.id)) {
                lista::poner(&mut v, &mut y, Fila::Menu, alto_menu(&Menu::Aviso(n.id)));
            }
        }
        for (g, grupo) in grupos.iter().enumerate() {
            let varios = grupo.idxs.len() > 1;
            if varios && self.grupo.as_deref() != Some(grupo.app.as_str()) {
                let delante = &self.lista[grupo.idxs[0]];
                lista::poner(
                    &mut v,
                    &mut y,
                    Fila::Pila(g),
                    self.alto_aviso(delante, false) + PILA_EXTRA,
                );
                let m = Menu::Pila(grupo.app.clone());
                if menu_de(&m) {
                    lista::poner(&mut v, &mut y, Fila::Menu, alto_menu(&m));
                }
                continue;
            }
            if varios {
                lista::poner(&mut v, &mut y, Fila::Grupo(g), GRUPO_CAB);
            }
            let todos = self.todos.contains(&grupo.app);
            let visibles = if varios && !todos {
                grupo.idxs.len().min(POR_GRUPO)
            } else {
                grupo.idxs.len()
            };
            for &i in &grupo.idxs[..visibles] {
                let n = &self.lista[i];
                lista::poner(
                    &mut v,
                    &mut y,
                    Fila::Aviso(i),
                    self.alto_aviso(n, self.esta_desplegada(n)),
                );
                if menu_de(&Menu::Aviso(n.id)) {
                    lista::poner(&mut v, &mut y, Fila::Menu, alto_menu(&Menu::Aviso(n.id)));
                }
            }
            if visibles < grupo.idxs.len() {
                lista::poner(&mut v, &mut y, Fila::Mas(g), MAS);
            }
        }
        v
    }

    /// Lo que mide la lista entera, sin el hueco de después de la última.
    fn total(&self) -> f32 {
        self.colocar().last().map_or(0.0, |c| c.y + c.alto)
    }

    /// El alto de la ventana: lo que mide la lista hasta [`MAX_LISTA`], o el
    /// aviso de «vacío».
    // añadido en la 0.2.1
    fn alto_ventana(&self) -> f32 {
        if self.lista.is_empty() {
            VACIO
        } else {
            lista::ventana_de(self.total(), MAX_LISTA)
        }
    }

    fn maximo(&self) -> f32 {
        if self.lista.is_empty() {
            0.0
        } else {
            (self.total() - self.alto_ventana()).max(0.0)
        }
    }

    /// La `y` donde empieza la lista: bajo la cabecera.
    // 0.1.0
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA
    }

    /// La `y` donde empieza la fila de «No molestar»: tras la lista y el divisor.
    // 0.1.0
    fn y_bloque(&self) -> f32 {
        self.y_lista() + self.alto_ventana() + SOBRE_BLOQUE
    }

    /// La `y` donde empieza la rejilla de duraciones.
    // 0.1.0
    fn y_duraciones(&self) -> f32 {
        self.y_bloque() + BLOQUE + 2.0
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        // Las duraciones solo ocupan sitio cuando el silencio está puesto: es
        // lo que dice el diseño y evita cuatro botones que no hacen nada.
        let duraciones = if self.silenciado() {
            DURACIONES_ALTO
        } else {
            0.0
        };
        (
            lista::ANCHO,
            self.y_bloque() + BLOQUE + duraciones + lista::MARGEN,
        )
    }

    // 0.1.0
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("notificaciones")
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.señalada.animando()
            || self.fila.animando()
            || self.boton.animando()
            || self.interruptor.animando()
    }

    /// Rueda o touchpad sobre la lista. `true` si se movió.
    // añadido en la 0.2.1
    pub fn desplazar(&mut self, dy: f32) -> bool {
        let nuevo = (self.desplazamiento + dy).clamp(0.0, self.maximo());
        let cambio = (nuevo - self.desplazamiento).abs() > f32::EPSILON;
        self.desplazamiento = nuevo;
        cambio
    }

    /// Desplaza lo justo para que se vea `[y, y + alto)` de la lista.
    // añadido en la 0.2.1
    fn mostrar(&mut self, y: f32, alto: f32) {
        let ventana = self.alto_ventana();
        if y < self.desplazamiento {
            self.desplazamiento = y;
        } else if y + alto > self.desplazamiento + ventana {
            self.desplazamiento = y + alto - ventana;
        }
        self.desplazamiento = self.desplazamiento.clamp(0.0, self.maximo());
    }

    // --- Menú ⋯ ---------------------------------------------------------------

    /// El aviso con acción `default` más nuevo de una app, para «Abrir».
    // añadido en la 0.2.1
    fn con_default_de(&self, app: &str) -> Option<&Notificacion> {
        self.lista
            .iter()
            .find(|n| n.app == app && Self::tiene_default(n))
    }

    /// Las opciones del menú y su texto.
    // añadido en la 0.2.1
    // modified 0.2.4 -- añade «Silenciar / Activar avisos de <app>»
    fn items_menu(&self, menu: &Menu) -> Vec<(Opcion, String)> {
        let (app, borrar) = match menu {
            Menu::Aviso(id) => (
                self.lista
                    .iter()
                    .find(|n| n.id == *id)
                    .map_or("", |n| n.app.as_str()),
                super::t("Borrar", "Clear").to_string(),
            ),
            Menu::Pila(app) => {
                let n = self.lista.iter().filter(|n| n.app == *app).count();
                let texto = if super::t("es", "en") == "es" {
                    format!("Borrar los {n} avisos")
                } else {
                    format!("Clear all {n}")
                };
                (app.as_str(), texto)
            }
        };
        let mut items = Vec::new();
        if self.con_default_de(app).is_some() {
            items.push((Opcion::Abrir, format!("{} {app}", super::t("Abrir", "Open"))));
        }
        if !app.is_empty() {
            let texto = if self.app_silenciada(app) {
                if super::t("es", "en") == "es" {
                    format!("Activar avisos de {app}")
                } else {
                    format!("Unmute {app}")
                }
            } else {
                format!("{} {app}", super::t("Silenciar", "Mute"))
            };
            items.push((Opcion::Silenciar, texto));
        }
        items.push((
            Opcion::Ajustes,
            super::t("Ajustes de notificaciones…", "Notification settings…").into(),
        ));
        items.push((Opcion::Borrar, borrar));
        items
    }

    /// Lo que hace una opción del menú.
    // añadido en la 0.2.1
    // modified 0.2.4 -- silenciar o reactivar una aplicación desde el menú ⋯
    fn hacer_opcion(&mut self, menu: &Menu, opcion: Opcion) -> Option<Accion> {
        self.menu = None;
        match (menu, opcion) {
            (_, Opcion::Ajustes) => Some(Accion::Lanzar(
                "bookos-settings --page notificaciones".into(),
            )),
            (Menu::Aviso(id), Opcion::Silenciar) => {
                let app = self.lista.iter().find(|n| n.id == *id)?.app.clone();
                let silenciar = !self.app_silenciada(&app);
                Some(Accion::SilenciarAppNotificaciones { app, silenciar })
            }
            (Menu::Pila(app), Opcion::Silenciar) => {
                let silenciar = !self.app_silenciada(app);
                Some(Accion::SilenciarAppNotificaciones {
                    app: app.clone(),
                    silenciar,
                })
            }
            (Menu::Aviso(id), Opcion::Borrar) => Some(Accion::CerrarNotificacion(*id)),
            (Menu::Pila(app), Opcion::Borrar) => Some(Accion::CerrarNotificaciones(
                self.lista
                    .iter()
                    .filter(|n| n.app == *app)
                    .map(|n| n.id)
                    .collect(),
            )),
            (Menu::Aviso(id), Opcion::Abrir) => {
                let app = self.lista.iter().find(|n| n.id == *id)?.app.clone();
                self.abrir(&app)
            }
            (Menu::Pila(app), Opcion::Abrir) => self.abrir(app),
        }
    }

    /// «Abrir» una app: la acción `default` de su aviso más nuevo que la tenga.
    /// Es lo que la especificación reserva para pulsar el aviso; el compositor
    /// la entrega a la app, que es quien trae su ventana al frente.
    // añadido en la 0.2.1
    fn abrir(&self, app: &str) -> Option<Accion> {
        self.con_default_de(app)
            .map(|n| Accion::NotificacionAccion {
                id: n.id,
                clave: "default".into(),
            })
    }

    // --- Ratón ----------------------------------------------------------------

    /// Qué hay bajo un punto de la lista (coordenadas de la tarjeta).
    // añadido en la 0.2.1
    fn zona(&self, x: f32, y: f32) -> Option<Zona> {
        let y0 = self.y_lista();
        if x < lista::MARGEN
            || x > lista::MARGEN + ANCHO_TARJETA
            || y < y0
            || y >= y0 + self.alto_ventana()
            || self.lista.is_empty()
        {
            return None;
        }
        let rel = y - y0 + self.desplazamiento;
        let dx = x - lista::MARGEN;
        let (_, grupos) = self.grupos();
        let c = self
            .colocar()
            .into_iter()
            .find(|c| (c.y..c.y + c.alto).contains(&rel))?;
        let dy = rel - c.y;
        match c.fila {
            Fila::Aviso(i) => Some(self.zona_tarjeta(i, dx, dy, None)),
            Fila::Pila(g) => Some(self.zona_tarjeta(grupos[g].idxs[0], dx, dy, Some(g))),
            Fila::Mas(g) => Some(Zona::Mostrar(g)),
            Fila::Grupo(g) => {
                let app = &grupos[g].app;
                let derecha = ANCHO_TARJETA - 4.0;
                let cerrar = derecha - BTN;
                let plegar_ancho =
                    crate::widget::ancho_de(super::t("Mostrar menos", "Show less"), 12.0) + 16.0;
                let abrir_ancho = crate::widget::ancho_de(app, 12.0) + 14.0 + 16.0;
                if dx >= cerrar {
                    Some(Zona::BorrarGrupo(g))
                } else if dx >= cerrar - 4.0 - plegar_ancho {
                    Some(Zona::Plegar)
                } else if dx <= 4.0 + abrir_ancho {
                    Some(Zona::AbrirApp(g))
                } else {
                    None
                }
            }
            Fila::Menu => {
                let i = ((dy - PAD_MENU) / ITEM_MENU).floor();
                (i >= 0.0).then_some(Zona::Item(i as usize))
            }
        }
    }

    /// Qué hay dentro de una tarjeta, en coordenadas de ella: los tres botones
    /// de la esquina, los botones de acción si está desplegada, o el cuerpo.
    // añadido en la 0.2.1
    fn zona_tarjeta(&self, i: usize, dx: f32, dy: f32, pila: Option<usize>) -> Zona {
        let notif = &self.lista[i];
        let derecha = ANCHO_TARJETA - 8.0;
        if (4.0..=4.0 + BTN).contains(&dy) {
            if dx >= derecha - BTN {
                return match pila {
                    Some(g) => Zona::CerrarPila(g),
                    None => Zona::Cerrar(i),
                };
            }
            if dx >= derecha - BTN * 2.0 {
                return match pila {
                    Some(g) => Zona::OpcionesPila(g),
                    None => Zona::Opciones(i),
                };
            }
            if pila.is_none() && dx >= derecha - BTN * 3.0 {
                return Zona::Expandir(i);
            }
        }
        if let Some(g) = pila {
            return Zona::Pila(g);
        }
        if let Some(clave) = self.boton_en(notif, dx, dy) {
            return Zona::Boton(i, clave);
        }
        Zona::Aviso(i)
    }

    /// Qué botón de acción de una tarjeta desplegada cae en un punto.
    // 0.1.0
    fn boton_en(&self, notif: &Notificacion, dx: f32, dy: f32) -> Option<String> {
        let botones = Self::botones(notif);
        if !self.esta_desplegada(notif) || botones.is_empty() {
            return None;
        }
        let progreso = if Self::con_progreso(notif) {
            PROGRESO
        } else {
            0.0
        };
        let arriba = PAD + CAB + TIT + Self::lineas(notif, true).len() as f32 * LINEA + progreso;
        let y0 = arriba + (ACCIONES - BOTON);
        if !(y0..=y0 + BOTON).contains(&dy) {
            return None;
        }
        let n = botones.len();
        let ancho = (ANCHO_TEXTO - HUECO * (n as f32 - 1.0)) / n as f32;
        let x0 = PAD + ICONO_LADO + 12.0;
        botones.into_iter().enumerate().find_map(|(k, (clave, _))| {
            let ini = x0 + k as f32 * (ancho + HUECO);
            (ini..=ini + ancho).contains(&dx).then_some(clave)
        })
    }

    /// El índice de `lista` de la tarjeta bajo un punto, para el realce.
    // añadido en la 0.2.1
    fn tarjeta_en(&self, x: f32, y: f32) -> Option<usize> {
        let (_, grupos) = self.grupos();
        match self.zona(x, y)? {
            Zona::Aviso(i)
            | Zona::Expandir(i)
            | Zona::Cerrar(i)
            | Zona::Opciones(i)
            | Zona::Boton(i, _) => Some(i),
            Zona::Pila(g) | Zona::CerrarPila(g) | Zona::OpcionesPila(g) => {
                grupos.get(g).map(|g| g.idxs[0])
            }
            _ => None,
        }
    }

    /// El botón de ajustes de la cabecera, a la izquierda de «Borrar».
    // 0.1.0
    fn rect_config(&self) -> Rectangle {
        let borrar = self.rect_borrar();
        Rectangle {
            x: borrar.x - 8.0 - AJUSTES,
            y: lista::MARGEN + (lista::CABECERA - AJUSTES) / 2.0,
            width: AJUSTES,
            height: AJUSTES,
        }
    }

    /// «Borrar», pegado al borde derecho.
    // 0.1.0
    fn rect_borrar(&self) -> Rectangle {
        let ancho = crate::widget::ancho_de(super::t("Borrar", "Clear"), 13.0) + 28.0;
        Rectangle {
            x: lista::ANCHO - lista::MARGEN - SANGRIA - ancho,
            y: lista::MARGEN + (lista::CABECERA - AJUSTES) / 2.0,
            width: ancho,
            height: AJUSTES,
        }
    }

    /// La fila de «No molestar» entera, que es el interruptor.
    // 0.1.0
    fn rect_interruptor(&self) -> Rectangle {
        Rectangle {
            x: lista::MARGEN,
            y: self.y_bloque(),
            width: ANCHO_TARJETA,
            height: BLOQUE,
        }
    }

    /// Qué botón de duración cae en un punto.
    // 0.1.0
    fn duracion_en(&self, x: f32, y: f32) -> Option<usize> {
        if !self.silenciado() {
            return None;
        }
        let y0 = self.y_duraciones();
        if x < lista::MARGEN || x > lista::MARGEN + ANCHO_TARJETA || y < y0 {
            return None;
        }
        let rel = y - y0;
        let fila = (rel / (DURACION + HUECO)) as usize;
        if fila > 1 || rel - fila as f32 * (DURACION + HUECO) > DURACION {
            return None;
        }
        let columna = usize::from(x > lista::ANCHO / 2.0);
        Some(fila * 2 + columna)
    }

    // 0.1.0
    // modified 0.2.1 -- el realce sigue a la tarjeta, no a `n × FILA`
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| self.duracion_en(x, y));
        let sobre = punto.is_some_and(|(x, y)| {
            self.rect_interruptor().contains(Point::new(x, y)) && self.duracion_en(x, y).is_none()
        });
        let botones = punto.and_then(|(x, y)| {
            let p = Point::new(x, y);
            [self.rect_config(), self.rect_borrar()]
                .iter()
                .position(|r| r.contains(p))
        });
        // Todos los `señalar` **siempre**, sin cortocircuito: pasar de una
        // tarjeta a un botón de duración tiene que apagar la una y encender el
        // otro.
        let cambio = self.señalada.señalar(señalada);
        let cambio = self
            .fila
            .señalar(punto.and_then(|(x, y)| self.tarjeta_en(x, y)))
            || cambio;
        let cambio = self.boton.señalar(botones) || cambio;
        if sobre == self.sobre_interruptor {
            return cambio;
        }
        self.sobre_interruptor = sobre;
        true
    }

    // 0.1.0
    // modified 0.2.1 -- pulsar un aviso hace su acción por defecto; el chevron despliega
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = Point::new(x, y);
        if let Some(zona) = self.zona(x, y) {
            return self.pulsar_zona(zona);
        }
        if self.rect_borrar().contains(punto) && !self.lista.is_empty() {
            return Some(Accion::BorrarNotificaciones);
        }
        if self.rect_config().contains(punto) {
            return Some(Accion::Lanzar(
                "bookos-settings --page notificaciones".into(),
            ));
        }
        if let Some(i) = self.duracion_en(x, y) {
            self.duracion = i;
            // Elegir una duración enciende el silencio: nadie pulsa «4 horas»
            // para dejarlo apagado.
            self.silencio = Some((Instant::now(), DURACIONES[i].2));
            self.interruptor.ir_a(1.0);
            return None;
        }
        if self.rect_interruptor().contains(punto) {
            self.silencio = if self.silenciado() {
                None
            } else {
                Some((Instant::now(), DURACIONES[self.duracion].2))
            };
            self.interruptor
                .ir_a(f32::from(u8::from(self.silenciado())));
        }
        None
    }

    /// Lo que hace cada zona de la lista al pulsarla.
    // añadido en la 0.2.1
    fn pulsar_zona(&mut self, zona: Zona) -> Option<Accion> {
        let (_, grupos) = self.grupos();
        match zona {
            Zona::Aviso(i) => self.activar(i),
            Zona::Pila(g) => {
                self.grupo = Some(grupos[g].app.clone());
                self.desplegada = None;
                self.menu = None;
                None
            }
            Zona::Expandir(i) => {
                self.alternar_desplegada(i);
                None
            }
            Zona::Cerrar(i) => Some(Accion::CerrarNotificacion(self.lista[i].id)),
            Zona::CerrarPila(g) => Some(Accion::CerrarNotificaciones(
                grupos[g].idxs.iter().map(|&i| self.lista[i].id).collect(),
            )),
            Zona::Opciones(i) => {
                self.alternar_menu(Menu::Aviso(self.lista[i].id));
                None
            }
            Zona::OpcionesPila(g) => {
                self.alternar_menu(Menu::Pila(grupos[g].app.clone()));
                None
            }
            Zona::Boton(i, clave) => Some(Accion::NotificacionAccion {
                id: self.lista[i].id,
                clave,
            }),
            Zona::AbrirApp(g) => self.abrir(&grupos[g].app),
            Zona::Plegar => {
                self.grupo = None;
                self.desplegada = None;
                self.menu = None;
                None
            }
            Zona::BorrarGrupo(g) => Some(Accion::CerrarNotificaciones(
                grupos[g].idxs.iter().map(|&i| self.lista[i].id).collect(),
            )),
            Zona::Mostrar(g) => {
                self.todos.push(grupos[g].app.clone());
                None
            }
            Zona::Item(k) => {
                let menu = self.menu.clone()?;
                let (opcion, _) = *self.items_menu(&menu).get(k)?;
                self.hacer_opcion(&menu, opcion)
            }
        }
    }

    /// Lo que hace pulsar el cuerpo de un aviso: su acción por defecto; sin
    /// ella, desplegarlo; y si no hay nada que desplegar, cerrarlo.
    // añadido en la 0.2.1
    fn activar(&mut self, i: usize) -> Option<Accion> {
        let notif = &self.lista[i];
        if Self::tiene_default(notif) {
            return Some(Accion::NotificacionAccion {
                id: notif.id,
                clave: "default".into(),
            });
        }
        if Self::desplegable(notif) {
            self.alternar_desplegada(i);
            return None;
        }
        if let Some(accion) = notif.acciones.first() {
            return Some(Accion::NotificacionAccion {
                id: notif.id,
                clave: accion.clave.clone(),
            });
        }
        Some(Accion::CerrarNotificacion(notif.id))
    }

    fn alternar_desplegada(&mut self, i: usize) {
        let id = self.lista[i].id;
        self.desplegada = (self.desplegada != Some(id)).then_some(id);
        self.menu = None;
        self.mostrar_aviso(i);
    }

    fn alternar_menu(&mut self, m: Menu) {
        self.menu = (self.menu.as_ref() != Some(&m)).then_some(m);
    }

    /// Desplaza lo justo para ver una tarjeta entera.
    // añadido en la 0.2.1
    fn mostrar_aviso(&mut self, i: usize) {
        let (_, grupos) = self.grupos();
        let delante = |g: usize| grupos[g].idxs[0];
        if let Some(c) = self.colocar().into_iter().find(|c| match c.fila {
            Fila::Aviso(k) => k == i,
            Fila::Pila(g) => delante(g) == i,
            _ => false,
        }) {
            self.mostrar(c.y, c.alto);
        }
    }

    // --- Teclado --------------------------------------------------------------

    /// Los avisos que se ven, de arriba abajo: es el orden de las flechas.
    // añadido en la 0.2.1
    fn orden(&self) -> Vec<usize> {
        let (_, grupos) = self.grupos();
        self.colocar()
            .into_iter()
            .filter_map(|c| match c.fila {
                Fila::Aviso(i) => Some(i),
                Fila::Pila(g) => Some(grupos[g].idxs[0]),
                _ => None,
            })
            .collect()
    }

    // 0.1.0
    // modified 0.2.1 -- sigue el orden de lo que se ve y entiende → ← y Retroceso
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        let orden = self.orden();
        let mover = |s: &mut Self, nueva: usize| {
            s.seleccionada = Some(nueva);
            s.fila.señalar(Some(nueva));
            s.mostrar_aviso(nueva);
        };
        match tecla {
            T::Escape => {
                if self.menu.take().is_some() {
                    Tecla::Consumida
                } else {
                    Tecla::Cerrar
                }
            }
            T::Arriba | T::Abajo => {
                if orden.is_empty() {
                    return Tecla::Ignorada;
                }
                let pos = self
                    .seleccionada
                    .and_then(|s| orden.iter().position(|o| *o == s));
                let nueva = match (pos, tecla == T::Arriba) {
                    (None, true) => orden.len() - 1,
                    (None, false) => 0,
                    (Some(p), true) => p.saturating_sub(1),
                    (Some(p), false) => (p + 1).min(orden.len() - 1),
                };
                mover(self, orden[nueva]);
                Tecla::Consumida
            }
            T::Inicio | T::Fin => match if tecla == T::Inicio {
                orden.first()
            } else {
                orden.last()
            } {
                Some(&i) => {
                    mover(self, i);
                    Tecla::Consumida
                }
                None => Tecla::Ignorada,
            },
            T::Derecha | T::Izquierda => {
                let Some(i) = self.seleccionada.filter(|i| *i < self.lista.len()) else {
                    return Tecla::Ignorada;
                };
                let abierta = self.esta_desplegada(&self.lista[i]);
                if Self::desplegable(&self.lista[i]) && abierta == (tecla == T::Izquierda) {
                    self.alternar_desplegada(i);
                }
                Tecla::Consumida
            }
            T::Retroceso => match self.seleccionada.and_then(|i| self.lista.get(i)) {
                Some(n) => Tecla::Hacer(Accion::CerrarNotificacion(n.id)),
                None => Tecla::Ignorada,
            },
            T::Intro => {
                let Some(i) = self.seleccionada.filter(|i| *i < self.lista.len()) else {
                    return Tecla::Ignorada;
                };
                let (_, grupos) = self.grupos();
                // Una pila se abre con Intro.
                if let Some(g) = grupos.iter().position(|g| {
                    g.idxs.len() > 1
                        && g.idxs[0] == i
                        && self.grupo.as_deref() != Some(g.app.as_str())
                }) {
                    self.grupo = Some(grupos[g].app.clone());
                    return Tecla::Consumida;
                }
                let notif = &self.lista[i];
                // Con acción por defecto, Intro la hace, como el ratón. Sin ella,
                // el primer Intro despliega y el segundo hace lo que haría el
                // botón principal: sin ratón no hay otro modo de llegar a los
                // botones.
                if Self::tiene_default(notif) {
                    return Tecla::Hacer(Accion::NotificacionAccion {
                        id: notif.id,
                        clave: "default".into(),
                    });
                }
                if Self::desplegable(notif) {
                    if self.esta_desplegada(notif)
                        && let Some((clave, _)) = Self::botones(notif).into_iter().next()
                    {
                        return Tecla::Hacer(Accion::NotificacionAccion {
                            id: notif.id,
                            clave,
                        });
                    }
                    self.alternar_desplegada(i);
                    return Tecla::Consumida;
                }
                self.activar(i).map_or(Tecla::Consumida, Tecla::Hacer)
            }
            _ => Tecla::Ignorada,
        }
    }

    // --- Dibujo ---------------------------------------------------------------

    /// Un glifo del tema de iconos propio, teñido, o un hueco si falta.
    // añadido en la 0.2.1
    fn glifo<'a>(nombre: &str, px: f32, color: Color) -> PanelElement<'a> {
        match crate::icono::propio(nombre) {
            Some(ic) => crate::icono::ver_teñido_propio(&ic, px, color),
            None => Space::new().width(Length::Fixed(px)).into(),
        }
    }

    /// ¿Es el icono genérico? Si la app no manda ninguno, la cola lo deja en la
    /// campana de `propio("notificaciones")`, que es de trazo blanco y solo se
    /// ve sobre fondo oscuro; ese se pinta teñido con la tinta del tema.
    // añadido en la 0.2.1
    fn es_generico(icono: &crate::icono::Icono) -> bool {
        use crate::icono::Icono;
        match (icono, crate::icono::propio("notificaciones")) {
            (Icono::Svg(a), Some(Icono::Svg(b))) => a.id() == b.id(),
            _ => false,
        }
    }

    /// El icono de la app en su cuadrado, o la campana teñida si no trae ninguno.
    // añadido en la 0.2.1
    fn icono_app<'a>(notif: &'a Notificacion) -> PanelElement<'a> {
        let dentro: PanelElement<'a> = match &notif.icono {
            Some(ic) if !Self::es_generico(ic) => crate::icono::ver(ic, 30.0, 30.0),
            _ => Self::glifo("notificaciones", 20.0, tema::texto()),
        };
        container(dentro)
            .width(Length::Fixed(ICONO_LADO))
            .height(Length::Fixed(ICONO_LADO))
            .center_x(Length::Fixed(ICONO_LADO))
            .center_y(Length::Fixed(ICONO_LADO))
            .style(|_: &iced_widget::Theme| container::Style {
                background: Some(tema::elevado().into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Lo de la derecha de la línea de arriba: la hora, o con el puntero encima
    /// los tres botones. Ocupan el mismo sitio, como en macOS.
    // añadido en la 0.2.1
    fn esquina<'a>(
        notif: &'a Notificacion,
        señalada: f32,
        con_chevron: bool,
        cuenta: Option<usize>,
    ) -> PanelElement<'a> {
        let contenido: PanelElement<'a> = if señalada > 0.02 {
            let tinta = tema::alfa(tema::texto(), señalada);
            let caja = |dentro: PanelElement<'a>| -> PanelElement<'a> {
                container(dentro)
                    .width(Length::Fixed(BTN))
                    .height(Length::Fixed(CAB))
                    .center_x(Length::Fixed(BTN))
                    .center_y(Length::Fixed(CAB))
                    .into()
            };
            let puntos: PanelElement<'a> = row![
                Self::punto(tinta),
                Space::new().width(Length::Fixed(2.5)),
                Self::punto(tinta),
                Space::new().width(Length::Fixed(2.5)),
                Self::punto(tinta),
            ]
            .align_y(Vertical::Center)
            .into();
            let mut fila = row![].align_y(Vertical::Center);
            if con_chevron {
                fila = fila.push(caja(Self::glifo("chevron-abajo", 11.0, tinta)));
            }
            fila.push(caja(puntos))
                .push(caja(Self::glifo("cerrar", 11.0, tinta)))
                .into()
        } else {
            let mut fila = row![].spacing(6).align_y(Vertical::Center);
            if let Some(n) = cuenta {
                fila = fila.push(
                    container(
                        text(n.to_string())
                            .size(10.0)
                            .font(control::peso(Weight::Semibold))
                            .color(tema::texto()),
                    )
                    .padding([0, 5])
                    .height(Length::Fixed(CAB))
                    .center_y(Length::Fixed(CAB))
                    .style(|_: &iced_widget::Theme| container::Style {
                        background: Some(tema::elevado().into()),
                        border: Border {
                            radius: (CAB / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
            }
            fila.push(text(notif.hace()).size(11.0).color(tema::texto2()))
                .into()
        };
        container(contenido)
            .width(Length::Fixed(ESQUINA))
            .height(Length::Fixed(CAB))
            .align_right(Length::Fixed(ESQUINA))
            .center_y(Length::Fixed(CAB))
            .into()
    }

    fn punto<'a>(color: Color) -> PanelElement<'a> {
        container(Space::new())
            .width(Length::Fixed(2.5))
            .height(Length::Fixed(2.5))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: 1.25.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// La barra de progreso de un aviso. Indeterminada, un tramo fijo a la
    /// mitad: un destello recorriéndola repintaría la tarjeta sin parar.
    // añadido en la 0.2.1
    fn barra_progreso<'a>(notif: &Notificacion) -> PanelElement<'a> {
        let fraccion = match notif.progreso {
            Some(p) => f32::from(p.min(100)) / 100.0,
            None => 0.4,
        };
        let lleno = (ANCHO_TEXTO * fraccion).max(4.0);
        container(
            container(Space::new())
                .width(Length::Fixed(lleno))
                .height(Length::Fixed(4.0))
                .style(|_: &iced_widget::Theme| container::Style {
                    background: Some(tema::acento().into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(ANCHO_TEXTO))
        .height(Length::Fixed(4.0))
        .style(|_: &iced_widget::Theme| container::Style {
            background: Some(tema::elevado().into()),
            border: Border {
                radius: 2.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Una tarjeta de aviso. Con `cuenta` es la de delante de una pila: lleva el
    /// número y dos capas asomando por debajo.
    // añadido en la 0.2.1
    fn tarjeta<'a>(
        &self,
        notif: &'a Notificacion,
        señalada: f32,
        cuenta: Option<usize>,
    ) -> PanelElement<'a> {
        let pila = cuenta.is_some();
        let abierta = !pila && self.esta_desplegada(notif);
        let alto = self.alto_aviso(notif, abierta);
        let mut cabecera = row![].align_y(Vertical::Center).spacing(6);
        if !notif.leida {
            cabecera = cabecera.push(
                container(Space::new())
                    .width(Length::Fixed(6.0))
                    .height(Length::Fixed(6.0))
                    .style(|_: &iced_widget::Theme| container::Style {
                        background: Some(tema::acento().into()),
                        border: Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        let silenciada = self.app_silenciada(&notif.app);
        let ancho_app = ANCHO_TEXTO
            - ESQUINA
            - 8.0
            - if notif.critica { 60.0 } else { 0.0 }
            - if silenciada { 66.0 } else { 0.0 };
        cabecera = cabecera.push(
            text(lista::recortar(&notif.app, ancho_app, 11.0))
                .size(11.0)
                .font(control::peso(Weight::Medium))
                .color(tema::texto2()),
        );
        if silenciada {
            cabecera = cabecera.push(
                container(
                    text(super::t("Silenciada", "Muted"))
                        .size(10.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto2()),
                )
                .padding([1, 6])
                .style(|_: &iced_widget::Theme| container::Style {
                    background: Some(tema::elevado().into()),
                    border: Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        if notif.critica {
            cabecera = cabecera.push(
                row![
                    container(Space::new())
                        .width(Length::Fixed(6.0))
                        .height(Length::Fixed(6.0))
                        .style(|_: &iced_widget::Theme| container::Style {
                            background: Some(tema::rojo().into()),
                            border: Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    text(super::t("Urgente", "Urgent"))
                        .size(11.0)
                        .font(control::peso(Weight::Semibold))
                        .color(tema::texto()),
                ]
                .spacing(4)
                .align_y(Vertical::Center),
            );
        }
        let con_chevron = !pila;
        let linea_arriba = container(row![
            cabecera,
            Space::new().width(Length::Fill),
            Self::esquina(notif, señalada, con_chevron, cuenta)
        ])
        .width(Length::Fixed(ANCHO_TEXTO))
        .height(Length::Fixed(CAB));
        let mut textos = column![
            linea_arriba,
            container(
                text(lista::recortar(&notif.resumen, ANCHO_TEXTO, 14.0))
                    .size(14.0)
                    .font(control::peso(Weight::Semibold))
                    .color(tema::texto())
            )
            .padding(Padding::ZERO.top(2.0))
            .height(Length::Fixed(TIT)),
        ];
        for linea in Self::lineas(notif, abierta) {
            textos = textos.push(
                container(text(linea).size(12.0).color(tema::texto2())).height(Length::Fixed(LINEA)),
            );
        }
        if Self::con_progreso(notif) {
            textos = textos.push(
                container(Self::barra_progreso(notif))
                    .padding(Padding::ZERO.top(9.0))
                    .height(Length::Fixed(PROGRESO)),
            );
        }
        let botones = Self::botones(notif);
        if abierta && !botones.is_empty() {
            let n = botones.len();
            let ancho = (ANCHO_TEXTO - HUECO * (n as f32 - 1.0)) / n as f32;
            let (disco, glifo) = control::encendido();
            let mut fila = row![].spacing(HUECO);
            for (k, (_, etiqueta)) in botones.into_iter().enumerate() {
                let primario = k == 0;
                let (fondo, tinta) = if primario {
                    (disco, glifo)
                } else {
                    (tema::elevado(), tema::texto())
                };
                fila = fila.push(
                    container(
                        text(lista::recortar(&etiqueta, ancho - 12.0, 12.0))
                            .size(12.0)
                            .font(control::peso(Weight::Semibold))
                            .color(tinta),
                    )
                    .width(Length::Fixed(ancho))
                    .height(Length::Fixed(BOTON))
                    .center_x(Length::Fixed(ancho))
                    .center_y(Length::Fixed(BOTON))
                    .style(move |_: &iced_widget::Theme| container::Style {
                        background: Some(fondo.into()),
                        border: Border {
                            radius: (BOTON / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
            }
            textos = textos.push(
                container(fila)
                    .padding(Padding::ZERO.top(ACCIONES - BOTON))
                    .height(Length::Fixed(ACCIONES)),
            );
        }
        // Opaco: un relleno translúcido en tiny-skia cuesta ~17× más y la
        // tarjeta se repinta en cada paso del realce.
        let fondo = tema::mezclar(tema::superficie_en_tarjeta(), tema::elevado(), señalada);
        let cuerpo = container(
            row![
                Self::icono_app(notif),
                Space::new().width(Length::Fixed(12.0)),
                container(textos).width(Length::Fixed(ANCHO_TEXTO)),
            ]
            .align_y(Vertical::Top),
        )
        .width(Length::Fixed(ANCHO_TARJETA))
        .height(Length::Fixed(alto))
        .padding(PAD)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        if !pila {
            return cuerpo.into();
        }
        // Las dos capas de detrás, cada vez más estrechas y más claras hacia el
        // fondo: dicen que hay más debajo sin gastar una fila entera.
        let capa = |sangria: f32, bajada: f32, mezcla: f32| -> PanelElement<'a> {
            let color = tema::mezclar(tema::card(), tema::superficie_en_tarjeta(), mezcla);
            container(
                container(Space::new())
                    .width(Length::Fixed(ANCHO_TARJETA - sangria * 2.0))
                    .height(Length::Fixed(alto))
                    .style(move |_: &iced_widget::Theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            radius: tema::R_TARJETA.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .padding(Padding::ZERO.left(sangria).top(bajada))
            .width(Length::Fixed(ANCHO_TARJETA))
            .height(Length::Fixed(alto + PILA_EXTRA))
            .into()
        };
        stack![capa(18.0, 12.0, 0.45), capa(9.0, 6.0, 0.75), cuerpo]
            .width(Length::Fixed(ANCHO_TARJETA))
            .height(Length::Fixed(alto + PILA_EXTRA))
            .into()
    }

    /// La cabecera de un grupo abierto: la app como botón para abrirla,
    /// «Mostrar menos» y la ✕ que borra el grupo.
    // añadido en la 0.2.1
    fn cabecera_grupo<'a>(&self, app: &str) -> PanelElement<'a> {
        row![
            container(
                row![
                    text(app.to_string())
                        .size(12.0)
                        .font(control::peso(Weight::Semibold))
                        .color(tema::texto2()),
                    Self::glifo("abrir", 11.0, tema::texto2()),
                ]
                .spacing(4)
                .align_y(Vertical::Center)
            )
            .padding(Padding::ZERO.left(8.0)),
            Space::new().width(Length::Fill),
            text(super::t("Mostrar menos", "Show less"))
                .size(12.0)
                .color(tema::texto2()),
            Space::new().width(Length::Fixed(4.0)),
            container(Self::glifo("cerrar", 11.0, tema::texto2()))
                .width(Length::Fixed(BTN))
                .height(Length::Fixed(BTN))
                .center_x(Length::Fixed(BTN))
                .center_y(Length::Fixed(BTN)),
        ]
        .align_y(Vertical::Center)
        .width(Length::Fixed(ANCHO_TARJETA))
        .height(Length::Fixed(GRUPO_CAB))
        .into()
    }

    /// El menú ⋯ en línea, bajo el aviso o la pila que lo abrió.
    // añadido en la 0.2.1
    fn menu_en_linea<'a>(&self, menu: &Menu) -> PanelElement<'a> {
        let mut items = column![];
        for (_, etiqueta) in self.items_menu(menu) {
            items = items.push(
                container(
                    text(etiqueta)
                        .size(13.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto()),
                )
                .padding([0, 12])
                .width(Length::Fill)
                .height(Length::Fixed(ITEM_MENU))
                .center_y(Length::Fixed(ITEM_MENU)),
            );
        }
        container(items)
            .padding(PAD_MENU)
            .width(Length::Fixed(ANCHO_TARJETA))
            .style(|_: &iced_widget::Theme| container::Style {
                background: Some(tema::superficie_en_tarjeta().into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Una fila de dos botones de duración con su hueco en medio.
    // 0.1.0
    fn fila_duraciones(&self, i: usize) -> PanelElement<'_> {
        row![
            self.boton_duracion(i),
            Space::new().width(Length::Fixed(HUECO)),
            self.boton_duracion(i + 1),
        ]
        .into()
    }

    // 0.1.0
    // modified 0.2.1 -- píldoras: la elegida, la tinta del tema con su contraste
    fn boton_duracion(&self, i: usize) -> PanelElement<'_> {
        let (es, en, _) = DURACIONES[i];
        let etiqueta = super::t(es, en);
        let elegido = self.duracion == i && self.silenciado();
        let señalado = self.señalada.intensidad(i);
        let (fondo, color, peso) = if elegido {
            (
                tema::texto(),
                tema::tinta_sobre(tema::texto()),
                Weight::Semibold,
            )
        } else {
            (
                tema::mezclar(tema::superficie_en_tarjeta(), tema::elevado(), señalado),
                tema::texto(),
                Weight::Medium,
            )
        };
        let ancho = (ANCHO_TARJETA - HUECO) / 2.0;
        container(
            text(etiqueta)
                .size(13.0)
                .font(control::peso(peso))
                .color(color),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(DURACION))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(DURACION))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: (DURACION / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// La fila de «No molestar»: disco con la luna, título, estado e interruptor.
    // añadido en la 0.2.1
    fn bloque_no_molestar(&self) -> PanelElement<'_> {
        let (fondo, tinta) = if self.silenciado() {
            control::encendido()
        } else {
            (tema::elevado(), tema::texto())
        };
        let disco = container(Self::glifo("noche", 18.0, tinta))
            .width(Length::Fixed(ICONO_LADO))
            .height(Length::Fixed(ICONO_LADO))
            .center_x(Length::Fixed(ICONO_LADO))
            .center_y(Length::Fixed(ICONO_LADO))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: (ICONO_LADO / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let estado = if self.silenciado() {
            let (es, en, cuanto) = DURACIONES[self.duracion];
            match (cuanto, super::t("es", "en") == "es") {
                (None, _) => {
                    super::t("Activado hasta desactivarlo", "On until turned off").to_string()
                }
                (Some(_), true) => format!("Activado durante {es}"),
                (Some(_), false) => format!("On for {en}"),
            }
        } else {
            super::t(
                "Las notificaciones solo se guardan aquí",
                "Notifications are only kept here",
            )
            .to_string()
        };
        container(
            row![
                disco,
                Space::new().width(Length::Fixed(12.0)),
                container(column![
                    text(super::t("No molestar", "Do Not Disturb"))
                        .size(14.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto()),
                    text(lista::recortar(&estado, ANCHO_ESTADO, 11.0))
                        .size(11.0)
                        .color(tema::texto2()),
                ])
                .width(Length::Fixed(ANCHO_ESTADO)),
                Space::new().width(Length::Fill),
                interruptor(self.interruptor.valor()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO_TARJETA))
        .height(Length::Fixed(BLOQUE))
        .center_y(Length::Fixed(BLOQUE))
        .padding([0, 4])
        .into()
    }

    // 0.1.0
    // modified 0.2.1 -- pilas, grupos, menú y No molestar con duraciones en píldoras
    pub fn view(&self) -> PanelElement<'_> {
        let vacia = self.lista.is_empty();
        let ajustes = {
            let fondo = tema::mezclar(
                tema::superficie_en_tarjeta(),
                tema::elevado(),
                self.boton.intensidad(0),
            );
            container(Self::glifo("preferencias", 17.0, tema::texto()))
                .width(Length::Fixed(AJUSTES))
                .height(Length::Fixed(AJUSTES))
                .center_x(Length::Fixed(AJUSTES))
                .center_y(Length::Fixed(AJUSTES))
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: (AJUSTES / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        // «Borrar» en gris, no en rojo: el HIG reserva el rojo para el
        // diálogo, y un botón de lista que destruye con el rojo pesa más que
        // el título. Con la lista vacía se apaga en vez de desaparecer.
        let borrar = {
            let (fondo, tinta) = if vacia {
                (tema::superficie_en_tarjeta(), tema::alfa(tema::texto2(), 0.6))
            } else {
                (
                    tema::mezclar(
                        tema::superficie_en_tarjeta(),
                        tema::elevado(),
                        self.boton.intensidad(1),
                    ),
                    tema::texto(),
                )
            };
            container(
                text(super::t("Borrar", "Clear"))
                    .size(13.0)
                    .font(control::peso(Weight::Medium))
                    .color(tinta),
            )
            .padding([0, 14])
            .height(Length::Fixed(AJUSTES))
            .center_y(Length::Fixed(AJUSTES))
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: (AJUSTES / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        let cabecera = container(
            row![
                control::titulo(super::t("Notificaciones", "Notifications")),
                Space::new().width(Length::Fill),
                ajustes,
                Space::new().width(Length::Fixed(8.0)),
                borrar,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(lista::CABECERA))
        .padding([0, SANGRIA as u16])
        .center_y(Length::Fixed(lista::CABECERA));

        let cuerpo: PanelElement<'_> = if vacia {
            container(
                column![
                    text(super::t("No hay notificaciones", "No notifications"))
                        .size(15.0)
                        .font(control::peso(Weight::Semibold))
                        .color(tema::texto()),
                    text(if self.silenciado() {
                        super::t("Las nuevas se guardan aquí.", "New ones are kept here.")
                    } else {
                        super::t("Estás al día.", "You're all caught up.")
                    })
                    .size(12.0)
                    .color(tema::texto2()),
                ]
                .spacing(6)
                .align_x(Horizontal::Center),
            )
            .width(Length::Fixed(ANCHO_TARJETA))
            .height(Length::Fixed(VACIO))
            .center_x(Length::Fixed(ANCHO_TARJETA))
            .center_y(Length::Fixed(VACIO))
            .into()
        } else {
            let (_, grupos) = self.grupos();
            let mut todas = column![];
            let mut fin = 0.0;
            for c in self.colocar() {
                if c.y > fin {
                    todas = todas.push(Space::new().height(Length::Fixed(c.y - fin)));
                }
                let pieza: PanelElement<'_> = match c.fila {
                    Fila::Aviso(i) => {
                        let resaltado = self.seleccionada == Some(i);
                        self.tarjeta(
                            &self.lista[i],
                            self.fila
                                .intensidad(i)
                                .max(if resaltado { 0.65 } else { 0.0 }),
                            None,
                        )
                    }
                    Fila::Pila(g) => {
                        let i = grupos[g].idxs[0];
                        let resaltado = self.seleccionada == Some(i);
                        self.tarjeta(
                            &self.lista[i],
                            self.fila
                                .intensidad(i)
                                .max(if resaltado { 0.65 } else { 0.0 }),
                            Some(grupos[g].idxs.len()),
                        )
                    }
                    Fila::Grupo(g) => self.cabecera_grupo(&grupos[g].app),
                    Fila::Mas(g) => {
                        let resto = grupos[g].idxs.len() - POR_GRUPO;
                        container(
                            text(if super::t("es", "en") == "es" {
                                format!("Mostrar {resto} más")
                            } else {
                                format!("Show {resto} more")
                            })
                            .size(12.0)
                            .font(control::peso(Weight::Medium))
                            .color(tema::texto2()),
                        )
                        .width(Length::Fixed(ANCHO_TARJETA))
                        .height(Length::Fixed(MAS))
                        .center_x(Length::Fixed(ANCHO_TARJETA))
                        .center_y(Length::Fixed(MAS))
                        .into()
                    }
                    Fila::Menu => match &self.menu {
                        Some(m) => self.menu_en_linea(m),
                        None => Space::new().into(),
                    },
                };
                todas = todas.push(pieza);
                fin = c.y + c.alto;
            }
            lista::ventana_ancha(
                todas.into(),
                ANCHO_TARJETA,
                self.alto_ventana(),
                self.maximo(),
                self.desplazamiento,
            )
        };

        let mut contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            cuerpo,
            Space::new().height(Length::Fixed(SOBRE_BLOQUE / 2.0)),
            control::divisor(ANCHO_TARJETA),
            Space::new().height(Length::Fixed(SOBRE_BLOQUE / 2.0 - 0.5)),
            self.bloque_no_molestar(),
        ];
        if self.silenciado() {
            contenido = contenido
                .push(Space::new().height(Length::Fixed(2.0)))
                .push(self.fila_duraciones(0))
                .push(Space::new().height(Length::Fixed(HUECO)))
                .push(self.fila_duraciones(2))
                .push(Space::new().height(Length::Fixed(6.0)))
                .push(
                    container(
                        text(super::t(
                            "Las urgentes siguen sonando.",
                            "Urgent ones still make a sound.",
                        ))
                        .size(11.0)
                        .color(tema::texto2()),
                    )
                    .padding([0, 4])
                    .height(Length::Fixed(18.0))
                    .center_y(Length::Fixed(18.0)),
                );
        }
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notificaciones::Accion as AccionApp;

    fn accion(clave: &str, etiqueta: &str) -> AccionApp {
        AccionApp {
            clave: clave.into(),
            etiqueta: etiqueta.into(),
        }
    }

    fn aviso(
        id: u32,
        app: &str,
        cuerpo: &str,
        acciones: Vec<AccionApp>,
        critica: bool,
    ) -> Notificacion {
        Notificacion::nueva_con_datos(
            id,
            app.into(),
            format!("Resumen {id}"),
            cuerpo.into(),
            "",
            critica,
            acciones,
            None,
            false,
        )
    }

    fn con(lista: Vec<Notificacion>) -> Notificaciones {
        let mut n = Notificaciones::new();
        n.actualizar(lista);
        n
    }

    /// El centro de una tarjeta, en coordenadas de la emergente.
    fn centro(n: &Notificaciones, fila: Fila) -> (f32, f32) {
        let c = n
            .colocar()
            .into_iter()
            .find(|c| c.fila == fila)
            .expect("la pieza está en la lista");
        (
            lista::MARGEN + 40.0,
            n.y_lista() + c.y + c.alto / 2.0 - n.desplazamiento,
        )
    }

    /// El interruptor enciende y apaga el silencio.
    #[test]
    fn el_interruptor_alterna_el_silencio() {
        let mut n = Notificaciones::new();
        assert!(!n.silenciado());
        let r = n.rect_interruptor();
        n.pulsar(r.x + 20.0, r.y + 20.0);
        assert!(n.silenciado());
        n.pulsar(r.x + 20.0, r.y + 20.0);
        assert!(!n.silenciado(), "el segundo toque lo apaga");
    }

    /// Las duraciones solo existen con el silencio puesto —es lo que dice el
    /// diseño—, y elegir una lo mantiene con esa cuenta.
    #[test]
    fn las_duraciones_solo_estan_con_el_silencio_puesto() {
        let mut n = Notificaciones::new();
        let y = n.y_duraciones() + 2.0;
        n.pulsar(30.0, y);
        assert!(!n.silenciado(), "sin silencio ahí no hay ningún botón");

        let r = n.rect_interruptor();
        n.pulsar(r.x + 20.0, r.y + 20.0);
        assert!(n.silenciado());
        n.pulsar(30.0, n.y_duraciones() + 2.0);
        assert_eq!(n.duracion, 0, "el primero es «1 hora»");
        assert!(n.silenciado());
    }

    /// Un silencio caducado no silencia. Se comprueba con una duración de cero,
    /// que es lo único que se puede medir sin esperar una hora.
    #[test]
    fn el_silencio_caduca_solo() {
        let mut n = Notificaciones::new();
        n.silencio = Some((Instant::now(), Some(Duration::ZERO)));
        assert!(!n.silenciado());
    }

    #[test]
    fn las_flechas_y_intro_recuperan_una_accion() {
        let mut n = con(vec![aviso(
            7,
            "Prueba",
            "",
            vec![accion("abrir", "Abrir")],
            false,
        )]);
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Abajo),
            Tecla::Consumida
        ));
        // Sin acción por defecto, el primer Intro despliega y el segundo ejecuta.
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Intro),
            Tecla::Consumida
        ));
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::NotificacionAccion { id: 7, .. })
        ));
    }

    /// Un cuerpo largo se despliega al pulsar y se recoge al volver a pulsar; la
    /// tarjeta crece y encoge con él.
    #[test]
    fn pulsar_una_fila_larga_la_despliega_y_la_recoge() {
        let mut n = con(vec![aviso(1, "App", &"palabra ".repeat(40), vec![], false)]);
        let cerrada = n.size().1;
        let (x, y) = centro(&n, Fila::Aviso(0));
        assert!(n.pulsar(x, y).is_none(), "desplegar no es una acción");
        assert!(n.size().1 > cerrada);
        let (x, y) = centro(&n, Fila::Aviso(0));
        assert!(n.pulsar(x, y).is_none());
        assert_eq!(n.size().1, cerrada);
    }

    /// Una tarjeta corta y sin acciones sigue como antes: un toque la cierra.
    #[test]
    fn una_fila_corta_se_sigue_cerrando_con_un_toque() {
        let mut n = con(vec![aviso(7, "App", "hola", vec![], false)]);
        let (x, y) = centro(&n, Fila::Aviso(0));
        assert!(matches!(
            n.pulsar(x, y),
            Some(Accion::CerrarNotificacion(7))
        ));
    }

    /// Pulsar un aviso con acción por defecto la hace, que es lo que la
    /// especificación reserva a pulsar el aviso, y no lo despliega.
    #[test]
    fn pulsar_un_aviso_hace_su_accion_por_defecto() {
        let mut n = con(vec![aviso(
            3,
            "Chat",
            "hola",
            vec![accion("default", ""), accion("r", "Responder")],
            false,
        )]);
        let (x, y) = centro(&n, Fila::Aviso(0));
        assert!(matches!(
            n.pulsar(x, y),
            Some(Accion::NotificacionAccion { id: 3, clave }) if clave == "default"
        ));
        assert!(n.desplegada.is_none(), "no se despliega");
    }

    /// El chevron despliega sin abrir, y desplegada cada botón devuelve su
    /// acción, con «Abrir» el primero.
    #[test]
    fn el_chevron_despliega_y_los_botones_actuan() {
        let mut n = con(vec![aviso(
            3,
            "Chat",
            "hola",
            vec![accion("r", "Responder"), accion("default", "")],
            false,
        )]);
        let c = n.colocar()[0];
        let y_chevron = n.y_lista() + c.y + 4.0 + BTN / 2.0;
        let x_chevron = lista::MARGEN + ANCHO_TARJETA - 8.0 - BTN * 2.5;
        assert!(n.pulsar(x_chevron, y_chevron).is_none());
        assert_eq!(n.desplegada, Some(3));

        let notif = &n.lista[0];
        let y = n.y_lista()
            + PAD
            + CAB
            + TIT
            + Notificaciones::lineas(notif, true).len() as f32 * LINEA
            + (ACCIONES - BOTON)
            + BOTON / 2.0;
        let x0 = lista::MARGEN + PAD + ICONO_LADO + 12.0;
        let clave = |x: f32| match n.zona(x, y) {
            Some(Zona::Boton(_, c)) => c,
            otra => panic!("se esperaba un botón y salió {otra:?}"),
        };
        assert_eq!(clave(x0 + 5.0), "default", "«Abrir» va el primero");
        assert_eq!(clave(x0 + ANCHO_TEXTO - 5.0), "r");
    }

    /// Los avisos de una misma app se agrupan en una pila, y los urgentes
    /// salen de ella y van arriba.
    #[test]
    fn agrupa_por_app_y_los_urgentes_van_arriba() {
        let n = con(vec![
            aviso(1, "Chat", "uno", vec![], false),
            aviso(2, "Otra", "dos", vec![], false),
            aviso(3, "Chat", "tres", vec![], false),
            aviso(4, "Chat", "urgente", vec![], true),
        ]);
        let filas: Vec<Fila> = n.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(
            filas,
            [Fila::Aviso(3), Fila::Pila(0), Fila::Aviso(1)],
            "el urgente arriba, la pila de Chat y el de Otra"
        );
    }

    /// Una pila se abre al pulsarla y deja ver sus avisos con su cabecera; la
    /// cabecera plega.
    #[test]
    fn la_pila_se_abre_y_se_pliega() {
        let mut n = con(vec![
            aviso(1, "Chat", "uno", vec![], false),
            aviso(2, "Chat", "dos", vec![], false),
        ]);
        let (x, y) = centro(&n, Fila::Pila(0));
        assert!(n.pulsar(x, y).is_none());
        assert_eq!(n.grupo.as_deref(), Some("Chat"));
        let filas: Vec<Fila> = n.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(filas, [Fila::Grupo(0), Fila::Aviso(0), Fila::Aviso(1)]);
        let (x, y) = centro(&n, Fila::Grupo(0));
        let plegar = lista::MARGEN + ANCHO_TARJETA - 4.0 - BTN - 4.0 - 20.0;
        assert!(n.pulsar(plegar.min(x + 200.0), y).is_none());
        assert!(n.grupo.is_none(), "«Mostrar menos» pliega");
    }

    /// Un grupo largo enseña cuatro y un «Mostrar N más».
    #[test]
    fn un_grupo_largo_enseña_cuatro_y_mostrar_mas() {
        let avisos: Vec<_> = (0..7)
            .map(|i| aviso(i, "Chat", "x", vec![], false))
            .collect();
        let mut n = con(avisos);
        n.grupo = Some("Chat".into());
        let filas: Vec<Fila> = n.colocar().into_iter().map(|c| c.fila).collect();
        assert_eq!(
            filas.iter().filter(|f| matches!(f, Fila::Aviso(_))).count(),
            4
        );
        assert_eq!(filas.last(), Some(&Fila::Mas(0)));
        let (x, y) = centro(&n, Fila::Mas(0));
        n.pulsar(x, y);
        let todas = n
            .colocar()
            .into_iter()
            .filter(|c| matches!(c.fila, Fila::Aviso(_)))
            .count();
        assert_eq!(todas, 7);
    }

    /// La ✕ de una pila cierra todos sus avisos de una vez.
    #[test]
    fn la_cruz_de_una_pila_cierra_todos_sus_avisos() {
        let mut n = con(vec![
            aviso(1, "Chat", "uno", vec![], false),
            aviso(2, "Chat", "dos", vec![], false),
        ]);
        let c = n.colocar()[0];
        let y = n.y_lista() + c.y + 4.0 + BTN / 2.0;
        let x = lista::MARGEN + ANCHO_TARJETA - 8.0 - BTN / 2.0;
        assert!(matches!(
            n.pulsar(x, y),
            Some(Accion::CerrarNotificaciones(ids)) if ids == vec![1, 2]
        ));
    }

    /// El menú ⋯ ofrece abrir (solo si la app tiene acción por defecto), los
    /// ajustes y borrar, y cada opción hace lo suyo.
    #[test]
    fn el_menu_ofrece_abrir_ajustes_y_borrar() {
        let mut n = con(vec![aviso(
            5,
            "Chat",
            "hola",
            vec![accion("default", "")],
            false,
        )]);
        let c = n.colocar()[0];
        let y = n.y_lista() + c.y + 4.0 + BTN / 2.0;
        let x = lista::MARGEN + ANCHO_TARJETA - 8.0 - BTN * 1.5;
        assert!(n.pulsar(x, y).is_none());
        assert_eq!(n.menu, Some(Menu::Aviso(5)));
        let etiquetas: Vec<String> = n
            .items_menu(&Menu::Aviso(5))
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        assert_eq!(
            etiquetas,
            [
                format!("{} Chat", crate::t("Abrir", "Open")),
                format!("{} Chat", crate::t("Silenciar", "Mute")),
                crate::t("Ajustes de notificaciones…", "Notification settings…").to_string(),
                crate::t("Borrar", "Clear").to_string(),
            ]
        );
        let (xm, ym) = centro(&n, Fila::Menu);
        // El primer renglón del menú es «Abrir».
        let y_abrir =
            ym - (n.items_menu(&Menu::Aviso(5)).len() as f32 * ITEM_MENU) / 2.0 + ITEM_MENU / 2.0;
        assert!(matches!(
            n.pulsar(xm, y_abrir),
            Some(Accion::NotificacionAccion { id: 5, clave }) if clave == "default"
        ));
        let sin_default = con(vec![aviso(6, "Otra", "x", vec![], false)]);
        let sin: Vec<_> = sin_default.items_menu(&Menu::Aviso(6));
        assert!(
            !sin.iter().any(|(o, _)| *o == Opcion::Abrir),
            "sin default no hay «Abrir»"
        );
    }

    /// La lista vacía mide lo mismo que el aviso de «vacío», para que la tarjeta
    /// no dé un salto al llegar la primera; y «Borrar» no hace nada.
    #[test]
    fn la_lista_vacia_ocupa_su_aviso() {
        let mut n = Notificaciones::new();
        assert_eq!(n.alto_ventana(), VACIO);
        let r = n.rect_borrar();
        assert!(n.pulsar(r.x + 5.0, r.y + 5.0).is_none());
        n.actualizar(vec![aviso(1, "App", "x", vec![], false)]);
        let r = n.rect_borrar();
        assert!(matches!(
            n.pulsar(r.x + 5.0, r.y + 5.0),
            Some(Accion::BorrarNotificaciones)
        ));
    }

    /// El alto de la tarjeta cuadra con lo que se coloca: la lista, la fila de
    /// «No molestar» y el margen acaban donde acaba la emergente.
    #[test]
    fn la_geometria_cuadra_con_el_alto() {
        let mut n = con(vec![
            aviso(1, "Chat", "uno", vec![], false),
            aviso(2, "Chat", &"palabra ".repeat(40), vec![], false),
            aviso(3, "Otra", "x", vec![], true),
        ]);
        n.grupo = Some("Chat".into());
        let bloque = n.rect_interruptor();
        assert_eq!(bloque.y + bloque.height + lista::MARGEN, n.size().1);
        assert!(n.alto_ventana() <= MAX_LISTA);
    }

    /// Vuelca la tarjeta a PNG para mirarla: con pila, grupo abierto,
    /// desplegado, menú, silencio y vacía. `BOOKOS_NOTIF_TARJETA_PNG=/tmp/notif`
    /// escribe `/tmp/notif-<variante>-<tema>.png`.
    ///
    /// Los avisos entran por el shell y se llega a cada variante pulsando, como
    /// lo haría una persona: `abrir` rehace la tarjeta con la cola del shell, y
    /// una `Notificaciones` montada a mano se descartaría.
    #[test]
    fn vuelca_la_tarjeta_a_png() {
        let Ok(prefijo) = std::env::var("BOOKOS_NOTIF_TARJETA_PNG") else {
            return;
        };
        let avisos = || {
            let largo = "¿Has visto el cambio del panel? Me parece que la isla queda mucho mejor ahora que no tapa el título.";
            let mut progreso = aviso(4, "Explorador", "A «Documentos» · 62 %", vec![], false);
            progreso.progreso = Some(62);
            vec![
                aviso(
                    1,
                    "vesktop",
                    largo,
                    vec![accion("default", ""), accion("inline-reply", "Responder")],
                    false,
                ),
                aviso(
                    2,
                    "vesktop",
                    "Te paso el enlace cuando termine.",
                    vec![accion("default", "")],
                    false,
                ),
                aviso(3, "BookOS", "Ya se puede pegar.", vec![], false),
                progreso,
                aviso(
                    5,
                    "Energía",
                    "Quedan 12 min. Conecta el cargador.",
                    vec![],
                    true,
                ),
            ]
        };
        // La misma pulsación en el shell y en la copia con la que se calcula
        // dónde pulsar: parten de lo mismo, así que no se separan.
        fn pulsar(shell: &mut crate::Shell, copia: &mut Notificaciones, x: f32, y: f32) {
            shell.emergente_pulsar(x, y);
            copia.pulsar(x, y);
        }
        for (tema, nombre) in [
            (crate::tema::Tema::Oscuro, "oscuro"),
            (crate::tema::Tema::Claro, "claro"),
        ] {
            for variante in ["pila", "grupo", "desplegado", "menu", "silencio", "vacio"] {
                let config = crate::Config {
                    tema,
                    ..crate::Config::default()
                };
                let mut shell = crate::Shell::con_config(1646, 1.0, config);
                // Con todos, la lista se desplaza y lo de abajo queda fuera de
                // la ventana: las variantes que pulsan allí usan menos avisos.
                let completo = matches!(variante, "pila" | "silencio");
                if variante != "vacio" {
                    for n in avisos()
                        .into_iter()
                        .filter(|n| completo || n.app == "vesktop" || n.app == "BookOS")
                    {
                        shell.notificar(n, -1);
                    }
                }
                shell.abrir_de_widget("notificaciones");
                let mut copia = con(shell.notificaciones().to_vec());
                let en = |n: &Notificaciones, c: Colocada<Fila>, dx: f32, dy: f32| {
                    (
                        lista::MARGEN + dx,
                        n.y_lista() + c.y - n.desplazamiento + dy,
                    )
                };
                let pila = |n: &Notificaciones| {
                    n.colocar()
                        .into_iter()
                        .find(|c| matches!(c.fila, Fila::Pila(_)))
                        .expect("hay una pila de vesktop")
                };
                let esquina = |k: f32| ANCHO_TARJETA - 8.0 - BTN * k;
                if matches!(variante, "grupo" | "desplegado") {
                    let (x, y) = en(&copia, pila(&copia), 40.0, 20.0);
                    pulsar(&mut shell, &mut copia, x, y);
                }
                if variante == "desplegado" {
                    let i = copia
                        .lista
                        .iter()
                        .position(|n| n.id == 1)
                        .expect("el aviso 1");
                    let c = copia
                        .colocar()
                        .into_iter()
                        .find(|c| c.fila == Fila::Aviso(i))
                        .expect("el grupo está abierto");
                    let (x, y) = en(&copia, c, esquina(2.5), 4.0 + BTN / 2.0);
                    pulsar(&mut shell, &mut copia, x, y);
                }
                if variante == "menu" {
                    let (x, y) = en(&copia, pila(&copia), esquina(1.5), 4.0 + BTN / 2.0);
                    pulsar(&mut shell, &mut copia, x, y);
                }
                if variante == "silencio" {
                    let r = copia.rect_interruptor();
                    pulsar(&mut shell, &mut copia, r.x + 20.0, r.y + 20.0);
                }
                // Los muelles son de tiempo real: sin esperar, el interruptor
                // sale a mitad de recorrido.
                std::thread::sleep(Duration::from_millis(400));
                let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
                let mut buf = vec![0u8; (w * h * 4) as usize];
                shell.draw_emergente(&mut buf);
                for p in buf.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                }
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

    #[test]
    fn menu_ofrece_silenciar_y_reactivar_app() {
        let mut notif = Notificaciones::new();
        let menu = Menu::Pila("Discord".into());
        let antes = notif.items_menu(&menu);
        assert!(antes.iter().any(|(op, t)| *op == Opcion::Silenciar && t.contains("Discord")));
        assert!(antes.iter().any(|(_, t)| t.starts_with("Silenciar") || t.starts_with("Mute")));

        notif.poner_silenciadas(vec!["discord".into()]);
        assert!(notif.app_silenciada("Discord"));
        let despues = notif.items_menu(&menu);
        assert!(
            despues
                .iter()
                .any(|(op, t)| *op == Opcion::Silenciar && (t.contains("Activar") || t.contains("Unmute")))
        );
    }

    #[test]
    fn escape_cierra_menu_antes_que_el_emergente_y_actualizar_limpia_todos() {
        let mut notif = Notificaciones::new();
        let n = Notificacion::nueva(
            7,
            "Discord".into(),
            "Hola".into(),
            "Mundo".into(),
            "discord",
            false,
        );
        notif.actualizar(vec![n]);
        notif.todos.push("Discord".into());
        notif.menu = Some(Menu::Aviso(7));

        assert!(matches!(notif.tecla(crate::TeclaPulsada::Escape), Tecla::Consumida));
        assert!(notif.menu.is_none());
        assert!(matches!(notif.tecla(crate::TeclaPulsada::Escape), Tecla::Cerrar));

        notif.actualizar(Vec::new());
        assert!(notif.todos.is_empty());
    }
}
