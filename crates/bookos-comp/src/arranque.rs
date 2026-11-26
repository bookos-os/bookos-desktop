//! Pantalla de carga durante el arranque de la sesión gráfica.
//!
//! Nada más abrir KMS —antes de cargar el shell, compilar shaders o
//! decodificar el fondo— enseña el emblema de BookOS y una barra fina sobre
//! negro puro (continuando sin salto el VT en negro). Al terminar la carga
//! completa la barra y se disuelve en GPU hacia el escritorio.

use std::time::{Duration, Instant};

use resvg::tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Transform as SkiaTransform};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Transform};

use crate::cursor::OverlayElement;

use bookos_shell::carga_sesion::{
    ALTO_BARRA as BARRA_ALTO, ANCHO_BARRA as BARRA_ANCHO, DESFASE_Y,
    LOGO as LOGO_LOGICO, PISTA, PROGRESO_INICIAL, PROGRESO_REDUCIDO, RELLENO, SEPARACION,
};

const PROGRESO_ESPERA_MAX: f32 = 0.56;
const DURACION_ESPERA: Duration = Duration::from_millis(450);
const DURACION_LLENADO: Duration = Duration::from_millis(320);
const PAUSA_COMPLETO: Duration = Duration::from_millis(60);
const DURACION_SALIDA: Duration = bookos_shell::tema::D_PAGINA;

const LOGO_SALIDA_LOGICO: i32 = 88;
const DURACION_SALIDA_PANTALLA: Duration = bookos_shell::tema::D_VENTANA;
const PAUSA_SALIDA_PANTALLA: Duration = Duration::from_millis(70);

/// Acción que se ejecutará al completar el fundido elegante de salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotivoSalida {
    CerrarSesion,
    Reiniciar,
    Apagar,
}

/// Pantalla de salida suave al cerrar sesión, reiniciar o apagar.
pub struct PantallaSalida {
    inicio: Instant,
    motivo: MotivoSalida,
    ejecutado: bool,
    escala: f64,
    logo: Option<MemoryRenderBuffer>,
    logo_fisico: (i32, i32),
    fondo_id: Id,
}

fn normalizar_escala(escala: f64) -> f64 {
    if escala.is_finite() { escala } else { 1.0 }.clamp(0.5, 4.0)
}

impl PantallaSalida {
    /// Prepara el fundido a negro con el emblema de BookOS.
    pub fn nueva(motivo: MotivoSalida, escala: f64) -> Self {
        let escala = normalizar_escala(escala);
        let logo_lado = (f64::from(LOGO_SALIDA_LOGICO) * escala).round().max(16.0) as i32;
        let logo = rasterizar_logo(logo_lado as u32).map(|pixmap| {
            MemoryRenderBuffer::from_slice(
                pixmap.data(),
                Fourcc::Abgr8888,
                (logo_lado, logo_lado),
                1,
                Transform::Normal,
                None,
            )
        });
        Self {
            inicio: Instant::now(),
            motivo,
            ejecutado: false,
            escala,
            logo,
            logo_fisico: (logo_lado, logo_lado),
            fondo_id: Id::new(),
        }
    }

    /// ¿Sigue animando la transición hacia negro antes de lanzar la acción?
    pub fn animando(&self) -> bool {
        self.animando_en(Instant::now(), bookos_shell::tema::efectos_reducidos())
    }

    fn animando_en(&self, ahora: Instant, reducidos: bool) -> bool {
        !self.ejecutado
            && !reducidos
            && ahora.saturating_duration_since(self.inicio)
                < DURACION_SALIDA_PANTALLA + PAUSA_SALIDA_PANTALLA
    }

    /// Devuelve la acción una única vez cuando ya se ha cubierto la pantalla de negro.
    pub fn tomar_accion_en(&mut self, ahora: Instant) -> Option<MotivoSalida> {
        if self.ejecutado {
            return None;
        }
        let listo = bookos_shell::tema::efectos_reducidos()
            || ahora.saturating_duration_since(self.inicio)
                >= DURACION_SALIDA_PANTALLA + PAUSA_SALIDA_PANTALLA;
        if listo {
            self.ejecutado = true;
            Some(self.motivo)
        } else {
            None
        }
    }

    fn opacidades_en(&self, ahora: Instant, reducidos: bool) -> (f32, f32) {
        if reducidos {
            return (1.0, 1.0);
        }
        let t = bookos_shell::tema::fraccion(
            ahora.saturating_duration_since(self.inicio),
            DURACION_SALIDA_PANTALLA,
        );
        let alfa_fondo = bookos_shell::tema::C_ENTRADA.eval(t).clamp(0.0, 1.0);
        let t_logo = ((t - 0.22) / 0.78).clamp(0.0, 1.0);
        let alfa_logo = bookos_shell::tema::C_ENTRADA.eval(t_logo).clamp(0.0, 1.0);
        (alfa_fondo, alfa_logo)
    }

    /// Devuelve los elementos de salida ordenados de delante hacia atrás.
    pub fn elementos(
        &self,
        renderer: &mut GlesRenderer,
        origen_logico: Point<i32, Logical>,
        tamano_fisico: (i32, i32),
        escala: f64,
        ahora: Instant,
    ) -> Vec<OverlayElement> {
        let (alfa_fondo, alfa_logo) =
            self.opacidades_en(ahora, bookos_shell::tema::efectos_reducidos());
        if alfa_fondo <= 0.002 {
            return Vec::new();
        }
        let origen_fisico: Point<i32, Physical> = origen_logico
            .to_f64()
            .to_physical_precise_round(escala);
        let mut salida = Vec::with_capacity(2);
        if alfa_logo > 0.005
            && let Some(logo) = self.logo.as_ref()
        {
            let cx = f64::from(origen_fisico.x) + f64::from(tamano_fisico.0) * 0.5;
            let cy = f64::from(origen_fisico.y) + f64::from(tamano_fisico.1) * 0.5;
            let logo_w = f64::from(LOGO_SALIDA_LOGICO) * self.escala;
            let logo_x = (cx - logo_w * 0.5).round();
            let logo_y = (cy - logo_w * 0.5).round();
            if let Ok(el_logo) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (logo_x, logo_y),
                logo,
                Some(alfa_logo),
                Some(Rectangle::from_size(
                    (f64::from(self.logo_fisico.0), f64::from(self.logo_fisico.1)).into(),
                )),
                Some(
                    (
                        (f64::from(LOGO_SALIDA_LOGICO) * self.escala / escala)
                            .round()
                            .max(1.0) as i32,
                        (f64::from(LOGO_SALIDA_LOGICO) * self.escala / escala)
                            .round()
                            .max(1.0) as i32,
                    )
                        .into(),
                ),
                Kind::Unspecified,
            ) {
                salida.push(OverlayElement::Memory(el_logo));
            }
        }
        salida.push(OverlayElement::Color(SolidColorRenderElement::new(
            self.fondo_id.clone(),
            Rectangle::new(origen_fisico, tamano_fisico.into()),
            0,
            [0.0, 0.0, 0.0, alfa_fondo],
            Kind::Unspecified,
        )));
        salida
    }
}

/// Inicia la salida suave si hay sesión gráfica activa, o ejecuta la orden al instante.
pub fn solicitar_salida(state: &mut crate::state::BookosComp, motivo: MotivoSalida) {
    if state.pantalla_salida.is_some() {
        return;
    }
    let escala = state
        .space
        .outputs()
        .next()
        .map(|o| o.current_scale().fractional_scale())
        .or(state.escala_forzada);
    if bookos_shell::tema::efectos_reducidos() || state.modo.kiosko() || escala.is_none() {
        ejecutar_salida(state, motivo);
        return;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
    }
    state.pantalla_salida = Some(PantallaSalida::nueva(motivo, escala.unwrap_or(1.0)));
    state.needs_redraw = true;
}

/// Ejecuta la orden de salida real tras completar el fundido a negro.
pub fn ejecutar_salida(state: &mut crate::state::BookosComp, motivo: MotivoSalida) {
    match motivo {
        MotivoSalida::CerrarSesion => state.loop_signal.stop(),
        MotivoSalida::Reiniciar => crate::keybinds::lanzar(state, "systemctl reboot"),
        MotivoSalida::Apagar => crate::keybinds::lanzar(state, "systemctl poweroff"),
    }
}

#[derive(Debug, Clone, Copy)]
struct EstadoCarga {
    progreso: f32,
    alfa_contenido: f32,
    alfa_fondo: f32,
    terminado: bool,
}

/// Estado visual de la pantalla de carga hasta que el escritorio está listo.
pub struct PantallaCarga {
    inicio: Instant,
    listo_en: Option<(Instant, f32)>,
    escala: f64,
    logo: MemoryRenderBuffer,
    logo_fisico: (i32, i32),
    barra: MemoryRenderBuffer,
    barra_fisico: (i32, i32),
    ultimo_relleno_px: i32,
    fondo_id: Id,
}

impl PantallaCarga {
    /// Prepara el emblema de BookOS y la barra de progreso a la escala de la salida.
    pub fn nuevo(escala: f64) -> Option<Self> {
        let escala = normalizar_escala(escala);
        let logo_lado = (f64::from(LOGO_LOGICO) * escala).round().max(16.0) as i32;
        let logo_pixmap = rasterizar_logo(logo_lado as u32)?;
        let logo = MemoryRenderBuffer::from_slice(
            logo_pixmap.data(),
            Fourcc::Abgr8888,
            (logo_lado, logo_lado),
            1,
            Transform::Normal,
            None,
        );
        let barra_w = (f64::from(BARRA_ANCHO) * escala).round().max(32.0) as i32;
        let barra_h = (f64::from(BARRA_ALTO) * escala).round().max(3.0) as i32;
        let (barra, ultimo_relleno_px) = rasterizar_barra(barra_w, barra_h, PROGRESO_INICIAL)?;
        Some(Self {
            inicio: Instant::now(),
            listo_en: None,
            escala,
            logo,
            logo_fisico: (logo_lado, logo_lado),
            barra,
            barra_fisico: (barra_w, barra_h),
            ultimo_relleno_px,
            fondo_id: Id::new(),
        })
    }

    /// Marca el fin de la inicialización del compositor para completar la barra
    /// y disolver hacia el escritorio.
    pub fn marcar_listo(&mut self) {
        self.marcar_listo_en(Instant::now());
    }

    fn marcar_listo_en(&mut self, ahora: Instant) {
        if self.listo_en.is_none() {
            let p = self.estado_en(ahora, false).progreso;
            self.listo_en = Some((ahora, p));
        }
    }

    /// ¿Ha concluido por completo la carga y su fundido de salida?
    pub fn terminado(&self) -> bool {
        self.terminado_en(Instant::now())
    }

    pub fn terminado_en(&self, ahora: Instant) -> bool {
        self.estado_en(ahora, bookos_shell::tema::efectos_reducidos())
            .terminado
    }

    fn estado_en(&self, ahora: Instant, reducidos: bool) -> EstadoCarga {
        match self.listo_en {
            None => {
                let t = bookos_shell::tema::fraccion(
                    ahora.saturating_duration_since(self.inicio),
                    DURACION_ESPERA,
                );
                let progreso = if reducidos {
                    PROGRESO_REDUCIDO
                } else {
                    PROGRESO_INICIAL
                        + (PROGRESO_ESPERA_MAX - PROGRESO_INICIAL) * bookos_shell::tema::C_ENTRADA.eval(t)
                };
                EstadoCarga {
                    progreso,
                    alfa_contenido: 1.0,
                    alfa_fondo: 1.0,
                    terminado: false,
                }
            }
            Some((desde, base)) => {
                if reducidos {
                    return EstadoCarga {
                        progreso: 1.0,
                        alfa_contenido: 0.0,
                        alfa_fondo: 0.0,
                        terminado: true,
                    };
                }
                let transcurrido = ahora.saturating_duration_since(desde);
                if transcurrido < DURACION_LLENADO {
                    let t = bookos_shell::tema::fraccion(transcurrido, DURACION_LLENADO);
                    let progreso = base + (1.0 - base) * bookos_shell::tema::C_ENTRADA.eval(t);
                    EstadoCarga {
                        progreso,
                        alfa_contenido: 1.0,
                        alfa_fondo: 1.0,
                        terminado: false,
                    }
                } else if transcurrido < DURACION_LLENADO + PAUSA_COMPLETO {
                    EstadoCarga {
                        progreso: 1.0,
                        alfa_contenido: 1.0,
                        alfa_fondo: 1.0,
                        terminado: false,
                    }
                } else {
                    let salida = transcurrido - (DURACION_LLENADO + PAUSA_COMPLETO);
                    let t = bookos_shell::tema::fraccion(salida, DURACION_SALIDA);
                    if t >= 1.0 {
                        EstadoCarga {
                            progreso: 1.0,
                            alfa_contenido: 0.0,
                            alfa_fondo: 0.0,
                            terminado: true,
                        }
                    } else {
                        let alfa = (1.0 - bookos_shell::tema::C_ENTRADA.eval(t)).clamp(0.0, 1.0);
                        EstadoCarga {
                            progreso: 1.0,
                            alfa_contenido: alfa,
                            alfa_fondo: alfa,
                            terminado: false,
                        }
                    }
                }
            }
        }
    }

    /// Devuelve los elementos de la pantalla de carga ordenados de delante hacia atrás.
    pub fn elementos(
        &mut self,
        renderer: &mut GlesRenderer,
        origen_logico: Point<i32, Logical>,
        tamano_fisico: (i32, i32),
        escala: f64,
        ahora: Instant,
    ) -> Vec<OverlayElement> {
        let estado = self.estado_en(ahora, bookos_shell::tema::efectos_reducidos());
        if estado.terminado || estado.alfa_fondo <= 0.002 {
            return Vec::new();
        }
        let relleno_px = (f32::from(self.barra_fisico.0 as i16) * estado.progreso)
            .round()
            .clamp(self.barra_fisico.1 as f32, self.barra_fisico.0 as f32) as i32;
        if relleno_px != self.ultimo_relleno_px
            && let Some((nueva, px)) =
                rasterizar_barra(self.barra_fisico.0, self.barra_fisico.1, estado.progreso)
        {
            self.barra = nueva;
            self.ultimo_relleno_px = px;
        }

        let origen_fisico: Point<i32, Physical> = origen_logico
            .to_f64()
            .to_physical_precise_round(escala);
        let cx = f64::from(origen_fisico.x) + f64::from(tamano_fisico.0) * 0.5;
        let cy = f64::from(origen_fisico.y) + f64::from(tamano_fisico.1) * 0.5;

        let logo_w = f64::from(LOGO_LOGICO) * self.escala;
        let barra_w = f64::from(BARRA_ANCHO) * self.escala;
        let alto_total = f64::from(LOGO_LOGICO + SEPARACION + BARRA_ALTO) * self.escala;
        let logo_x = (cx - logo_w * 0.5).round();
        let logo_y = (cy - alto_total * 0.5 - f64::from(DESFASE_Y) * self.escala).round();
        let barra_x = (cx - barra_w * 0.5).round();
        let barra_y = (logo_y + f64::from(LOGO_LOGICO + SEPARACION) * self.escala).round();

        let mut salida = Vec::with_capacity(3);
        if estado.alfa_contenido > 0.005 {
            if let Ok(el_barra) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (barra_x, barra_y),
                &self.barra,
                Some(estado.alfa_contenido),
                Some(Rectangle::from_size(
                    (f64::from(self.barra_fisico.0), f64::from(self.barra_fisico.1)).into(),
                )),
                Some(
                    (
                        (f64::from(BARRA_ANCHO) * self.escala / escala).round().max(1.0) as i32,
                        (f64::from(BARRA_ALTO) * self.escala / escala).round().max(1.0) as i32,
                    )
                        .into(),
                ),
                Kind::Unspecified,
            ) {
                salida.push(OverlayElement::Memory(el_barra));
            }
            if let Ok(el_logo) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (logo_x, logo_y),
                &self.logo,
                Some(estado.alfa_contenido),
                Some(Rectangle::from_size(
                    (f64::from(self.logo_fisico.0), f64::from(self.logo_fisico.1)).into(),
                )),
                Some(
                    (
                        (f64::from(LOGO_LOGICO) * self.escala / escala).round().max(1.0) as i32,
                        (f64::from(LOGO_LOGICO) * self.escala / escala).round().max(1.0) as i32,
                    )
                        .into(),
                ),
                Kind::Unspecified,
            ) {
                salida.push(OverlayElement::Memory(el_logo));
            }
        }
        salida.push(OverlayElement::Color(SolidColorRenderElement::new(
            self.fondo_id.clone(),
            Rectangle::new(origen_fisico, tamano_fisico.into()),
            0,
            [0.0, 0.0, 0.0, estado.alfa_fondo],
            Kind::Unspecified,
        )));
        salida
    }
}

fn rasterizar_logo(lado_px: u32) -> Option<Pixmap> {
    let (circulo, onda) = bookos_shell::marca::Canal::leer().colores();
    let svg = include_str!("../../bookos-shell/assets/bookos.svg")
        .replace("#A4BBFE", circulo)
        .replace("#7E9FFF", onda);
    let arbol = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(lado_px, lado_px)?;
    let tam = arbol.size();
    let esc = (lado_px as f32 / tam.width().max(1.0)).min(lado_px as f32 / tam.height().max(1.0));
    resvg::render(
        &arbol,
        SkiaTransform::from_scale(esc, esc),
        &mut pixmap.as_mut(),
    );
    Some(pixmap)
}

fn construir_pildora(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    let r = (h * 0.5).min(w * 0.5);
    let k = r * 0.552_284_8;
    let x1 = x + w;
    let y1 = y + h;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - r + k, y, x1, y + r - k, x1, y + r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + r - k, y1, x, y1 - r + k, x, y1 - r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn rasterizar_barra(w: i32, h: i32, progreso: f32) -> Option<(MemoryRenderBuffer, i32)> {
    let mut pixmap = Pixmap::new(w as u32, h as u32)?;
    let fw = w as f32;
    let fh = h as f32;
    if let Some(pista) = construir_pildora(0.0, 0.0, fw, fh) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(PISTA[0], PISTA[1], PISTA[2], PISTA[3]));
        paint.anti_alias = true;
        pixmap.fill_path(&pista, &paint, FillRule::Winding, SkiaTransform::identity(), None);
    }
    let relleno_px = (fw * progreso.clamp(0.0, 1.0)).round().clamp(fh, fw);
    if let Some(relleno) = construir_pildora(0.0, 0.0, relleno_px, fh) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(RELLENO[0], RELLENO[1], RELLENO[2], RELLENO[3]));
        paint.anti_alias = true;
        pixmap.fill_path(&relleno, &paint, FillRule::Winding, SkiaTransform::identity(), None);
    }
    let buffer = MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (w, h),
        1,
        Transform::Normal,
        None,
    );
    Some((buffer, relleno_px as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_logo_y_la_barra_se_rasterizan_sin_error() {
        let pixmap = rasterizar_logo(182).expect("debe rasterizar el logo de BookOS");
        assert!(pixmap.pixels().iter().any(|p| p.alpha() > 200));

        let carga = PantallaCarga::nuevo(1.75).expect("debe construir la pantalla de carga HiDPI");
        assert_eq!(carga.logo_fisico, (182, 182));
        assert_eq!(carga.barra_fisico, (350, 9));
    }

    #[test]
    fn la_carga_avanza_y_termina_tras_marcar_listo() {
        let mut carga = PantallaCarga::nuevo(1.0).unwrap();
        let t0 = carga.inicio;
        let e0 = carga.estado_en(t0, false);
        assert!((e0.progreso - PROGRESO_INICIAL).abs() < 0.01);
        assert_eq!(e0.alfa_fondo, 1.0);
        assert!(!e0.terminado);

        let t_espera = t0 + DURACION_ESPERA;
        let e_espera = carga.estado_en(t_espera, false);
        assert!(e_espera.progreso <= PROGRESO_ESPERA_MAX + 0.001);
        assert!(e_espera.progreso > PROGRESO_INICIAL);

        carga.marcar_listo_en(t_espera);
        let e_completo = carga.estado_en(t_espera + DURACION_LLENADO, false);
        assert!((e_completo.progreso - 1.0).abs() < 0.001);
        assert_eq!(e_completo.alfa_fondo, 1.0);

        let t_mitad_salida = t_espera + DURACION_LLENADO + PAUSA_COMPLETO + DURACION_SALIDA / 2;
        let e_salida = carga.estado_en(t_mitad_salida, false);
        assert!(e_salida.alfa_fondo > 0.0 && e_salida.alfa_fondo < 1.0);
        assert!(!e_salida.terminado);

        let t_fin = t_espera + DURACION_LLENADO + PAUSA_COMPLETO + DURACION_SALIDA + Duration::from_millis(5);
        assert!(carga.estado_en(t_fin, false).terminado);
    }

    #[test]
    fn efectos_reducidos_cierra_al_terminar_sin_fundido() {
        let mut carga = PantallaCarga::nuevo(1.0).unwrap();
        let t = carga.inicio + Duration::from_millis(120);
        assert!(!carga.estado_en(t, true).terminado);
        carga.marcar_listo_en(t);
        assert!(carga.estado_en(t, true).terminado);
    }

    #[test]
    fn la_salida_funde_a_negro_y_solo_dispara_una_vez() {
        let mut salida = PantallaSalida::nueva(MotivoSalida::CerrarSesion, 1.0);
        let t0 = salida.inicio;
        let (f0, l0) = salida.opacidades_en(t0, false);
        assert_eq!(f0, 0.0);
        assert_eq!(l0, 0.0);
        assert!(salida.tomar_accion_en(t0 + Duration::from_millis(100)).is_none());

        let t_fin = t0 + DURACION_SALIDA_PANTALLA + PAUSA_SALIDA_PANTALLA;
        let (f1, l1) = salida.opacidades_en(t_fin, false);
        assert_eq!(f1, 1.0);
        assert_eq!(l1, 1.0);
        assert_eq!(salida.tomar_accion_en(t_fin), Some(MotivoSalida::CerrarSesion));
        assert_eq!(salida.tomar_accion_en(t_fin + Duration::from_millis(20)), None);
    }

    #[test]
    fn escala_no_finita_y_efectos_reducidos_en_salida_no_animan() {
        let carga = PantallaCarga::nuevo(f64::NAN).expect("escala no finita normalizada");
        assert_eq!(carga.escala, 1.0);

        let salida = PantallaSalida::nueva(MotivoSalida::Apagar, f64::NAN);
        assert_eq!(salida.escala, 1.0);
        let t0 = salida.inicio;
        assert!(salida.animando_en(t0, false));
        assert!(!salida.animando_en(t0, true));
    }
}
