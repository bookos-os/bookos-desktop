//! El minimizar en «magic lamp»: la ventana se estira y se cuela por su icono.
//!
//! ## Por qué hace falta una textura y no vale la superficie del cliente
//!
//! Encoger una ventana es una escala, y eso se hace con la superficie viva del
//! cliente y sin coste (es lo que hacía este mismo minimizar antes). Deformarla
//! —estrecharla por abajo mientras la parte de arriba sigue ancha— no es una
//! escala: cada fila de píxeles va a un ancho distinto. Eso solo se puede hacer
//! leyendo la ventana como una imagen, o sea con la ventana **congelada en una
//! textura** y un shader que decida, para cada píxel de destino, de qué píxel de
//! esa imagen viene.
//!
//! Congelarla no es una pérdida: macOS hace lo mismo, y una terminal que siga
//! imprimiendo mientras se va al dock no aporta nada durante 280 ms.
//!
//! ## La deformación
//!
//! Es un mapeo inverso, que es lo que permite hacerlo en un fragment shader sin
//! mallas ni geometría: se dibuja un rectángulo que cubre desde la ventana hasta
//! el icono y, para cada píxel de ese rectángulo, el shader calcula si le toca
//! algo de la ventana y qué.
//!
//! - Los dos bordes se mueven desde el principio, pero el inferior acelera y el
//!   superior empieza despacio. Así la ventana se estira sin quedarse clavada
//!   a su rectángulo original durante media animación.
//! - El ancho de cada fila interpola entre el de la ventana y el del icono con
//!   un `smoothstep`, que es la curva del cuello del genio.
//! - Verticalmente la ventana se comprime en el tramo que queda, así que se ve
//!   entera todo el rato mientras se aplasta.
//!
//! Si el driver no compila el shader no pasa nada: quien llama se queda con el
//! encogido de siempre, que es una escala normal y corriente.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{AsRenderElements, Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, Frame, Offscreen, Renderer, Texture};
use smithay::desktop::Window;
use smithay::utils::{Buffer as BufferCoords, Logical, Physical, Rectangle, Scale, Transform};

/// GLSL ES 1.00, como el resto de shaders del compositor: es lo que compila
/// Smithay. La línea `//_DEFINES_` es obligatoria —Smithay la sustituye por sus
/// `#define`— y sin ella el shader no compila y no dice por qué.
const FRAGMENTO: &str = r#"#version 100

//_DEFINES_

precision mediump float;
uniform sampler2D tex;
uniform float alpha;
varying vec2 v_coords;

// Cuánto lleva recorrido, de 0 a 1. La vuelta del dock es esto mismo al revés,
// así que el shader no sabe si la ventana va o viene.
uniform float progreso;
// La ventana y el icono dentro del rectángulo que se dibuja, en fracciones de
// ese rectángulo: (x0, y0, x1, y1) y (x0, x1). Van normalizados porque
// `v_coords` también lo está y así el shader no necesita saber cuántos píxeles
// mide nada.
uniform vec4 ventana;
uniform vec2 icono_x;
uniform float icono_y;

void main() {
    vec2 uv = v_coords;
    // Ambos extremos salen en el primer fotograma. El inferior acelera hacia
    // el dock y el superior avanza despacio: mantienen la tensión del efecto
    // sin dejar la parte alta pegada a los bordes durante media animación.
    float avance_bajo = 1.0 - (1.0 - progreso) * (1.0 - progreso);
    float avance_alto = progreso * progreso;
    float bajo = mix(ventana.w, icono_y, avance_bajo);
    float alto = mix(ventana.y, icono_y, avance_alto);
    float tramo = max(bajo - alto, 0.0001);
    if (uv.y < alto || uv.y > bajo) {
        discard;
    }
    // Dónde cae esta fila dentro de lo que queda de ventana.
    float f = (uv.y - alto) / tramo;
    // El shader anterior aplicaba `smoothstep(f)` incluso con progreso=0: el
    // primer fotograma ya era un trapecio y por eso minimizar daba un salto.
    // La fuerza nace de cero y el cuello avanza de arriba abajo. Interpolar
    // centro y semiancho por separado conserva una silueta continua aunque el
    // icono no esté centrado bajo la ventana.
    float fuerza = smoothstep(0.0, 0.22, progreso);
    float fila = smoothstep(0.0, 1.0, f);
    // La base entra primero en el icono, pero la parte alta también abandona
    // gradualmente los bordes. `global` llega a uno al final, de modo que toda
    // la ventana termina dentro del mismo cuello y la vuelta es exactamente la
    // deformación inversa.
    float global = smoothstep(0.0, 1.0, progreso);
    global *= global;
    float cuello = mix(global, 1.0, fila) * fuerza;
    float centro_ventana = (ventana.x + ventana.z) * 0.5;
    float centro_icono = (icono_x.x + icono_x.y) * 0.5;
    float mitad_ventana = (ventana.z - ventana.x) * 0.5;
    float mitad_icono = (icono_x.y - icono_x.x) * 0.5;
    float centro = mix(centro_ventana, centro_icono, cuello);
    float mitad = mix(mitad_ventana, mitad_icono, cuello);
    float x0 = centro - mitad;
    float x1 = centro + mitad;
    if (uv.x < x0 || uv.x > x1) {
        discard;
    }
    // Mapeo inverso: de dónde de la ventana congelada sale este píxel.
    vec2 origen = vec2((uv.x - x0) / max(x1 - x0, 0.0001), f);
    // El desvanecido va al final del recorrido: apagarla desde el principio
    // deja el genio a medias y no se ve la forma.
    // Solo desaparece cuando ya está dentro del icono. Al recorrerlo al revés
    // la ventana empieza a verse enseguida, en vez de gastar el primer cuarto
    // de la apertura siendo completamente transparente.
    float fundido = 1.0 - smoothstep(0.90, 1.0, progreso);
    gl_FragColor = texture2D(tex, origen) * alpha * fundido;
}
"#;

/// El shader compilado, que vive lo que viva la sesión.
pub struct Genio {
    programa: GlesTexProgram,
}

impl Genio {
    /// Compila el shader. `None` si el driver no lo acepta: el minimizar sigue
    /// funcionando, encogiendo la ventana sin deformarla.
    // 0.1.0
    pub fn new(renderer: &mut GlesRenderer) -> Option<Self> {
        if std::env::var_os("BOOKOS_SIN_GENIO").is_some() {
            tracing::info!("magic lamp desactivado por BOOKOS_SIN_GENIO");
            return None;
        }
        match renderer.compile_custom_texture_shader(
            FRAGMENTO,
            &[
                UniformName::new("progreso", UniformType::_1f),
                UniformName::new("ventana", UniformType::_4f),
                UniformName::new("icono_x", UniformType::_2f),
                UniformName::new("icono_y", UniformType::_1f),
            ],
        ) {
            Ok(programa) => Some(Self { programa }),
            Err(err) => {
                tracing::warn!("sin magic lamp, el driver no compiló el shader: {err}");
                None
            }
        }
    }
}

/// Una ventana congelada en una textura, con su tamaño físico.
#[derive(Clone)]
pub struct Captura {
    textura: GlesTexture,
    /// Identificador propio y **estable**: el damage tracker compara elementos
    /// por él, y uno nuevo cada frame le diría que toda la pantalla ha cambiado.
    id: Id,
}

/// Dibuja la ventana en una textura del tamaño que ocupa en pantalla.
///
/// Se llama una sola vez por minimizado, no por frame: es un render completo de
/// la ventana a un framebuffer aparte, y repetirlo sesenta veces por segundo
/// costaría más que la animación entera.
// 0.1.0
pub fn capturar(renderer: &mut GlesRenderer, window: &Window, scale: f64) -> Option<Captura> {
    let geo = window.geometry();
    let tam: smithay::utils::Size<i32, Physical> = (
        ((geo.size.w as f64 * scale).round() as i32).max(1),
        ((geo.size.h as f64 * scale).round() as i32).max(1),
    )
        .into();

    // La ventana se dibuja con su esquina en el origen de la textura: hay que
    // descontar `geometry().loc`, que es el hueco de las sombras del cliente.
    let origen = (
        -(geo.loc.x as f64 * scale).round() as i32,
        -(geo.loc.y as f64 * scale).round() as i32,
    );
    let elementos: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        window.render_elements(renderer, origen.into(), Scale::from(scale), 1.0);

    let mut textura = renderer
        .create_buffer(Fourcc::Abgr8888, (tam.w, tam.h).into())
        .inspect_err(|err| tracing::warn!("no se pudo crear la textura del genio: {err}"))
        .ok()?;
    let completo = Rectangle::from_size(tam);
    {
        let mut fb = renderer.bind(&mut textura).ok()?;
        let mut frame = renderer.render(&mut fb, tam, Transform::Normal).ok()?;
        // Transparente de partida: lo que el cliente no pinte —las esquinas
        // redondeadas de su decoración— tiene que seguir siendo hueco.
        frame
            .clear(
                smithay::backend::renderer::Color32F::TRANSPARENT,
                &[completo],
            )
            .ok()?;
        // Los elementos vienen de delante hacia atrás, así que se dibujan al
        // revés para que lo de delante quede encima.
        for elemento in elementos.iter().rev() {
            let dst = elemento.geometry(Scale::from(scale));
            if let Err(err) = elemento.draw(&mut frame, elemento.src(), dst, &[completo], &[]) {
                tracing::warn!("una superficie no entró en la captura: {err}");
            }
        }
        let _ = frame
            .finish()
            .inspect_err(|err| tracing::warn!("no se pudo cerrar el frame de la captura: {err}"));
    }
    Some(Captura {
        textura,
        id: Id::new(),
    })
}

/// La ventana deformada de este fotograma, lista para la escena.
#[derive(Clone)]
pub struct Elemento {
    id: Id,
    commit: CommitCounter,
    /// El rectángulo que se dibuja: cubre la ventana y el icono, porque el
    /// mapeo inverso necesita poder pintar en cualquier punto del camino.
    rect: Rectangle<i32, Physical>,
    textura: GlesTexture,
    programa: GlesTexProgram,
    progreso: f32,
    /// La ventana y el icono dentro de `rect`, ya en fracciones.
    ventana: [f32; 4],
    icono_x: (f32, f32),
    icono_y: f32,
}

impl Elemento {
    /// `origen` y `destino` van en lógicos, como todo lo que sale del `Space`.
    // 0.1.0
    pub fn new(
        captura: &Captura,
        genio: &Genio,
        commit: CommitCounter,
        origen: Rectangle<i32, Logical>,
        destino: Rectangle<i32, Logical>,
        progreso: f32,
        scale: f64,
    ) -> Self {
        // El rectángulo de dibujo es la caja que contiene a los dos, con un
        // margen: el `smoothstep` del cuello se sale un poco de la recta que une
        // la ventana con el icono, y sin margen se ve cortado.
        let x0 = origen.loc.x.min(destino.loc.x) - 8;
        let y0 = origen.loc.y.min(destino.loc.y) - 8;
        let x1 = (origen.loc.x + origen.size.w).max(destino.loc.x + destino.size.w) + 8;
        let y1 = (origen.loc.y + origen.size.h).max(destino.loc.y + destino.size.h) + 8;
        let (ancho, alto) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
        let frac = |v: i32, cero: i32, total: f32| (v - cero) as f32 / total;

        let rect = Rectangle::new(
            (
                (x0 as f64 * scale).round() as i32,
                (y0 as f64 * scale).round() as i32,
            )
                .into(),
            (
                ((x1 - x0) as f64 * scale).round().max(1.0) as i32,
                ((y1 - y0) as f64 * scale).round().max(1.0) as i32,
            )
                .into(),
        );

        Self {
            id: captura.id.clone(),
            commit,
            rect,
            textura: captura.textura.clone(),
            programa: genio.programa.clone(),
            progreso,
            ventana: [
                frac(origen.loc.x, x0, ancho),
                frac(origen.loc.y, y0, alto),
                frac(origen.loc.x + origen.size.w, x0, ancho),
                frac(origen.loc.y + origen.size.h, y0, alto),
            ],
            icono_x: (
                frac(destino.loc.x, x0, ancho),
                frac(destino.loc.x + destino.size.w, x0, ancho),
            ),
            icono_y: frac(destino.loc.y + destino.size.h / 2, y0, alto),
        }
    }
}

impl Element for Elemento {
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
        // La textura entera: así `v_coords` recorre de 0 a 1 el rectángulo de
        // destino, que es el sistema de coordenadas en el que están escritos los
        // uniforms.
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

impl RenderElement<GlesRenderer> for Elemento {
    // 0.1.0
    fn draw(
        &self,
        frame: &mut <GlesRenderer as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let tam = self.textura.size();
        frame.render_texture_from_to(
            &self.textura,
            Rectangle::from_size((tam.w as f64, tam.h as f64).into()),
            dst,
            damage,
            &[],
            Transform::Normal,
            1.0,
            Some(&self.programa),
            &[
                Uniform::new("progreso", self.progreso),
                Uniform::new("ventana", self.ventana),
                Uniform::new("icono_x", self.icono_x),
                Uniform::new("icono_y", self.icono_y),
            ],
        )
    }
}
