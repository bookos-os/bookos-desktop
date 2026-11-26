//! Esquinas redondeadas para las ventanas vivas de la vista de escritorios.
//!
//! Un cliente maximizado se quita sus esquinas redondeadas (es lo que significa
//! `Maximized` en xdg-shell), y en pantalla no se nota porque el panel tapa el
//! borde de arriba. En la miniatura el panel no se pinta y la ventana asomaba
//! con dos esquinas en pico dentro de un marco redondeado.
//!
//! Se recorta en el shader de texturas y no congelando la ventana en una
//! textura (como el genio): la miniatura sigue viva —una terminal que imprime
//! se ve imprimir— y no cuesta un render aparte por fotograma. Mientras se
//! dibuja la ventana, [`Redondeada`] cambia el programa por defecto del frame
//! por este, que descarta lo que cae fuera del rectángulo redondeado.
//!
//! El rectángulo se pasa en coordenadas de `gl_FragCoord`, calculadas con la
//! misma proyección del frame: así da igual cómo esté girada la salida o si el
//! framebuffer va invertido en Y (winit y udev no coinciden en eso).

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{
    CropRenderElement, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform};

/// El de texturas de Smithay (`shaders/implicit/texture.frag`) con el recorte
/// añadido: hay que conservar sus variantes `EXTERNAL`, `NO_ALPHA` y
/// `DEBUG_FLAGS`, que Smithay elige según el formato del buffer del cliente.
const FRAGMENTO: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// (x0, y0, x1, y1) en píxeles de gl_FragCoord.
uniform vec4 recorte;
uniform float radio;

void main() {
    vec2 centro = (recorte.xy + recorte.zw) * 0.5;
    vec2 mitad = (recorte.zw - recorte.xy) * 0.5;
    // Distancia con signo a la caja redondeada: negativa dentro.
    vec2 q = abs(gl_FragCoord.xy - centro) - mitad + vec2(radio);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radio;
    // Medio píxel de suavizado: el borde de la miniatura del fondo también
    // va antialiasado, y uno duro se notaría junto a él.
    float cubre = clamp(0.5 - d, 0.0, 1.0);
    if (cubre <= 0.0) {
        discard;
    }

    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color * cubre;
}
"#;

/// El shader compilado, que vive lo que viva la sesión.
pub struct Esquinas {
    programa: GlesTexProgram,
}

impl Esquinas {
    /// `None` si el driver no lo compila: las miniaturas salen como antes.
    // añadido en la 0.2.3
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        renderer
            .compile_custom_texture_shader(
                FRAGMENTO,
                &[
                    UniformName::new("recorte", UniformType::_4f),
                    UniformName::new("radio", UniformType::_1f),
                ],
            )
            .inspect_err(|err| {
                tracing::warn!("miniaturas sin esquinas redondeadas, el shader no compiló: {err}")
            })
            .ok()
            .map(|programa| Self { programa })
    }
}

/// Lo que sale de `constrain_space_element` para una superficie de la ventana.
pub type Ajustada = CropRenderElement<
    RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
>;

/// La barra de título SSD escalada a su tamaño de miniatura.
pub type BarraEscalada = RescaleRenderElement<MemoryRenderBufferRenderElement<GlesRenderer>>;

/// Elemento interior que recorta [`Redondeada`].
pub enum Contenido {
    Ventana(Ajustada),
    Barra(BarraEscalada),
}

impl Element for Contenido {
    fn id(&self) -> &Id {
        match self {
            Self::Ventana(e) => e.id(),
            Self::Barra(e) => e.id(),
        }
    }

    fn current_commit(&self) -> CommitCounter {
        match self {
            Self::Ventana(e) => e.current_commit(),
            Self::Barra(e) => e.current_commit(),
        }
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        match self {
            Self::Ventana(e) => e.src(),
            Self::Barra(e) => e.src(),
        }
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        match self {
            Self::Ventana(e) => e.geometry(scale),
            Self::Barra(e) => e.geometry(scale),
        }
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        match self {
            Self::Ventana(e) => e.location(scale),
            Self::Barra(e) => e.location(scale),
        }
    }

    fn transform(&self) -> Transform {
        match self {
            Self::Ventana(e) => e.transform(),
            Self::Barra(e) => e.transform(),
        }
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        match self {
            Self::Ventana(e) => e.damage_since(scale, commit),
            Self::Barra(e) => e.damage_since(scale, commit),
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        match self {
            Self::Ventana(e) => e.alpha(),
            Self::Barra(e) => e.alpha(),
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Ventana(e) => e.kind(),
            Self::Barra(e) => e.kind(),
        }
    }
}

impl RenderElement<GlesRenderer> for Contenido {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        match self {
            Self::Ventana(e) => e.draw(frame, src, dst, damage, opaque_regions),
            Self::Barra(e) => e.draw(frame, src, dst, damage, opaque_regions),
        }
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        match self {
            Self::Ventana(e) => e.underlying_storage(renderer),
            Self::Barra(e) => e.underlying_storage(renderer),
        }
    }
}

/// Una superficie de la ventana (o su barra SSD) recortada a `recorte` con
/// `radio` de esquina, los dos en físicos de la salida.
pub struct Redondeada {
    dentro: Contenido,
    programa: GlesTexProgram,
    recorte: Rectangle<i32, Physical>,
    radio: f32,
}

impl Redondeada {
    // añadido en la 0.2.3
    pub fn new(
        dentro: Ajustada,
        esquinas: &Esquinas,
        recorte: Rectangle<i32, Physical>,
        radio: f32,
    ) -> Self {
        Self {
            dentro: Contenido::Ventana(dentro),
            programa: esquinas.programa.clone(),
            recorte,
            radio,
        }
    }

    /// Recorta la barra de título SSD con el mismo `recorte` total (barra +
    /// cuerpo) para redondear sus esquinas superiores sin dejar muescas abajo.
    pub fn barra(
        dentro: BarraEscalada,
        esquinas: &Esquinas,
        recorte: Rectangle<i32, Physical>,
        radio: f32,
    ) -> Self {
        Self {
            dentro: Contenido::Barra(dentro),
            programa: esquinas.programa.clone(),
            recorte,
            radio,
        }
    }
}

impl Element for Redondeada {
    fn id(&self) -> &Id {
        self.dentro.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.dentro.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.dentro.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dentro.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.dentro.location(scale)
    }

    fn transform(&self) -> Transform {
        self.dentro.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.dentro.damage_since(scale, commit)
    }

    /// Ninguna: las esquinas ya no son opacas, y si el damage tracker creyera
    /// que sí, dejaría sin pintar lo que hay detrás de ellas.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.dentro.alpha()
    }

    fn kind(&self) -> Kind {
        self.dentro.kind()
    }
}

impl RenderElement<GlesRenderer> for Redondeada {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let recorte = en_fragcoord(frame.projection(), self.recorte);
        frame.override_default_tex_program(
            self.programa.clone(),
            vec![
                Uniform::new("recorte", recorte),
                Uniform::new("radio", self.radio),
            ],
        );
        let hecho = self.dentro.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        hecho
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.dentro.underlying_storage(renderer)
    }
}

/// Un rectángulo en físicos de la salida, pasado a píxeles de `gl_FragCoord`.
///
/// La proyección lleva de físicos a NDC; `gl_FragCoord` es NDC escalado al
/// viewport, que es el framebuffer sin girar. El tamaño del viewport sale de
/// la propia proyección: cada fila tiene un único término de escala, `2/lado`
/// (en una salida girada 90° está en la otra columna, por eso se suman).
// añadido en la 0.2.3
fn en_fragcoord(m: &[f32; 9], r: Rectangle<i32, Physical>) -> [f32; 4] {
    // Columna mayor (cgmath): la fila i es m[i], m[3 + i], m[6 + i].
    let vp_w = 2.0 / (m[0].abs() + m[3].abs());
    let vp_h = 2.0 / (m[1].abs() + m[4].abs());
    let a_frag = |x: f32, y: f32| {
        let nx = m[0] * x + m[3] * y + m[6];
        let ny = m[1] * x + m[4] * y + m[7];
        ((nx + 1.0) * 0.5 * vp_w, (ny + 1.0) * 0.5 * vp_h)
    };
    let (x0, y0) = (r.loc.x as f32, r.loc.y as f32);
    let (x1, y1) = (x0 + r.size.w as f32, y0 + r.size.h as f32);
    let (ax, ay) = a_frag(x0, y0);
    let (bx, by) = a_frag(x1, y1);
    [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Con la proyección que arma `GlesRenderer::render` para una salida sin
    /// girar de 1000×500 (`flip180 * ortho`, columna mayor), los físicos caen
    /// tal cual en fragcoord: el `flip180` compensa el origen de GL abajo.
    /// Y con la Y al revés (otro backend), el rectángulo se refleja.
    #[test]
    fn el_recorte_cae_donde_se_ve() {
        let m = [
            2.0 / 1000.0,
            0.0,
            0.0,
            0.0,
            2.0 / 500.0,
            0.0,
            -1.0,
            -1.0,
            1.0,
        ];
        let r = Rectangle::new((100, 50).into(), (200, 100).into());
        let cerca = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
        let recto = en_fragcoord(&m, r);
        assert!(cerca(recto, [100.0, 50.0, 300.0, 150.0]), "{recto:?}");
        let invertida = [m[0], 0.0, 0.0, 0.0, -m[4], 0.0, -1.0, 1.0, 1.0];
        let reflejo = en_fragcoord(&invertida, r);
        assert!(cerca(reflejo, [100.0, 350.0, 300.0, 450.0]), "{reflejo:?}");
    }
}
