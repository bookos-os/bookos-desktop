//! El escritorio para un lector de pantalla (HIG §11, semántica).
//!
//! El panel, el dock, la isla y las tarjetas no son ventanas de ningún
//! toolkit: los pinta el compositor, y sin esto no existen para Orca. Aquí se
//! describen como un árbol de AccessKit que el compositor publica en AT-SPI2.
//!
//! El shell sabe **qué** hay y dónde dentro de cada superficie; el compositor
//! sabe **dónde** está cada superficie en pantalla y se lo pasa en
//! [`Origenes`]. Todas las coordenadas son lógicas y globales.
//!
//! Pulsar un nodo desde Orca no tiene camino propio: el compositor lo resuelve
//! con un clic en el centro de su rectángulo ([`Arbol::centro`]), así que hace
//! exactamente lo mismo que el ratón.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use accesskit::{Live, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};

use crate::emergente::t;
use crate::{Shell, TeclaPulsada};

/// Dónde está cada superficie del shell, en lógicos globales.
#[derive(Debug, Clone, Copy, Default)]
pub struct Origenes {
    pub pantalla: (f64, f64),
    /// La esquina del panel; `None` si está apartado.
    pub panel: Option<(f64, f64)>,
    pub dock: Option<(f64, f64)>,
    pub emergente: Option<(f64, f64)>,
    /// El rectángulo de la isla, si se ve.
    pub isla: Option<(f64, f64, f64, f64)>,
}

/// El árbol listo para AccessKit y, para cada nodo pulsable, su centro.
pub struct Arbol {
    pub actualizacion: TreeUpdate,
    /// El teclado está en el shell (una barra o una tarjeta): para AT-SPI es
    /// la «ventana activa», y sin eso Orca no lee los cambios de foco.
    pub con_foco: bool,
    centros: HashMap<NodeId, (f64, f64)>,
}

impl Arbol {
    /// Dónde hay que hacer clic para activar el nodo, en lógicos globales.
    pub fn centro(&self, id: NodeId) -> Option<(f64, f64)> {
        self.centros.get(&id).copied()
    }
}

const RAIZ: NodeId = NodeId(1);

/// Un identificador estable a partir de un nombre: Orca sigue el foco y la
/// navegación por identificador, y un índice cambiaría al reordenar el dock.
fn id(clave: &str) -> NodeId {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    clave.hash(&mut h);
    // 0 y 1 están reservados (sin uso y raíz).
    NodeId(h.finish() | 2)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        x0: x,
        y0: y,
        x1: x + w,
        y1: y + h,
    }
}

struct Constructor {
    nodos: Vec<(NodeId, Node)>,
    centros: HashMap<NodeId, (f64, f64)>,
}

impl Constructor {
    fn poner(&mut self, clave: &str, mut nodo: Node, r: Rect, pulsable: bool) -> NodeId {
        let id = id(clave);
        nodo.set_bounds(r);
        if pulsable {
            nodo.add_action(accesskit::Action::Click);
            self.centros
                .insert(id, ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0));
        }
        self.nodos.push((id, nodo));
        id
    }
}

/// El valor va **dentro del nombre** («Batería, 72 %»): el adaptador de AT-SPI
/// de AccessKit (accesskit_atspi_common 0.21) solo publica `value` en campos
/// de texto y deslizadores, y en un botón Orca decía «Batería, botón» sin el
/// número. Comprobado leyendo el árbol con `gi.Atspi`.
fn nodo(rol: Role, nombre: &str, valor: &str) -> Node {
    let mut n = Node::new(rol);
    if valor.is_empty() {
        n.set_label(nombre);
    } else {
        n.set_label(format!("{nombre}, {valor}"));
    }
    n
}

/// «Desplegado» va en la descripción por lo mismo: el adaptador no traduce
/// `expanded` a ningún estado de AT-SPI.
fn desplegado(n: &mut Node, abierto: bool) {
    if abierto {
        n.set_description(t("desplegado", "expanded"));
    }
}

impl Shell {
    /// Todo lo que se ve del escritorio, como árbol accesible.
    // añadido en la 0.2.2
    pub fn arbol_accesible(&self, o: &Origenes) -> Arbol {
        let mut c = Constructor {
            nodos: Vec::new(),
            centros: HashMap::new(),
        };
        let mut hijos = Vec::new();
        let mut foco = match self.foco_barra {
            Some(Foco::Panel(i)) => self
                .controles_panel()
                .get(i)
                .map(|(n, _, _)| id(&format!("panel:{}", n.unwrap_or("logo")))),
            Some(Foco::Dock(i)) => self
                .dock_items
                .items()
                .get(i)
                .map(|item| id(&format!("dock:{}:{}", item.app_id(), item.exec()))),
            None => None,
        };

        if let Some((px, py)) = o.panel {
            hijos.push(self.panel_accesible(&mut c, px, py));
        }
        if let (Some(r), Some((actividad, _))) = (o.isla, self.actividad.as_ref()) {
            let (titulo, subtitulo) = actividad.rotulos();
            // Solo el título en el nombre: AccessKit anuncia una región viva
            // cuando cambia su nombre, y el subtítulo de una copia lleva la
            // velocidad, que cambia cada segundo. Cambiar de canción sí se
            // anuncia; el segundero no (HIG §11: sin releer cada actualización).
            let mut n = nodo(Role::Status, titulo, "");
            n.set_role_description(t("isla", "island"));
            // Cortés: lee el cambio cuando el usuario calla, sin interrumpir.
            n.set_live(Live::Polite);
            let mut descripcion = subtitulo.to_string();
            if actividad.desplegada() {
                descripcion.push_str(t(", desplegada", ", expanded"));
            }
            n.set_description(descripcion);
            hijos.push(c.poner("isla", n, rect(r.0, r.1, r.2, r.3), true));
        }
        // Los avisos se anuncian sin robar el foco (HIG §11): región viva, que
        // AccessKit lee al aparecer. La crítica interrumpe; las demás esperan
        // a que Orca calle. Sin acción: el aviso se pulsa con el puntero o se
        // atiende desde el centro de notificaciones, que sí es navegable.
        for (toast, _) in &self.toasts {
            let n = toast.notificacion();
            let mut nodo = nodo(Role::Alert, &format!("{}: {}", n.app, n.resumen), "");
            nodo.set_description(n.cuerpo.as_str());
            nodo.set_live(if n.critica {
                Live::Assertive
            } else {
                Live::Polite
            });
            hijos.push(c.poner(
                &format!("aviso:{}", n.id),
                nodo,
                rect(0.0, 0.0, 0.0, 0.0),
                false,
            ));
        }
        if let Some((dx, dy)) = o.dock {
            hijos.push(self.dock_accesible(&mut c, dx, dy));
        }
        if let (Some((ex, ey)), Some((e, _))) = (o.emergente, self.emergente.as_ref()) {
            let mut filas = Vec::new();
            let mut en_fila = None;
            for f in e.accesible() {
                let valor = if f.rol == Role::PasswordInput {
                    ""
                } else {
                    &f.valor
                };
                let mut n = nodo(f.rol, &f.nombre, valor);
                if let Some(m) = f.marcada {
                    n.set_toggled(Toggled::from(m));
                }
                if f.rol == Role::ListBoxOption {
                    n.set_selected(f.seleccionada);
                }
                if f.rol == Role::PasswordInput {
                    // Un campo de clave nunca enseña su contenido; el valor
                    // es la ayuda («8 caracteres, Intro para conectar»).
                    n.set_description(f.valor.as_str());
                }
                let (x, y, w, h) = f.rect;
                let clave = format!("{}:{}", e.nombre(), f.id);
                // Con la tarjeta abierta el foco es suyo: la fila señalada, o
                // la tarjeta entera si no hay ninguna.
                if f.seleccionada {
                    en_fila = Some(id(&clave));
                }
                filas.push(c.poner(
                    &clave,
                    n,
                    rect(ex + x as f64, ey + y as f64, w as f64, h as f64),
                    f.rol != Role::PasswordInput,
                ));
            }
            let (w, h) = e.size();
            let mut n = nodo(Role::Dialog, e.titulo_accesible(), "");
            n.set_children(filas);
            let clave = format!("emergente:{}", e.nombre());
            // Con la tarjeta abierta las teclas son suyas, así que el foco
            // también, aunque se abriera desde un botón del panel con foco.
            foco = Some(en_fila.unwrap_or_else(|| id(&clave)));
            hijos.push(c.poner(&clave, n, rect(ex, ey, w as f64, h as f64), false));
        }

        let mut raiz = nodo(Role::Window, "BookUI", "");
        raiz.set_bounds(rect(0.0, 0.0, o.pantalla.0, o.pantalla.1));
        raiz.set_children(hijos);
        c.nodos.push((RAIZ, raiz));

        let mut info = TreeInfo::new(RAIZ);
        info.toolkit_name = Some("BookUI".into());
        info.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
        // Un foco que no está en el árbol (el panel apartado) sería un error
        // de AccessKit: se queda en la raíz.
        let foco = foco.filter(|f| c.nodos.iter().any(|(i, _)| i == f));
        Arbol {
            actualizacion: TreeUpdate {
                nodes: c.nodos,
                tree: Some(info),
                tree_id: TreeId::ROOT,
                focus: foco.unwrap_or(RAIZ),
            },
            con_foco: foco.is_some(),
            centros: c.centros,
        }
    }

    fn panel_accesible(&self, c: &mut Constructor, px: f64, py: f64) -> NodeId {
        let alto = self.panel_alto as f64;
        let abierta = self.emergente_nombre();
        // En orden de lectura, el mismo que recorre el foco de teclado.
        let mut botones = Vec::new();
        for (nombre, x0, x1) in self.controles_panel() {
            let n = match nombre {
                None => {
                    let mut n = nodo(Role::Button, t("Menú de BookUI", "BookUI menu"), "");
                    n.set_has_popup(accesskit::HasPopup::Menu);
                    desplegado(&mut n, abierta == Some("menu"));
                    n
                }
                Some(nombre) => {
                    let Some((rotulo, valor)) = self.widgets.accesible_de(nombre) else {
                        continue;
                    };
                    let mut n = nodo(Role::Button, rotulo, &valor);
                    if let Some(tarjeta) = crate::nombre_emergente_de(nombre) {
                        n.set_has_popup(accesskit::HasPopup::Dialog);
                        desplegado(&mut n, abierta == Some(tarjeta));
                    }
                    n
                }
            };
            botones.push(c.poner(
                &format!("panel:{}", nombre.unwrap_or("logo")),
                n,
                rect(px + x0 as f64, py, (x1 - x0) as f64, alto),
                true,
            ));
        }
        let mut panel = nodo(Role::Toolbar, t("Panel", "Panel"), "");
        panel.set_children(botones);
        c.poner(
            "panel",
            panel,
            rect(
                px,
                py,
                self.panel.size.width as f64 * self.panel_zoom() as f64,
                alto,
            ),
            false,
        )
    }

    fn dock_accesible(&self, c: &mut Constructor, dx: f64, dy: f64) -> NodeId {
        let items = self.dock_items.items();
        let iconos: Vec<NodeId> = self
            .dock_items
            .capas()
            .into_iter()
            .zip(items)
            .map(|((x, y, w, h, _), item)| {
                let mut n = nodo(Role::Button, &item.label, "");
                if item.abierta() {
                    n.set_description(t("Abierta", "Open"));
                }
                c.poner(
                    &format!("dock:{}:{}", item.app_id(), item.exec()),
                    n,
                    rect(dx + x as f64, dy + y as f64, w as f64, h as f64),
                    true,
                )
            })
            .collect();
        let (w, h) = self.dock_logical_size();
        let mut dock = nodo(Role::Toolbar, "Dock", "");
        dock.set_children(iconos);
        c.poner("dock", dock, rect(dx, dy, w as f64, h as f64), false)
    }
}

/// El control de una barra que tiene el foco de teclado (HIG §11, teclado).
///
/// El panel y el dock no son ventanas y no reciben el teclado por sí solos:
/// sin esto, lo que no fuera una tarjeta solo se podía usar con el puntero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foco {
    /// Índice en [`Shell::controles_panel`], de izquierda a derecha.
    Panel(usize),
    /// Índice del icono en el dock.
    Dock(usize),
}

/// Qué ha hecho una tecla con el foco en una barra.
pub enum TeclaFoco {
    /// No era para la barra: que siga su camino.
    Ignorada,
    /// Movió el foco o lo soltó; hay que repintar.
    Consumida,
    /// Activó el control.
    Hacer(crate::Accion),
}

impl Shell {
    /// Los controles del panel en orden de lectura: el logo y los widgets de
    /// izquierda a derecha, con su nombre de widget (`None` el logo) y su
    /// franja en lógicos del panel.
    fn controles_panel(&self) -> Vec<(Option<&'static str>, f32, f32)> {
        let mut v: Vec<_> = self
            .zonas_panel()
            .into_iter()
            .map(|(n, x0, x1)| (Some(n), x0, x1))
            .collect();
        v.push((None, 0.0, crate::view::ANCHO_LOGO * self.panel_zoom()));
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    /// Lleva el foco de teclado al primer control del panel. Si ya estaba en
    /// una barra, lo suelta: el mismo atajo entra y sale.
    // añadido en la 0.2.2
    pub fn enfocar_panel(&mut self) -> bool {
        self.foco_barra = match self.foco_barra {
            Some(_) => None,
            None => Some(Foco::Panel(0)),
        };
        self.panel.painted_once = false;
        true
    }

    // añadido en la 0.2.2
    pub fn foco_barra(&self) -> Option<Foco> {
        self.foco_barra
    }

    /// Suelta el foco de la barra: se pulsó fuera o una ventana lo pidió.
    // añadido en la 0.2.2
    pub fn soltar_foco_barra(&mut self) -> bool {
        self.foco_barra.take().is_some()
    }

    /// Una tecla con el foco en una barra. Con una tarjeta abierta no llega
    /// aquí: la tarjeta tiene sus propias teclas y, al cerrarse, el foco
    /// sigue en el botón que la abrió (HIG §11, foco).
    // añadido en la 0.2.2
    pub fn tecla_foco(&mut self, tecla: TeclaPulsada) -> TeclaFoco {
        let Some(foco) = self.foco_barra else {
            return TeclaFoco::Ignorada;
        };
        let (n_panel, n_dock) = (self.controles_panel().len(), self.dock_items.items().len());
        let mover =
            |i: usize, n: usize, paso: isize| (i as isize + paso).clamp(0, n as isize - 1) as usize;
        let nuevo = match (tecla, foco) {
            (TeclaPulsada::Escape, _) => None,
            (TeclaPulsada::Izquierda, Foco::Panel(i)) => Some(Foco::Panel(mover(i, n_panel, -1))),
            (TeclaPulsada::Derecha, Foco::Panel(i)) => Some(Foco::Panel(mover(i, n_panel, 1))),
            (TeclaPulsada::Izquierda, Foco::Dock(i)) => Some(Foco::Dock(mover(i, n_dock, -1))),
            (TeclaPulsada::Derecha, Foco::Dock(i)) => Some(Foco::Dock(mover(i, n_dock, 1))),
            (TeclaPulsada::Inicio, Foco::Panel(_)) => Some(Foco::Panel(0)),
            (TeclaPulsada::Fin, Foco::Panel(_)) => Some(Foco::Panel(n_panel - 1)),
            (TeclaPulsada::Inicio, Foco::Dock(_)) => Some(Foco::Dock(0)),
            (TeclaPulsada::Fin, Foco::Dock(_)) => Some(Foco::Dock(n_dock.saturating_sub(1))),
            // Tab pasa de una barra a la otra, en orden de lectura: panel
            // arriba, dock abajo.
            (TeclaPulsada::Tabulador, Foco::Panel(_)) if n_dock > 0 => Some(Foco::Dock(0)),
            (TeclaPulsada::Tabulador, _) => Some(Foco::Panel(0)),
            (TeclaPulsada::Intro | TeclaPulsada::Caracter(' '), _) => {
                return self.activar_foco(foco);
            }
            // Las flechas verticales no mueven nada en una fila; se tragan
            // igual: con el foco en el shell no deben llegar a la ventana.
            _ => Some(foco),
        };
        self.foco_barra = nuevo;
        self.panel.painted_once = false;
        TeclaFoco::Consumida
    }

    fn activar_foco(&mut self, foco: Foco) -> TeclaFoco {
        match foco {
            Foco::Panel(i) => {
                let Some(&(_, x0, x1)) = self.controles_panel().get(i) else {
                    return TeclaFoco::Consumida;
                };
                match self.panel_pulsado((x0 + x1) / 2.0, 0.0) {
                    Some(accion) => TeclaFoco::Hacer(accion),
                    None => TeclaFoco::Consumida,
                }
            }
            Foco::Dock(i) => {
                let Some(&(x, y, w, h, _)) = self.dock_items.capas().get(i) else {
                    return TeclaFoco::Consumida;
                };
                // Abrir o traer una app le da el teclado a su ventana.
                self.foco_barra = None;
                match self.dock_pulsar(x + w / 2.0, y + h / 2.0) {
                    Some(accion) => TeclaFoco::Hacer(accion),
                    None => TeclaFoco::Consumida,
                }
            }
        }
    }

    /// Dónde dibujar el anillo de foco: la barra y el rectángulo en lógicos
    /// relativos a ella.
    // añadido en la 0.2.2
    pub fn anillo_foco(&self) -> Option<(Foco, (f32, f32, f32, f32))> {
        let foco = self.foco_barra?;
        let r = match foco {
            Foco::Panel(i) => {
                let &(_, x0, x1) = self.controles_panel().get(i)?;
                (x0, 0.0, x1 - x0, self.panel_alto)
            }
            Foco::Dock(i) => {
                let &(x, y, w, h, _) = self.dock_items.capas().get(i)?;
                (x, y, w, h)
            }
        };
        Some((foco, r))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::config::Config;

    fn shell() -> Shell {
        Shell::con_config(1440, 1.0, Config::default())
    }

    fn origenes() -> Origenes {
        Origenes {
            pantalla: (1440.0, 900.0),
            panel: Some((0.0, 0.0)),
            dock: Some((500.0, 820.0)),
            emergente: Some((900.0, 40.0)),
            isla: None,
        }
    }

    fn nombre(arbol: &Arbol, id: NodeId) -> String {
        arbol
            .actualizacion
            .nodes
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, n)| n.label().map(str::to_string))
            .unwrap_or_default()
    }

    fn hijos_de(arbol: &Arbol, clave: &str) -> Vec<String> {
        let (_, n) = arbol
            .actualizacion
            .nodes
            .iter()
            .find(|(i, _)| *i == id(clave))
            .expect("el nodo está en el árbol");
        n.children().iter().map(|h| nombre(arbol, *h)).collect()
    }

    /// El panel se lee de izquierda a derecha, empezando por el menú, y cada
    /// botón lleva su valor en el nombre (Orca no lee `value` en un botón).
    #[test]
    fn el_panel_se_lee_en_orden_y_con_valor() {
        let s = shell();
        let arbol = s.arbol_accesible(&origenes());
        let botones = hijos_de(&arbol, "panel");
        assert_eq!(botones.first().map(String::as_str), Some("Menú de BookUI"));
        assert!(
            botones.iter().any(|b| b.starts_with("Reloj, ")),
            "{botones:?}"
        );
        assert_eq!(arbol.actualizacion.focus, RAIZ, "sin foco de teclado");
        assert!(!arbol.con_foco);
    }

    /// Meta+Alt+P, flechas y Tab: el foco del árbol sigue al del teclado, y
    /// Esc lo devuelve a la ventana.
    #[test]
    fn el_foco_de_teclado_llega_al_arbol() {
        let mut s = shell();
        s.enfocar_panel();
        let arbol = s.arbol_accesible(&origenes());
        assert!(arbol.con_foco);
        assert_eq!(nombre(&arbol, arbol.actualizacion.focus), "Menú de BookUI");
        s.tecla_foco(TeclaPulsada::Derecha);
        let segundo = hijos_de(&s.arbol_accesible(&origenes()), "panel")[1].clone();
        let arbol = s.arbol_accesible(&origenes());
        assert_eq!(nombre(&arbol, arbol.actualizacion.focus), segundo);
        s.tecla_foco(TeclaPulsada::Tabulador);
        assert_eq!(s.foco_barra(), Some(Foco::Dock(0)));
        s.tecla_foco(TeclaPulsada::Escape);
        assert_eq!(s.foco_barra(), None);
        assert!(!s.arbol_accesible(&origenes()).con_foco);
    }

    /// Intro sobre un botón abre su tarjeta y el foco pasa a ella; al cerrarla
    /// con Esc vuelve al botón que la abrió (HIG §11, foco).
    #[test]
    fn la_tarjeta_recibe_el_foco_y_lo_devuelve() {
        let mut s = shell();
        let i = s
            .controles_panel()
            .iter()
            .position(|(n, _, _)| *n == Some("reloj"))
            .expect("el panel de serie lleva el reloj");
        s.enfocar_panel();
        s.foco_barra = Some(Foco::Panel(i));
        s.tecla_foco(TeclaPulsada::Intro);
        assert_eq!(s.emergente_nombre(), Some("calendario"));
        let arbol = s.arbol_accesible(&origenes());
        assert_eq!(nombre(&arbol, arbol.actualizacion.focus), "Calendario");
        s.emergente_tecla(TeclaPulsada::Escape);
        assert_eq!(s.emergente_nombre(), None);
        assert_eq!(s.foco_barra(), Some(Foco::Panel(i)));
    }

    /// Con el panel apartado su botón no está en el árbol: el foco cae a la
    /// raíz en vez de señalar a un nodo que no existe (AccessKit lo rechaza).
    #[test]
    fn un_foco_fuera_del_arbol_se_queda_en_la_raiz() {
        let mut s = shell();
        s.enfocar_panel();
        let arbol = s.arbol_accesible(&Origenes {
            panel: None,
            ..origenes()
        });
        assert_eq!(arbol.actualizacion.focus, RAIZ);
    }
}
