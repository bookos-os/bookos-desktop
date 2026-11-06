//! La banda elástica del escritorio, con las esquinas redondeadas de verdad.
//!
//! Antes eran cinco rectángulos de color: el relleno metido un radio hacia
//! dentro y cuatro filos que no llegaban a las esquinas. Entre el relleno y
//! los filos quedaba una franja sin pintar y las esquinas, vacías: la banda se
//! veía cortada. Una esquina curva no sale con rectángulos, así que va con un
//! shader, como el velo de la captura (`crate::captura::Velo`).
//!
//! El elemento mide lo que la banda y no la pantalla entera: así el damage
//! tracker solo repinta lo que barre el ratón.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::utils::Rectangle;

pub struct Banda {
    elemento: PixelShaderElement,
    /// Lo último que se le pasó, para no subir el contador de commits cuando
    /// no ha cambiado nada.
    ultimo: Option<[f32; 8]>,
}

const FRAGMENTO: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float radio;
uniform float escala;
uniform vec3 acento;

float caja(vec2 p, vec2 centro, vec2 medio, float r) {
    vec2 q = abs(p - centro) - medio + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 p = v_coords * size;
    vec2 medio = size * 0.5;
    float aa = 0.5 / escala;
    float r = min(radio, min(medio.x, medio.y));
    float d = caja(p, medio, medio, r);
    float dentro = 1.0 - smoothstep(-aa, aa, d);
    // Un lógico de filo, por dentro: el elemento no puede pintar fuera de sí.
    float filo = dentro * smoothstep(-1.0 - aa, -1.0 + aa, d);
    float a = max(dentro * 0.14, filo * 0.9);
    gl_FragColor = vec4(acento * a, a) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

impl Banda {
    /// `None` si el driver no compila el shader: la banda sale entonces con
    /// esquinas rectas, de `ShellHost::banda_elementos`.
    // 0.1.1
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        use smithay::backend::renderer::element::Kind;
        use smithay::backend::renderer::gles::{UniformName, UniformType};

        let programa = renderer
            .compile_custom_pixel_shader(
                FRAGMENTO,
                &[
                    UniformName::new("radio", UniformType::_1f),
                    UniformName::new("escala", UniformType::_1f),
                    UniformName::new("acento", UniformType::_3f),
                ],
            )
            .inspect_err(|err| {
                tracing::warn!(
                    "la banda irá con esquinas rectas, el driver no compiló el shader: {err}"
                )
            })
            .ok()?;
        Some(Self {
            elemento: PixelShaderElement::new(
                programa,
                Rectangle::default(),
                None,
                1.0,
                Vec::new(),
                Kind::Unspecified,
            ),
            ultimo: None,
        })
    }

    /// El elemento para la banda `(x, y, ancho, alto)` en lógicos.
    // 0.1.1
    pub fn elemento(&mut self, banda: (f64, f64, f64, f64), escala: f64) -> PixelShaderElement {
        use smithay::backend::renderer::gles::Uniform;

        let (x, y, w, h) = banda;
        let acento = bookos_shell::tema::acento();
        let valores = [
            x as f32,
            y as f32,
            w as f32,
            h as f32,
            escala as f32,
            acento.r,
            acento.g,
            acento.b,
        ];
        if self.ultimo != Some(valores) {
            // El elemento va en lógicos enteros; el shader recibe ese mismo
            // tamaño en `size`, así que la curva no se deforma al redondear.
            let rect = Rectangle::new(
                (x.round() as i32, y.round() as i32).into(),
                (w.round().max(1.0) as i32, h.round().max(1.0) as i32).into(),
            );
            self.elemento.resize(rect, None);
            self.elemento.update_uniforms(vec![
                // El de un chip: la banda es una selección efímera, y con el
                // de un control (14) una banda pequeña se volvía un óvalo.
                Uniform::new("radio", bookos_shell::tema::R_CHIP),
                Uniform::new("escala", escala as f32),
                Uniform::new("acento", (acento.r, acento.g, acento.b)),
            ]);
            self.ultimo = Some(valores);
        }
        self.elemento.clone()
    }
}
