//! Interfaz Wayland (`wlr-layer-shell`) de `bookos-extras` siguiendo `HIG-IA.md`
//! y `bookos-desktop-motion`:
//! - **100 % opaca** (`--bg` y `--card` sólidos del HIG §2, sin texto sangrando por detrás).
//! - **Fluidez nativa a 120 Hz:** la cortina se rasteriza una sola vez al cambiar su
//!   contenido y se desplaza por GPU mediante `layer.set_margin(...)` durante todo el
//!   arrastre 1:1 y el muelle — cero re-rasterización de texto por fotograma.
//! - **Indicadores flotantes fluidos** (64×64, despegados del borde) para «Atrás»
//!   (izquierda) y «Adelante» (derecha), movidos por GPU en X e Y siguiendo al dedo.

use std::collections::{HashSet, VecDeque};

use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
    Weight,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_touch,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    reexports::protocols::wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, PixmapMut, Rect, Stroke,
    Transform,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface, wl_touch},
};

use crate::fisica::{M_RAPIDO, M_REBOTE, M_SUAVE, Muelle, VentanaVelocidad, goma, proyectar_impulso};
use crate::gestos::Lado;
use crate::paleta::Paleta;
use crate::protocolo::NotificacionDto;

/// Margen superior cuando la cortina está abierta (limpia por debajo del panel).
const Y_CORTINA_ABIERTA: f32 = 44.0;
/// Tamaño de la cápsula flotante Atrás/Adelante.
const IND_LADO: u32 = 64;
/// Inicio y margen inferior de la lista desplazable de notificaciones (en lógicos).
const LISTA_Y: f32 = 18.0 + 54.0 + 16.0;
const ALTO_PIE: f32 = 54.0;
const MARGEN_FONDO_LISTA: f32 = 18.0 + ALTO_PIE;

#[derive(Debug, Clone)]
pub enum AccionUi {
    AlternarGestos,
    AlternarNoMolestar,
    AbrirAjustesNotificaciones,
    CerrarNotificacion(u32),
    LimpiarNotificaciones,
    InvocarAccion(u32, String),
}

#[derive(Debug, Clone)]
struct BotonRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    accion: AccionBoton,
}

#[derive(Debug, Clone)]
enum AccionBoton {
    AlternarGestos,
    AlternarNoMolestar,
    AbrirAjustes,
    LimpiarTodo,
    CerrarCortina,
    CerrarNotif(u32),
    AlternarExpandir(u32),
    ActivarTarjeta(u32),
    Invocar(u32, String),
}

struct ArrastreCortina {
    id: i32,
    origen_x: f64,
    origen_superficie_y: f64,
    origen_pantalla_y: f64,
    progreso_inicial: f32,
    modo_scroll: bool,
    scroll_inicial: f32,
    arrastrando: bool,
    velocidad: VentanaVelocidad,
}

struct EstadoCortina {
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional_scale: Option<WpFractionalScaleV1>,
    ancho: u32,
    alto: u32,
    configurada: bool,
    muelle: Muelle,
    toggle_muelle: Muelle,
    cerrando: bool,
    ultimo_margen: i32,
    scroll_y: f32,
    scroll_max: f32,
    botones: Vec<BotonRect>,
    toque: Option<ArrastreCortina>,
}

impl Drop for EstadoCortina {
    fn drop(&mut self) {
        if let Some(vp) = self.viewport.take() {
            vp.destroy();
        }
        if let Some(fs) = self.fractional_scale.take() {
            fs.destroy();
        }
    }
}

/// Cápsula circular flotante (64×64), despegada del borde y movida por GPU en X e Y.
struct EstadoIndicador {
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional_scale: Option<WpFractionalScaleV1>,
    lado: Lado,
    configurada: bool,
    muelle_x: Muelle,
    muelle_escala: Muelle,
    muelle_y: Muelle,
    armado: bool,
    saliendo: bool,
    ultima_escala_pintada: f32,
    ultimo_armado_pintado: bool,
}

impl Drop for EstadoIndicador {
    fn drop(&mut self) {
        if let Some(vp) = self.viewport.take() {
            vp.destroy();
        }
        if let Some(fs) = self.fractional_scale.take() {
            fs.destroy();
        }
    }
}

pub struct UiState {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    layer_shell: LayerShell,
    viewporter: Option<WpViewporter>,
    fractional_scale_mgr: Option<WpFractionalScaleManagerV1>,
    pool: SlotPool,
    font_system: FontSystem,
    swash_cache: SwashCache,

    pub pantalla_ancho: u32,
    pub pantalla_alto: u32,
    pub escala: f64,
    pub oscuro: bool,
    pub efectos_reducidos: bool,
    pub paleta: Paleta,
    pub gestos_activos: bool,
    pub no_molestar: bool,
    pub expandidas: HashSet<u32>,
    pub notificaciones: Vec<NotificacionDto>,
    pub acciones_pendientes: VecDeque<AccionUi>,

    cortina: Option<EstadoCortina>,
    indicador: Option<EstadoIndicador>,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
}

fn decidir_reposo_cortina(valor: f32, cerrando: bool) -> bool {
    !(cerrando && valor < 0.5)
}

impl UiState {
    pub fn nuevo(
        globals: &wayland_client::globals::GlobalList,
        qh: &QueueHandle<Self>,
        gestos_activos: bool,
    ) -> anyhow::Result<Self> {
        let compositor = CompositorState::bind(globals, qh)?;
        let layer_shell = LayerShell::bind(globals, qh)?;
        let shm = Shm::bind(globals, qh)?;
        let viewporter = globals.bind::<WpViewporter, _, _>(qh, 1..=1, ()).ok();
        let fractional_scale_mgr = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(qh, 1..=1, ())
            .ok();
        let pool = SlotPool::new(1024 * 1024 * 4, &shm)?;
        Ok(Self {
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, qh),
            output_state: OutputState::new(globals, qh),
            compositor,
            shm,
            layer_shell,
            viewporter,
            fractional_scale_mgr,
            pool,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            pantalla_ancho: 1280,
            pantalla_alto: 800,
            escala: 1.0,
            oscuro: true,
            efectos_reducidos: false,
            paleta: Paleta::cargar(true),
            gestos_activos,
            no_molestar: false,
            expandidas: HashSet::new(),
            notificaciones: Vec::new(),
            acciones_pendientes: VecDeque::new(),
            cortina: None,
            indicador: None,
            keyboard: None,
            pointer: None,
            touch: None,
        })
    }

    pub fn animando(&self) -> bool {
        let c = self
            .cortina
            .as_ref()
            .is_some_and(|c| c.muelle.animando() || c.toggle_muelle.animando());
        let i = self.indicador.as_ref().is_some_and(|i| {
            i.muelle_x.animando() || i.muelle_escala.animando() || i.muelle_y.animando()
        });
        c || i
    }

    /// Avanza los muelles activos. La cortina solo actualiza `set_margin` (0 ms CPU);
    /// solo repinta píxeles si se está animando el toggle interior.
    pub fn tick_animaciones(&mut self) {
        let mut mover_cortina = false;
        let mut repintar_cortina = false;
        let mut cerrar_cortina = false;
        if let Some(c) = self.cortina.as_mut() {
            if c.muelle.avanzar() {
                mover_cortina = true;
            }
            if c.toggle_muelle.avanzar() {
                repintar_cortina = true;
            }
            if c.cerrando && !c.muelle.animando() && c.muelle.valor() <= 0.01 {
                cerrar_cortina = true;
            }
        }
        if cerrar_cortina {
            self.cortina = None;
        } else {
            if repintar_cortina {
                self.dibujar_cortina();
            }
            if mover_cortina {
                self.actualizar_posicion_cortina();
            }
        }

        let mut mover_ind = false;
        let mut repintar_ind = false;
        let mut cerrar_ind = false;
        if let Some(i) = self.indicador.as_mut() {
            let ax = i.muelle_x.avanzar();
            let ay = i.muelle_y.avanzar();
            let ae = i.muelle_escala.avanzar();
            if ax || ay {
                mover_ind = true;
            }
            if ae {
                repintar_ind = true;
            }
            if i.saliendo
                && !i.muelle_x.animando()
                && !i.muelle_escala.animando()
                && i.muelle_escala.valor() <= 0.02
            {
                cerrar_ind = true;
            }
        }
        if cerrar_ind {
            self.indicador = None;
        } else {
            if mover_ind {
                self.actualizar_posicion_indicador();
            }
            if repintar_ind {
                self.dibujar_indicador();
            }
        }
    }

    fn reaplicar_escala(&mut self) {
        if self.cortina.is_some() {
            let (nw, nh) = self.medidas_cortina();
            if let Some(c) = self.cortina.as_mut()
                && (c.ancho != nw || c.alto != nh)
            {
                c.ancho = nw;
                c.alto = nh;
                c.layer.set_size(nw, nh);
                c.layer.commit();
            }
            self.dibujar_cortina();
            self.actualizar_posicion_cortina();
        }
        if let Some(ind) = self.indicador.as_mut() {
            ind.ultima_escala_pintada = -1.0;
            self.dibujar_indicador();
        }
    }

    pub fn actualizar_pantalla(
        &mut self,
        ancho: f64,
        alto: f64,
        escala: f64,
        oscuro: bool,
        efectos_reducidos: bool,
    ) {
        let ancho = if ancho.is_finite() { ancho } else { 1280.0 };
        let alto = if alto.is_finite() { alto } else { 800.0 };
        self.pantalla_ancho = ancho.max(320.0) as u32;
        self.pantalla_alto = alto.max(320.0) as u32;
        self.escala = normalizar_escala(escala);
        self.oscuro = oscuro;
        self.efectos_reducidos = efectos_reducidos;
        self.paleta = Paleta::cargar(oscuro);
        self.reaplicar_escala();
    }

    pub fn actualizar_notificaciones(&mut self, lista: Vec<NotificacionDto>, no_molestar: bool) {
        self.notificaciones = lista;
        self.no_molestar = no_molestar;
        self.expandidas
            .retain(|id| self.notificaciones.iter().any(|n| n.id == *id));
        if let Some(c) = self.cortina.as_mut()
            && self.notificaciones.is_empty()
        {
            c.scroll_y = 0.0;
            c.scroll_max = 0.0;
        }
        if self.cortina.is_some() {
            let (nw, nh) = self.medidas_cortina();
            if let Some(c) = self.cortina.as_mut()
                && (c.ancho != nw || c.alto != nh)
            {
                c.ancho = nw;
                c.alto = nh;
                c.layer.set_size(nw, nh);
                c.layer.commit();
            }
            self.dibujar_cortina();
            self.actualizar_posicion_cortina();
        }
    }

    pub fn actualizar_gestos_activos(&mut self, activo: bool) {
        self.gestos_activos = activo;
        let red = self.efectos_reducidos;
        if let Some(c) = self.cortina.as_mut() {
            c.toggle_muelle
                .lanzar(if activo { 1.0 } else { 0.0 }, None, M_RAPIDO, red);
            self.dibujar_cortina();
        }
    }

    pub fn cortina_progreso(&mut self, qh: &QueueHandle<Self>, progreso: f32) {
        if self.cortina.is_none() {
            self.crear_cortina(qh, progreso);
        } else if let Some(c) = self.cortina.as_mut() {
            c.cerrando = false;
            c.muelle.fijar(progreso);
            self.actualizar_posicion_cortina();
        }
    }

    pub fn cortina_soltar(
        &mut self,
        qh: &QueueHandle<Self>,
        progreso: f32,
        velocidad: f32,
        cancelada: bool,
    ) {
        if self.cortina.is_none() {
            if cancelada {
                return;
            }
            self.crear_cortina(qh, progreso);
        }
        let red = self.efectos_reducidos;
        if let Some(c) = self.cortina.as_mut() {
            if cancelada {
                c.cerrando = true;
                c.muelle.lanzar(0.0, Some(velocidad), M_RAPIDO, red);
                if red {
                    self.cortina = None;
                    return;
                }
            } else {
                c.cerrando = false;
                let params = if velocidad > 1.4 { M_REBOTE } else { M_SUAVE };
                c.muelle.lanzar(1.0, Some(velocidad), params, red);
                c.layer
                    .set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
                c.layer.commit();
            }
            self.actualizar_posicion_cortina();
        }
    }

    pub fn cerrar_cortina_animada(&mut self, velocidad: f32) {
        let red = self.efectos_reducidos;
        if let Some(c) = self.cortina.as_mut() {
            if c.cerrando {
                return;
            }
            if red {
                self.cortina = None;
                return;
            }
            c.cerrando = true;
            c.muelle.lanzar(0.0, Some(velocidad), M_RAPIDO, false);
            self.actualizar_posicion_cortina();
        }
    }

    fn rescatar_cortina_en_reposo(&mut self) {
        let red = self.efectos_reducidos;
        let mut cerrar = false;
        let mut reabrir = false;
        if let Some(c) = self.cortina.as_mut()
            && !c.muelle.animando()
            && (c.cerrando || (c.muelle.valor() - 1.0).abs() > 0.002)
        {
            if decidir_reposo_cortina(c.muelle.valor(), c.cerrando) {
                c.cerrando = false;
                c.muelle.lanzar(1.0, None, M_SUAVE, red);
                reabrir = true;
            } else {
                cerrar = true;
            }
        }
        if cerrar {
            self.cerrar_cortina_animada(-1.0);
        } else if reabrir {
            self.actualizar_posicion_cortina();
        }
    }

    /// Cápsula circular flotante (64×64) despegada del borde, movida por GPU en X e Y.
    pub fn indicador_flotante(
        &mut self,
        qh: &QueueHandle<Self>,
        lado: Lado,
        y: f64,
        desplazamiento_x: f32,
        progreso: f32,
        armado: bool,
    ) {
        let y_clamped = (y as f32).clamp(48.0, (self.pantalla_alto as f32 - 48.0).max(48.0));
        let escala_obj = (0.55 + progreso * 0.45 + if armado { 0.08 } else { 0.0 }).min(1.10);
        let red = self.efectos_reducidos;

        let recrear = self.indicador.as_ref().is_none_or(|i| i.lado != lado);
        if recrear {
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Overlay,
                Some("bookos-extras-nav"),
                None,
            );
            let ancla = match lado {
                Lado::Izquierda => Anchor::LEFT | Anchor::TOP,
                Lado::Derecha => Anchor::RIGHT | Anchor::TOP,
            };
            layer.set_anchor(ancla);
            layer.set_size(IND_LADO, IND_LADO);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            let top = (y_clamped - IND_LADO as f32 * 0.5).round() as i32;
            let lat = (14.0 + desplazamiento_x).round() as i32;
            match lado {
                Lado::Izquierda => layer.set_margin(top, 0, 0, lat),
                Lado::Derecha => layer.set_margin(top, lat, 0, 0),
            }
            let viewport = self
                .viewporter
                .as_ref()
                .map(|vp| vp.get_viewport(layer.wl_surface(), qh, ()));
            let fractional_scale = self
                .fractional_scale_mgr
                .as_ref()
                .map(|fs| fs.get_fractional_scale(layer.wl_surface(), qh, ()));
            layer.commit();

            let mut muelle_escala = Muelle::nuevo(0.45, M_RAPIDO);
            muelle_escala.lanzar(escala_obj, None, M_RAPIDO, red);

            self.indicador = Some(EstadoIndicador {
                layer,
                viewport,
                fractional_scale,
                lado,
                configurada: false,
                muelle_x: Muelle::nuevo(desplazamiento_x, M_RAPIDO),
                muelle_escala,
                muelle_y: Muelle::nuevo(y_clamped, M_RAPIDO),
                armado,
                saliendo: false,
                ultima_escala_pintada: -1.0,
                ultimo_armado_pintado: !armado,
            });
        } else if let Some(ind) = self.indicador.as_mut() {
            ind.saliendo = false;
            ind.armado = armado;
            ind.muelle_x.fijar(desplazamiento_x);
            ind.muelle_escala
                .lanzar(escala_obj, None, if armado { M_REBOTE } else { M_RAPIDO }, red);
            ind.muelle_y.lanzar(y_clamped, None, M_RAPIDO, red);
            self.actualizar_posicion_indicador();
            self.dibujar_indicador();
        }
    }

    pub fn indicador_soltar(&mut self, disparado: bool) {
        let red = self.efectos_reducidos;
        if let Some(ind) = self.indicador.as_mut() {
            if red {
                self.indicador = None;
                return;
            }
            ind.saliendo = true;
            if disparado {
                let actual_x = ind.muelle_x.valor();
                ind.muelle_x.lanzar(actual_x + 14.0, None, M_RAPIDO, false);
                ind.muelle_escala.lanzar(0.0, Some(-2.6), M_RAPIDO, false);
            } else {
                ind.muelle_x.lanzar(-18.0, None, M_RAPIDO, false);
                ind.muelle_escala.lanzar(0.0, None, M_RAPIDO, false);
            }
            self.actualizar_posicion_indicador();
            self.dibujar_indicador();
        }
    }

    /// Calcula ancho y alto adaptados al contenido (HIG §16: el diseño sirve al contenido).
    fn medidas_cortina(&self) -> (u32, u32) {
        let w = alinear_logico(
            (self.pantalla_ancho as i32 - 48).clamp(360, 680) as u32,
            self.escala,
        )
        .clamp(360, 680);
        let max_h = ((self.pantalla_alto as f32 * 0.78) as u32).clamp(320, 700);
        if self.notificaciones.is_empty() {
            let h = alinear_logico((276 + ALTO_PIE as u32).min(max_h), self.escala).clamp(240, max_h);
            return (w, h);
        }
        let mut h = LISTA_Y + MARGEN_FONDO_LISTA;
        for (i, n) in self.notificaciones.iter().enumerate() {
            if i > 0 {
                h += 12.0;
            }
            h += alto_tarjeta(n, self.expandidas.contains(&n.id), es_espanol());
        }
        let h = alinear_logico((h as u32).clamp(300, max_h), self.escala).clamp(300, max_h);
        (w, h)
    }

    fn crear_cortina(&mut self, qh: &QueueHandle<Self>, progreso_inicial: f32) {
        self.paleta = Paleta::cargar(self.oscuro);
        let (ancho, alto) = self.medidas_cortina();
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("bookos-extras-notificaciones"),
            None,
        );
        layer.set_anchor(Anchor::TOP);
        layer.set_size(ancho, alto);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let y_init = calcular_y_cortina(progreso_inicial, alto);
        layer.set_margin(y_init, 0, 0, 0);
        let viewport = self
            .viewporter
            .as_ref()
            .map(|vp| vp.get_viewport(layer.wl_surface(), qh, ()));
        let fractional_scale = self
            .fractional_scale_mgr
            .as_ref()
            .map(|fs| fs.get_fractional_scale(layer.wl_surface(), qh, ()));
        layer.commit();

        let toggle_init = if self.gestos_activos { 1.0 } else { 0.0 };
        let (_, scroll_max) =
            medidas_lista_notifs(&self.notificaciones, &self.expandidas, alto as f32, es_espanol());
        self.cortina = Some(EstadoCortina {
            layer,
            viewport,
            fractional_scale,
            ancho,
            alto,
            configurada: false,
            muelle: Muelle::nuevo(progreso_inicial, M_SUAVE),
            toggle_muelle: Muelle::nuevo(toggle_init, M_RAPIDO),
            cerrando: false,
            ultimo_margen: y_init,
            scroll_y: 0.0,
            scroll_max,
            botones: Vec::new(),
            toque: None,
        });
    }

    /// Mueve la cortina por GPU sin tocar píxeles (0 ms CPU, 120 Hz reales).
    fn actualizar_posicion_cortina(&mut self) {
        let Some(c) = self.cortina.as_mut() else {
            return;
        };
        let y = calcular_y_cortina(c.muelle.valor(), c.alto);
        if y != c.ultimo_margen {
            c.ultimo_margen = y;
            c.layer.set_margin(y, 0, 0, 0);
            c.layer.commit();
        }
    }

    /// Mueve la cápsula flotante en X e Y por GPU sin re-rasterizar innecesariamente.
    fn actualizar_posicion_indicador(&mut self) {
        let Some(ind) = self.indicador.as_mut() else {
            return;
        };
        let top = (ind.muelle_y.valor() - IND_LADO as f32 * 0.5).round() as i32;
        let lat = (14.0 + ind.muelle_x.valor()).round() as i32;
        match ind.lado {
            Lado::Izquierda => ind.layer.set_margin(top, 0, 0, lat),
            Lado::Derecha => ind.layer.set_margin(top, lat, 0, 0),
        }
        ind.layer.commit();
    }

    fn activar_tarjeta(&mut self, id: u32) {
        let Some(n) = self.notificaciones.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        if n.tiene_default || n.es_captura {
            self.acciones_pendientes
                .push_back(AccionUi::InvocarAccion(id, "default".into()));
            return;
        }
        if es_expandible(&n) {
            if !self.expandidas.remove(&id) {
                self.expandidas.insert(id);
            }
            let (nw, nh) = self.medidas_cortina();
            if let Some(c) = self.cortina.as_mut()
                && (c.ancho != nw || c.alto != nh)
            {
                c.ancho = nw;
                c.alto = nh;
                c.layer.set_size(nw, nh);
                c.layer.commit();
            }
            self.dibujar_cortina();
            self.actualizar_posicion_cortina();
            return;
        }
        if let Some(act) = n.acciones.first() {
            self.acciones_pendientes
                .push_back(AccionUi::InvocarAccion(id, act.clave.clone()));
            return;
        }
        self.acciones_pendientes
            .push_back(AccionUi::CerrarNotificacion(id));
    }

    fn procesar_clic(&mut self, x: f64, y: f64) {
        let Some(cortina) = self.cortina.as_ref() else {
            return;
        };
        let (xf, yf) = (x as f32, y as f32);
        let mut encontrada = None;
        for btn in &cortina.botones {
            if xf >= btn.x && xf <= btn.x + btn.w && yf >= btn.y && yf <= btn.y + btn.h {
                encontrada = Some(btn.accion.clone());
                break;
            }
        }
        let Some(accion) = encontrada else {
            return;
        };
        match accion {
            AccionBoton::AlternarGestos => {
                self.acciones_pendientes.push_back(AccionUi::AlternarGestos);
            }
            AccionBoton::AlternarNoMolestar => {
                self.acciones_pendientes
                    .push_back(AccionUi::AlternarNoMolestar);
            }
            AccionBoton::AbrirAjustes => {
                self.acciones_pendientes
                    .push_back(AccionUi::AbrirAjustesNotificaciones);
            }
            AccionBoton::LimpiarTodo => {
                self.acciones_pendientes
                    .push_back(AccionUi::LimpiarNotificaciones);
            }
            AccionBoton::CerrarCortina => {
                self.cerrar_cortina_animada(-1.8);
            }
            AccionBoton::CerrarNotif(id) => {
                self.acciones_pendientes
                    .push_back(AccionUi::CerrarNotificacion(id));
            }
            AccionBoton::AlternarExpandir(id) => {
                if !self.expandidas.remove(&id) {
                    self.expandidas.insert(id);
                }
                let (nw, nh) = self.medidas_cortina();
                if let Some(c) = self.cortina.as_mut()
                    && (c.ancho != nw || c.alto != nh)
                {
                    c.ancho = nw;
                    c.alto = nh;
                    c.layer.set_size(nw, nh);
                    c.layer.commit();
                }
                self.dibujar_cortina();
                self.actualizar_posicion_cortina();
            }
            AccionBoton::ActivarTarjeta(id) => {
                self.activar_tarjeta(id);
            }
            AccionBoton::Invocar(id, clave) => {
                self.acciones_pendientes
                    .push_back(AccionUi::InvocarAccion(id, clave));
            }
        }
    }

    /// Rasteriza la tarjeta de notificaciones **100 % opaca** (solo al abrir o cambiar datos).
    fn dibujar_cortina(&mut self) {
        let Some(c) = self.cortina.as_ref() else {
            return;
        };
        if !c.configurada || c.ancho == 0 || c.alto == 0 {
            return;
        }
        let ancho = c.ancho;
        let alto = c.alto;
        let (buf_w, buf_h, buffer_scale, sx, sy) =
            tamano_buffer(ancho, alto, self.escala, c.viewport.is_some());
        let scale_f = sy;
        let tx = Transform::from_scale(sx, sy);
        let toggle_p = c.toggle_muelle.valor().clamp(0.0, 1.0);
        let pal = self.paleta;
        let oscuro = self.oscuro;
        let notifs = self.notificaciones.clone();
        let expandidas = self.expandidas.clone();
        let no_molestar = self.no_molestar;
        let espanol = es_espanol();
        let textos = textos_idioma(espanol);

        let panel_w = ancho as f32;
        let panel_h = alto as f32;
        let (contenido_h, scroll_max) =
            medidas_lista_notifs(&notifs, &expandidas, panel_h, espanol);
        let scroll_y = c.scroll_y.clamp(0.0, scroll_max);

        let stride = (buf_w * 4) as i32;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            buf_w as i32,
            buf_h as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        canvas.fill(0);

        let mut botones = Vec::new();

        if let Some(mut pixmap) = PixmapMut::from_bytes(canvas, buf_w, buf_h) {
            // Hoja 100 % opaca (`--bg` sólido del HIG §2, radio 26 §4)
            fill_rounded_rect(&mut pixmap, tx, None, 0.0, 0.0, panel_w, panel_h, 26.0, pal.bg);
            stroke_rounded_rect(
                &mut pixmap,
                tx,
                0.5,
                0.5,
                panel_w - 1.0,
                panel_h - 1.0,
                26.0,
                1.0,
                pal.brd,
            );

            // --- Cabecera (HIG §3: Título página 22/700) ---
            let pad = 22.0;
            let header_y = 18.0;
            let cab = layout_cabecera(panel_w, pad, !notifs.is_empty(), espanol);

            draw_text(
                &mut pixmap,
                &mut self.font_system,
                &mut self.swash_cache,
                textos.titulo,
                pad,
                header_y + if cab.titulo_tam < 20.0 { 11.0 } else { 8.0 },
                cab.titulo_max_w,
                cab.titulo_tam,
                scale_f,
                None,
                pal.tx,
                Weight::BOLD,
                Align::Left,
            );

            if !notifs.is_empty()
                && let Some(bx) = cab.badge_x
            {
                let badge = notifs.len().to_string();
                fill_rounded_rect(&mut pixmap, tx, None, bx, header_y + 10.0, 28.0, 24.0, 12.0, pal.sbg);
                draw_text(
                    &mut pixmap,
                    &mut self.font_system,
                    &mut self.swash_cache,
                    &badge,
                    bx,
                    header_y + 13.0,
                    28.0,
                    12.5,
                    scale_f,
                    None,
                    pal.tx2,
                    Weight::SEMIBOLD,
                    Align::Center,
                );
            }

            // Botón cerrar (zona táctil 44×44, icono vectorial X trazo 2)
            let close_sz = 38.0;
            fill_rounded_rect(
                &mut pixmap,
                tx,
                None,
                cab.cerrar_x,
                header_y + 3.0,
                close_sz,
                close_sz,
                19.0,
                pal.hov,
            );
            draw_icon_x(
                &mut pixmap,
                tx,
                None,
                cab.cerrar_x + close_sz * 0.5,
                header_y + 3.0 + close_sz * 0.5,
                6.0,
                pal.tx,
            );
            botones.push(BotonRect {
                x: cab.cerrar_x - 3.0,
                y: header_y,
                w: 44.0,
                h: 44.0,
                accion: AccionBoton::CerrarCortina,
            });

            if !notifs.is_empty()
                && let Some((bx, bw, etiqueta)) = cab.borrar
            {
                fill_rounded_rect(&mut pixmap, tx, None, bx, header_y + 4.0, bw, 36.0, 12.0, pal.hov);
                draw_text(
                    &mut pixmap,
                    &mut self.font_system,
                    &mut self.swash_cache,
                    etiqueta,
                    bx,
                    header_y + 13.0,
                    bw,
                    13.0,
                    scale_f,
                    None,
                    pal.tx,
                    Weight::SEMIBOLD,
                    Align::Center,
                );
                botones.push(BotonRect {
                    x: bx,
                    y: header_y,
                    w: bw,
                    h: 44.0,
                    accion: AccionBoton::LimpiarTodo,
                });
            }

            // Toggle oficial del HIG §9 (46×28 r14, thumb 22 inset 3)
            let tw = 46.0;
            let th = 28.0;
            let ty = header_y + 8.0;
            let track_col = lerp_color(pal.toff, pal.blue, toggle_p);
            fill_rounded_rect(&mut pixmap, tx, None, cab.toggle_x, ty, tw, th, 14.0, track_col);
            let thumb_x = cab.toggle_x + 3.0 + toggle_p * (tw - 22.0 - 6.0);
            fill_rounded_rect(&mut pixmap, tx, None, thumb_x, ty + 3.0, 22.0, 22.0, 11.0, Color::WHITE);

            if cab.mostrar_etiqueta_gestos {
                let lbl_w = textos.gestos_w;
                draw_text(
                    &mut pixmap,
                    &mut self.font_system,
                    &mut self.swash_cache,
                    textos.gestos,
                    cab.toggle_x - lbl_w - 6.0,
                    header_y + 13.0,
                    lbl_w,
                    13.0,
                    scale_f,
                    None,
                    pal.tx2,
                    Weight::MEDIUM,
                    Align::Right,
                );
                botones.push(BotonRect {
                    x: cab.toggle_x - lbl_w - 8.0,
                    y: header_y,
                    w: tw + lbl_w + 10.0,
                    h: 44.0,
                    accion: AccionBoton::AlternarGestos,
                });
            } else {
                botones.push(BotonRect {
                    x: cab.toggle_x - 4.0,
                    y: header_y,
                    w: tw + 8.0,
                    h: 44.0,
                    accion: AccionBoton::AlternarGestos,
                });
            }

            // Divisor 1 px `--div`
            let div_y = header_y + 54.0;
            fill_rect(&mut pixmap, tx, pad, div_y, panel_w - pad * 2.0, 1.0, pal.div);

            let limite_y = panel_h - MARGEN_FONDO_LISTA;
            let vista_h = (limite_y - LISTA_Y).max(1.0);

            if notifs.is_empty() {
                let cx = panel_w * 0.5;
                let ey = LISTA_Y + 44.0;
                fill_rounded_rect(&mut pixmap, tx, None, cx - 26.0, ey - 26.0, 52.0, 52.0, 26.0, pal.sbg);
                draw_icon_bell(&mut pixmap, tx, cx, ey, pal.tx2);
                draw_text(
                    &mut pixmap,
                    &mut self.font_system,
                    &mut self.swash_cache,
                    textos.sin_notifs,
                    pad,
                    ey + 38.0,
                    panel_w - pad * 2.0,
                    15.0,
                    scale_f,
                    None,
                    pal.tx,
                    Weight::SEMIBOLD,
                    Align::Center,
                );
                draw_text(
                    &mut pixmap,
                    &mut self.font_system,
                    &mut self.swash_cache,
                    textos.aviso_vacio,
                    pad,
                    ey + 62.0,
                    panel_w - pad * 2.0,
                    13.0,
                    scale_f,
                    None,
                    pal.tx2,
                    Weight::NORMAL,
                    Align::Center,
                );
            } else {
                let clip_mask = (scroll_max > 0.0)
                    .then(|| crear_mascara_y(buf_w, buf_h, LISTA_Y * scale_f, vista_h * scale_f))
                    .flatten();
                let clip_y = Some((
                    (LISTA_Y * scale_f).round() as i32,
                    (limite_y * scale_f).round() as i32,
                ));
                let scroll_px = (scroll_y * scale_f).round() / scale_f;
                let mut cy = LISTA_Y - scroll_px;
                for n in &notifs {
                    let expandida = expandidas.contains(&n.id);
                    let tiene_cuerpo = !n.cuerpo.trim().is_empty();
                    let acciones = acciones_visibles(n, expandida, espanol);
                    let tiene_acciones = !acciones.is_empty();
                    let expandible = es_expandible(n);
                    let card_h = alto_tarjeta(n, expandida, espanol);
                    if cy + card_h <= LISTA_Y {
                        cy += card_h + 12.0;
                        continue;
                    }
                    if cy >= limite_y {
                        break;
                    }
                    let card_x = pad;
                    let card_w = panel_w - pad * 2.0;
                    let mask_ref = clip_mask.as_ref();

                    if !oscuro {
                        fill_rounded_rect(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            card_x,
                            cy + 1.5,
                            card_w,
                            card_h,
                            20.0,
                            rgba(0, 0, 0, 12),
                        );
                    }
                    fill_rounded_rect(
                        &mut pixmap,
                        tx,
                        mask_ref,
                        card_x,
                        cy,
                        card_w,
                        card_h,
                        20.0,
                        pal.card,
                    );

                    // Previsualización / icono de la app (48×38 para capturas, 36×36 normal)
                    let av_x = card_x + 16.0;
                    let av_y = cy + 16.0;
                    let (av_w, av_h, av_r) = if n.es_captura && n.imagen.is_some() {
                        (52.0, 38.0, 10.0)
                    } else {
                        (36.0, 36.0, 18.0)
                    };
                    if let Some(img) = n.imagen.as_ref()
                        && dibujar_imagen_notif(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            img,
                            av_x,
                            av_y,
                            av_w,
                            av_h,
                            av_r,
                            pal.sbg,
                        )
                    {
                    } else {
                        let av_bg = if n.critica { pal.red } else { pal.sbg };
                        let av_fg = if n.critica { pal.on_red } else { pal.tx };
                        fill_rounded_rect(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            av_x,
                            av_y,
                            av_w,
                            av_h,
                            av_r,
                            av_bg,
                        );
                        let inicial = n
                            .app
                            .chars()
                            .next()
                            .unwrap_or('B')
                            .to_uppercase()
                            .to_string();
                        draw_text(
                            &mut pixmap,
                            &mut self.font_system,
                            &mut self.swash_cache,
                            &inicial,
                            av_x,
                            av_y + 8.0,
                            av_w,
                            15.0,
                            scale_f,
                            clip_y,
                            av_fg,
                            Weight::SEMIBOLD,
                            Align::Center,
                        );
                    }

                    let tx_x = av_x + av_w + 12.0;
                    let derecha_reservada = if expandible { 92.0 } else { 52.0 };
                    let tx_w = (card_x + card_w - tx_x - derecha_reservada).max(80.0);
                    let meta = if n.es_captura {
                        format!(
                            "{}  ·  {}  ·  {}",
                            n.app,
                            formato_tiempo_idioma(n.segundos, espanol),
                            textos.abrir_captura
                        )
                    } else {
                        format!("{}  ·  {}", n.app, formato_tiempo_idioma(n.segundos, espanol))
                    };
                    draw_text(
                        &mut pixmap,
                        &mut self.font_system,
                        &mut self.swash_cache,
                        &meta,
                        tx_x,
                        cy + 15.0,
                        tx_w,
                        12.0,
                        scale_f,
                        clip_y,
                        if !n.leida || n.es_captura { pal.blue } else { pal.tx2 },
                        Weight::MEDIUM,
                        Align::Left,
                    );

                    // Botón cerrar individual (44×44 táctil)
                    let bx = card_x + card_w - 44.0;
                    let by = cy + 8.0;
                    fill_rounded_rect(
                        &mut pixmap,
                        tx,
                        mask_ref,
                        bx + 7.0,
                        by + 7.0,
                        30.0,
                        30.0,
                        15.0,
                        pal.hov,
                    );
                    draw_icon_x(&mut pixmap, tx, mask_ref, bx + 22.0, by + 22.0, 5.0, pal.tx2);
                    push_boton_recortado(
                        &mut botones,
                        bx,
                        by,
                        44.0,
                        44.0,
                        LISTA_Y,
                        limite_y,
                        AccionBoton::CerrarNotif(n.id),
                    );

                    if expandible {
                        let ex = bx - 38.0;
                        fill_rounded_rect(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            ex + 4.0,
                            by + 7.0,
                            30.0,
                            30.0,
                            15.0,
                            pal.hov,
                        );
                        draw_icon_chevron(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            ex + 19.0,
                            by + 22.0,
                            expandida,
                            pal.tx2,
                        );
                        push_boton_recortado(
                            &mut botones,
                            ex - 3.0,
                            by,
                            40.0,
                            44.0,
                            LISTA_Y,
                            limite_y,
                            AccionBoton::AlternarExpandir(n.id),
                        );
                    }

                    draw_text(
                        &mut pixmap,
                        &mut self.font_system,
                        &mut self.swash_cache,
                        &n.resumen,
                        tx_x,
                        cy + 34.0,
                        tx_w,
                        14.5,
                        scale_f,
                        clip_y,
                        pal.tx,
                        Weight::SEMIBOLD,
                        Align::Left,
                    );

                    let mut iy = cy + 58.0;
                    if tiene_cuerpo {
                        draw_text(
                            &mut pixmap,
                            &mut self.font_system,
                            &mut self.swash_cache,
                            &n.cuerpo,
                            tx_x,
                            iy,
                            card_x + card_w - tx_x - 16.0,
                            13.0,
                            scale_f,
                            clip_y,
                            pal.tx2,
                            Weight::NORMAL,
                            Align::Left,
                        );
                        iy += if expandida { 72.0 } else { 36.0 };
                    }

                    if let Some(p) = n.progreso {
                        let bw = (card_x + card_w - tx_x - 16.0).max(60.0);
                        fill_rounded_rect(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            tx_x,
                            iy + 2.0,
                            bw,
                            4.0,
                            2.0,
                            pal.toff,
                        );
                        fill_rounded_rect(
                            &mut pixmap,
                            tx,
                            mask_ref,
                            tx_x,
                            iy + 2.0,
                            bw * (p as f32 / 100.0),
                            4.0,
                            2.0,
                            pal.blue,
                        );
                        iy += 14.0;
                    }

                    if tiene_acciones {
                        let mut ax = tx_x;
                        for (idx, (clave, etiqueta)) in acciones.iter().take(3).enumerate() {
                            let aw = (etiqueta.chars().count() as f32 * 8.0 + 32.0)
                                .clamp(84.0, 190.0);
                            if ax + aw > card_x + card_w - 16.0 {
                                break;
                            }
                            let (bg_btn, fg_btn) = if idx == 0 {
                                (pal.blue, pal.on_blue)
                            } else {
                                (pal.hov, pal.tx)
                            };
                            fill_rounded_rect(
                                &mut pixmap,
                                tx,
                                mask_ref,
                                ax,
                                iy,
                                aw,
                                36.0,
                                12.0,
                                bg_btn,
                            );
                            draw_text(
                                &mut pixmap,
                                &mut self.font_system,
                                &mut self.swash_cache,
                                etiqueta,
                                ax,
                                iy + 9.0,
                                aw,
                                13.0,
                                scale_f,
                                clip_y,
                                fg_btn,
                                Weight::SEMIBOLD,
                                Align::Center,
                            );
                            push_boton_recortado(
                                &mut botones,
                                ax,
                                iy - 4.0,
                                aw,
                                44.0,
                                LISTA_Y,
                                limite_y,
                                AccionBoton::Invocar(n.id, clave.clone()),
                            );
                            ax += aw + 10.0;
                        }
                    }

                    // Clic en cualquier otra parte de la tarjeta: abrir captura / acción por defecto / expandir
                    push_boton_recortado(
                        &mut botones,
                        card_x,
                        cy,
                        card_w,
                        card_h,
                        LISTA_Y,
                        limite_y,
                        AccionBoton::ActivarTarjeta(n.id),
                    );

                    cy += card_h + 12.0;
                }

                if scroll_max > 0.0 && contenido_h > vista_h {
                    let thumb_h = ((vista_h / contenido_h) * vista_h).clamp(28.0, vista_h);
                    let frac = scroll_y / scroll_max;
                    let thumb_y = LISTA_Y + frac * (vista_h - thumb_h);
                    fill_rounded_rect(
                        &mut pixmap,
                        tx,
                        None,
                        panel_w - 9.0,
                        thumb_y,
                        3.0,
                        thumb_h,
                        1.5,
                        con_alpha(pal.tx2, 0.45),
                    );
                }
            }

            // Pie: «No molestar» + «Ajustes…» (coherente con el widget del panel)
            let pie_y = panel_h - MARGEN_FONDO_LISTA + 6.0;
            fill_rect(&mut pixmap, tx, pad, pie_y, panel_w - pad * 2.0, 1.0, pal.div);
            let dnd_y = pie_y + 11.0;
            let dnd_tw = 46.0;
            let dnd_th = 28.0;
            let dnd_track = if no_molestar { pal.blue } else { pal.toff };
            fill_rounded_rect(&mut pixmap, tx, None, pad, dnd_y, dnd_tw, dnd_th, 14.0, dnd_track);
            let dnd_thumb_x = pad + 3.0 + if no_molestar { dnd_tw - 22.0 - 6.0 } else { 0.0 };
            fill_rounded_rect(&mut pixmap, tx, None, dnd_thumb_x, dnd_y + 3.0, 22.0, 22.0, 11.0, Color::WHITE);
            draw_text(
                &mut pixmap,
                &mut self.font_system,
                &mut self.swash_cache,
                textos.no_molestar,
                pad + dnd_tw + 10.0,
                dnd_y + 5.0,
                160.0,
                13.5,
                scale_f,
                None,
                if no_molestar { pal.tx } else { pal.tx2 },
                Weight::SEMIBOLD,
                Align::Left,
            );
            botones.push(BotonRect {
                x: pad - 4.0,
                y: pie_y + 4.0,
                w: dnd_tw + 140.0,
                h: 44.0,
                accion: AccionBoton::AlternarNoMolestar,
            });

            let aj_w = 104.0;
            let aj_x = panel_w - pad - aj_w;
            fill_rounded_rect(&mut pixmap, tx, None, aj_x, dnd_y - 2.0, aj_w, 32.0, 12.0, pal.hov);
            draw_text(
                &mut pixmap,
                &mut self.font_system,
                &mut self.swash_cache,
                textos.ajustes,
                aj_x,
                dnd_y + 5.0,
                aj_w,
                13.0,
                scale_f,
                None,
                pal.tx,
                Weight::MEDIUM,
                Align::Center,
            );
            botones.push(BotonRect {
                x: aj_x,
                y: pie_y + 4.0,
                w: aj_w,
                h: 44.0,
                accion: AccionBoton::AbrirAjustes,
            });

            // Agarrador inferior
            let grab_w = 44.0;
            fill_rounded_rect(
                &mut pixmap,
                tx,
                None,
                (panel_w - grab_w) * 0.5,
                panel_h - 14.0,
                grab_w,
                4.0,
                2.0,
                pal.div,
            );

            rgba_a_bgra(pixmap.data_mut());
        }

        if let Some(c) = self.cortina.as_mut() {
            c.botones = botones;
            c.scroll_y = scroll_y;
            c.scroll_max = scroll_max;
            let surface = c.layer.wl_surface();
            surface.set_buffer_scale(buffer_scale);
            if let Some(vp) = c.viewport.as_ref() {
                vp.set_destination(ancho as i32, alto as i32);
            }
            buffer.attach_to(surface).ok();
            surface.damage_buffer(0, 0, buf_w as i32, buf_h as i32);
            surface.commit();
        }
    }

    /// Dibuja la cápsula circular flotante (64×64, 16 KB = < 0.05 ms).
    fn dibujar_indicador(&mut self) {
        let Some(ind) = self.indicador.as_ref() else {
            return;
        };
        if !ind.configurada {
            return;
        }
        let escala_vis = ind.muelle_escala.valor().clamp(0.0, 1.15);
        let armado = ind.armado;
        if (escala_vis - ind.ultima_escala_pintada).abs() < 0.015
            && armado == ind.ultimo_armado_pintado
        {
            return;
        }
        let lado = ind.lado;
        let pal = self.paleta;
        let oscuro = self.oscuro;
        let (ind_px, ind_py, buffer_scale, sx, sy) =
            tamano_buffer(IND_LADO, IND_LADO, self.escala, ind.viewport.is_some());
        let tx = Transform::from_scale(sx, sy);

        let stride = (ind_px * 4) as i32;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            ind_px as i32,
            ind_py as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        canvas.fill(0);

        if escala_vis > 0.03
            && let Some(mut pixmap) = PixmapMut::from_bytes(canvas, ind_px, ind_py)
        {
            let cx = IND_LADO as f32 * 0.5;
            let cy = IND_LADO as f32 * 0.5;
            let radio = (22.0 * escala_vis).min(28.0);
            let diam = radio * 2.0;
            let x0 = cx - radio;
            let y0 = cy - radio;

            let alpha_vis = (escala_vis * 1.15).clamp(0.0, 1.0);
            let bg = if armado {
                con_alpha(pal.blue, alpha_vis)
            } else {
                con_alpha(pal.card, alpha_vis)
            };
            let fg = if armado {
                con_alpha(pal.on_blue, alpha_vis)
            } else {
                con_alpha(pal.tx, alpha_vis)
            };

            fill_rounded_rect(
                &mut pixmap,
                tx,
                None,
                x0,
                y0 + 3.0,
                diam,
                diam,
                radio,
                rgba(0, 0, 0, ((if oscuro { 64.0 } else { 36.0 }) * alpha_vis) as u8),
            );
            fill_rounded_rect(&mut pixmap, tx, None, x0, y0, diam, diam, radio, bg);
            if !armado {
                stroke_rounded_rect(
                    &mut pixmap,
                    tx,
                    x0,
                    y0,
                    diam,
                    diam,
                    radio,
                    1.0,
                    con_alpha(pal.brd, alpha_vis),
                );
            }

            let sz = 6.5 * escala_vis;
            let mut pb = PathBuilder::new();
            match lado {
                Lado::Izquierda => {
                    pb.move_to(cx + sz * 0.45, cy - sz);
                    pb.line_to(cx - sz * 0.55, cy);
                    pb.line_to(cx + sz * 0.45, cy + sz);
                }
                Lado::Derecha => {
                    pb.move_to(cx - sz * 0.45, cy - sz);
                    pb.line_to(cx + sz * 0.55, cy);
                    pb.line_to(cx - sz * 0.45, cy + sz);
                }
            }
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(fg);
                paint.anti_alias = true;
                let stroke = Stroke {
                    width: (2.2 * escala_vis).max(1.4),
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, tx, None);
            }

            rgba_a_bgra(pixmap.data_mut());
        }

        if let Some(ind) = self.indicador.as_mut() {
            ind.ultima_escala_pintada = escala_vis;
            ind.ultimo_armado_pintado = armado;
            let surface = ind.layer.wl_surface();
            surface.set_buffer_scale(buffer_scale);
            if let Some(vp) = ind.viewport.as_ref() {
                vp.set_destination(IND_LADO as i32, IND_LADO as i32);
            }
            buffer.attach_to(surface).ok();
            surface.damage_buffer(0, 0, ind_px as i32, ind_py as i32);
            surface.commit();
        }
    }
}

fn calcular_y_cortina(progreso: f32, alto: u32) -> i32 {
    let p = progreso.clamp(0.0, 1.15);
    let y_cerrada = -(alto as f32) - 12.0;
    let y = if p <= 1.0 {
        y_cerrada + (Y_CORTINA_ABIERTA - y_cerrada) * p
    } else {
        Y_CORTINA_ABIERTA + (p - 1.0) * 56.0
    };
    y.round() as i32
}

fn normalizar_escala(escala: f64) -> f64 {
    if escala.is_finite() {
        escala.clamp(0.5, 4.0)
    } else {
        1.0
    }
}

fn paso_logico(escala: f64) -> u32 {
    for d in [1u32, 2, 3, 4, 5, 6, 8, 10, 12] {
        let p = escala * f64::from(d);
        if (p - p.round()).abs() < 1e-4 {
            return d;
        }
    }
    1
}

fn alinear_logico(v: u32, escala: f64) -> u32 {
    let paso = paso_logico(escala);
    if paso <= 1 {
        return v.max(1);
    }
    let redondeado = ((v + paso / 2) / paso) * paso;
    redondeado.max(paso)
}

fn tamano_buffer(ancho: u32, alto: u32, escala: f64, con_viewport: bool) -> (u32, u32, i32, f32, f32) {
    if con_viewport {
        let buf_w = (f64::from(ancho) * escala).round().max(1.0) as u32;
        let buf_h = (f64::from(alto) * escala).round().max(1.0) as u32;
        let sx = buf_w as f32 / ancho.max(1) as f32;
        let sy = buf_h as f32 / alto.max(1) as f32;
        (buf_w, buf_h, 1, sx, sy)
    } else {
        let s = escala.round().clamp(1.0, 4.0) as i32;
        let su = s as u32;
        let sf = s as f32;
        (ancho * su, alto * su, s, sf, sf)
    }
}

struct TextosCortina {
    titulo: &'static str,
    titulo_min_ancho: f32,
    titulo_min_estrecho: f32,
    gestos: &'static str,
    gestos_w: f32,
    borrar: &'static str,
    borrar_todo: &'static str,
    borrar_ancho: f32,
    sin_notifs: &'static str,
    aviso_vacio: &'static str,
    no_molestar: &'static str,
    ajustes: &'static str,
    abrir: &'static str,
    responder: &'static str,
    abrir_captura: &'static str,
}

fn es_espanol() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .is_some_and(|l| l.starts_with("es"))
}

fn textos_idioma(espanol: bool) -> TextosCortina {
    if espanol {
        TextosCortina {
            titulo: "Notificaciones",
            titulo_min_ancho: 166.0,
            titulo_min_estrecho: 136.0,
            gestos: "Gestos",
            gestos_w: 54.0,
            borrar: "Borrar",
            borrar_todo: "Borrar todo",
            borrar_ancho: 104.0,
            sin_notifs: "No hay notificaciones",
            aviso_vacio: "Los avisos nuevos aparecerán aquí",
            no_molestar: "No molestar",
            ajustes: "Ajustes…",
            abrir: "Abrir",
            responder: "Responder",
            abrir_captura: "Tocar para abrir",
        }
    } else {
        TextosCortina {
            titulo: "Notifications",
            titulo_min_ancho: 156.0,
            titulo_min_estrecho: 128.0,
            gestos: "Gestures",
            gestos_w: 62.0,
            borrar: "Clear",
            borrar_todo: "Clear all",
            borrar_ancho: 88.0,
            sin_notifs: "No notifications",
            aviso_vacio: "New alerts will appear here",
            no_molestar: "Do Not Disturb",
            ajustes: "Settings…",
            abrir: "Open",
            responder: "Reply",
            abrir_captura: "Tap to open",
        }
    }
}

fn es_expandible(n: &NotificacionDto) -> bool {
    n.cuerpo.chars().count() > 72 || n.cuerpo.contains('\n') || n.tiene_default || n.es_captura
}

fn acciones_visibles(n: &NotificacionDto, expandida: bool, espanol: bool) -> Vec<(String, String)> {
    let txt = textos_idioma(espanol);
    let mut out = Vec::new();
    for a in &n.acciones {
        if a.clave == "default" {
            if expandida {
                let et = if a.etiqueta.trim().is_empty() {
                    txt.abrir.to_string()
                } else {
                    a.etiqueta.clone()
                };
                out.push((a.clave.clone(), et));
            }
            continue;
        }
        let et = if a.clave == "inline-reply" && a.etiqueta.trim().is_empty() {
            txt.responder.to_string()
        } else {
            a.etiqueta.clone()
        };
        out.push((a.clave.clone(), et));
    }
    out
}

fn alto_tarjeta(n: &NotificacionDto, expandida: bool, espanol: bool) -> f32 {
    let tiene_cuerpo = !n.cuerpo.trim().is_empty();
    let tiene_acciones = !acciones_visibles(n, expandida, espanol).is_empty();
    70.0 + if tiene_cuerpo {
        if expandida { 72.0 } else { 38.0 }
    } else {
        0.0
    } + if n.progreso.is_some() { 14.0 } else { 0.0 }
        + if tiene_acciones { 46.0 } else { 0.0 }
}

fn medidas_lista_notifs(
    notifs: &[NotificacionDto],
    expandidas: &HashSet<u32>,
    panel_h: f32,
    espanol: bool,
) -> (f32, f32) {
    if notifs.is_empty() {
        return (0.0, 0.0);
    }
    let mut contenido_h = 0.0f32;
    for (i, n) in notifs.iter().enumerate() {
        if i > 0 {
            contenido_h += 12.0;
        }
        contenido_h += alto_tarjeta(n, expandidas.contains(&n.id), espanol);
    }
    let vista_h = (panel_h - MARGEN_FONDO_LISTA - LISTA_Y).max(1.0);
    let scroll_max = (contenido_h - vista_h).max(0.0);
    (contenido_h, scroll_max)
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.chunks_exact(4) {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let d = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };
        let n = (a << 18) | (b << 12) | (c << 6) | d;
        out.push(((n >> 16) & 0xFF) as u8);
        if chunk[2] != b'=' {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if chunk[3] != b'=' {
            out.push((n & 0xFF) as u8);
        }
    }
    Some(out)
}

#[allow(clippy::too_many_arguments)]
fn dibujar_imagen_notif(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    clip_mask: Option<&Mask>,
    img: &crate::protocolo::ImagenNotifDto,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    bg: Color,
) -> bool {
    let Some(mut rgba) = decode_base64(&img.rgba_b64) else {
        return false;
    };
    let expected = (img.ancho as usize)
        .saturating_mul(img.alto as usize)
        .saturating_mul(4);
    if img.ancho == 0 || img.alto == 0 || rgba.len() != expected {
        return false;
    }
    // tiny_skia espera RGBA premultiplicado
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u16;
        if a < 255 {
            px[0] = ((px[0] as u16 * a) / 255) as u8;
            px[1] = ((px[1] as u16 * a) / 255) as u8;
            px[2] = ((px[2] as u16 * a) / 255) as u8;
        }
    }
    let Some(src) = tiny_skia::Pixmap::from_vec(
        rgba,
        tiny_skia::IntSize::from_wh(img.ancho, img.alto).unwrap(),
    ) else {
        return false;
    };
    fill_rounded_rect(pixmap, tx, clip_mask, x, y, w, h, radius, bg);
    let sx = w / img.ancho as f32;
    let sy = h / img.alto as f32;
    let img_tx = tx.pre_translate(x, y).pre_scale(sx, sy);
    pixmap.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        },
        img_tx,
        clip_mask,
    );
    true
}

fn draw_icon_chevron(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    mask: Option<&Mask>,
    cx: f32,
    cy: f32,
    abierta: bool,
    color: Color,
) {
    let mut pb = PathBuilder::new();
    let dy = if abierta { 2.5 } else { -2.5 };
    pb.move_to(cx - 4.5, cy + dy);
    pb.line_to(cx, cy - dy);
    pb.line_to(cx + 4.5, cy + dy);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.8,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, tx, mask);
    }
}

fn crear_mascara_y(buf_w: u32, buf_h: u32, top_px: f32, height_px: f32) -> Option<Mask> {
    let mut mask = Mask::new(buf_w, buf_h)?;
    let rect = Rect::from_xywh(0.0, top_px, buf_w as f32, height_px.max(1.0))?;
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let path = pb.finish()?;
    mask.fill_path(&path, FillRule::Winding, false, Transform::identity());
    Some(mask)
}

#[allow(clippy::too_many_arguments)]
fn push_boton_recortado(
    botones: &mut Vec<BotonRect>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    min_y: f32,
    max_y: f32,
    accion: AccionBoton,
) {
    let top = y.max(min_y);
    let bottom = (y + h).min(max_y);
    if bottom > top + 4.0 {
        botones.push(BotonRect {
            x,
            y: top,
            w,
            h: bottom - top,
            accion,
        });
    }
}

struct CabeceraLayout {
    titulo_max_w: f32,
    titulo_tam: f32,
    badge_x: Option<f32>,
    cerrar_x: f32,
    borrar: Option<(f32, f32, &'static str)>,
    toggle_x: f32,
    mostrar_etiqueta_gestos: bool,
}

fn layout_cabecera(
    panel_w: f32,
    pad: f32,
    con_notifs: bool,
    espanol: bool,
) -> CabeceraLayout {
    let txt = textos_idioma(espanol);
    let mut rx = panel_w - pad;
    let cerrar_x = rx - 38.0;
    rx = cerrar_x - 10.0;

    let borrar = if con_notifs {
        let (bw, etiqueta) = if panel_w < 460.0 {
            (74.0, txt.borrar)
        } else {
            (txt.borrar_ancho, txt.borrar_todo)
        };
        let bx = rx - bw;
        rx = bx - 12.0;
        Some((bx, bw, etiqueta))
    } else {
        None
    };

    let toggle_x = rx - 46.0;
    let titulo_tam = if panel_w < 440.0 { 18.0 } else { 22.0 };
    let ancho_titulo_min = if titulo_tam < 20.0 {
        txt.titulo_min_estrecho
    } else {
        txt.titulo_min_ancho
    };
    let reserva_etiqueta = txt.gestos_w + 6.0;
    let mostrar_etiqueta_gestos = toggle_x - reserva_etiqueta >= pad + ancho_titulo_min + 12.0;
    let controles_izq_x = if mostrar_etiqueta_gestos {
        toggle_x - reserva_etiqueta
    } else {
        toggle_x - 8.0
    };
    let titulo_max_w = (controles_izq_x - pad - 8.0).max(80.0);
    let badge_base = pad + if titulo_tam < 20.0 { 142.0 } else { 174.0 };
    let badge_x = (con_notifs && badge_base + 28.0 + 8.0 <= controles_izq_x).then_some(badge_base);

    CabeceraLayout {
        titulo_max_w,
        titulo_tam,
        badge_x,
        cerrar_x,
        borrar,
        toggle_x,
        mostrar_etiqueta_gestos,
    }
}

fn desplazamiento_cortina_pantalla(
    pos_y_local: f64,
    ultimo_margen: i32,
    origen_pantalla_y: f64,
) -> f64 {
    (pos_y_local + f64::from(ultimo_margen)) - origen_pantalla_y
}

// --- Primitivas vectoriales y tipografía -----------------------------------

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn con_alpha(c: Color, a: f32) -> Color {
    Color::from_rgba(c.red(), c.green(), c.blue(), (c.alpha() * a).clamp(0.0, 1.0)).unwrap_or(c)
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(
        a.red() + (b.red() - a.red()) * t,
        a.green() + (b.green() - a.green()) * t,
        a.blue() + (b.blue() - a.blue()) * t,
        a.alpha() + (b.alpha() - a.alpha()) * t,
    )
    .unwrap_or(b)
}

fn rgba_a_bgra(buf: &mut [u8]) {
    for px in buf.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
}

fn fill_rect(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
) {
    let Some(rect) = Rect::from_xywh(x, y, w.max(0.0), h.max(0.0)) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_rect(rect, &paint, tx, None);
}

fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[allow(clippy::too_many_arguments)]
fn fill_rounded_rect(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    mask: Option<&Mask>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    color: Color,
) {
    let Some(path) = rounded_rect_path(x, y, w, h, r) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, FillRule::Winding, tx, mask);
}

#[allow(clippy::too_many_arguments)]
fn stroke_rounded_rect(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    width: f32,
    color: Color,
) {
    let Some(path) = rounded_rect_path(x, y, w, h, r) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, tx, None);
}

fn draw_icon_x(
    pixmap: &mut PixmapMut<'_>,
    tx: Transform,
    mask: Option<&Mask>,
    cx: f32,
    cy: f32,
    r: f32,
    color: Color,
) {
    let mut pb = PathBuilder::new();
    pb.move_to(cx - r, cy - r);
    pb.line_to(cx + r, cy + r);
    pb.move_to(cx + r, cy - r);
    pb.line_to(cx - r, cy + r);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 2.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, tx, mask);
    }
}

fn draw_icon_bell(pixmap: &mut PixmapMut<'_>, tx: Transform, cx: f32, cy: f32, color: Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(cx - 8.0, cy + 5.0);
    pb.line_to(cx + 8.0, cy + 5.0);
    pb.line_to(cx + 6.5, cy + 2.0);
    pb.line_to(cx + 6.5, cy - 2.0);
    pb.cubic_to(cx + 6.5, cy - 7.5, cx - 6.5, cy - 7.5, cx - 6.5, cy - 2.0);
    pb.line_to(cx - 6.5, cy + 2.0);
    pb.close();
    pb.move_to(cx - 2.5, cy + 8.5);
    pb.cubic_to(cx - 1.0, cy + 10.2, cx + 1.0, cy + 10.2, cx + 2.5, cy + 8.5);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 2.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, tx, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut PixmapMut<'_>,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    text: &str,
    x: f32,
    y: f32,
    max_w: f32,
    size: f32,
    scale: f32,
    clip_y: Option<(i32, i32)>,
    color: Color,
    weight: Weight,
    align: Align,
) {
    let metrics = Metrics::new(size * scale, size * 1.4 * scale);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((max_w * scale).max(20.0)),
        Some(size * 3.2 * scale),
    );
    let attrs = Attrs::new().family(Family::SansSerif).weight(weight);
    buffer.set_text(font_system, text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(font_system, false);

    let tc = TextColor::rgba(
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8,
        (color.alpha() * 255.0).round() as u8,
    );
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let (min_py, max_py) = clip_y.unwrap_or((0, ph));
    let min_py = min_py.max(0);
    let max_py = max_py.min(ph);
    let ox = (x * scale).round() as i32;
    let oy = (y * scale).round() as i32;
    let data = pixmap.data_mut();

    buffer.draw(font_system, swash_cache, tc, |gx, gy, w, h, c| {
        if c.a() == 0 || w == 0 || h == 0 {
            return;
        }
        let px = ox + gx;
        let py = oy + gy;
        if px < 0 || py < min_py || px >= pw || py >= max_py {
            return;
        }
        let sa = c.a() as u32;
        let inv = 255 - sa;
        let idx = ((py * pw + px) * 4) as usize;
        let sr = (c.r() as u32 * sa + 127) / 255;
        let sg = (c.g() as u32 * sa + 127) / 255;
        let sb = (c.b() as u32 * sa + 127) / 255;
        data[idx] = (sr + (data[idx] as u32 * inv + 127) / 255).min(255) as u8;
        data[idx + 1] = (sg + (data[idx + 1] as u32 * inv + 127) / 255).min(255) as u8;
        data[idx + 2] = (sb + (data[idx + 2] as u32 * inv + 127) / 255).min(255) as u8;
        data[idx + 3] = (sa + (data[idx + 3] as u32 * inv + 127) / 255).min(255) as u8;
    });
}

fn formato_tiempo_idioma(segundos: u64, espanol: bool) -> String {
    if espanol {
        if segundos < 60 {
            "ahora".into()
        } else if segundos < 3600 {
            format!("hace {} min", segundos / 60)
        } else {
            format!("hace {} h", segundos / 3600)
        }
    } else if segundos < 60 {
        "now".into()
    } else if segundos < 3600 {
        format!("{}m ago", segundos / 60)
    } else {
        format!("{}h ago", segundos / 3600)
    }
}

// --- Delegados smithay-client-toolkit --------------------------------------

impl CompositorHandler for UiState {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for UiState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for UiState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.cortina.as_ref().is_some_and(|c| c.layer == *layer) {
            self.cortina = None;
        }
        if self.indicador.as_ref().is_some_and(|i| i.layer == *layer) {
            self.indicador = None;
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(c) = self.cortina.as_mut().filter(|c| c.layer == *layer) {
            if configure.new_size.0 > 0 {
                c.ancho = configure.new_size.0;
            }
            if configure.new_size.1 > 0 {
                c.alto = configure.new_size.1;
            }
            c.configurada = true;
            self.dibujar_cortina();
            return;
        }
        if let Some(i) = self.indicador.as_mut().filter(|i| i.layer == *layer) {
            i.configurada = true;
            self.dibujar_indicador();
        }
    }
}

impl SeatHandler for UiState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seat_state.get_touch(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
        if capability == Capability::Touch
            && let Some(t) = self.touch.take()
        {
            t.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for UiState {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if event.keysym == Keysym::Escape {
            self.cerrar_cortina_animada(-1.8);
        }
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: Modifiers, _: RawModifiers, _: u32) {}
}

impl PointerHandler for UiState {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let mut scroll_delta = 0.0f32;
        for ev in events {
            match ev.kind {
                PointerEventKind::Release { button: 0x110, .. } => {
                    self.procesar_clic(ev.position.0, ev.position.1);
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let dy = if vertical.discrete != 0 {
                        vertical.discrete as f32 * 40.0
                    } else {
                        vertical.absolute as f32
                    };
                    scroll_delta += dy;
                }
                _ => {}
            }
        }
        if scroll_delta.abs() > 0.05
            && let Some(c) = self.cortina.as_mut()
            && c.scroll_max > 0.0
        {
            let prev = c.scroll_y;
            c.scroll_y = (c.scroll_y + scroll_delta).clamp(0.0, c.scroll_max);
            if (c.scroll_y - prev).abs() >= 0.5 {
                self.dibujar_cortina();
            }
        }
    }
}

impl TouchHandler for UiState {
    fn down(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        time: u32,
        _: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if !position.0.is_finite() || !position.1.is_finite() {
            return;
        }
        if let Some(c) = self.cortina.as_mut() {
            if c.toque.is_some() {
                return;
            }
            let p_actual = c.muelle.valor();
            c.muelle.fijar(p_actual);
            let mut velocidad = VentanaVelocidad::default();
            velocidad.reiniciar(time, p_actual);
            let en_zona_scroll = c.scroll_max > 0.0
                && position.1 >= f64::from(LISTA_Y)
                && position.1 < f64::from(c.alto as f32 - MARGEN_FONDO_LISTA - 6.0);
            c.toque = Some(ArrastreCortina {
                id,
                origen_x: position.0,
                origen_superficie_y: position.1,
                origen_pantalla_y: position.1 + f64::from(c.ultimo_margen),
                progreso_inicial: p_actual,
                modo_scroll: en_zona_scroll,
                scroll_inicial: c.scroll_y,
                arrastrando: false,
                velocidad,
            });
        }
    }

    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if !position.0.is_finite() || !position.1.is_finite() {
            return;
        }
        let mut mover = false;
        let mut repintar_scroll = false;
        if let Some(c) = self.cortina.as_mut()
            && let Some(t) = c.toque.as_mut().filter(|t| t.id == id)
        {
            let dx = position.0 - t.origen_x;
            let dy = desplazamiento_cortina_pantalla(
                position.1,
                c.ultimo_margen,
                t.origen_pantalla_y,
            );
            if !t.arrastrando && dx.hypot(dy) > 10.0 && dy.abs() >= dx.abs() {
                t.arrastrando = true;
            }
            if t.arrastrando {
                if t.modo_scroll {
                    let prev = c.scroll_y;
                    c.scroll_y = (t.scroll_inicial - dy as f32).clamp(0.0, c.scroll_max);
                    if (c.scroll_y - prev).abs() >= 1.0 {
                        repintar_scroll = true;
                    }
                } else {
                    let raw = t.progreso_inicial + (dy as f32 / (c.alto as f32).max(240.0));
                    let p = if raw <= 1.0 {
                        raw.max(0.0)
                    } else {
                        1.0 + goma(raw - 1.0, 0.12)
                    };
                    t.velocidad.anotar(time, raw);
                    c.muelle.fijar(p);
                    mover = true;
                }
            }
        }
        if repintar_scroll {
            self.dibujar_cortina();
        }
        if mover {
            self.actualizar_posicion_cortina();
        }
    }

    fn up(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        time: u32,
        id: i32,
    ) {
        let Some(t) = self.cortina.as_mut().and_then(|c| {
            if c.toque.as_ref().is_some_and(|t| t.id == id) {
                c.toque.take()
            } else {
                None
            }
        }) else {
            return;
        };

        if !t.arrastrando {
            self.procesar_clic(t.origen_x, t.origen_superficie_y);
            self.rescatar_cortina_en_reposo();
            return;
        }
        if t.modo_scroll {
            self.dibujar_cortina();
            self.rescatar_cortina_en_reposo();
            return;
        }
        let mut t = t;
        let vel = t.velocidad.al_soltar(time);
        let pos = self.cortina.as_ref().map_or(1.0, |c| c.muelle.valor());
        let proyectado = proyectar_impulso(pos, vel);
        if proyectado < 0.65 {
            self.cerrar_cortina_animada(vel.min(-1.0));
        } else {
            let red = self.efectos_reducidos;
            if let Some(c) = self.cortina.as_mut() {
                c.cerrando = false;
                c.muelle.lanzar(1.0, Some(vel), M_SUAVE, red);
            }
            self.actualizar_posicion_cortina();
        }
    }

    fn shape(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64, _: f64) {}
    fn orientation(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64) {}
    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {
        let red = self.efectos_reducidos;
        let mut cerrar = false;
        if let Some(c) = self.cortina.as_mut()
            && c.toque.take().is_some()
        {
            if c.cerrando || c.muelle.valor() < 0.65 {
                cerrar = true;
            } else {
                c.cerrando = false;
                c.muelle.lanzar(1.0, Some(0.0), M_SUAVE, red);
            }
        }
        if cerrar {
            self.cerrar_cortina_animada(-1.0);
        } else {
            self.actualizar_posicion_cortina();
        }
    }
}

impl ShmHandler for UiState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for UiState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(UiState);
delegate_output!(UiState);
delegate_shm!(UiState);
delegate_seat!(UiState);
delegate_keyboard!(UiState);
delegate_pointer!(UiState);
delegate_touch!(UiState);
delegate_layer!(UiState);
delegate_registry!(UiState);
delegate_noop!(UiState: ignore WpViewporter);
delegate_noop!(UiState: ignore WpViewport);
delegate_noop!(UiState: ignore WpFractionalScaleManagerV1);

impl Dispatch<WpFractionalScaleV1, ()> for UiState {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let nueva = normalizar_escala(f64::from(scale) / 120.0);
            if (nueva - state.escala).abs() > 1e-4 {
                state.escala = nueva;
                state.reaplicar_escala();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desplazamiento_cortina_compensa_margen_sin_oscilar() {
        let origen_pantalla = 200.0 + 44.0;
        // Dedo sube 80 px en pantalla; la superficie se mueve -80 px en margen (-36),
        // por lo que la coordenada local vista por la superficie sigue en 200.0.
        let dy = desplazamiento_cortina_pantalla(200.0, -36, origen_pantalla);
        assert!((dy + 80.0).abs() < 1e-6);
    }

    #[test]
    fn cabecera_estrecha_no_solapa_titulo_ni_controles() {
        for espanol in [true, false] {
            for con_notifs in [false, true] {
                for ancho in [360.0, 440.0, 520.0, 680.0] {
                    let cab = layout_cabecera(ancho, 22.0, con_notifs, espanol);
                    let txt = textos_idioma(espanol);
                    let izq_fin = 22.0 + cab.titulo_max_w;
                    let ctrl_inicio = if cab.mostrar_etiqueta_gestos {
                        cab.toggle_x - (txt.gestos_w + 6.0)
                    } else {
                        cab.toggle_x
                    };
                    assert!(
                        izq_fin <= ctrl_inicio,
                        "solape en es={espanol}, ancho={ancho}, con_notifs={con_notifs}: {izq_fin} > {ctrl_inicio}"
                    );
                    if let Some((bx, bw, _)) = cab.borrar {
                        assert!(cab.toggle_x + 46.0 <= bx);
                        assert!(bx + bw <= cab.cerrar_x);
                    } else {
                        assert!(cab.toggle_x + 46.0 <= cab.cerrar_x);
                    }
                }
            }
        }
    }

    #[test]
    fn escala_hidpi_normaliza_valores_no_finitos_y_extremos() {
        assert_eq!(normalizar_escala(f64::NAN), 1.0);
        assert_eq!(normalizar_escala(0.25), 0.5);
        assert_eq!(normalizar_escala(1.75), 1.75);
        assert_eq!(normalizar_escala(8.0), 4.0);
    }

    #[test]
    fn alineado_logico_evita_estiramiento_durante_deslizamiento_a_175() {
        assert_eq!(paso_logico(1.75), 4);
        assert_eq!(paso_logico(1.5), 2);
        assert_eq!(paso_logico(2.0), 1);

        let alto = alinear_logico(457, 1.75);
        assert_eq!(alto % 4, 0);
        let (_, buf_h, buffer_scale, sx, sy) = tamano_buffer(520, alto, 1.75, true);
        assert_eq!(buffer_scale, 1);
        assert!((sx - 1.75).abs() < 1e-5);
        assert!((sy - 1.75).abs() < 1e-5);

        for y in -500..=60 {
            let loc = (f64::from(y) * 1.75).round();
            let phys_h = (f64::from(alto) * 1.75 + loc).round() - loc;
            assert_eq!(phys_h as u32, buf_h);
        }
    }

    #[test]
    fn medidas_lista_calcula_scroll_y_recorte_de_botones() {
        let sample = |id| NotificacionDto {
            id,
            app: "App".into(),
            resumen: "Resumen".into(),
            cuerpo: "Cuerpo".into(),
            critica: false,
            leida: false,
            progreso: None,
            segundos: 10,
            acciones: Vec::new(),
            tiene_default: false,
            es_captura: false,
            imagen: None,
        };
        let vacio = medidas_lista_notifs(&[], &HashSet::new(), 400.0, true);
        assert_eq!(vacio, (0.0, 0.0));
        let varias: Vec<_> = (0..6).map(sample).collect();
        let (cont, max_s) = medidas_lista_notifs(&varias, &HashSet::new(), 400.0, true);
        assert!(cont > 400.0);
        assert!(max_s > 0.0);

        let mut botones = Vec::new();
        push_boton_recortado(&mut botones, 10.0, 40.0, 50.0, 30.0, LISTA_Y, 374.0, AccionBoton::CerrarCortina);
        assert!(botones.is_empty());
        push_boton_recortado(&mut botones, 10.0, 75.0, 50.0, 40.0, LISTA_Y, 374.0, AccionBoton::CerrarCortina);
        assert_eq!(botones.len(), 1);
        assert!((botones[0].y - LISTA_Y).abs() < 1e-4);
    }

    #[test]
    fn textos_en_y_es_difieren_con_coherencia() {
        assert_eq!(textos_idioma(true).titulo, "Notificaciones");
        assert_eq!(textos_idioma(false).titulo, "Notifications");
        assert_eq!(formato_tiempo_idioma(30, true), "ahora");
        assert_eq!(formato_tiempo_idioma(30, false), "now");
    }

    #[test]
    fn decidir_reposo_cortina_resuelve_apertura_y_cierre() {
        assert!(decidir_reposo_cortina(0.4, false));
        assert!(decidir_reposo_cortina(0.6, true));
        assert!(!decidir_reposo_cortina(0.3, true));
    }

    #[test]
    fn base64_y_acciones_visibles_funcionan_para_capturas_y_default() {
        assert_eq!(decode_base64("SG9sYQ=="), Some(b"Hola".to_vec()));
        assert_eq!(decode_base64("abc"), None);

        let captura = NotificacionDto {
            id: 42,
            app: "BookOS".into(),
            resumen: "Captura guardada".into(),
            cuerpo: "captura.png".into(),
            critica: false,
            leida: false,
            progreso: None,
            segundos: 1,
            acciones: vec![
                crate::protocolo::AccionNotifDto {
                    clave: "default".into(),
                    etiqueta: String::new(),
                },
                crate::protocolo::AccionNotifDto {
                    clave: "inline-reply".into(),
                    etiqueta: String::new(),
                },
            ],
            tiene_default: true,
            es_captura: true,
            imagen: None,
        };
        assert!(es_expandible(&captura));
        let colapsada = acciones_visibles(&captura, false, true);
        assert_eq!(colapsada.len(), 1);
        assert_eq!(colapsada[0], ("inline-reply".into(), "Responder".into()));

        let expandida = acciones_visibles(&captura, true, true);
        assert_eq!(expandida.len(), 2);
        assert_eq!(expandida[0], ("default".into(), "Abrir".into()));
    }
}
