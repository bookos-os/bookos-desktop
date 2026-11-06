//! El fondo esmerilado que va detrás del panel, el dock y las emergentes.
//!
//! Es el primer efecto que se hace **en la GPU** y no rasterizando en CPU, y el
//! motivo de que se haga aquí y no dentro del shell: para desenfocar hace falta
//! lo que ya está pintado debajo, y eso solo existe en el framebuffer del
//! compositor. El shell dibuja en su propio buffer y no ve el escritorio.
//!
//! ## Cómo se desenfoca sin una segunda pasada
//!
//! Lo caro de un desenfoque gaussiano son las muestras: un radio grande a
//! resolución completa son cientos de lecturas por píxel. Aquí se usa el truco
//! de siempre: se copia la franja del framebuffer a una textura, se generan sus
//! **mipmaps** —que es un promediado por hardware, gratis—, se fija ese nivel
//! como base de la textura y el shader toma nueve muestras sobre él. El
//! resultado es un desenfoque amplio con el coste de nueve lecturas sobre una
//! textura ocho veces más pequeña.
//!
//! El precio es que la copia obliga a repintar la franja siempre que cambie
//! algo debajo, y eso lo sabe el llamante, no este módulo: aquí solo se dice
//! que el elemento cambió cuando se le avisa.
//!
//! ## Por qué la textura vive en el compositor y no en el elemento
//!
//! Los elementos de render se construyen y se tiran en **cada frame**. Una
//! textura de la anchura de la pantalla creada y destruida sesenta veces por
//! segundo es una asignación de GPU por frame; guardándola fuera se crea una
//! vez y se reutiliza mientras la franja no cambie de tamaño.

use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, ffi,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Transform};

/// El shader del desenfoque.
///
/// GLSL ES **1.00**, que es lo que compila Smithay: nada de `textureLod` ni de
/// `textureSize`, que son de la 3.00. El nivel reducido no se pide desde aquí
/// sino con `GL_TEXTURE_BASE_LEVEL` al preparar la textura; ver [`NIVEL`].
///
/// La línea `//_DEFINES_` es obligatoria y va con el guion bajo del final:
/// Smithay la sustituye por sus `#define` antes de compilar, y sin ella —o con
/// el nombre mal escrito— el shader no compila y no dice por qué.
const FRAGMENTO: &str = r#"#version 100

//_DEFINES_

precision mediump float;
uniform sampler2D tex;
uniform float alpha;
varying vec2 v_coords;

// Separación entre muestras, ya en coordenadas de textura: el shader no puede
// preguntar el tamaño de la textura en GLSL ES 1.00.
uniform vec2 paso;
// Cuánto se oscurece el resultado. El cristal de un tema oscuro no es solo el
// fondo borroso: sin bajar el brillo, el texto blanco del panel no se lee
// encima de una ventana clara.
uniform float tinte;
// Radio de las esquinas y tamaño de la zona, ambos en píxeles físicos. Sin
// esto el cristal sale rectangular y asoma por las esquinas del dock, que son
// redondeadas: se veía un rectángulo oscuro alrededor.
uniform float radio;
uniform vec2 tamano;
// Dónde empieza y cuánto ocupa, en coordenadas de textura, el trozo de fondo
// que va detrás de esta zona. Hace falta porque `v_coords` recorre **el trozo
// del fondo**, no el rectángulo que se dibuja: sin normalizar con esto, el
// redondeo se calculaba sobre coordenadas de la imagen entera y el dock salía
// con las esquinas cuadradas.
uniform vec2 uv0;
uniform vec2 uv_tam;

/// Distancia con signo a un rectángulo redondeado centrado en el origen.
/// Negativa dentro, positiva fuera. Es la fórmula de siempre para esto.
float dist_caja(vec2 p, vec2 medio, float r) {
    vec2 q = abs(p) - medio + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    // Nueve muestras: el centro pesa cuatro, las cuatro en cruz dos y las
    // diagonales una. Es un gaussiano de 3x3 sobre un nivel que el hardware ya
    // ha promediado, que es de donde sale el radio grande casi gratis.
    vec4 c = texture2D(tex, v_coords) * 4.0;
    c += texture2D(tex, v_coords + vec2( paso.x, 0.0)) * 2.0;
    c += texture2D(tex, v_coords + vec2(-paso.x, 0.0)) * 2.0;
    c += texture2D(tex, v_coords + vec2( 0.0, paso.y)) * 2.0;
    c += texture2D(tex, v_coords + vec2( 0.0,-paso.y)) * 2.0;
    c += texture2D(tex, v_coords + vec2( paso.x, paso.y));
    c += texture2D(tex, v_coords + vec2(-paso.x, paso.y));
    c += texture2D(tex, v_coords + vec2( paso.x,-paso.y));
    c += texture2D(tex, v_coords + vec2(-paso.x,-paso.y));
    c /= 16.0;

    // El borde se difumina en un píxel: cortar en seco deja las esquinas
    // dentadas, que es lo primero que delata un redondeo hecho a mano.
    vec2 p = (v_coords - uv0) / uv_tam * tamano;
    float d = dist_caja(p - tamano * 0.5, tamano * 0.5, radio);
    float dentro = 1.0 - smoothstep(-1.0, 0.0, d);

    gl_FragColor = vec4(c.rgb * tinte, 1.0) * alpha * dentro;
}
"#;

/// Cuánto se reduce la copia antes de desenfocar.
///
/// Nivel de mipmap que se usa como base al dibujar.
///
/// No es un sesgo del LOD: eso **no funciona aquí**, y costó una prueba
/// averiguarlo. El sesgo solo interviene cuando la textura se minifica, y el
/// cristal se dibuja a tamaño natural, o sea magnificando — el hardware se
/// queda en el nivel 0 y el desenfoque no aparecía por más que se subiera el
/// número. Lo que funciona es que la textura **sea** ya ese nivel: se sube a
/// un octavo de lado y se amplía con filtro lineal. Antes se subía entera y se
/// fijaba `GL_TEXTURE_BASE_LEVEL` en 3, con el mismo resultado, pero los
/// niveles 0 a 2 —que nadie podía leer— ocupaban 27 MB de GTT a 2880×1800.
///
/// 3 es un octavo de lado. Con 2 se lee el texto de detrás a través del dock,
/// que es lo que un cristal no debe dejar hacer; con 4 se pierde el color de
/// lo que tapa y queda un rectángulo gris.
const NIVEL: i32 = 3;

/// Separación de las muestras sobre ese nivel, en texels de ese nivel.
const PASO: f32 = 1.5;

/// Multiplicador del color ya desenfocado.
///
/// Por debajo de 1 oscurece. Hace falta algo, porque el texto blanco del panel
/// tiene que leerse también sobre una ventana clara, pero poco: a 0,55 el
/// cristal se veía casi negro y el color de detrás no llegaba a asomar. El
/// resto del contraste lo pone el propio fondo del panel, que va encima.
const TINTE: f32 = 0.85;

/// El shader compilado y la textura donde se copia el fondo, que sobreviven
/// entre frames.
pub struct Cristal {
    programa: GlesTexProgram,
    /// El fondo del escritorio ya reducido a [`NIVEL`] y subido con mipmaps,
    /// del que sale el desenfoque, junto al tamaño **original** del fondo: las
    /// cuentas de qué trozo queda detrás de cada zona se hacen en ese tamaño.
    ///
    /// Se guarda el `GlesTexture` y no el identificador crudo porque Smithay
    /// libera la textura cuando el último `GlesTexture` que la envuelve se
    /// suelta: quedándonos con uno, sobrevive mientras haya escritorio.
    fondo: Option<(GlesTexture, (i32, i32))>,
}

impl Cristal {
    /// Compila el shader. `None` si el driver no lo acepta: el escritorio
    /// funciona igual sin cristal, con el panel opaco de siempre.
    // 0.1.0
    pub fn new(renderer: &mut GlesRenderer) -> Option<Rc<RefCell<Self>>> {
        // Para medir con y sin, y para salir del paso si un driver lo hace mal.
        if std::env::var_os("BOOKOS_SIN_CRISTAL").is_some() {
            tracing::info!("cristal desactivado por BOOKOS_SIN_CRISTAL");
            return None;
        }
        match renderer.compile_custom_texture_shader(
            FRAGMENTO,
            &[
                smithay::backend::renderer::gles::UniformName::new(
                    "paso",
                    smithay::backend::renderer::gles::UniformType::_2f,
                ),
                smithay::backend::renderer::gles::UniformName::new(
                    "tinte",
                    smithay::backend::renderer::gles::UniformType::_1f,
                ),
                smithay::backend::renderer::gles::UniformName::new(
                    "radio",
                    smithay::backend::renderer::gles::UniformType::_1f,
                ),
                smithay::backend::renderer::gles::UniformName::new(
                    "tamano",
                    smithay::backend::renderer::gles::UniformType::_2f,
                ),
                smithay::backend::renderer::gles::UniformName::new(
                    "uv0",
                    smithay::backend::renderer::gles::UniformType::_2f,
                ),
                smithay::backend::renderer::gles::UniformName::new(
                    "uv_tam",
                    smithay::backend::renderer::gles::UniformType::_2f,
                ),
            ],
        ) {
            Ok(programa) => Some(Rc::new(RefCell::new(Self {
                programa,
                fondo: None,
            }))),
            Err(err) => {
                tracing::warn!("sin desenfoque, el driver no compiló el shader: {err}");
                None
            }
        }
    }

    /// Sube el fondo del escritorio con mipmaps. Se llama al cargarlo y cuando
    /// cambia el fotograma de una animación.
    // 0.1.0
    // modified 0.1.1 -- reutiliza la textura si el tamaño no cambia y rechaza un fondo con menos píxeles de los que dice
    // modified 0.2.0 -- reduce el fondo en CPU antes de subirlo: −27 MB de GTT
    pub fn preparar(&mut self, renderer: &mut GlesRenderer, rgba: &[u8], tam: (i32, i32)) {
        if rgba.len() < (tam.0 * tam.1 * 4) as usize {
            tracing::warn!("el fondo no tiene los píxeles que dice su tamaño");
            return;
        }
        let (reducido, tam_reducido) = reducir(rgba, tam);
        if let Some((ref tex, tam_actual)) = self.fondo
            && tam_actual == tam
        {
            let tex_id = tex.tex_id();
            if renderer
                .with_context(|gl| unsafe {
                    gl.BindTexture(ffi::TEXTURE_2D, tex_id);
                    gl.TexSubImage2D(
                        ffi::TEXTURE_2D,
                        0,
                        0,
                        0,
                        tam_reducido.0,
                        tam_reducido.1,
                        ffi::RGBA,
                        ffi::UNSIGNED_BYTE,
                        reducido.as_ptr() as *const _,
                    );
                    gl.GenerateMipmap(ffi::TEXTURE_2D);
                    gl.BindTexture(ffi::TEXTURE_2D, 0);
                })
                .is_ok()
            {
                return;
            }
        }
        self.fondo = crear_textura(renderer, &reducido, tam_reducido).map(|t| (t, tam));
    }

    /// La textura del fondo, si ya está subida.
    // 0.1.0
    pub fn textura(&self) -> Option<(GlesTexture, (i32, i32))> {
        self.fondo.clone()
    }

    // 0.1.0
    pub fn programa(&self) -> &GlesTexProgram {
        &self.programa
    }
}

/// Promedia bloques de 2^[`NIVEL`] píxeles de lado: lo mismo que hacían los
/// tres primeros mipmaps de la GPU, salvo el redondeo intermedio (±1 de 255).
/// Los bordes de bloque se reparten sobre la imagen entera para que un lado
/// que no es múltiplo de ocho siga cubriendo el fondo completo.
// añadido en la 0.2.0
fn reducir(rgba: &[u8], tam: (i32, i32)) -> (Vec<u8>, (i32, i32)) {
    let (ancho, alto) = (tam.0 as usize, tam.1 as usize);
    let (ancho_r, alto_r) = ((ancho >> NIVEL).max(1), (alto >> NIVEL).max(1));
    let columnas: Vec<(usize, usize)> = (0..ancho_r)
        .map(|x| (x * ancho / ancho_r, (x + 1) * ancho / ancho_r))
        .collect();
    let mut salida = vec![0u8; ancho_r * alto_r * 4];
    let mut suma = vec![0u32; ancho_r * 4];
    for (y, fila_salida) in salida.chunks_exact_mut(ancho_r * 4).enumerate() {
        let (y0, y1) = (y * alto / alto_r, (y + 1) * alto / alto_r);
        suma.fill(0);
        for fila in rgba[y0 * ancho * 4..y1 * ancho * 4].chunks_exact(ancho * 4) {
            for (acumulado, &(x0, x1)) in suma.chunks_exact_mut(4).zip(&columnas) {
                for pixel in fila[x0 * 4..x1 * 4].chunks_exact(4) {
                    for (a, &c) in acumulado.iter_mut().zip(pixel) {
                        *a += c as u32;
                    }
                }
            }
        }
        for ((destino, acumulado), &(x0, x1)) in fila_salida
            .chunks_exact_mut(4)
            .zip(suma.chunks_exact(4))
            .zip(&columnas)
        {
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for (d, &a) in destino.iter_mut().zip(acumulado) {
                *d = ((a + n / 2) / n) as u8;
            }
        }
    }
    (salida, (ancho_r as i32, alto_r as i32))
}

/// Sube el fondo ya reducido a una textura con mipmaps. Los mipmaps siguen
/// haciendo falta aunque casi siempre se magnifique: con `LINEAR_MIPMAP_LINEAR`
/// una textura sin la cadena entera está incompleta y GLES la lee negra.
// 0.1.0
// modified 0.2.0 -- recibe el fondo ya reducido y no fija el nivel base
fn crear_textura(renderer: &mut GlesRenderer, rgba: &[u8], tam: (i32, i32)) -> Option<GlesTexture> {
    if rgba.len() < (tam.0 * tam.1 * 4) as usize {
        tracing::warn!("el fondo no tiene los píxeles que dice su tamaño");
        return None;
    }
    let id = renderer
        .with_context(|gl| unsafe {
            let mut id = 0;
            gl.GenTextures(1, &mut id);
            gl.BindTexture(ffi::TEXTURE_2D, id);
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_MIN_FILTER,
                ffi::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            // Sin esto, las muestras del borde traen el color del lado contrario
            // y el cristal sale con una raya de otro color en cada canto.
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexImage2D(
                ffi::TEXTURE_2D,
                0,
                ffi::RGBA as i32,
                tam.0,
                tam.1,
                0,
                ffi::RGBA,
                ffi::UNSIGNED_BYTE,
                rgba.as_ptr() as *const _,
            );
            gl.GenerateMipmap(ffi::TEXTURE_2D);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            id
        })
        .ok()?;
    // `false` en `opaque` es mentira piadosa: el shader escribe alfa 1, pero
    // decirle a Smithay que la textura es opaca le hace saltarse el blend y el
    // panel de encima dejaría de mezclarse.
    Some(unsafe { GlesTexture::from_raw(renderer, None, false, id, (tam.0, tam.1).into()) })
}

/// Un trozo de pantalla desenfocado, listo para meterse en la escena.
///
/// Se coloca **justo debajo** del elemento que lo usa de fondo: se dibuja
/// después de las ventanas —de ahí saca lo que desenfoca— y antes del panel.
#[derive(Clone)]
pub struct Desenfoque {
    id: Id,
    commit: CommitCounter,
    /// La zona a esmerilar, en físicos y relativa a la salida.
    rect: Rectangle<i32, Physical>,
    /// Radio de las esquinas, en píxeles físicos. El panel va a cero —llega a
    /// los bordes de la pantalla— y el dock al suyo, que flota.
    radio: f32,
    /// Tamaño del fondo en píxeles y de la pantalla en físicos: con los dos se
    /// sabe qué trozo de la imagen queda detrás de esta zona.
    fondo_tam: (i32, i32),
    pantalla: (i32, i32),
    /// Cómo cae el fondo en la pantalla: sin él, el cristal desenfocaría otro
    /// trozo de imagen que el que se ve alrededor de la barra.
    encuadre: crate::fondo::Encuadre,
    textura: GlesTexture,
    cristal: Rc<RefCell<Cristal>>,
    /// Multiplicador de [`PASO`] para esta zona.
    ///
    /// El panel y el dock van a 1: son franjas estrechas sobre las que hay que
    /// leer texto, y difuminar más los deja grises. El launchpad tapa la
    /// pantalla entera y ahí el fondo tiene que desaparecer de verdad, no
    /// quedarse a medio reconocer detrás de los iconos.
    fuerza: f32,
}

impl Desenfoque {
    // Un argumento por campo: una estructura intermedia solo repetiría esta
    // misma lista.
    #[allow(clippy::too_many_arguments)]
    // 0.1.0
    // modificado en la 0.2.0 -- el cristal sigue al encaje elegido para el fondo
    pub fn new(
        id: Id,
        commit: CommitCounter,
        rect: Rectangle<i32, Physical>,
        radio: f32,
        fondo_tam: (i32, i32),
        pantalla: (i32, i32),
        encuadre: crate::fondo::Encuadre,
        textura: GlesTexture,
        cristal: Rc<RefCell<Cristal>>,
        fuerza: f32,
    ) -> Self {
        Self {
            id,
            commit,
            rect,
            radio,
            fondo_tam,
            pantalla,
            encuadre,
            textura,
            cristal,
            fuerza,
        }
    }
}

impl Element for Desenfoque {
    // 0.1.0
    fn id(&self) -> &Id {
        &self.id
    }

    // 0.1.0
    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    // 0.1.0
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((self.rect.size.w as f64, self.rect.size.h as f64).into())
    }

    // 0.1.0
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.rect
    }

    // 0.1.0
    fn location(&self, _scale: Scale<f64>) -> smithay::utils::Point<i32, Physical> {
        self.rect.loc
    }

    // 0.1.0
    fn transform(&self) -> Transform {
        Transform::Normal
    }

    // 0.1.0
    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for Desenfoque {
    // 0.1.0
    fn draw(
        &self,
        frame: &mut <GlesRenderer as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let cristal = self.cristal.clone();
        let cristal = cristal.borrow();
        let tam = (dst.size.w.max(1), dst.size.h.max(1));

        // El trozo del fondo que queda justo detrás de esta zona, según cómo
        // esté encajado en la pantalla.
        let (ancho_fondo, alto_fondo) = (self.fondo_tam.0 as f64, self.fondo_tam.1 as f64);
        let (ancho_pantalla, alto_pantalla) = (self.pantalla.0 as f64, self.pantalla.1 as f64);
        let detras = self.encuadre.detras_de(crate::fondo::Trozo {
            x: dst.loc.x as f64 / ancho_pantalla,
            y: dst.loc.y as f64 / alto_pantalla,
            w: tam.0 as f64 / ancho_pantalla,
            h: tam.1 as f64 / alto_pantalla,
        });
        let src: Rectangle<f64, BufferCoords> = Rectangle::new(
            (detras.x * ancho_fondo, detras.y * alto_fondo).into(),
            (detras.w * ancho_fondo, detras.h * alto_fondo).into(),
        );

        // La textura está reducida: `src` va en píxeles del fondo original y
        // Smithay lo quiere en los de la textura.
        let reducida = self.textura.size();
        let (rx, ry) = (
            reducida.w as f64 / ancho_fondo,
            reducida.h as f64 / alto_fondo,
        );
        frame.render_texture_from_to(
            &self.textura,
            Rectangle::new(
                (src.loc.x * rx, src.loc.y * ry).into(),
                (src.size.w * rx, src.size.h * ry).into(),
            ),
            dst,
            damage,
            &[],
            Transform::Normal,
            1.0,
            Some(cristal.programa()),
            &[
                // De texels del nivel base a coordenadas de textura. Las
                // coordenadas siguen siendo las de la textura completa, así que
                // un texel del nivel 3 son ocho de la original.
                Uniform::new(
                    "paso",
                    (
                        PASO * self.fuerza * (1 << NIVEL) as f32 / ancho_fondo as f32,
                        PASO * self.fuerza * (1 << NIVEL) as f32 / alto_fondo as f32,
                    ),
                ),
                Uniform::new("tinte", TINTE),
                Uniform::new("radio", self.radio),
                Uniform::new("tamano", (tam.0 as f32, tam.1 as f32)),
                Uniform::new(
                    "uv0",
                    (
                        (src.loc.x / ancho_fondo) as f32,
                        (src.loc.y / alto_fondo) as f32,
                    ),
                ),
                Uniform::new(
                    "uv_tam",
                    (
                        (src.size.w / ancho_fondo) as f32,
                        (src.size.h / alto_fondo) as f32,
                    ),
                ),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // añadido en la 0.2.0
    #[test]
    fn reducir_promedia_bloques_de_ocho() {
        // Mitad izquierda negra y derecha blanca: cada bloque cae entero en un
        // lado, así que no puede salir ningún gris.
        let (ancho, alto) = (32, 16);
        let rgba: Vec<u8> = (0..ancho * alto)
            .flat_map(|i| {
                if i % ancho < ancho / 2 {
                    [0, 0, 0, 255]
                } else {
                    [255; 4]
                }
            })
            .collect();
        let (salida, tam) = reducir(&rgba, (ancho as i32, alto as i32));
        assert_eq!(tam, (4, 2));
        let fila: Vec<u8> = salida[..16].iter().step_by(4).copied().collect();
        assert_eq!(fila, [0, 0, 255, 255]);
    }

    // añadido en la 0.2.0
    #[test]
    fn reducir_con_lados_que_no_son_multiplo_de_ocho() {
        // 2880×1800 es múltiplo; un monitor de 1366 de ancho no. El color
        // uniforme tiene que salir intacto y cubrir todo el fondo.
        let (ancho, alto) = (1366, 7);
        let rgba = [10u8, 20, 30, 255].repeat(ancho * alto);
        let (salida, tam) = reducir(&rgba, (ancho as i32, alto as i32));
        assert_eq!(tam, (170, 1));
        assert!(salida.chunks_exact(4).all(|p| p == [10, 20, 30, 255]));
    }
}
