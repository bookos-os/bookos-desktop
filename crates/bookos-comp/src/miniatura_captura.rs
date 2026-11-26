//! Miniatura flotante tras hacer una captura de pantalla (estilo macOS / One UI / iOS).
//!
//! Aparece en la esquina inferior derecha con la imagen recién capturada,
//! esquinas redondeadas de radio `14` (HIG §4/§9), sombra suave (HIG §6) y
//! borde sutil adaptado al tema (HIG §2). Al pulsarla o soltar un clic, vuela
//! con un muelle continuo hacia el centro del área útil y permanece opaca como
//! vista previa héroe hasta acoplarse directamente con la ventana real del
//! visor sin transición por medio ni salto al dock.
//!
//! **Sin coste en reposo.** Tanto la tarjeta pequeña como el buffer héroe de
//! alta resolución se rasterizan fuera de la ruta crítica del clic con filtro
//! de área (box filter) y sombra pre-horneada; las animaciones usan solo
//! transformación y opacidad en GPU (`OverlayElement`).

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::utils::{IsAlive, Logical, Physical, Point, Rectangle, Size, Transform};

use bookos_shell::tema;

use crate::cursor::OverlayElement;
use crate::state::BookosComp;

/// Cuánto tiempo permanece visible antes de retirarse sola (si no está el ratón encima).
pub const DURACION: Duration = Duration::from_secs(5);
/// Duración del destello breve en GPU al hacer la captura.
const D_DESTELLO: Duration = Duration::from_millis(160);
/// Cross-dissolve breve entre la vista previa héroe y la ventana viva del visor.
const D_ACOPLAR: Duration = tema::D_POPOVER;
/// Tiempo máximo de espera mientras el visor abre o activa su ventana tras el clic.
pub const ESPERA_MAX_VISOR: Duration = Duration::from_millis(1500);

const MAX_ANCHO: u32 = 220;
const MAX_ALTO: u32 = 140;
const MAX_HEROE_ANCHO: u32 = 960;
const MAX_HEROE_ALTO: u32 = 640;
const MARGEN_DER: i32 = 20;
const MARGEN_ABAJO: i32 = 88;
/// Radio de miniatura del HIG (§4 y §9: 14 px).
const RADIO: f32 = 14.0;
/// Margen reservado alrededor de la miniatura dentro del buffer para la sombra.
const MARGEN_SOMBRA: i32 = 16;
/// Margen de sombra para la vista previa héroe de alta resolución.
const MARGEN_SOMBRA_HEROE: i32 = 24;
/// Desplazamiento máximo sin contar como arrastre al pulsar y soltar.
const UMBRAL_CLIC: f64 = 6.0;
/// Distancia hacia la derecha a partir de la cual soltar descarta la tarjeta.
const UMBRAL_ARRASTRE_DESCARTE: f64 = 56.0;
/// Velocidad mínima hacia la derecha (px/s) para descartar con un gesto breve.
const UMBRAL_VELOCIDAD_DESCARTE: f64 = 320.0;
/// Tolerancias en píxeles lógicos para dar por asentado el muelle rectangular.
const UMBRAL_PX: f32 = 0.5;
const UMBRAL_VEL: f32 = 2.0;

/// Estado compartido del archivo PNG o bytes en memoria de la captura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoRecurso {
    Pendiente,
    PngEnMemoria(Arc<Vec<u8>>),
    Guardado(PathBuf),
    Error(String),
}

/// Recurso sincronizable entre la codificación PNG en segundo plano y el lanzador del visor.
#[derive(Debug, Clone)]
pub struct RecursoCaptura {
    inner: Arc<(Mutex<EstadoRecurso>, Condvar)>,
}

impl RecursoCaptura {
    /// Crea un recurso pendiente de codificación o guardado en segundo plano.
    pub fn pendiente() -> Self {
        Self {
            inner: Arc::new((Mutex::new(EstadoRecurso::Pendiente), Condvar::new())),
        }
    }

    /// Consulta el estado actual sin bloquearse más que el mutex corto.
    pub fn estado_actual(&self) -> EstadoRecurso {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Publica el resultado y despierta al hilo del visor si estaba esperando.
    pub fn publicar(&self, estado: EstadoRecurso) {
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        *guard = estado;
        cvar.notify_all();
    }

    /// Espera hasta `max_espera` mientras el estado siga en `Pendiente`.
    pub fn esperar(&self, max_espera: Duration) -> EstadoRecurso {
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let limite = Instant::now() + max_espera;
        while matches!(*guard, EstadoRecurso::Pendiente) {
            let restante = limite.saturating_duration_since(Instant::now());
            if restante.is_zero() {
                break;
            }
            let (ng, res) = cvar
                .wait_timeout(guard, restante)
                .unwrap_or_else(|e| e.into_inner());
            guard = ng;
            if res.timed_out() {
                break;
            }
        }
        guard.clone()
    }

    /// Si el archivo ya quedó guardado en disco, devuelve su ruta sin esperar.
    pub fn ruta_ya_guardada(&self) -> Option<PathBuf> {
        match self.estado_actual() {
            EstadoRecurso::Guardado(ruta) => Some(ruta),
            _ => None,
        }
    }
}

/// Píxeles ARGB8888 premultiplicados rasterizados en un hilo de trabajo para el héroe.
pub struct BufferGrande {
    pub argb: Vec<u8>,
    pub pw: u32,
    pub ph: u32,
    pub lw: i32,
    pub lh: i32,
    pub bw: i32,
    pub bh: i32,
}

struct HeroeRender {
    buffer: MemoryRenderBuffer,
    tamano_logico: Size<i32, Logical>,
    tamano_buffer_logico: Size<i32, Logical>,
    tamano_fisico: Size<i32, Logical>,
}

/// Eventos diferidos enviados al bucle del compositor desde los hilos de captura y visor.
pub enum EventoCaptura {
    PngPortapapeles {
        generacion: u64,
        png: Arc<Vec<u8>>,
    },
    Guardado {
        generacion: u64,
        ruta: PathBuf,
        nombre: String,
    },
    ErrorGuardado {
        generacion: u64,
        mensaje: String,
    },
    HeroeListo {
        generacion: u64,
        heroe: BufferGrande,
    },
    HijoVisor {
        generacion: u64,
        hijo: std::process::Child,
    },
    FalloAbrirVisor {
        generacion: u64,
    },
}

/// Acción que pide una interacción del usuario sobre la miniatura.
#[derive(Debug, Clone)]
pub enum Accion {
    Abrir {
        generacion: u64,
        recurso: RecursoCaptura,
    },
    Descartar,
}

/// Modo de salida o acoplamiento en curso de la miniatura.
#[derive(Clone)]
enum Salida {
    Ninguna,
    Descartar,
    Abriendo { desde: Instant },
    Acoplando { ventana: Window, desde: Instant },
}

/// Estado del arrastre horizontal en curso sobre la miniatura.
#[derive(Debug, Clone, Copy)]
struct Arrastre {
    inicio_x: f64,
    dx: f64,
    ultimo_x: f64,
    ultimo_t: Instant,
    velocidad_x: f64,
}

/// Cuatro muelles críticos continuos (x, y, w, h) en píxeles lógicos.
#[derive(Debug, Clone, Copy)]
struct VueloRect {
    x: tema::Muelle,
    y: tema::Muelle,
    w: tema::Muelle,
    h: tema::Muelle,
    destino: Rectangle<f64, Logical>,
}

impl VueloRect {
    fn nuevo(
        origen: Rectangle<f64, Logical>,
        destino: Rectangle<f64, Logical>,
        parametros: tema::ParametrosMuelle,
    ) -> Self {
        let mut x = tema::Muelle::nuevo(origen.loc.x as f32, parametros);
        let mut y = tema::Muelle::nuevo(origen.loc.y as f32, parametros);
        let mut w = tema::Muelle::nuevo(origen.size.w.max(1.0) as f32, parametros);
        let mut h = tema::Muelle::nuevo(origen.size.h.max(1.0) as f32, parametros);
        x.ir_a(destino.loc.x as f32);
        y.ir_a(destino.loc.y as f32);
        w.ir_a(destino.size.w.max(1.0) as f32);
        h.ir_a(destino.size.h.max(1.0) as f32);
        Self {
            x,
            y,
            w,
            h,
            destino,
        }
    }

    fn ir_a(&mut self, destino: Rectangle<f64, Logical>, parametros: tema::ParametrosMuelle) {
        self.x.poner_parametros(parametros);
        self.y.poner_parametros(parametros);
        self.w.poner_parametros(parametros);
        self.h.poner_parametros(parametros);
        self.x.ir_a(destino.loc.x as f32);
        self.y.ir_a(destino.loc.y as f32);
        self.w.ir_a(destino.size.w.max(1.0) as f32);
        self.h.ir_a(destino.size.h.max(1.0) as f32);
        self.destino = destino;
    }

    fn redirigir(
        &mut self,
        origen: Rectangle<f64, Logical>,
        destino: Rectangle<f64, Logical>,
        parametros: tema::ParametrosMuelle,
    ) {
        let vel = self.velocidades();
        self.x.fijar(origen.loc.x as f32);
        self.y.fijar(origen.loc.y as f32);
        self.w.fijar(origen.size.w.max(1.0) as f32);
        self.h.fijar(origen.size.h.max(1.0) as f32);
        self.x.poner_parametros(parametros);
        self.y.poner_parametros(parametros);
        self.w.poner_parametros(parametros);
        self.h.poner_parametros(parametros);
        self.x.soltar(destino.loc.x as f32, vel.0);
        self.y.soltar(destino.loc.y as f32, vel.1);
        self.w.soltar(destino.size.w.max(1.0) as f32, vel.2);
        self.h.soltar(destino.size.h.max(1.0) as f32, vel.3);
        self.destino = destino;
    }

    fn fijar(&mut self, rect: Rectangle<f64, Logical>) {
        self.x.fijar(rect.loc.x as f32);
        self.y.fijar(rect.loc.y as f32);
        self.w.fijar(rect.size.w.max(1.0) as f32);
        self.h.fijar(rect.size.h.max(1.0) as f32);
        self.destino = rect;
    }

    fn desplazar_origen_x(&mut self, dx: f64) {
        let cur = self.rect();
        self.x.fijar((cur.loc.x + dx) as f32);
    }

    fn avanzar(&mut self, ahora: Instant) {
        self.x.avanzar(ahora);
        self.y.avanzar(ahora);
        self.w.avanzar(ahora);
        self.h.avanzar(ahora);
    }

    fn rect(&self) -> Rectangle<f64, Logical> {
        Rectangle::new(
            (self.x.valor() as f64, self.y.valor() as f64).into(),
            (self.w.valor().max(1.0) as f64, self.h.valor().max(1.0) as f64).into(),
        )
    }

    fn velocidades(&self) -> (f32, f32, f32, f32) {
        (
            self.x.velocidad(),
            self.y.velocidad(),
            self.w.velocidad(),
            self.h.velocidad(),
        )
    }

    fn quieto(&self) -> bool {
        eje_px_quieto(&self.x)
            && eje_px_quieto(&self.y)
            && eje_px_quieto(&self.w)
            && eje_px_quieto(&self.h)
    }
}

fn eje_px_quieto(m: &tema::Muelle) -> bool {
    (m.valor() - m.destino()).abs() < UMBRAL_PX && m.velocidad().abs() < UMBRAL_VEL
}

fn rect_f_a_i(r: Rectangle<f64, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (r.loc.x.round() as i32, r.loc.y.round() as i32).into(),
        (
            r.size.w.round().max(1.0) as i32,
            r.size.h.round().max(1.0) as i32,
        )
            .into(),
    )
}

fn rect_i_a_f(r: Rectangle<i32, Logical>) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (r.loc.x as f64, r.loc.y as f64).into(),
        (r.size.w.max(1) as f64, r.size.h.max(1) as f64).into(),
    )
}

/// Prepara en un hilo auxiliar el buffer de alta resolución para el vuelo hacia el visor.
pub fn preparar_heroe(
    rgba: &[u8],
    ancho: u32,
    alto: u32,
    escala: f64,
    claro: bool,
) -> Option<BufferGrande> {
    if ancho == 0 || alto == 0 || rgba.len() < (ancho as usize * alto as usize * 4) {
        return None;
    }
    let escala = escala.clamp(1.0, 4.0);
    let ratio = (MAX_HEROE_ANCHO as f64 / ancho as f64)
        .min(MAX_HEROE_ALTO as f64 / alto as f64)
        .min(1.0);
    let lw = ((ancho as f64 * ratio).round() as u32).clamp(64, MAX_HEROE_ANCHO);
    let lh = ((alto as f64 * ratio).round() as u32).clamp(48, MAX_HEROE_ALTO);
    let bw = lw + (MARGEN_SOMBRA_HEROE as u32) * 2;
    let bh = lh + (MARGEN_SOMBRA_HEROE as u32) * 2;
    let pw = ((bw as f64 * escala).round() as u32).max(1);
    let ph = ((bh as f64 * escala).round() as u32).max(1);

    let argb = rasterizar_miniatura(
        rgba,
        ancho,
        alto,
        pw,
        ph,
        MARGEN_SOMBRA_HEROE as f32 * escala as f32,
        RADIO * escala as f32,
        escala as f32,
        claro,
    );
    Some(BufferGrande {
        argb,
        pw,
        ph,
        lw: lw as i32,
        lh: lh as i32,
        bw: bw as i32,
        bh: bh as i32,
    })
}

/// Estado de la miniatura flotante activa.
pub struct Miniatura {
    generacion: u64,
    recurso: RecursoCaptura,
    buffer: MemoryRenderBuffer,
    heroe: Option<HeroeRender>,
    tamano_logico: Size<i32, Logical>,
    tamano_buffer_logico: Size<i32, Logical>,
    tamano_fisico: Size<i32, Logical>,
    origen_logico: Option<Rectangle<i32, Logical>>,
    creada: Instant,
    puntero_encima: bool,
    salida: Salida,
    vuelo: VueloRect,
    alfa: tema::Muelle,
    hover: tema::Muelle,
    desfase_x: tema::Muelle,
    arrastre: Option<Arrastre>,
    visor_app_id: Option<String>,
    visor_wm_class: Option<String>,
    pid_visor: Option<u32>,
    acepta_cualquier_nueva: bool,
}

impl Miniatura {
    /// Crea la miniatura reduciendo `rgba` (`ancho × alto`) con filtro de área,
    /// esquinas redondeadas `r14`, borde sutil según tema y sombra suave.
    // añadido en la 0.2.4
    #[allow(clippy::too_many_arguments)]
    pub fn nueva(
        generacion: u64,
        rgba: &[u8],
        ancho: u32,
        alto: u32,
        escala: f64,
        pantalla: Rectangle<i32, Logical>,
        recurso: RecursoCaptura,
        origen_logico: Option<Rectangle<i32, Logical>>,
    ) -> Option<Self> {
        if ancho == 0 || alto == 0 || rgba.len() < (ancho as usize * alto as usize * 4) {
            return None;
        }
        let escala = escala.clamp(1.0, 4.0);
        let ratio = (MAX_ANCHO as f64 / ancho as f64)
            .min(MAX_ALTO as f64 / alto as f64)
            .min(1.0);
        let lw = ((ancho as f64 * ratio).round() as u32).clamp(64, MAX_ANCHO);
        let lh = ((alto as f64 * ratio).round() as u32).clamp(48, MAX_ALTO);
        let bw = lw + (MARGEN_SOMBRA as u32) * 2;
        let bh = lh + (MARGEN_SOMBRA as u32) * 2;
        let pw = ((bw as f64 * escala).round() as u32).max(1);
        let ph = ((bh as f64 * escala).round() as u32).max(1);

        let argb = rasterizar_miniatura(
            rgba,
            ancho,
            alto,
            pw,
            ph,
            MARGEN_SOMBRA as f32 * escala as f32,
            RADIO * escala as f32,
            escala as f32,
            tema::es_claro(),
        );
        let buffer = MemoryRenderBuffer::from_slice(
            &argb,
            Fourcc::Argb8888,
            (pw as i32, ph as i32),
            1,
            Transform::Normal,
            None,
        );
        let tamano_logico: Size<i32, Logical> = (lw as i32, lh as i32).into();
        let destino = rect_reposo_para(pantalla, tamano_logico);

        let (origen_vuelo, mut alfa) = match origen_logico {
            Some(orig) if orig.size.w > 0 && orig.size.h > 0 => {
                let esc = ((orig.size.w as f64 / lw as f64).min(orig.size.h as f64 / lh as f64))
                    .clamp(1.25, 2.5);
                let w0 = lw as f64 * esc;
                let h0 = lh as f64 * esc;
                let cx = orig.loc.x as f64 + orig.size.w as f64 * 0.5;
                let cy = orig.loc.y as f64 + orig.size.h as f64 * 0.5;
                (
                    Rectangle::new((cx - w0 * 0.5, cy - h0 * 0.5).into(), (w0, h0).into()),
                    tema::Muelle::nuevo(1.0, tema::M_RAPIDO),
                )
            }
            _ => {
                let w0 = lw as f64 * 0.82;
                let h0 = lh as f64 * 0.82;
                let cx = destino.loc.x + destino.size.w * 0.5 + 20.0;
                let cy = destino.loc.y + destino.size.h * 0.5 + 28.0;
                let mut a = tema::Muelle::nuevo(0.0, tema::M_RAPIDO);
                a.ir_a(1.0);
                (
                    Rectangle::new((cx - w0 * 0.5, cy - h0 * 0.5).into(), (w0, h0).into()),
                    a,
                )
            }
        };

        let mut vuelo = VueloRect::nuevo(origen_vuelo, destino, tema::M_SUAVE);
        if tema::efectos_reducidos() {
            vuelo.fijar(destino);
            alfa.fijar(1.0);
        }

        Some(Self {
            generacion,
            recurso,
            buffer,
            heroe: None,
            tamano_logico,
            tamano_buffer_logico: (bw as i32, bh as i32).into(),
            tamano_fisico: (pw as i32, ph as i32).into(),
            origen_logico,
            creada: Instant::now(),
            puntero_encima: false,
            salida: Salida::Ninguna,
            vuelo,
            alfa,
            hover: tema::Muelle::nuevo(0.0, tema::M_RAPIDO),
            desfase_x: tema::Muelle::nuevo(0.0, tema::M_RAPIDO),
            arrastre: None,
            visor_app_id: None,
            visor_wm_class: None,
            pid_visor: None,
            acepta_cualquier_nueva: false,
        })
    }

    pub fn generacion(&self) -> u64 {
        self.generacion
    }

    /// Instala el buffer de alta resolución cuando termina de rasterizarse en segundo plano.
    pub fn poner_heroe(&mut self, heroe: BufferGrande) {
        let buffer = MemoryRenderBuffer::from_slice(
            &heroe.argb,
            Fourcc::Argb8888,
            (heroe.pw as i32, heroe.ph as i32),
            1,
            Transform::Normal,
            None,
        );
        self.heroe = Some(HeroeRender {
            buffer,
            tamano_logico: (heroe.lw, heroe.lh).into(),
            tamano_buffer_logico: (heroe.bw, heroe.bh).into(),
            tamano_fisico: (heroe.pw as i32, heroe.ph as i32).into(),
        });
    }

    /// Registra la identidad del visor lanzado para acoplar su ventana en cuanto aparezca.
    pub fn configurar_visor(
        &mut self,
        app_id: Option<String>,
        wm_class: Option<String>,
        pid: Option<u32>,
        acepta_cualquier_nueva: bool,
    ) {
        self.visor_app_id = app_id;
        self.visor_wm_class = wm_class;
        self.acepta_cualquier_nueva = acepta_cualquier_nueva;
        if pid.is_some() {
            self.pid_visor = pid;
        }
    }

    pub fn registrar_pid_visor(&mut self, pid: u32) {
        self.pid_visor = Some(pid);
    }

    pub fn esperando_visor(&self) -> bool {
        matches!(self.salida, Salida::Abriendo { .. })
    }

    fn coincide_visor(
        &self,
        pid_ventana: Option<u32>,
        app_ventana: Option<&str>,
        es_nueva: bool,
    ) -> bool {
        if let (Some(pv), Some(pw)) = (self.pid_visor, pid_ventana)
            && pv == pw
        {
            return true;
        }
        if let Some(id) = app_ventana {
            if let Some(ref app) = self.visor_app_id
                && bookos_shell::mismo_programa(app, id)
            {
                return true;
            }
            if let Some(ref wm) = self.visor_wm_class
                && bookos_shell::mismo_programa(wm, id)
            {
                return true;
            }
        }
        es_nueva
            && self.acepta_cualquier_nueva
            && self.visor_app_id.is_none()
            && self.visor_wm_class.is_none()
    }

    /// Cancela la espera de apertura (p. ej. con Esc o clic fuera) y retira la tarjeta.
    pub fn cancelar_apertura(&mut self, ahora: Instant) -> bool {
        if !matches!(self.salida, Salida::Abriendo { .. }) {
            return false;
        }
        self.salida = Salida::Ninguna;
        self.descartar(ahora);
        true
    }

    /// Destino centrado para la vista previa héroe dentro de `area_util`.
    pub fn rect_visor_objetivo(&self, area_util: Rectangle<i32, Logical>) -> Rectangle<f64, Logical> {
        let lw = self.tamano_logico.w.max(1) as f64;
        let lh = self.tamano_logico.h.max(1) as f64;
        let max_w = (area_util.size.w as f64 * 0.72)
            .max(lw)
            .min(960.0)
            .min((area_util.size.w as f64 * 0.92).max(lw));
        let max_h = (area_util.size.h as f64 * 0.72)
            .max(lh)
            .min(640.0)
            .min((area_util.size.h as f64 * 0.90).max(lh));
        let factor = (max_w / lw).min(max_h / lh).max(1.0);
        let tw = lw * factor;
        let th = lh * factor;
        Rectangle::new(
            (
                area_util.loc.x as f64 + (area_util.size.w as f64 - tw) * 0.5,
                area_util.loc.y as f64 + (area_util.size.h as f64 - th) * 0.5,
            )
                .into(),
            (tw, th).into(),
        )
    }

    fn desfase_arrastre(&self) -> f64 {
        self.arrastre
            .as_ref()
            .map(|a| a.dx)
            .unwrap_or_else(|| self.desfase_x.valor() as f64)
    }

    /// Rectángulo visual continuo en coordenadas lógicas (incluye hover y arrastre si está en reposo).
    fn rect_visual(&self) -> Rectangle<f64, Logical> {
        let mut r = self.vuelo.rect();
        if matches!(self.salida, Salida::Ninguna) {
            let dx = self.desfase_arrastre();
            r.loc.x += dx;
            if !tema::efectos_reducidos() {
                let h = self.hover.valor().clamp(0.0, 1.0) as f64;
                let factor = 1.0 + 0.035 * h;
                let nw = r.size.w * factor;
                let nh = r.size.h * factor;
                r.loc.x -= (nw - r.size.w) * 0.5;
                r.loc.y = r.loc.y - 2.0 * h - (nh - r.size.h) * 0.5;
                r.size.w = nw;
                r.size.h = nh;
            }
        }
        r
    }

    fn alfa_actual(&self, ahora: Instant) -> f32 {
        match self.salida {
            Salida::Ninguna | Salida::Abriendo { .. } => {
                let base = self.alfa.valor().clamp(0.0, 1.0);
                let dx = if matches!(self.salida, Salida::Ninguna) {
                    self.desfase_arrastre()
                } else {
                    0.0
                };
                let alfa_arrastre = if dx > 32.0 {
                    (1.0 - (dx as f32 - 32.0) / 140.0).clamp(0.15, 1.0)
                } else {
                    1.0
                };
                (base * alfa_arrastre).clamp(0.0, 1.0)
            }
            Salida::Descartar => self.alfa.valor().clamp(0.0, 1.0),
            Salida::Acoplando { desde, .. } => {
                if tema::efectos_reducidos() {
                    return 0.0;
                }
                let t = tema::C_SUAVE.eval(tema::avance(
                    ahora.saturating_duration_since(desde),
                    D_ACOPLAR,
                ));
                (1.0 - t).clamp(0.0, 1.0)
            }
        }
    }

    /// Rectángulo lógico actual en pantalla (zona interactiva de la tarjeta).
    // añadido en la 0.2.4
    pub fn rect(&self, _pantalla: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        rect_f_a_i(self.rect_visual())
    }

    pub fn contiene(&self, pos: Point<f64, Logical>, pantalla: Rectangle<i32, Logical>) -> bool {
        let r = self.rect(pantalla);
        pos.x >= r.loc.x as f64
            && pos.x <= (r.loc.x + r.size.w) as f64
            && pos.y >= r.loc.y as f64
            && pos.y <= (r.loc.y + r.size.h) as f64
    }

    /// Avanza los muelles y comprueba si ya caducó. Devuelve `true` si debe retirarse.
    // añadido en la 0.2.4
    pub fn avanzar(&mut self, ahora: Instant) -> bool {
        self.vuelo.avanzar(ahora);
        self.alfa.avanzar(ahora);
        self.hover.avanzar(ahora);
        self.desfase_x.avanzar(ahora);

        match self.salida {
            Salida::Ninguna => {
                if !self.puntero_encima
                    && self.arrastre.is_none()
                    && ahora.saturating_duration_since(self.creada) >= DURACION
                {
                    self.descartar(ahora);
                }
                false
            }
            Salida::Abriendo { desde } => {
                if ahora.saturating_duration_since(desde) >= ESPERA_MAX_VISOR {
                    self.salida = Salida::Ninguna;
                    self.descartar(ahora);
                }
                false
            }
            Salida::Acoplando { ref ventana, desde } => {
                !ventana.alive()
                    || tema::efectos_reducidos()
                    || ahora.saturating_duration_since(desde) >= D_ACOPLAR
            }
            Salida::Descartar => self.alfa.quieto() && self.alfa.valor() <= 0.02,
        }
    }

    // añadido en la 0.2.4
    pub fn animando(&self) -> bool {
        !self.vuelo.quieto()
            || !self.alfa.quieto()
            || !self.hover.quieto()
            || !self.desfase_x.quieto()
            || matches!(self.salida, Salida::Acoplando { .. } | Salida::Descartar)
            || (!tema::efectos_reducidos() && self.creada.elapsed() < D_DESTELLO)
    }

    // añadido en la 0.2.4
    pub fn cancelar_arrastre(&mut self) -> bool {
        let Some(a) = self.arrastre.take() else {
            return false;
        };
        if tema::efectos_reducidos() {
            self.desfase_x.fijar(0.0);
        } else {
            self.desfase_x.fijar(a.dx as f32);
            self.desfase_x.ir_a(0.0);
        }
        true
    }

    // añadido en la 0.2.4
    pub fn descartar(&mut self, _ahora: Instant) {
        if matches!(self.salida, Salida::Ninguna) {
            let cur = self.rect_visual();
            self.arrastre = None;
            self.desfase_x.fijar(0.0);
            self.salida = Salida::Descartar;
            if tema::efectos_reducidos() {
                self.hover.fijar(0.0);
                self.vuelo.fijar(cur);
                self.alfa.fijar(0.0);
                return;
            }
            self.hover.ir_a(0.0);
            let destino = Rectangle::new(
                (cur.loc.x + 56.0, cur.loc.y + cur.size.h * 0.04).into(),
                (cur.size.w * 0.92, cur.size.h * 0.92).into(),
            );
            self.vuelo.redirigir(cur, destino, tema::M_RAPIDO);
            self.alfa.poner_parametros(tema::M_RAPIDO);
            self.alfa.ir_a(0.0);
        }
    }

    /// Inicia el vuelo continuo hacia el centro para acoplar la ventana del visor.
    pub fn abrir(&mut self, area_util: Rectangle<i32, Logical>, ahora: Instant) {
        if matches!(self.salida, Salida::Ninguna) {
            let cur = self.rect_visual();
            self.arrastre = None;
            self.desfase_x.fijar(0.0);
            self.hover.fijar(0.0);
            let destino = self.rect_visor_objetivo(area_util);
            self.vuelo.redirigir(cur, destino, tema::M_SUAVE);
            self.alfa.poner_parametros(tema::M_RAPIDO);
            self.alfa.ir_a(1.0);
            self.salida = Salida::Abriendo { desde: ahora };
        }
    }

    pub fn arrastrando(&self) -> bool {
        self.arrastre.is_some()
    }

    /// Empieza la interacción al pulsar un botón sobre la pantalla.
    /// El botón derecho descarta en el acto; el izquierdo inicia un seguimiento
    /// continuo para poder arrastrar hacia la derecha o tocar para abrir el visor.
    pub fn pulsar(
        &mut self,
        pos: Point<f64, Logical>,
        pantalla: Rectangle<i32, Logical>,
        derecho: bool,
        ahora: Instant,
    ) -> Option<Accion> {
        if !matches!(self.salida, Salida::Ninguna) {
            return None;
        }
        if !self.contiene(pos, pantalla) {
            return None;
        }
        if derecho {
            self.descartar(ahora);
            return Some(Accion::Descartar);
        }
        let dx_base = self.desfase_x.valor() as f64;
        self.arrastre = Some(Arrastre {
            inicio_x: pos.x - dx_base,
            dx: dx_base,
            ultimo_x: pos.x,
            ultimo_t: ahora,
            velocidad_x: 0.0,
        });
        None
    }

    /// Actualiza la posición del puntero mientras se arrastra o al señalar.
    /// Devuelve `(cambio_visual, programar_retirada)`.
    pub fn mover_en(
        &mut self,
        pos: Point<f64, Logical>,
        pantalla: Rectangle<i32, Logical>,
        ahora: Instant,
    ) -> (bool, bool) {
        if !matches!(self.salida, Salida::Ninguna) {
            return (false, false);
        }
        if let Some(a) = self.arrastre.as_mut() {
            let dt = ahora
                .saturating_duration_since(a.ultimo_t)
                .as_secs_f64()
                .max(0.001);
            let inst_v = (pos.x - a.ultimo_x) / dt;
            a.velocidad_x = a.velocidad_x * 0.4 + inst_v * 0.6;
            a.ultimo_x = pos.x;
            a.ultimo_t = ahora;
            a.dx = (pos.x - a.inicio_x).clamp(-18.0, 240.0);
            return (true, false);
        }
        let dentro = self.contiene(pos, pantalla);
        if dentro != self.puntero_encima {
            self.puntero_encima = dentro;
            self.hover.ir_a(if dentro { 1.0 } else { 0.0 });
            let mut programar_retirada = false;
            if !dentro
                && matches!(self.salida, Salida::Ninguna)
                && let Some(limite) = ahora.checked_sub(DURACION.saturating_sub(Duration::from_secs(2)))
            {
                // Al salir el ratón, darle 2 s de margen antes de retirarse sin underflow en arranques recientes.
                self.creada = limite;
                programar_retirada = true;
            }
            return (true, programar_retirada);
        }
        (false, false)
    }

    /// Termina el arrastre izquierdo: si apenas se movió lanza el visor con vuelo héroe;
    /// si se deslizó a la derecha descarta con continuidad; si no, vuelve al sitio.
    pub fn soltar(
        &mut self,
        pos: Point<f64, Logical>,
        area_util: Rectangle<i32, Logical>,
        ahora: Instant,
    ) -> Option<Accion> {
        let a = self.arrastre.take()?;
        let dx = (pos.x - a.inicio_x).clamp(-18.0, 240.0);
        if dx.abs() <= UMBRAL_CLIC {
            self.abrir(area_util, ahora);
            return Some(Accion::Abrir {
                generacion: self.generacion,
                recurso: self.recurso.clone(),
            });
        }
        if dx >= UMBRAL_ARRASTRE_DESCARTE
            || (a.velocidad_x >= UMBRAL_VELOCIDAD_DESCARTE && dx >= 18.0)
        {
            self.desfase_x.fijar(0.0);
            self.vuelo.desplazar_origen_x(dx);
            let destino = self.vuelo.rect();
            self.salida = Salida::Descartar;
            if tema::efectos_reducidos() {
                self.hover.fijar(0.0);
                self.vuelo.fijar(destino);
                self.alfa.fijar(0.0);
                return Some(Accion::Descartar);
            }
            self.vuelo.ir_a(
                Rectangle::new(
                    (destino.loc.x + 84.0, destino.loc.y).into(),
                    destino.size,
                ),
                tema::M_RAPIDO,
            );
            self.alfa.poner_parametros(tema::M_RAPIDO);
            self.alfa.ir_a(0.0);
            return Some(Accion::Descartar);
        }
        self.desfase_x.fijar(dx as f32);
        self.desfase_x.ir_a(0.0);
        None
    }

    #[cfg(test)]
    pub fn clic(
        &mut self,
        pos: Point<f64, Logical>,
        pantalla: Rectangle<i32, Logical>,
        derecho: bool,
    ) -> Option<Accion> {
        let ahora = Instant::now();
        if let Some(accion) = self.pulsar(pos, pantalla, derecho, ahora) {
            return Some(accion);
        }
        if self.arrastre.is_some() {
            self.soltar(pos, pantalla, ahora)
        } else {
            None
        }
    }

    /// Destello breve en GPU sobre la región recién capturada.
    pub fn elemento_destello(
        &self,
        pantalla: Rectangle<i32, Logical>,
        escala: f64,
        ahora: Instant,
    ) -> Option<OverlayElement> {
        if tema::efectos_reducidos() {
            return None;
        }
        let dt = ahora.saturating_duration_since(self.creada);
        if dt >= D_DESTELLO {
            return None;
        }
        let t = dt.as_secs_f32() / D_DESTELLO.as_secs_f32();
        let alfa = 0.36 * (1.0 - t) * (1.0 - t);
        if alfa <= 0.002 {
            return None;
        }
        static ID: std::sync::OnceLock<Id> = std::sync::OnceLock::new();
        let r = self.origen_logico.unwrap_or(pantalla);
        let fisico = Rectangle::new(
            (
                (r.loc.x as f64 * escala).round() as i32,
                (r.loc.y as f64 * escala).round() as i32,
            )
                .into(),
            (
                (r.size.w as f64 * escala).round().max(1.0) as i32,
                (r.size.h as f64 * escala).round().max(1.0) as i32,
            )
                .into(),
        );
        Some(OverlayElement::Color(SolidColorRenderElement::new(
            ID.get_or_init(Id::new).clone(),
            fisico,
            0,
            [alfa, alfa, alfa, alfa],
            Kind::Unspecified,
        )))
    }

    /// Elemento listo para componer en el renderer GLES (con escala y opacidad en GPU).
    // añadido en la 0.2.4
    pub fn elemento(
        &self,
        state: &BookosComp,
        renderer: &mut GlesRenderer,
        _pantalla: Rectangle<i32, Logical>,
        escala: f64,
        ahora: Instant,
    ) -> Option<OverlayElement> {
        let alfa = self.alfa_actual(ahora);
        if alfa <= 0.005 {
            return None;
        }
        let r = match self.salida {
            Salida::Acoplando { ref ventana, .. } => {
                crate::ventanas::rect_animado(state, ventana).unwrap_or_else(|| self.rect_visual())
            }
            _ => self.rect_visual(),
        };

        // Si ya tenemos el buffer héroe listo y la tarjeta supera 1.15× su tamaño reposo,
        // usamos la textura nítida de alta resolución centrada en exactamente el mismo rectángulo.
        let escala_respecto_mini =
            (r.size.w / self.tamano_logico.w.max(1) as f64).min(r.size.h / self.tamano_logico.h.max(1) as f64);
        let (buf, tam_log, tam_buf_log, tam_fis) = match self.heroe.as_ref() {
            Some(h) if escala_respecto_mini > 1.15 => (
                &h.buffer,
                h.tamano_logico,
                h.tamano_buffer_logico,
                h.tamano_fisico,
            ),
            _ => (
                &self.buffer,
                self.tamano_logico,
                self.tamano_buffer_logico,
                self.tamano_fisico,
            ),
        };

        let cx = r.loc.x + r.size.w * 0.5;
        let cy = r.loc.y + r.size.h * 0.5;
        let escala_gpu =
            (r.size.w / tam_log.w.max(1) as f64).min(r.size.h / tam_log.h.max(1) as f64);
        let loc = (
            ((cx - tam_buf_log.w as f64 * 0.5) * escala).round(),
            ((cy - tam_buf_log.h as f64 * 0.5) * escala).round(),
        );
        let src = Rectangle::from_size((tam_fis.w as f64, tam_fis.h as f64).into());
        let el = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc,
            buf,
            Some(alfa),
            Some(src),
            Some(tam_buf_log),
            Kind::Unspecified,
        )
        .ok()?;
        if (escala_gpu - 1.0).abs() > 0.002 {
            let centro: Point<i32, Physical> = (
                (cx * escala).round() as i32,
                (cy * escala).round() as i32,
            )
                .into();
            Some(OverlayElement::MemoriaEscalada(
                RescaleRenderElement::from_element(el, centro, escala_gpu),
            ))
        } else {
            Some(OverlayElement::Memory(el))
        }
    }
}

fn rect_reposo_para(
    pantalla: Rectangle<i32, Logical>,
    tamano: Size<i32, Logical>,
) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (
            (pantalla.loc.x + pantalla.size.w - tamano.w - MARGEN_DER) as f64,
            (pantalla.loc.y + pantalla.size.h - tamano.h - MARGEN_ABAJO) as f64,
        )
            .into(),
        (tamano.w as f64, tamano.h as f64).into(),
    )
}

/// Intenta acoplar una ventana recién mapeada con la miniatura que esperaba al visor.
/// Si coincide por PID o `app_id`, sustituye la animación del dock por un vuelo continuo
/// heredando rectángulo y velocidad actuales.
pub fn intentar_acoplar_nueva(
    state: &mut BookosComp,
    window: &Window,
    destino: Rectangle<i32, Logical>,
) -> bool {
    let pid_win = crate::ventanas::pid_de(state, window);
    let app_win = crate::handlers::app_id(window);
    let acoplar = {
        let Some(mini) = state.miniatura_captura.as_mut() else {
            return false;
        };
        if !matches!(mini.salida, Salida::Abriendo { .. })
            || !mini.coincide_visor(pid_win, app_win.as_deref(), true)
        {
            return false;
        }
        let ahora = Instant::now();
        mini.vuelo.avanzar(ahora);
        let origen = rect_f_a_i(mini.vuelo.rect());
        let vel = mini.vuelo.velocidades();
        mini.salida = Salida::Acoplando {
            ventana: window.clone(),
            desde: ahora,
        };
        mini.vuelo.ir_a(rect_i_a_f(destino), tema::M_SUAVE);
        Some((origen, vel))
    };
    let Some((origen, vel)) = acoplar else {
        return false;
    };
    crate::ventanas::nace_acoplada(window, origen, vel, destino);
    state.needs_redraw = true;
    true
}

/// Intenta acoplar una ventana ya existente (visor instancia única) que recibe activación o título nuevo.
pub fn intentar_acoplar_existente(state: &mut BookosComp, window: &Window) -> bool {
    let Some(destino) = state.space.element_geometry(window) else {
        return false;
    };
    let pid_win = crate::ventanas::pid_de(state, window);
    let app_win = crate::handlers::app_id(window);
    let acoplar = {
        let Some(mini) = state.miniatura_captura.as_mut() else {
            return false;
        };
        if !matches!(mini.salida, Salida::Abriendo { .. })
            || !mini.coincide_visor(pid_win, app_win.as_deref(), false)
        {
            return false;
        }
        let ahora = Instant::now();
        mini.vuelo.avanzar(ahora);
        let origen = rect_f_a_i(mini.vuelo.rect());
        let vel = mini.vuelo.velocidades();
        mini.salida = Salida::Acoplando {
            ventana: window.clone(),
            desde: ahora,
        };
        mini.vuelo.ir_a(rect_i_a_f(destino), tema::M_SUAVE);
        Some((origen, vel))
    };
    let Some((origen, vel)) = acoplar else {
        return false;
    };
    crate::ventanas::nace_acoplada(window, origen, vel, destino);
    state.enfocar(window);
    state.needs_redraw = true;
    true
}

/// Reduce `rgba` con filtro de caja (promedio de área) y pinta esquinas
/// redondeadas, borde sutil de tema y sombra suave en formato ARGB8888
/// premultiplicado (BGRA en memoria little-endian).
#[allow(clippy::too_many_arguments)]
fn rasterizar_miniatura(
    rgba: &[u8],
    sw: u32,
    sh: u32,
    pw: u32,
    ph: u32,
    margen: f32,
    radio: f32,
    escala: f32,
    claro: bool,
) -> Vec<u8> {
    let mut out = vec![0u8; pw as usize * ph as usize * 4];
    let iw = (pw as f32 - margen * 2.0).max(1.0);
    let ih = (ph as f32 - margen * 2.0).max(1.0);
    let r = radio.min(iw * 0.5).min(ih * 0.5);
    let cx = margen + iw * 0.5;
    let cy = margen + ih * 0.5;
    let sw_f = sw as f32;
    let sh_f = sh as f32;
    let paso_x = sw_f / iw;
    let paso_y = sh_f / ih;
    let sombra_dy = 6.0 * escala;
    let sombra_radio = (margen * 0.72).max(1.0);
    let grosor_borde = (1.0 * escala).max(1.0);

    for y in 0..ph {
        let fy = y as f32 + 0.5 - margen;
        for x in 0..pw {
            let fx = x as f32 + 0.5 - margen;

            // Distancia firmada al rectángulo redondeado de la imagen.
            let qx = (x as f32 + 0.5 - cx).abs() - (iw * 0.5 - r);
            let qy = (y as f32 + 0.5 - cy).abs() - (ih * 0.5 - r);
            let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;

            // Distancia para la sombra desplazada hacia abajo (HIG §6 nivel 1-2).
            let sqy = (y as f32 + 0.5 - (cy + sombra_dy)).abs() - (ih * 0.5 - r);
            let sdist = qx.max(0.0).hypot(sqy.max(0.0)) + qx.max(sqy).min(0.0) - r;
            let sombra_a = if sdist > 0.0 && sdist < sombra_radio {
                let t = 1.0 - sdist / sombra_radio;
                0.24 * t * t
            } else if sdist <= 0.0 {
                0.24
            } else {
                0.0
            };

            let card_a = (0.5 - dist).clamp(0.0, 1.0);
            if card_a <= 0.0 && sombra_a <= 0.0 {
                continue;
            }

            let di = (y as usize * pw as usize + x as usize) * 4;
            if card_a <= 0.0 {
                out[di + 3] = (255.0 * sombra_a).round() as u8;
                continue;
            }

            // Filtro de área (hasta 4×4 muestras en la celda origen) para evitar aliasing.
            let sx0 = ((fx - 0.5) * paso_x).clamp(0.0, sw_f - 1.0);
            let sx1 = ((fx + 0.5) * paso_x).clamp(sx0 + 1.0, sw_f);
            let sy0 = ((fy - 0.5) * paso_y).clamp(0.0, sh_f - 1.0);
            let sy1 = ((fy + 0.5) * paso_y).clamp(sy0 + 1.0, sh_f);
            let nx = ((sx1 - sx0).ceil() as u32).clamp(1, 4);
            let ny = ((sy1 - sy0).ceil() as u32).clamp(1, 4);
            let mut sr = 0.0f32;
            let mut sg = 0.0f32;
            let mut sb = 0.0f32;
            let muestras = (nx * ny) as f32;
            for iy in 0..ny {
                let sy = (sy0 + (iy as f32 + 0.5) * (sy1 - sy0) / ny as f32)
                    .floor()
                    .min(sh_f - 1.0) as usize;
                for ix in 0..nx {
                    let sx = (sx0 + (ix as f32 + 0.5) * (sx1 - sx0) / nx as f32)
                        .floor()
                        .min(sw_f - 1.0) as usize;
                    let oi = (sy * sw as usize + sx) * 4;
                    sr += rgba[oi] as f32;
                    sg += rgba[oi + 1] as f32;
                    sb += rgba[oi + 2] as f32;
                }
            }
            let mut cr = sr / muestras;
            let mut cg = sg / muestras;
            let mut cb = sb / muestras;

            // Trazo interior sutil de 1 px según tema (HIG §2: --brd).
            if dist > -grosor_borde {
                let mezcla = if claro { 0.10 } else { 0.14 };
                let borde = if claro { 0.0 } else { 255.0 };
                cr = cr * (1.0 - mezcla) + borde * mezcla;
                cg = cg * (1.0 - mezcla) + borde * mezcla;
                cb = cb * (1.0 - mezcla) + borde * mezcla;
            }

            // Componer tarjeta sobre la sombra en borde anti-aliased.
            let out_a = card_a + sombra_a * (1.0 - card_a);
            // Fourcc::Argb8888 en little-endian = [B, G, R, A] premultiplicado.
            out[di] = (cb * card_a).round() as u8;
            out[di + 1] = (cg * card_a).round() as u8;
            out[di + 2] = (cr * card_a).round() as u8;
            out[di + 3] = (255.0 * out_a).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esquinas_quedan_transparentes_con_sombra_y_centro_opaco() {
        let src = vec![200u8; 80 * 60 * 4];
        let margen = 12.0;
        let pw = 64;
        let ph = 54;
        let out = rasterizar_miniatura(&src, 80, 60, pw, ph, margen, 8.0, 1.0, false);
        assert_eq!(out[3], 0);
        let centro = ((ph as usize / 2) * pw as usize + pw as usize / 2) * 4;
        assert_eq!(out[centro + 3], 255);
        let bajo = ((ph as usize - 6) * pw as usize + pw as usize / 2) * 4;
        assert!(out[bajo + 3] > 0 && out[bajo + 3] < 255);
    }

    #[test]
    fn ciclo_entrada_hover_y_descarte_con_filtro_de_area() {
        let src = vec![180u8; 320 * 180 * 4];
        let pantalla = Rectangle::from_size((1920, 1080).into());
        let mut mini = Miniatura::nueva(
            1,
            &src,
            320,
            180,
            1.0,
            pantalla,
            RecursoCaptura::pendiente(),
            None,
        )
        .expect("crea miniatura");
        let t0 = mini.creada;
        assert!(mini.animando());
        assert!(!mini.avanzar(t0 + Duration::from_millis(200)));

        // Dejar asentar para obtener el rectángulo de reposo.
        assert!(!mini.avanzar(t0 + Duration::from_secs(1)));
        let r = mini.rect(pantalla);
        let dentro = (r.loc.x as f64 + 10.0, r.loc.y as f64 + 10.0).into();
        assert_eq!(mini.mover_en(dentro, pantalla, t0 + Duration::from_secs(1)), (true, false));
        let fuera = (10.0, 10.0).into();
        assert_eq!(
            mini.mover_en(fuera, pantalla, t0 + Duration::from_millis(1050)),
            (true, true)
        );

        let t_desc = t0 + Duration::from_millis(1100);
        mini.descartar(t_desc);
        assert!(mini.avanzar(t_desc + Duration::from_secs(2)));
    }

    #[test]
    fn clic_izquierdo_expande_hacia_el_centro_opaco_y_caduca_si_no_hay_visor() {
        let src = vec![180u8; 320 * 180 * 4];
        let pantalla = Rectangle::from_size((1920, 1080).into());
        let mut mini = Miniatura::nueva(
            7,
            &src,
            320,
            180,
            1.0,
            pantalla,
            RecursoCaptura::pendiente(),
            None,
        )
        .expect("crea miniatura");
        let t0 = mini.creada;
        assert!(!mini.avanzar(t0 + Duration::from_secs(1)));

        let repo_rect = mini.rect(pantalla);
        let centro_clic = (
            repo_rect.loc.x as f64 + repo_rect.size.w as f64 * 0.5,
            repo_rect.loc.y as f64 + repo_rect.size.h as f64 * 0.5,
        )
            .into();
        let accion = mini.clic(centro_clic, pantalla, false);
        assert!(matches!(accion, Some(Accion::Abrir { generacion: 7, .. })));
        assert!(mini.esperando_visor());

        let t_clic = Instant::now();
        assert!(!mini.avanzar(t_clic + Duration::from_millis(60)));
        let mid_rect = mini.rect(pantalla);
        assert!(mid_rect.loc.x < repo_rect.loc.x);
        assert!(mid_rect.loc.y < repo_rect.loc.y);
        assert!(mid_rect.size.w > repo_rect.size.w);
        assert!(mid_rect.size.h > repo_rect.size.h);
        // Sigue completamente opaca mientras vuela a acoplarse con el visor.
        assert!((mini.alfa_actual(t_clic + Duration::from_millis(60)) - 1.0).abs() < 0.01);

        // Un segundo clic mientras ya está abriendo no debe relanzar.
        assert!(mini.clic(centro_clic, pantalla, false).is_none());

        // Si ningún visor responde antes de ESPERA_MAX_VISOR, pasa a descartar y se retira.
        let t_timeout = t_clic + ESPERA_MAX_VISOR + Duration::from_millis(20);
        assert!(!mini.avanzar(t_timeout));
        assert!(!mini.esperando_visor());
        assert!(mini.avanzar(t_timeout + Duration::from_secs(1)));
    }

    #[test]
    fn vuelo_desde_origen_destello_arrastre_y_clic_en_pleno_vuelo() {
        let src = vec![180u8; 320 * 180 * 4];
        let pantalla = Rectangle::from_size((1920, 1080).into());
        let origen = Rectangle::new((120, 100).into(), (600, 360).into());
        let mut mini = Miniatura::nueva(
            2,
            &src,
            320,
            180,
            1.0,
            pantalla,
            RecursoCaptura::pendiente(),
            Some(origen),
        )
        .expect("crea miniatura");

        assert!(
            mini.elemento_destello(pantalla, 1.0, mini.creada + Duration::from_millis(20))
                .is_some()
        );
        assert!(
            mini.elemento_destello(pantalla, 1.0, mini.creada + Duration::from_millis(220))
                .is_none()
        );
        let r0 = mini.rect(pantalla);
        assert!(r0.loc.x < 1200 && r0.loc.y < 600);

        // Asentada en la esquina.
        let t_asentada = mini.creada + Duration::from_secs(1);
        assert!(!mini.avanzar(t_asentada));
        let repo = mini.rect(pantalla);
        let inicio: Point<f64, Logical> = (
            repo.loc.x as f64 + repo.size.w as f64 * 0.5,
            repo.loc.y as f64 + repo.size.h as f64 * 0.5,
        )
            .into();

        // Arrastre corto: rebota de vuelta sin descartar.
        assert!(mini.pulsar(inicio, pantalla, false, t_asentada).is_none());
        assert!(mini.arrastrando());
        let corto: Point<f64, Logical> = (inicio.x + 24.0, inicio.y).into();
        assert_eq!(
            mini.mover_en(corto, pantalla, t_asentada + Duration::from_millis(120)),
            (true, false)
        );
        assert!(
            mini.soltar(corto, pantalla, t_asentada + Duration::from_millis(180))
                .is_none()
        );
        assert!(!mini.arrastrando());

        // Arrastre más allá del umbral: descarta al soltar.
        let t2 = t_asentada + Duration::from_millis(400);
        assert!(!mini.avanzar(t2));
        assert!(mini.pulsar(inicio, pantalla, false, t2).is_none());
        let lejos: Point<f64, Logical> = (inicio.x + 80.0, inicio.y).into();
        let _ = mini.mover_en(lejos, pantalla, t2 + Duration::from_millis(80));
        assert!(matches!(
            mini.soltar(lejos, pantalla, t2 + Duration::from_millis(100)),
            Some(Accion::Descartar)
        ));
        assert!(mini.avanzar(t2 + Duration::from_secs(2)));

        // Cancelar en pleno vuelo de apertura corta la espera al visor sin fugas.
        let mut mini2 = Miniatura::nueva(
            3,
            &src,
            320,
            180,
            1.0,
            pantalla,
            RecursoCaptura::pendiente(),
            Some(origen),
        )
        .expect("crea miniatura 2");
        let t_mid = mini2.creada + Duration::from_millis(45);
        assert!(!mini2.avanzar(t_mid));
        let r_mid = mini2.rect(pantalla);
        let p_mid: Point<f64, Logical> = (
            r_mid.loc.x as f64 + r_mid.size.w as f64 * 0.5,
            r_mid.loc.y as f64 + r_mid.size.h as f64 * 0.5,
        )
            .into();
        assert!(mini2.pulsar(p_mid, pantalla, false, t_mid).is_none());
        assert!(matches!(
            mini2.soltar(p_mid, pantalla, t_mid + Duration::from_millis(10)),
            Some(Accion::Abrir { generacion: 3, .. })
        ));
        assert!(mini2.esperando_visor());
        assert!(mini2.cancelar_apertura(t_mid + Duration::from_millis(30)));
        assert!(!mini2.esperando_visor());
        assert!(mini2.avanzar(t_mid + Duration::from_secs(2)));
    }

    #[test]
    fn recurso_captura_despierta_al_publicar_ruta() {
        let rec = RecursoCaptura::pendiente();
        let rec_hilo = rec.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            rec_hilo.publicar(EstadoRecurso::Guardado(PathBuf::from("/tmp/ok.png")));
        });
        assert_eq!(
            rec.esperar(Duration::from_secs(1)),
            EstadoRecurso::Guardado(PathBuf::from("/tmp/ok.png"))
        );
    }

    #[test]
    fn preparar_heroe_crea_buffer_alta_resolucion_con_misma_proporcion() {
        let src = vec![160u8; 1920 * 1080 * 4];
        let grande = preparar_heroe(&src, 1920, 1080, 1.0, false).expect("prepara heroe");
        assert_eq!(grande.lw, 960);
        assert_eq!(grande.lh, 540);
        assert_eq!(grande.bw, 960 + MARGEN_SOMBRA_HEROE * 2);
        assert_eq!(grande.bh, 540 + MARGEN_SOMBRA_HEROE * 2);
        assert_eq!(
            grande.argb.len(),
            grande.pw as usize * grande.ph as usize * 4
        );
    }

    #[test]
    fn cancelar_arrastre_no_abre_ni_descarta_y_descarte_reducido_termina_en_un_paso() {
        struct GuardiaReducidos(bool);
        impl Drop for GuardiaReducidos {
            fn drop(&mut self) {
                tema::aplicar_efectos_reducidos(self.0);
            }
        }
        let _g = GuardiaReducidos(tema::efectos_reducidos());
        tema::aplicar_efectos_reducidos(false);

        let src = vec![180u8; 320 * 180 * 4];
        let pantalla = Rectangle::from_size((1920, 1080).into());
        let mut mini = Miniatura::nueva(
            11,
            &src,
            320,
            180,
            1.0,
            pantalla,
            RecursoCaptura::pendiente(),
            None,
        )
        .expect("crea miniatura");
        let t0 = mini.creada + Duration::from_secs(1);
        assert!(!mini.avanzar(t0));
        let rect = mini.rect(pantalla);
        let inicio: Point<f64, Logical> = (
            rect.loc.x as f64 + rect.size.w as f64 * 0.5,
            rect.loc.y as f64 + rect.size.h as f64 * 0.5,
        )
            .into();
        assert!(mini.pulsar(inicio, pantalla, false, t0).is_none());
        let _ = mini.mover_en((inicio.x + 12.0, inicio.y).into(), pantalla, t0 + Duration::from_millis(20));
        assert!(mini.cancelar_arrastre());
        assert!(!mini.arrastrando());
        assert!(!mini.esperando_visor());

        tema::aplicar_efectos_reducidos(true);
        mini.descartar(t0 + Duration::from_millis(30));
        assert!(mini.avanzar(t0 + Duration::from_millis(30)));
    }
}
