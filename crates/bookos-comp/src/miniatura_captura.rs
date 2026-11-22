//! Miniatura flotante tras hacer una captura de pantalla (estilo macOS).
//!
//! Aparece en la esquina inferior derecha durante unos segundos con la imagen
//! recién capturada, esquinas redondeadas de radio `14` (HIG §4/§9), sombra
//! suave (HIG §6) y borde sutil adaptado al tema (HIG §2). Al pasar el ratón
//! por encima se queda quieta y se eleva ligeramente; un clic izquierdo la abre
//! (guardándola antes si iba al portapapeles); clic derecho o deslizarla hacia
//! la derecha la descarta.
//!
//! **Sin coste en reposo.** La miniatura se reduce una sola vez al crearla con
//! filtrado de área (box filter) y sombra pre-horneada en el buffer; las
//! animaciones usan solo transformación y opacidad en GPU (`OverlayElement`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

use bookos_shell::tema;

use crate::cursor::OverlayElement;

/// Cuánto tiempo permanece visible antes de retirarse sola (si no está el ratón encima).
pub const DURACION: Duration = Duration::from_secs(5);
const MAX_ANCHO: u32 = 220;
const MAX_ALTO: u32 = 140;
const MARGEN_DER: i32 = 20;
const MARGEN_ABAJO: i32 = 88;
/// Radio de miniatura del HIG (§4 y §9: 14 px).
const RADIO: f32 = 14.0;
/// Margen reservado alrededor de la miniatura dentro del buffer para la sombra.
const MARGEN_SOMBRA: i32 = 16;

/// Acción que pide una interacción del usuario sobre la miniatura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    Abrir(PathBuf),
    GuardarYAbrir {
        rgba: Vec<u8>,
        ancho: u32,
        alto: u32,
    },
    Descartar,
}

/// Estado de la miniatura flotante activa.
pub struct Miniatura {
    buffer: MemoryRenderBuffer,
    tamano_logico: Size<i32, Logical>,
    tamano_buffer_logico: Size<i32, Logical>,
    tamano_fisico: Size<i32, Logical>,
    escala: f64,
    /// Si ya está guardada en disco, su ruta; si vino del portapapeles, `None`
    /// y guardamos los píxeles originales para poder abrirla al pulsar.
    ruta: Option<PathBuf>,
    original: Option<(Vec<u8>, u32, u32)>,
    creada: Instant,
    puntero_encima: bool,
    saliendo: bool,
    /// Progreso visual `[0.0 = fuera, 1.0 = visible]`.
    progreso: tema::Muelle,
    /// Progreso de realce al señalar `[0.0 = reposo, 1.0 = encima]`.
    hover: tema::Muelle,
}

impl Miniatura {
    /// Crea la miniatura reduciendo `rgba` (`ancho × alto`) con filtro de área,
    /// esquinas redondeadas `r14`, borde sutil según tema y sombra suave.
    // añadido en la 0.2.4
    pub fn nueva(
        rgba: &[u8],
        ancho: u32,
        alto: u32,
        escala: f64,
        ruta: Option<PathBuf>,
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
        let mut progreso = tema::Muelle::nuevo(0.0, tema::M_SUAVE);
        progreso.ir_a(1.0);
        let hover = tema::Muelle::nuevo(0.0, tema::M_RAPIDO);
        let original = if ruta.is_none() {
            Some((rgba.to_vec(), ancho, alto))
        } else {
            None
        };
        Some(Self {
            buffer,
            tamano_logico: (lw as i32, lh as i32).into(),
            tamano_buffer_logico: (bw as i32, bh as i32).into(),
            tamano_fisico: (pw as i32, ph as i32).into(),
            escala,
            ruta,
            original,
            creada: Instant::now(),
            puntero_encima: false,
            saliendo: false,
            progreso,
            hover,
        })
    }

    fn desplazamiento(&self) -> (f64, f64) {
        let p = self.progreso.valor().clamp(0.0, 1.0) as f64;
        let h = self.hover.valor().clamp(0.0, 1.0) as f64;
        if self.saliendo {
            ((1.0 - p) * 44.0, -2.0 * h)
        } else {
            ((1.0 - p) * 24.0, (1.0 - p) * 20.0 - 2.0 * h)
        }
    }

    fn escala_y_alfa(&self) -> (f64, f32) {
        if tema::efectos_reducidos() {
            let p = self.progreso.valor().clamp(0.0, 1.0);
            return (1.0, p);
        }
        let p = self.progreso.valor().clamp(0.0, 1.0);
        let h = self.hover.valor().clamp(0.0, 1.0) as f64;
        let escala_base = if self.saliendo {
            0.92 + 0.08 * p as f64
        } else {
            0.84 + 0.16 * p as f64
        };
        let escala_anim = escala_base * (1.0 + 0.035 * h);
        let alfa = if self.saliendo { p } else { p.powf(0.75) };
        (escala_anim, alfa.clamp(0.0, 1.0))
    }

    /// Rectángulo lógico actual en pantalla (zona interactiva de la tarjeta).
    // añadido en la 0.2.4
    pub fn rect(&self, pantalla: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let (dx, dy) = self.desplazamiento();
        let x =
            pantalla.loc.x + pantalla.size.w - self.tamano_logico.w - MARGEN_DER + dx.round() as i32;
        let y = pantalla.loc.y + pantalla.size.h - self.tamano_logico.h - MARGEN_ABAJO
            + dy.round() as i32;
        Rectangle::new((x, y).into(), self.tamano_logico)
    }

    /// Avanza los muelles y comprueba si ya caducó. Devuelve `true` si debe retirarse.
    // añadido en la 0.2.4
    pub fn avanzar(&mut self, ahora: Instant) -> bool {
        if !self.saliendo && !self.puntero_encima && ahora.duration_since(self.creada) >= DURACION {
            self.descartar(ahora);
        }
        self.progreso.avanzar(ahora);
        self.hover.avanzar(ahora);
        self.saliendo && self.progreso.quieto() && self.progreso.valor() <= 0.02
    }

    // añadido en la 0.2.4
    pub fn animando(&self) -> bool {
        !self.progreso.quieto() || !self.hover.quieto()
    }

    // añadido en la 0.2.4
    pub fn descartar(&mut self, _ahora: Instant) {
        if !self.saliendo {
            self.saliendo = true;
            self.progreso.poner_parametros(tema::M_RAPIDO);
            self.progreso.ir_a(0.0);
        }
    }

    /// Actualiza si el puntero está encima (pausa el temporizador y anima el hover).
    /// Devuelve `(cambio_visual, programar_retirada)`.
    // añadido en la 0.2.4
    pub fn mover(&mut self, pos: Point<f64, Logical>, pantalla: Rectangle<i32, Logical>) -> (bool, bool) {
        let r = self.rect(pantalla);
        let dentro = pos.x >= r.loc.x as f64
            && pos.x <= (r.loc.x + r.size.w) as f64
            && pos.y >= r.loc.y as f64
            && pos.y <= (r.loc.y + r.size.h) as f64;
        if dentro != self.puntero_encima {
            self.puntero_encima = dentro;
            self.hover.ir_a(if dentro { 1.0 } else { 0.0 });
            let mut programar_retirada = false;
            if !dentro && !self.saliendo {
                // Al salir el ratón, darle 2 s de margen antes de retirarse.
                self.creada = Instant::now() - DURACION.saturating_sub(Duration::from_secs(2));
                programar_retirada = true;
            }
            return (true, programar_retirada);
        }
        (false, false)
    }

    /// Atiende un clic sobre la pantalla. Devuelve `Some(Accion)` si el clic
    /// cayó dentro de la miniatura.
    // añadido en la 0.2.4
    pub fn clic(
        &mut self,
        pos: Point<f64, Logical>,
        pantalla: Rectangle<i32, Logical>,
        derecho: bool,
    ) -> Option<Accion> {
        let r = self.rect(pantalla);
        let dentro = pos.x >= r.loc.x as f64
            && pos.x <= (r.loc.x + r.size.w) as f64
            && pos.y >= r.loc.y as f64
            && pos.y <= (r.loc.y + r.size.h) as f64;
        if !dentro {
            return None;
        }
        self.descartar(Instant::now());
        if derecho {
            return Some(Accion::Descartar);
        }
        if let Some(ruta) = self.ruta.clone() {
            Some(Accion::Abrir(ruta))
        } else if let Some((rgba, ancho, alto)) = self.original.take() {
            Some(Accion::GuardarYAbrir { rgba, ancho, alto })
        } else {
            Some(Accion::Descartar)
        }
    }

    /// Elemento listo para componer en el renderer GLES (con escala y opacidad en GPU).
    // añadido en la 0.2.4
    pub fn elemento(
        &self,
        renderer: &mut GlesRenderer,
        pantalla: Rectangle<i32, Logical>,
        escala: f64,
    ) -> Option<OverlayElement> {
        let r = self.rect(pantalla);
        let (escala_anim, alfa) = self.escala_y_alfa();
        if alfa <= 0.005 {
            return None;
        }
        let loc = (
            ((r.loc.x - MARGEN_SOMBRA) as f64 * escala).round(),
            ((r.loc.y - MARGEN_SOMBRA) as f64 * escala).round(),
        );
        let src = Rectangle::from_size(
            (self.tamano_fisico.w as f64, self.tamano_fisico.h as f64).into(),
        );
        let _ = self.escala;
        let el = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc,
            &self.buffer,
            Some(alfa),
            Some(src),
            Some(self.tamano_buffer_logico),
            Kind::Unspecified,
        )
        .ok()?;
        if (escala_anim - 1.0).abs() > 0.001 {
            let centro: Point<i32, Physical> = (
                ((r.loc.x as f64 + r.size.w as f64 * 0.5) * escala).round() as i32,
                ((r.loc.y as f64 + r.size.h as f64 * 0.5) * escala).round() as i32,
            )
                .into();
            Some(OverlayElement::MemoriaEscalada(
                RescaleRenderElement::from_element(el, centro, escala_anim),
            ))
        } else {
            Some(OverlayElement::Memory(el))
        }
    }
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
        // Esquina exterior (0,0) fuera del radio y sombra → alfa 0.
        assert_eq!(out[3], 0);
        // Centro → alfa 255.
        let centro = ((ph as usize / 2) * pw as usize + pw as usize / 2) * 4;
        assert_eq!(out[centro + 3], 255);
        // Debajo de la tarjeta dentro del margen de sombra → alfa suave > 0 y < 255.
        let bajo = ((ph as usize - 6) * pw as usize + pw as usize / 2) * 4;
        assert!(out[bajo + 3] > 0 && out[bajo + 3] < 255);
    }

    #[test]
    fn ciclo_entrada_hover_y_descarte_con_filtro_de_area() {
        let src = vec![180u8; 320 * 180 * 4];
        let mut mini = Miniatura::nueva(&src, 320, 180, 1.0, None).expect("crea miniatura");
        let pantalla = Rectangle::from_size((1920, 1080).into());
        let t0 = Instant::now();
        assert!(mini.animando());
        assert!(!mini.avanzar(t0 + Duration::from_millis(200)));

        // Hover entra y luego sale pidiendo programar retirada en 2 s.
        let r = mini.rect(pantalla);
        let dentro = (r.loc.x as f64 + 10.0, r.loc.y as f64 + 10.0).into();
        assert_eq!(mini.mover(dentro, pantalla), (true, false));
        let fuera = (10.0, 10.0).into();
        assert_eq!(mini.mover(fuera, pantalla), (true, true));

        mini.descartar(t0 + Duration::from_millis(300));
        assert!(mini.avanzar(t0 + Duration::from_secs(2)));
    }
}
