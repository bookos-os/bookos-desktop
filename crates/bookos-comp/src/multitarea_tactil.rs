//! Vista de aplicaciones recientes de la pantalla táctil. Sus tarjetas son
//! superficies Wayland vivas, agrupadas según la disposición del escritorio.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use bookos_shell::tema::{self, M_RAPIDO, M_SUAVE, Muelle, proyectar_impulso};
use smithay::backend::input::TouchSlot;
use smithay::backend::renderer::element::{Id, Kind, solid::SolidColorRenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::input::keyboard::keysyms;
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Point, Rectangle};

use crate::cursor::OverlayElement;
use crate::gestos::{Agarre, Velocidad};
use crate::shell::EtiquetaTactil;
use crate::state::BookosComp;
use crate::ventanas::Zona;

type Punto = Point<f64, Logical>;
type Rect = Rectangle<f64, Logical>;

const UMBRAL: f64 = 10.0;
const TOQUE: i32 = 44;
const AIRE: i32 = 8;
const PIE: usize = 2;

struct Tarjeta {
    foco: Window,
    nodo: accesskit::NodeId,
    ventanas: Vec<(Window, Rect)>,
    origen: Rect,
    rotulo: EtiquetaTactil,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Objetivo {
    Tarjeta(usize),
    Rotulo(usize),
    Escritorio,
    CerrarTodo,
    Fondo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gesto {
    Toque,
    Carrusel,
    Cerrar,
    Colocar,
    Cancelado,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colocacion {
    Dividir(Zona),
    Flotante,
}

struct AgarreTarjeta {
    slot: Option<TouchSlot>,
    origen: Punto,
    objetivo: Objetivo,
    gesto: Gesto,
    carrusel: Agarre,
    velocidad_x: Velocidad,
    velocidad_y: Velocidad,
}

struct Borde {
    origen: Punto,
    inicial: Rect,
    avance: f32,
    destino_anterior: f32,
    desplazamiento_x: f32,
    escritorio: bool,
    velocidad: Velocidad,
}

pub struct Vista {
    salida: Output,
    pantalla: Rect,
    tarjetas: Vec<Tarjeta>,
    activa: usize,
    desde_escritorio: bool,
    abrir_elegida: bool,
    foco_pie: Option<usize>,
    // El aro de foco solo orienta a quien navega con teclado; al dedo le
    // estorbaba encima del rótulo de la tarjeta central.
    teclado: bool,
    entrada: Muelle,
    x: Muelle,
    y: Muelle,
    carrusel: Muelle,
    escritorio: Muelle,
    borde: Option<Borde>,
    agarre: Option<AgarreTarjeta>,
    arrastrada: Option<usize>,
    arrastre_x: Muelle,
    arrastre_y: Muelle,
    colocar: Option<Colocacion>,
    pie: [EtiquetaTactil; PIE],
    dividir: EtiquetaTactil,
    flotante: EtiquetaTactil,
    velo: Id,
    indicador: Id,
}

fn emergente_permitido(nombre: Option<&str>) -> bool {
    !matches!(
        nombre,
        Some("polkit" | "acceso" | "elegir-app" | "compartir")
    )
}

fn habilitada(state: &BookosComp) -> bool {
    !state.modo.kiosko()
        && !state.cuentagotas
        && !state.salvapantallas.activo()
        && state.shell.as_ref().is_some_and(|s| {
            !s.esta_bloqueado()
                && !s.hay_captura()
                && emergente_permitido(s.emergente_nombre())
        })
}

fn vista_nueva(state: &mut BookosComp, salida: Output, pantalla: Rect) -> Option<Vista> {
    let actual = state.ventana_con_foco();
    let nombre_salida = salida.name();
    let ventanas: Vec<_> = state
        .space
        .elements()
        .rev()
        .filter(|w| w.alive() && !w.x11_surface().is_some_and(|x| x.is_override_redirect()))
        .filter(|w| !state.minimizando.iter().any(|(m, _)| m == *w))
        .filter(|w| crate::multitarea::visible(state, w))
        .filter(|w| crate::escritorios::salida_de(state, w).as_deref() == Some(&nombre_salida))
        .filter_map(|w| Some((w.clone(), state.space.element_geometry(w)?.to_f64())))
        .chain(
            state
                .escritorios
                .escondidas_en(&nombre_salida)
                .iter()
                .rev()
                .filter(|(w, _)| w.alive() && crate::multitarea::visible(state, w))
                .map(|(w, p)| (w.clone(), Rectangle::new(*p, w.geometry().size).to_f64())),
        )
        .collect();
    let mut orden: Vec<_> = ventanas.iter().map(|(w, _)| w).collect();
    orden.sort_by_key(|w| {
        std::cmp::Reverse((actual.as_ref() == Some(*w), crate::ventanas::enfocada_en(w)))
    });
    let mut tarjetas: Vec<Tarjeta> = Vec::new();
    for foco in orden {
        if tarjetas
            .iter()
            .any(|t| crate::multitarea::misma_unidad(state, &t.foco, foco))
        {
            continue;
        }
        let miembros: Vec<_> = ventanas
            .iter()
            .filter(|(w, _)| crate::multitarea::misma_unidad(state, w, foco))
            .cloned()
            .collect();
        let Some(origen) = miembros.iter().map(|(_, r)| *r).reduce(Rect::merge) else {
            continue;
        };
        if origen.size.is_empty() {
            continue;
        }
        let mut apps: Vec<_> = miembros
            .iter()
            .filter(|(w, _)| crate::multitarea::raiz(state, w) == *w)
            .collect();
        apps.sort_by_key(|(_, r)| r.loc.x as i32);
        let rotulos = apps
            .into_iter()
            .map(|(w, _)| {
                let id = crate::handlers::app_id(w).unwrap_or_default();
                let titulo = crate::handlers::titulo(w).unwrap_or_default();
                bookos_shell::entrada_de_ventana(&id, &titulo)
            })
            .collect();
        let mut hash = DefaultHasher::new();
        foco.hash(&mut hash);
        tarjetas.push(Tarjeta {
            foco: foco.clone(),
            nodo: accesskit::NodeId(hash.finish() | 4),
            ventanas: miembros,
            origen,
            rotulo: EtiquetaTactil::aplicaciones(rotulos),
        });
    }
    if tarjetas.is_empty() {
        return None;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
        shell.retirar_barras_tactiles();
    }
    // La vista se queda el teclado aunque la aplicación reproduzca contenido.
    state.desenfocar();
    Some(Vista {
        salida,
        pantalla,
        tarjetas,
        activa: 0,
        desde_escritorio: actual.is_none(),
        abrir_elegida: false,
        foco_pie: None,
        teclado: false,
        entrada: Muelle::nuevo(0.0, M_SUAVE),
        x: Muelle::nuevo(0.0, M_SUAVE),
        y: Muelle::nuevo(0.0, M_SUAVE),
        carrusel: Muelle::nuevo(0.0, M_SUAVE),
        escritorio: Muelle::nuevo(0.0, M_SUAVE),
        borde: None,
        agarre: None,
        arrastrada: None,
        arrastre_x: Muelle::nuevo(0.0, M_RAPIDO),
        arrastre_y: Muelle::nuevo(0.0, M_RAPIDO),
        colocar: None,
        pie: [
            EtiquetaTactil::texto(bookos_shell::t("Escritorio", "Desktop")),
            EtiquetaTactil::texto(bookos_shell::t("Cerrar todo", "Close all")),
        ],
        dividir: EtiquetaTactil::texto(bookos_shell::t("Pantalla dividida", "Split screen")),
        flotante: EtiquetaTactil::texto(bookos_shell::t("Ventana flotante", "Floating window")),
        velo: Id::new(),
        indicador: Id::new(),
    })
}

fn escalar(r: Rect, ancla: Punto, escala: f64) -> Rect {
    Rectangle::new(
        ancla + (r.loc - ancla).upscale(escala),
        r.size.upscale(escala),
    )
}

fn trasladar(r: Rect, delta: (f64, f64)) -> Rect {
    Rectangle::new(r.loc + Punto::from(delta), r.size)
}

fn interpolar(a: Rect, b: Rect, t: f64) -> Rect {
    Rectangle::new(
        a.loc + (b.loc - a.loc).upscale(t),
        a.size.upscale(1.0 - t) + b.size.upscale(t),
    )
}

impl Vista {
    fn paso(&self) -> f64 {
        // La vecina asoma sin invadir la tarjeta que se puede tocar en el centro.
        (self.pantalla.size.w * 0.47).min(self.pantalla.size.h * 1.05)
    }

    fn destino(&self, i: usize) -> Rect {
        let p = self.pantalla;
        let distancia = i as f64 - f64::from(self.carrusel.valor());
        let profundidad = distancia.abs().min(2.0);
        let margen_alto = f64::from((TOQUE + AIRE) * 2);
        let ancho = p.size.w * if p.size.w < p.size.h { 0.76 } else { 0.67 };
        let alto = (p.size.h * 0.69).min((p.size.h - 2.0 * margen_alto).max(1.0));
        let origen = self.tarjetas[i].origen;
        let factor = (ancho / origen.size.w).min(alto / origen.size.h) * (1.0 - profundidad * 0.18);
        let tam = origen.size.upscale(factor);
        let centro = p.loc + Punto::from((p.size.w / 2.0, p.size.h / 2.0 - f64::from(AIRE)));
        let x = distancia * self.paso() / (1.0 + profundidad * 0.13);
        Rectangle::new(centro + Punto::from((x - tam.w / 2.0, -tam.h / 2.0)), tam)
    }

    fn tarjeta_base(&self, i: usize) -> Rect {
        let destino = self.destino(i);
        let entrada = if tema::efectos_reducidos() {
            1.0
        } else {
            self.entrada.valor().clamp(0.0, 1.0)
        } as f64;
        if i == self.activa && (!self.desde_escritorio || self.abrir_elegida) {
            interpolar(self.tarjetas[i].origen, destino, entrada)
        } else {
            let centro = self.pantalla.loc.x + self.pantalla.size.w / 2.0;
            let direccion = (destino.loc.x + destino.size.w / 2.0 - centro).signum();
            trasladar(
                destino,
                (
                    direccion * (1.0 - entrada) * self.pantalla.size.w * 0.26,
                    0.0,
                ),
            )
        }
    }

    fn tarjeta(&self, i: usize) -> Rect {
        let base = self.tarjeta_base(i);
        if tema::efectos_reducidos() {
            return base;
        }
        let salida = f64::from(self.escritorio.valor().clamp(0.0, 1.0));
        let centro = base.loc + base.size.upscale(0.5);
        let arrastre = if self.arrastrada == Some(i) {
            (
                f64::from(self.arrastre_x.valor()) * self.pantalla.size.w,
                f64::from(self.arrastre_y.valor()) * self.pantalla.size.h,
            )
        } else {
            (0.0, 0.0)
        };
        let lateral = if i == self.activa || self.escritorio.valor() > 0.0 {
            f64::from(self.x.valor())
        } else {
            0.0
        };
        let vertical = if i == self.activa && self.escritorio.valor() <= 0.0 {
            f64::from(self.y.valor())
        } else {
            -salida * 0.72
        };
        trasladar(
            escalar(base, centro, 1.0 - salida * 0.30),
            (
                lateral * self.pantalla.size.w + arrastre.0,
                vertical * self.pantalla.size.h + arrastre.1,
            ),
        )
    }

    fn rotulo(&self, i: usize, tarjeta: Rect) -> Rect {
        let ancho = self.tarjetas[i]
            .rotulo
            .ancho()
            .min(self.pantalla.size.w as i32 - AIRE * 2)
            .max(TOQUE);
        Rectangle::new(
            (
                tarjeta.loc.x + (tarjeta.size.w - f64::from(ancho)) / 2.0,
                tarjeta.loc.y - f64::from(TOQUE + AIRE),
            )
                .into(),
            (f64::from(ancho), f64::from(TOQUE)).into(),
        )
    }

    fn boton(&self, i: usize) -> Rect {
        let [primero, segundo] = self.pie.each_ref().map(|b| f64::from(b.ancho()));
        let x = self.pantalla.loc.x
            + (self.pantalla.size.w - primero - segundo - f64::from(AIRE)) / 2.0;
        let y = self.pantalla.loc.y + self.pantalla.size.h - f64::from(TOQUE + 28);
        let (x, ancho) = if i == 0 {
            (x, primero)
        } else {
            (x + primero + f64::from(AIRE), segundo)
        };
        Rectangle::new((x, y).into(), (ancho, f64::from(TOQUE)).into())
    }

    fn orden(&self) -> impl Iterator<Item = usize> {
        let centro = self
            .carrusel
            .valor()
            .round()
            .clamp(0.0, (self.tarjetas.len() - 1) as f32) as usize;
        // Sale de delante a atrás y solo visita tarjetas que puedan entrar en la salida.
        let especial = self
            .arrastrada
            .or((self.entrada.destino() == 0.0).then_some(self.activa));
        especial
            .into_iter()
            .chain(
                std::iter::once(centro)
                    .chain((1..=3).flat_map(move |d| {
                        [centro.checked_sub(d), centro.checked_add(d)]
                            .into_iter()
                            .flatten()
                    }))
                    .filter(move |i| Some(*i) != especial),
            )
            .filter(|&i| i < self.tarjetas.len())
    }

    fn bajo(&self, p: Punto) -> Objetivo {
        if self.entrada.valor() >= 0.9 {
            if self.boton(0).contains(p) {
                return Objetivo::Escritorio;
            }
            if self.boton(1).contains(p) {
                return Objetivo::CerrarTodo;
            }
        }
        for i in self.orden() {
            let r = self.tarjeta(i);
            if r.contains(p) {
                return Objetivo::Tarjeta(i);
            }
            if self.entrada.valor() >= 0.9 && self.rotulo(i, r).contains(p) {
                return Objetivo::Rotulo(i);
            }
        }
        Objetivo::Fondo
    }

    fn mover_carrusel(&mut self, i: usize, velocidad: f32) {
        self.activa = i.min(self.tarjetas.len() - 1);
        self.carrusel.soltar_gesto(self.activa as f32, velocidad);
    }

    pub fn animando(&self) -> bool {
        [
            self.entrada,
            self.x,
            self.y,
            self.carrusel,
            self.escritorio,
            self.arrastre_x,
            self.arrastre_y,
        ]
        .iter()
        .any(|m| !m.quieto())
    }

    pub(crate) fn ocultar_barras(&self) -> bool {
        self.entrada.valor() > 0.15 && self.escritorio.valor() < 0.72
    }

    pub fn en_salida(&self, salida: &Output) -> bool {
        &self.salida == salida
    }

    pub(crate) fn visible(&self) -> bool {
        !tema::efectos_reducidos()
            || self
                .borde
                .as_ref()
                .map_or(self.entrada.destino() > 0.0, |b| {
                    b.escritorio || b.destino_anterior > 0.0
                })
    }
}

pub fn iniciar_borde(state: &mut BookosComp, origen: Punto, time: u32) -> bool {
    if !habilitada(state) {
        return false;
    }
    if state.vista_tactil.is_none() {
        let Some((salida, pantalla)) = state.space.outputs().find_map(|o| {
            let r = state.space.output_geometry(o)?.to_f64();
            r.contains(origen).then(|| (o.clone(), r))
        }) else {
            return false;
        };
        if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
            return false;
        }
        state.vista_tactil = vista_nueva(state, salida, pantalla);
    }
    state.pointer_location = origen;
    let Some(v) = state.vista_tactil.as_mut() else {
        return false;
    };
    let escritorio = v.entrada.destino() == 1.0 || v.escritorio.valor() > 0.0;
    let muelle = if escritorio {
        &mut v.escritorio
    } else {
        &mut v.entrada
    };
    let avance = muelle.valor();
    let destino_anterior = muelle.destino();
    muelle.fijar(avance);
    let mut velocidad = Velocidad::default();
    velocidad.anotar(u64::from(time) * 1000, avance);
    v.borde = Some(Borde {
        origen,
        inicial: v.tarjeta(v.activa),
        avance,
        destino_anterior,
        desplazamiento_x: v.x.valor(),
        escritorio,
        velocidad,
    });
    v.agarre = None;
    v.colocar = None;
    true
}

pub fn mover_borde(state: &mut BookosComp, punto: Punto, time: u32) {
    let Some(v) = state.vista_tactil.as_mut() else {
        return;
    };
    let Some(b) = v.borde.as_mut() else {
        return;
    };
    let delta = punto - b.origen;
    let w_pantalla = v.pantalla.size.w.max(1.0);
    let h_pantalla = v.pantalla.size.h.max(1.0);
    let recorrido = (h_pantalla * if b.escritorio { 0.72 } else { 0.30 }).max(1.0);
    let avance = (b.avance - (delta.y / recorrido) as f32).clamp(0.0, 1.0);
    b.velocidad.anotar(u64::from(time) * 1000, avance);
    if b.escritorio {
        v.escritorio.fijar(avance);
        v.x.fijar(b.desplazamiento_x + (delta.x / w_pantalla) as f32);
    } else {
        v.entrada.fijar(avance);
        let base = v.tarjeta_base(v.activa);
        let b = v
            .borde
            .as_ref()
            .expect("el borde permanece mientras lo mueve el dedo");
        // Escalar alrededor del punto de apoyo conserva el agarre incluso si la
        // ventana era flotante y el dedo empezó fuera de ella.
        let anclada = escalar(
            b.inicial,
            b.origen,
            base.size.w / b.inicial.size.w.max(1.0),
        );
        let desfase = anclada.loc + delta - base.loc;
        // Al deshacer el gesto, la ventana queda en su sitio al regresar
        // al recorrido del dock, aunque el dedo vuelva en diagonal.
        let peso = f64::from((avance / 0.15).min(1.0));
        v.x.fijar((desfase.x * peso / w_pantalla) as f32);
        v.y.fijar((desfase.y * peso / h_pantalla) as f32);
    }
    state.needs_redraw = true;
}

pub fn soltar_borde(state: &mut BookosComp, time: u32, cancelar: bool) -> bool {
    let Some(v) = state.vista_tactil.as_mut() else {
        return false;
    };
    let Some(mut b) = v.borde.take() else {
        return false;
    };
    let muelle = if b.escritorio {
        &mut v.escritorio
    } else {
        &mut v.entrada
    };
    let avance = muelle.valor();
    let velocidad = if cancelar {
        0.0
    } else {
        b.velocidad.al_soltar(u64::from(time) * 1000)
    };
    let destino = if cancelar {
        b.destino_anterior
    } else if confirma_borde(avance, velocidad, b.escritorio) {
        1.0
    } else {
        0.0
    };
    muelle.soltar_gesto(destino, velocidad);
    v.x.ir_a(0.0);
    v.y.ir_a(0.0);
    state.needs_redraw = true;
    destino == 1.0
}

fn confirma_borde(avance: f32, velocidad: f32, escritorio: bool) -> bool {
    let (minimo, objetivo) = if escritorio {
        (0.08, 0.30)
    } else {
        (0.15, 0.44)
    };
    avance >= minimo && proyectar_impulso(avance, velocidad) >= objetivo
}

pub fn pulsar(state: &mut BookosComp, slot: Option<TouchSlot>, punto: Punto, time: u32) -> bool {
    let Some(v) = state.vista_tactil.as_mut() else {
        return false;
    };
    if v.borde.is_some() || v.agarre.is_some() {
        return true;
    }
    if v.escritorio.valor() > 0.0 {
        v.escritorio.ir_a(0.0);
    }
    if v.entrada.destino() == 0.0 {
        v.entrada.ir_a(1.0);
        v.x.ir_a(0.0);
        v.y.ir_a(0.0);
    }
    let objetivo = v.bajo(punto);
    let carrusel = Agarre::nuevo(v.carrusel.valor(), v.carrusel.destino());
    v.carrusel.fijar(v.carrusel.valor());
    let mut velocidad_x = Velocidad::default();
    let mut velocidad_y = Velocidad::default();
    velocidad_x.anotar(u64::from(time) * 1000, 0.0);
    velocidad_y.anotar(u64::from(time) * 1000, 0.0);
    v.agarre = Some(AgarreTarjeta {
        slot,
        origen: punto,
        objetivo,
        gesto: Gesto::Toque,
        carrusel,
        velocidad_x,
        velocidad_y,
    });
    state.needs_redraw = true;
    true
}

fn colocacion(pantalla: Rect, punto: Punto) -> Option<Colocacion> {
    let x = punto.x - pantalla.loc.x;
    let franja = (pantalla.size.w * 0.16).clamp(44.0, 120.0);
    if x < franja {
        Some(Colocacion::Dividir(Zona::Izquierda))
    } else if x > pantalla.size.w - franja {
        Some(Colocacion::Dividir(Zona::Derecha))
    } else if punto.y >= pantalla.loc.y + pantalla.size.h * 0.78 {
        Some(Colocacion::Flotante)
    } else {
        None
    }
}

pub fn mover(state: &mut BookosComp, slot: Option<TouchSlot>, punto: Punto, time: u32) -> bool {
    let Some(v) = state.vista_tactil.as_mut() else {
        return false;
    };
    let paso = v.paso() as f32;
    let Some(a) = v.agarre.as_mut().filter(|a| a.slot == slot) else {
        return true;
    };
    let d = punto - a.origen;
    let x = d.x as f32 / paso;
    let y = (d.y / v.pantalla.size.h) as f32;
    a.velocidad_x.anotar(u64::from(time) * 1000, -x);
    a.velocidad_y.anotar(u64::from(time) * 1000, y);
    if a.gesto == Gesto::Toque && d.x.hypot(d.y) >= UMBRAL {
        a.gesto = match a.objetivo {
            Objetivo::Rotulo(i) if i < v.tarjetas.len() => {
                if v.arrastrada != Some(i) {
                    v.arrastre_x.fijar(0.0);
                    v.arrastre_y.fijar(0.0);
                }
                v.arrastrada = Some(i);
                Gesto::Colocar
            }
            Objetivo::Tarjeta(_) | Objetivo::Fondo if d.x.abs() > d.y.abs() => Gesto::Carrusel,
            Objetivo::Tarjeta(i) if d.y < 0.0 && i < v.tarjetas.len() => {
                if v.arrastrada != Some(i) {
                    v.arrastre_x.fijar(0.0);
                    v.arrastre_y.fijar(0.0);
                }
                v.arrastrada = Some(i);
                Gesto::Cerrar
            }
            _ => Gesto::Cancelado,
        };
    }
    match a.gesto {
        Gesto::Carrusel => v.carrusel.fijar(a.carrusel.seguir_con_goma(
            -x,
            0.0,
            (v.tarjetas.len() - 1) as f32,
            0.36,
            0.36,
        )),
        Gesto::Cerrar | Gesto::Colocar => {
            v.arrastre_x.fijar((d.x / v.pantalla.size.w) as f32);
            v.arrastre_y.fijar(y);
            if a.gesto == Gesto::Colocar {
                v.colocar = colocacion(v.pantalla, punto);
            }
        }
        _ => {}
    }
    state.needs_redraw = true;
    true
}

pub fn soltar(state: &mut BookosComp, slot: Option<TouchSlot>, time: u32, cancelar: bool) -> bool {
    let Some(v) = state.vista_tactil.as_mut() else {
        return false;
    };
    if v.agarre.as_ref().is_none_or(|a| a.slot != slot) {
        return true;
    }
    let mut a = v.agarre.take().expect("la suelta corresponde al agarre");
    let indice = match a.objetivo {
        Objetivo::Tarjeta(i) | Objetivo::Rotulo(i) => i,
        _ => v.activa,
    };
    let gesto = if cancelar { Gesto::Cancelado } else { a.gesto };
    let destino = v.colocar.take();
    let y = v.arrastre_y.valor();
    let vx = a.velocidad_x.al_soltar(u64::from(time) * 1000);
    let vy = a.velocidad_y.al_soltar(u64::from(time) * 1000);
    let cierra = gesto == Gesto::Cerrar && y < -0.025 && proyectar_impulso(y, vy) < -0.15;
    if matches!(a.gesto, Gesto::Cerrar | Gesto::Colocar) {
        v.arrastre_x.ir_a(0.0);
        if cierra {
            // Sale por arriba con el impulso del dedo en vez de volver a su sitio
            // y desaparecer de golpe cuando el cliente se cierra. Si el cliente no
            // se cierra (diálogo de guardar), `avanzar` la hace volver.
            v.arrastre_y.soltar_gesto(-1.1, vy);
        } else {
            v.arrastre_y.ir_a(0.0);
        }
    }
    match gesto {
        Gesto::Carrusel => {
            let i = proyectar_impulso(v.carrusel.valor(), vx).round().max(0.0) as usize;
            v.mover_carrusel(i, vx);
        }
        Gesto::Cerrar if cierra => cerrar_tarjeta(state, indice),
        Gesto::Colocar if let Some(zona) = destino => colocar(state, indice, zona),
        Gesto::Toque => match a.objetivo {
            Objetivo::Tarjeta(i) | Objetivo::Rotulo(i) => abrir(state, i),
            Objetivo::Escritorio => ir_al_escritorio(state),
            Objetivo::CerrarTodo => cerrar_todo(state),
            Objetivo::Fondo => volver(state),
        },
        _ => v.mover_carrusel(v.activa, 0.0),
    }
    state.needs_redraw = true;
    true
}

fn abrir(state: &mut BookosComp, i: usize) {
    let Some(v) = state.vista_tactil.as_mut() else {
        return;
    };
    let i = i.min(v.tarjetas.len().saturating_sub(1));
    v.activa = i;
    v.abrir_elegida = true;
    v.carrusel.ir_a(i as f32);
    v.entrada.ir_a(0.0);
    v.escritorio.ir_a(0.0);
    v.x.ir_a(0.0);
    v.y.ir_a(0.0);
    state.needs_redraw = true;
}

fn volver(state: &mut BookosComp) {
    if let Some(v) = state.vista_tactil.as_mut() {
        if v.desde_escritorio {
            v.entrada.ir_a(0.0);
        } else {
            abrir(state, 0);
        }
        state.needs_redraw = true;
    }
}

fn ir_al_escritorio(state: &mut BookosComp) {
    if let Some(v) = state.vista_tactil.as_mut() {
        v.escritorio.ir_a(1.0);
        state.needs_redraw = true;
    }
}

fn cerrar_tarjeta(state: &mut BookosComp, i: usize) {
    let Some(v) = state.vista_tactil.as_ref() else {
        return;
    };
    let Some(tarjeta) = v.tarjetas.get(i) else {
        return;
    };
    // Cerrar pide al cliente que confirme y conserva los diálogos de guardado.
    for (w, _) in &tarjeta.ventanas {
        if crate::multitarea::raiz(state, w) == *w {
            crate::keybinds::cerrar(w);
        }
    }
}

fn cerrar_todo(state: &mut BookosComp) {
    let Some(v) = state.vista_tactil.as_ref() else {
        return;
    };
    for i in 0..v.tarjetas.len() {
        cerrar_tarjeta(state, i);
    }
    cancelar(state);
}

fn colocar(state: &mut BookosComp, i: usize, destino: Colocacion) {
    let Some(v) = state.vista_tactil.take() else {
        return;
    };
    let Some(tarjeta) = v.tarjetas.get(i) else {
        return;
    };
    let w = crate::multitarea::raiz(state, &tarjeta.foco);
    if w.alive() {
        if state.space.element_location(&w).is_none() {
            crate::escritorios::recuperar_inmediato(state);
        }
        match destino {
            Colocacion::Dividir(zona) => crate::multitarea::emparejar_en_lado(state, &w, zona),
            Colocacion::Flotante => {
                crate::multitarea::retirar(state, &w);
                if crate::ventanas::completa(&w) {
                    state.pantalla_completa(&w, false);
                } else {
                    state.desencajar(&w);
                }
                state.enfocar(&w);
            }
        }
    }
    state.needs_redraw = true;
}

pub fn cancelar(state: &mut BookosComp) {
    if let Some(v) = state.vista_tactil.take() {
        if !v.desde_escritorio
            && let Some(t) = v.tarjetas.first().filter(|t| t.foco.alive())
        {
            state.enfocar(&t.foco);
        }
        state.needs_redraw = true;
    }
}

pub fn cancelar_toque(state: &mut BookosComp, time: u32) {
    if let Some(slot) = state
        .vista_tactil
        .as_ref()
        .and_then(|v| v.agarre.as_ref())
        .and_then(|a| a.slot)
    {
        soltar(state, Some(slot), time, true);
    }
    soltar_borde(state, time, true);
}

pub fn tecla(state: &mut BookosComp, tecla: u32, mayus: bool) {
    let Some(v) = state.vista_tactil.as_mut() else {
        return;
    };
    if v.entrada.destino() == 0.0 || v.escritorio.destino() == 1.0 {
        return;
    }
    v.teclado = true;
    let i = v.activa;
    match tecla {
        keysyms::KEY_Left | keysyms::KEY_Right => {
            v.foco_pie = None;
            v.mover_carrusel(
                if tecla == keysyms::KEY_Left {
                    i.saturating_sub(1)
                } else {
                    i + 1
                },
                0.0,
            );
        }
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => {
            v.foco_pie = match (v.foco_pie, mayus) {
                (None, false) | (Some(1), true) => Some(0),
                (None, true) | (Some(0), false) => Some(1),
                _ => None,
            };
        }
        keysyms::KEY_Return | keysyms::KEY_KP_Enter | keysyms::KEY_space => match v.foco_pie {
            Some(0) => ir_al_escritorio(state),
            Some(_) => cerrar_todo(state),
            None => abrir(state, i),
        },
        keysyms::KEY_Escape => volver(state),
        keysyms::KEY_Home => ir_al_escritorio(state),
        keysyms::KEY_Delete if mayus => cerrar_todo(state),
        keysyms::KEY_Delete => cerrar_tarjeta(state, i),
        keysyms::KEY_Up => colocar(state, i, Colocacion::Dividir(Zona::Izquierda)),
        keysyms::KEY_Down if mayus => colocar(state, i, Colocacion::Flotante),
        keysyms::KEY_Down => colocar(state, i, Colocacion::Dividir(Zona::Derecha)),
        _ => return,
    }
    state.needs_redraw = true;
}

impl Vista {
    pub(crate) fn arbol_accesible(&self) -> bookos_shell::accesible::Arbol {
        let rect = |r: Rect| {
            accesskit::Rect::new(r.loc.x, r.loc.y, r.loc.x + r.size.w, r.loc.y + r.size.h)
        };
        let controles = self
            .tarjetas
            .iter()
            .enumerate()
            .map(|(i, t)| (t.nodo, t.rotulo.nombre(), rect(self.tarjeta(i))))
            .chain(self.pie.iter().enumerate().map(|(i, b)| {
                (
                    accesskit::NodeId(i as u64 + 2),
                    b.nombre(),
                    rect(self.boton(i)),
                )
            }));
        let foco = self.foco_pie.map_or(self.tarjetas[self.activa].nodo, |i| {
            accesskit::NodeId(i as u64 + 2)
        });
        bookos_shell::accesible::arbol_recientes(rect(self.pantalla), controles, foco)
    }
}

pub(crate) fn activar_nodo(state: &mut BookosComp, id: accesskit::NodeId) {
    let Some(v) = state.vista_tactil.as_ref() else {
        return;
    };
    match id.0 {
        2 => ir_al_escritorio(state),
        3 => cerrar_todo(state),
        _ => {
            if let Some(i) = v.tarjetas.iter().position(|t| t.nodo == id) {
                abrir(state, i);
            }
        }
    }
}

fn reajustar_indices_tras_baja(
    activa: &mut usize,
    arrastrada: &mut Option<usize>,
    agarre_objetivo: Option<&mut Objetivo>,
    indice_baja: usize,
) -> (bool, bool) {
    if indice_baja < *activa {
        *activa -= 1;
    }
    let mut murio_arrastrada = false;
    match arrastrada.as_mut() {
        Some(a) if *a == indice_baja => {
            *arrastrada = None;
            murio_arrastrada = true;
        }
        Some(a) if *a > indice_baja => *a -= 1,
        _ => {}
    }
    let mut cancelar_agarre = false;
    if let Some(Objetivo::Tarjeta(k) | Objetivo::Rotulo(k)) = agarre_objetivo {
        if *k == indice_baja {
            cancelar_agarre = true;
        } else if *k > indice_baja {
            *k -= 1;
        }
    }
    (murio_arrastrada, cancelar_agarre)
}

pub fn avanzar(state: &mut BookosComp, ahora: Instant) {
    let puede = habilitada(state);
    let Some(v) = state.vista_tactil.as_mut() else {
        return;
    };
    if !puede || state.space.output_geometry(&v.salida).map(|g| g.to_f64()) != Some(v.pantalla) {
        state.vista_tactil = None;
        state.needs_redraw = true;
        return;
    }
    let anterior = v.activa;
    let centro = v.carrusel.valor().round().max(0.0) as usize;
    let mut desplazar = 0usize;
    let mut quitadas = 0usize;
    let total = v.tarjetas.len();
    let mut i = 0;
    v.tarjetas.retain_mut(|t| {
        t.ventanas.retain(|(w, _)| w.alive());
        if !t.foco.alive()
            && let Some((w, _)) = t.ventanas.first()
        {
            t.foco = w.clone();
        }
        let viva = !t.ventanas.is_empty();
        if !viva {
            // Los índices se corren al quitarla: el carrusel se corre con
            // ellos para que la vecina entre deslizándose y no de un salto.
            // Si era la última, se deja y el muelle trae a la anterior.
            let original = i + quitadas;
            quitadas += 1;
            if original < centro || (original == centro && centro + 1 < total) {
                desplazar += 1;
            }
            let (murio_arrastrada, cancelar_agarre) = reajustar_indices_tras_baja(
                &mut v.activa,
                &mut v.arrastrada,
                v.agarre.as_mut().map(|a| &mut a.objetivo),
                i,
            );
            if murio_arrastrada {
                v.colocar = None;
                v.arrastre_x.fijar(0.0);
                v.arrastre_y.fijar(0.0);
            }
            if cancelar_agarre {
                v.agarre = None;
            }
            state.needs_redraw = true;
        } else {
            i += 1;
        }
        viva
    });
    if v.tarjetas.is_empty() {
        state.vista_tactil = None;
        state.needs_redraw = true;
        return;
    }
    if desplazar > 0 {
        v.carrusel.fijar(v.carrusel.valor() - desplazar as f32);
        v.mover_carrusel(v.activa, 0.0);
    }
    if anterior != v.activa
        || v.activa >= v.tarjetas.len()
        || v.carrusel.destino() >= v.tarjetas.len() as f32
    {
        v.mover_carrusel(v.activa, 0.0);
    }
    for m in [
        &mut v.entrada,
        &mut v.x,
        &mut v.y,
        &mut v.carrusel,
        &mut v.escritorio,
        &mut v.arrastre_x,
        &mut v.arrastre_y,
    ] {
        m.avanzar(ahora);
    }
    if v.agarre.is_none() && v.arrastre_x.quieto() && v.arrastre_y.quieto() {
        if v.arrastre_y.destino() < -1.0 {
            // Fuera de la pantalla y la ventana sigue viva: el cliente pidió
            // confirmar. La tarjeta vuelve para que se vea su diálogo.
            v.arrastre_y.ir_a(0.0);
        } else {
            v.arrastrada = None;
        }
    }
    if v.borde.is_none() && v.escritorio.destino() == 1.0 && v.escritorio.quieto() {
        state.vista_tactil = None;
        crate::escritorios::despejar_inmediato(state);
        state.needs_redraw = true;
    } else if v.borde.is_none()
        && v.entrada.destino() == 0.0
        && v.entrada.quieto()
        && v.x.quieto()
        && v.y.quieto()
    {
        let elegida =
            (!v.desde_escritorio || v.abrir_elegida).then(|| v.tarjetas[v.activa].foco.clone());
        state.vista_tactil = None;
        if let Some(w) = elegida {
            if state.space.element_location(&w).is_none() {
                crate::escritorios::recuperar_inmediato(state);
            }
            state.enfocar(&w);
        }
        state.needs_redraw = true;
    }
}

pub fn dibujar(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    salida: &Output,
    elementos: &mut Vec<OverlayElement>,
) {
    let area = state.work_area().to_f64();
    let Some(v) = state.vista_tactil.as_mut().filter(|v| v.en_salida(salida)) else {
        return;
    };
    let area = if v.pantalla.contains_rect(area) {
        area
    } else {
        v.pantalla
    };
    let escala = salida.current_scale().fractional_scale();
    let local = |r: Rect| Rectangle::new(r.loc - v.pantalla.loc, r.size);
    let entrada = v.entrada.valor().clamp(0.0, 1.0);
    let salida_escritorio = v.escritorio.valor().clamp(0.0, 1.0);
    let alfa = 1.0 - salida_escritorio;
    let mandos = ((entrada - 0.45) / 0.55).clamp(0.0, 1.0) * alfa;
    let shell = state.shell.as_mut();
    if let Some(shell) = shell {
        if mandos > 0.0 && v.colocar.is_none() {
            for i in 0..PIE {
                let r = local(v.boton(i)).to_i32_round();
                shell.etiqueta_tactil(
                    renderer,
                    elementos,
                    &mut v.pie[i],
                    r,
                    escala,
                    mandos,
                    v.foco_pie == Some(i),
                );
            }
        }
        let mut orden = [None; 8];
        for (lugar, i) in orden.iter_mut().zip(v.orden()) {
            *lugar = Some(i);
        }
        for i in orden.into_iter().flatten() {
            let tarjeta = v.tarjeta(i);
            if tarjeta.intersection(v.pantalla).is_none() {
                continue;
            }
            if mandos > 0.0 {
                let r = local(v.rotulo(i, tarjeta)).to_i32_round();
                let foco = v.teclado
                    && v.foco_pie.is_none()
                    && v.colocar.is_none()
                    && v.carrusel.destino().round() as usize == i;
                shell.etiqueta_tactil(
                    renderer,
                    elementos,
                    &mut v.tarjetas[i].rotulo,
                    r,
                    escala,
                    mandos,
                    foco,
                );
            }
            let t = &v.tarjetas[i];
            let opacidad = if i == v.activa && (!v.desde_escritorio || v.abrir_elegida) {
                alfa
            } else {
                mandos
            };
            // Al subirla para cerrar se desvanece: anticipa que se va.
            let opacidad = if v.arrastrada == Some(i)
                && v.agarre.as_ref().is_none_or(|a| a.gesto == Gesto::Cerrar)
            {
                opacidad * (1.0 + v.arrastre_y.valor().min(0.0) * 1.4).clamp(0.0, 1.0)
            } else {
                opacidad
            };
            if opacidad > 0.0 {
                let zoom = tarjeta.size.w / t.origen.size.w;
                let contorno = local(tarjeta).to_i32_round();
                for (w, origen) in &t.ventanas {
                    let r = Rectangle::new(
                        tarjeta.loc + (origen.loc - t.origen.loc).upscale(zoom),
                        origen.size.upscale(zoom),
                    );
                    let radio_inicial =
                        if crate::ventanas::completa(w) || crate::ventanas::maximizada(w) {
                            0.0
                        } else {
                            tema::R_POPOVER
                        };
                    let radio = radio_inicial + (tema::R_TARJETA - radio_inicial) * entrada;
                    crate::backend::emitir_miniatura(
                        elementos,
                        renderer,
                        state.esquinas.as_ref(),
                        w,
                        local(r).to_i32_round(),
                        contorno,
                        opacidad,
                        escala,
                        radio * escala as f32,
                    );
                }
            }
            if v.arrastrada == Some(i)
                && let Some(zona) = v.colocar
            {
                let (destino, etiqueta) = match zona {
                    Colocacion::Dividir(z) => {
                        (z.rect(area.to_i32_round()).to_f64(), &mut v.dividir)
                    }
                    Colocacion::Flotante => (
                        escalar(area, area.loc + area.size.upscale(0.5), 0.68),
                        &mut v.flotante,
                    ),
                };
                let r = local(destino);
                let w = f64::from(etiqueta.ancho());
                let caja = Rectangle::new(
                    (r.loc.x + (r.size.w - w) / 2.0, r.loc.y + 16.0).into(),
                    (w, f64::from(TOQUE)).into(),
                );
                shell.etiqueta_tactil(
                    renderer,
                    elementos,
                    etiqueta,
                    caja.to_i32_round(),
                    escala,
                    1.0,
                    true,
                );
                crate::backend::emitir_previa_zona(elementos, r.to_i32_round(), escala);
            }
        }
    }
    let tema_fondo = tema::card();
    let a = entrada * alfa * if tema::alto_contraste() { 1.0 } else { 0.45 };
    if a > 0.0 {
        elementos.push(OverlayElement::Color(SolidColorRenderElement::new(
            v.velo.clone(),
            local(v.pantalla).to_physical_precise_round(escala),
            0,
            if tema::alto_contraste() {
                [tema_fondo.r * a, tema_fondo.g * a, tema_fondo.b * a, a]
            } else {
                [0.0, 0.0, 0.0, a]
            },
            Kind::Unspecified,
        )));
    }
    if mandos > 0.0 {
        let ancho = (v.pantalla.size.w * 0.22).clamp(44.0, 128.0);
        let r = Rectangle::new(
            ((v.pantalla.size.w - ancho) / 2.0, v.pantalla.size.h - 10.0).into(),
            (ancho, 4.0).into(),
        );
        let c = if tema::alto_contraste() {
            tema::texto()
        } else {
            tema::tinta_sobre(tema::alfa(tema::texto2(), 1.0))
        };
        // Por delante del velo para que el borde inferior siga siendo reconocible.
        elementos.insert(
            elementos.len() - usize::from(a > 0.0),
            OverlayElement::Color(SolidColorRenderElement::new(
                v.indicador.clone(),
                r.to_physical_precise_round(escala),
                0,
                [c.r * mandos, c.g * mandos, c.b * mandos, mandos],
                Kind::Unspecified,
            )),
        );
    }
}

#[cfg(test)]
mod pruebas {
    use super::{Colocacion, Objetivo, Zona, colocacion, confirma_borde, reajustar_indices_tras_baja};
    use smithay::utils::Rectangle;

    #[test]
    fn el_gesto_corto_reserva_el_dock_aunque_sea_rapido() {
        assert!(!confirma_borde(0.10, 4.0, false));
        assert!(!confirma_borde(0.25, 0.0, false));
        assert!(confirma_borde(0.50, 0.0, false));
        assert!(confirma_borde(0.25, 1.5, false));
        assert!(!confirma_borde(0.50, -1.5, false));
    }

    #[test]
    fn baja_en_segundo_plano_conserva_agarre_sobre_otra_tarjeta() {
        let mut activa = 2;
        let mut arrastrada = Some(2);
        let mut objetivo = Objetivo::Tarjeta(2);
        let (murio_arr, cancelar) =
            reajustar_indices_tras_baja(&mut activa, &mut arrastrada, Some(&mut objetivo), 0);
        assert!(!murio_arr);
        assert!(!cancelar);
        assert_eq!(activa, 1);
        assert_eq!(arrastrada, Some(1));
        assert!(matches!(objetivo, Objetivo::Tarjeta(1)));

        let (murio_arr, cancelar) =
            reajustar_indices_tras_baja(&mut activa, &mut arrastrada, Some(&mut objetivo), 1);
        assert!(murio_arr);
        assert!(cancelar);
        assert_eq!(arrastrada, None);
    }

    #[test]
    fn colocacion_detecta_lados_y_flotante_abajo() {
        let pantalla = Rectangle::new((0.0, 0.0).into(), (1280.0, 800.0).into());
        assert!(matches!(
            colocacion(pantalla, (40.0, 300.0).into()),
            Some(Colocacion::Dividir(Zona::Izquierda))
        ));
        assert!(matches!(
            colocacion(pantalla, (1240.0, 300.0).into()),
            Some(Colocacion::Dividir(Zona::Derecha))
        ));
        assert!(matches!(
            colocacion(pantalla, (640.0, 700.0).into()),
            Some(Colocacion::Flotante)
        ));
        assert!(colocacion(pantalla, (640.0, 400.0).into()).is_none());
    }

    #[test]
    fn emergentes_modales_bloquean_la_vista_tactil() {
        for modal in ["polkit", "acceso", "elegir-app", "compartir"] {
            assert!(!super::emergente_permitido(Some(modal)));
        }
        assert!(super::emergente_permitido(None));
        assert!(super::emergente_permitido(Some("red")));
    }
}
