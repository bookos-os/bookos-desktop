//! Salvapantallas sobre negro puro, con el escritorio trabajando debajo. Dos
//! estilos (`salvapantallas_estilo`):
//!
//! - **Estela**: ondas de luz que atraviesan una malla arqueada de puntos,
//!   inspiradas en el salvapantallas de las teles Samsung OLED.
//! - **Rebote**: BookOS rebotando contra los bordes como el logo de los DVD.

use std::time::{Duration, Instant};

use base64::Engine as _;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesRenderer, GlesTexture, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::{ContextId, Renderer};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Rectangle, Transform};

use bookos_shell::EstiloSalvapantallas as Estilo;

use crate::cursor::OverlayElement;
use crate::state::BookosComp;

/// Justo lo que miden las letras a 112 px de alto, más uno por lado para el
/// suavizado. Con 640 × 160 sobraban 125 px vacíos a cada lado y el texto
/// rebotaba lejos del borde.
const ANCHO: i32 = 392;
const ALTO: i32 = 114;
/// Puntos lógicos por segundo en cada eje: cruza un panel de 1440 de ancho en
/// unos siete segundos, lo bastante vivo sin que se pierda de vista.
const VELOCIDAD: f64 = 160.0;
/// Un color por rebote. Se rasterizan todos al activar porque la textura de
/// Smithay no admite teñirse al pintar; son 400 KB cada una.
const COLORES: [&str; 6] = [
    "#A4BBFE", "#FF6B8B", "#FFD166", "#5EE6A8", "#C78BFF", "#5CD3FF",
];

// Las fotos muestran verde, menta, crema y blanco sobre puntos residuales
// rojizos. El color sigue el frente de onda, no la posición fija en la hilera.
const DEGRADADOS: [[[f32; 3]; 5]; 3] = [
    [
        [0.75, 0.03, 0.09],
        [1.0, 0.86, 0.37],
        [0.18, 1.0, 0.48],
        [0.12, 1.0, 0.82],
        [0.95, 0.93, 1.0],
    ],
    [
        [0.48, 0.03, 0.20],
        [1.0, 0.73, 0.45],
        [0.31, 1.0, 0.53],
        [0.25, 0.95, 1.0],
        [0.92, 0.90, 1.0],
    ],
    [
        [0.65, 0.04, 0.07],
        [1.0, 0.91, 0.48],
        [0.40, 1.0, 0.38],
        [0.08, 1.0, 0.74],
        [0.98, 0.96, 1.0],
    ],
];
const HILERAS: usize = 13;
const MAX_POR_HILERA: usize = 29;
// 269 puntos frente a los 121 anteriores, dentro del mismo conjunto de
// 460 × 340 por cada 1080 píxeles de lado menor, también en HiDPI.
const PASO: f64 = 18.0;
const SEPARACION: f64 = 38.0 * 2.0 / 3.0;
const DISCO: f64 = 230.0;
const RADIO_GRUPO: f64 = 350.0;
const APARICION: f64 = 12.0;
const ENTRADA: f64 = 1.5;
const SALIDA: f64 = 1.5;
const PAUSA: f64 = 1.0;

// El radio fraccional vive en el shader: redondear el diámetro lógico hacía
// saltar el tamaño de los puntos, especialmente con escala de pantalla 1,75.
const FRAGMENTO_PUNTO: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;
uniform vec2 centro;
uniform float radio;
uniform float suavizado;
uniform vec4 luz;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
void main() {
    float d = length(v_coords * size - centro);
    float nucleo = 1.0 - smoothstep(max(0.0, radio - suavizado), radio + suavizado, d);
    float halo = 0.13 * exp(-3.5 * d * d / (radio * radio));
    halo *= 1.0 - smoothstep(radio, radio * 1.8 + suavizado, d);
    float cobertura = max(nucleo, halo) * luz.a;
    float brillo = 0.12 * exp(-2.5 * d * d / (radio * radio));
    gl_FragColor = vec4(mix(luz.rgb, vec3(1.0), brillo) * cobertura, cobertura) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

enum Modo {
    Escritorio,
    PantallaApagada,
    Salvapantallas {
        origen: Origen,
        desde: Instant,
        escena: Escena,
        negro: Id,
    },
}

/// Las texturas de cada estilo, una por color.
enum Escena {
    Rebote {
        letras: Vec<MemoryRenderBuffer>,
    },
    Estela {
        // Un elemento por hueco conserva su identidad y sus commits de daño
        // cuando cambian radio y color aunque la geometría quede en su sitio.
        puntos: Option<(ContextId<GlesTexture>, Vec<Disco>)>,
    },
}

struct Disco {
    elemento: PixelShaderElement,
    ultimo: Option<Punto>,
}

/// Quién lo puso decide qué lo quita. Puesto a mano, solo el mismo atajo: es
/// un «déjalo así» deliberado. Por inactividad, como en una tele, cualquier
/// tecla o movimiento: si no, volver al ordenador obligaría a saberse el atajo
/// para llegar a la contraseña del bloqueo que tiene debajo. La vista previa
/// también sale con la entrada, para poder volver directamente a Ajustes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origen {
    Atajo,
    Inactividad,
    VistaPrevia,
}

pub struct Estado {
    modo: Modo,
    pub estilo: Estilo,
    pub inactividad: Duration,
    /// Inactividad tras la que se apaga la pantalla, con o sin salvapantallas.
    pub apagado: Duration,
    alarma: Option<RegistrationToken>,
    alarma_apagado: Option<RegistrationToken>,
    fotograma: Option<RegistrationToken>,
}

impl Estado {
    pub fn nuevo(segundos: u64, apagado: u64, estilo: Estilo) -> Self {
        Self {
            modo: Modo::Escritorio,
            estilo,
            inactividad: Duration::from_secs(segundos),
            apagado: Duration::from_secs(apagado),
            alarma: None,
            alarma_apagado: None,
            fotograma: None,
        }
    }

    pub fn activo(&self) -> bool {
        matches!(self.modo, Modo::Salvapantallas { .. })
    }

    pub fn pantalla_apagada(&self) -> bool {
        matches!(self.modo, Modo::PantallaApagada)
    }

    pub fn elementos(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
    ) -> Vec<OverlayElement> {
        let Modo::Salvapantallas {
            desde,
            origen,
            escena,
            negro,
        } = &mut self.modo
        else {
            return Vec::new();
        };
        let Some(modo) = output.current_mode() else {
            return Vec::new();
        };
        let escala = output.current_scale().fractional_scale();
        let pantalla = output.current_transform().transform_size(modo.size);
        // En Ajustes interesa ver el dibujo desde el primer fotograma, sin
        // esperar el negro con el que empieza una aparición por inactividad.
        let t = desde.elapsed().as_secs_f64()
            + if *origen == Origen::VistaPrevia {
                2.0
            } else {
                0.0
            };
        let reducidos = bookos_shell::tema::efectos_reducidos();
        let mut elementos = Vec::new();
        match escena {
            Escena::Rebote { letras } => {
                let (x, y, ancho, alto, rebotes) =
                    geometria((pantalla.w, pantalla.h), escala, t, reducidos);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (x, y),
                    &letras[rebotes % letras.len()],
                    None,
                    Some(Rectangle::from_size(
                        (f64::from(ANCHO), f64::from(ALTO)).into(),
                    )),
                    Some((ancho, alto).into()),
                    Kind::Unspecified,
                ) {
                    Ok(letras) => elementos.push(OverlayElement::Memory(letras)),
                    Err(err) => tracing::warn!("no se pudo dibujar el salvapantallas: {err}"),
                }
            }
            Escena::Estela { puntos } => {
                let contexto = renderer.context_id();
                if puntos.as_ref().is_none_or(|(c, _)| *c != contexto) {
                    let programa = renderer.compile_custom_pixel_shader(
                        FRAGMENTO_PUNTO,
                        &[
                            UniformName::new("centro", UniformType::_2f),
                            UniformName::new("radio", UniformType::_1f),
                            UniformName::new("suavizado", UniformType::_1f),
                            UniformName::new("luz", UniformType::_4f),
                        ],
                    );
                    let nuevos = match programa {
                        Ok(programa) => (0..HILERAS * MAX_POR_HILERA)
                            .map(|_| Disco {
                                elemento: PixelShaderElement::new(
                                    programa.clone(),
                                    Rectangle::default(),
                                    None,
                                    1.0,
                                    Vec::new(),
                                    Kind::Unspecified,
                                ),
                                ultimo: None,
                            })
                            .collect(),
                        Err(err) => {
                            tracing::warn!("no se pudo preparar la onda del salvapantallas: {err}");
                            Vec::new()
                        }
                    };
                    // Si el driver rechaza el shader, queda el negro de fondo
                    // y no se recompila ni se repite el aviso en cada fotograma.
                    *puntos = Some((contexto, nuevos));
                }
                let (_, puntos) = puntos.as_mut().expect("puntos preparados justo arriba");
                if !puntos.is_empty() {
                    for p in estela((pantalla.w, pantalla.h), escala, t, reducidos) {
                        let disco = &mut puntos[p.hueco];
                        // Smithay incrementa el commit al actualizar uniforms;
                        // con efectos reducidos el mismo punto dura un minuto.
                        if disco.ultimo != Some(p) {
                            let margen = p.radio * 1.8 + 1.0 / escala;
                            let izquierda = (p.x / escala - margen).floor();
                            let arriba = (p.y / escala - margen).floor();
                            let lado = (margen * 2.0).ceil() as i32 + 1;
                            disco.elemento.resize(
                                Rectangle::new(
                                    (izquierda as i32, arriba as i32).into(),
                                    (lado, lado).into(),
                                ),
                                None,
                            );
                            disco.elemento.update_uniforms(vec![
                                Uniform::new(
                                    "centro",
                                    (
                                        (p.x / escala - izquierda) as f32,
                                        (p.y / escala - arriba) as f32,
                                    ),
                                ),
                                Uniform::new("radio", p.radio as f32),
                                Uniform::new("suavizado", (0.65 / escala) as f32),
                                Uniform::new("luz", (p.color[0], p.color[1], p.color[2], p.alfa)),
                            ]);
                            disco.ultimo = Some(p);
                        }
                        elementos.push(OverlayElement::Velo(disco.elemento.clone()));
                    }
                }
            }
        }
        elementos.push(OverlayElement::Color(SolidColorRenderElement::new(
            negro.clone(),
            Rectangle::from_size(pantalla),
            0,
            [0.0, 0.0, 0.0, 1.0],
            Kind::Unspecified,
        )));
        elementos
    }
}

fn geometria(
    (w, h): (i32, i32),
    escala: f64,
    segundos: f64,
    reducidos: bool,
) -> (f64, f64, i32, i32, usize) {
    let ancho = 240.0_f64
        .min(f64::from(w) / escala * 0.7)
        .min(f64::from(h) / escala * 2.0)
        .max(1.0)
        .round();
    // Redondeado antes de calcular el hueco: se pinta con el tamaño entero y
    // medio píxel de más asomaba por el borde.
    let alto = (ancho * f64::from(ALTO) / f64::from(ANCHO))
        .max(1.0)
        .round();
    let libre_x = (f64::from(w) - ancho * escala).max(0.0);
    let libre_y = (f64::from(h) - alto * escala).max(0.0);
    // En efectos reducidos también cambia de sitio para evitar una marca fija,
    // pero solo una vez por minuto, sin movimiento continuo.
    let t = if reducidos {
        (segundos / 60.0).floor() * 60.0
    } else {
        segundos
    };
    let recorrido = VELOCIDAD * escala * t;
    // Desfase distinto por eje para no arrancar en una esquina y que la
    // diagonal no repita siempre el mismo camino.
    let (x, rebotes_x) = rebote(recorrido + libre_x * 0.3, libre_x);
    let (y, rebotes_y) = rebote(recorrido + libre_y * 0.7, libre_y);
    (x, y, ancho as i32, alto as i32, rebotes_x + rebotes_y)
}

/// Centro físico y radio lógico: el shader suaviza el disco sin redondearlo.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Punto {
    x: f64,
    y: f64,
    radio: f64,
    color: [f32; 3],
    alfa: f32,
    hueco: usize,
}

/// Un número en `[0, 1)` fijo para cada aparición y cada uso: así cada una
/// sale en un sitio, con un ángulo y unos colores, sin guardar estado ni
/// depender de un generador. Es el mezclador de splitmix64.
fn azar(aparicion: u64, uso: u64) -> f64 {
    let mut z = aparicion
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(uso.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Las hileras de Estela en el instante `segundos`.
///
/// Sin estado: el mismo `t` da siempre el mismo fotograma, que es lo que
/// permite testearlo.
fn estela((w, h): (i32, i32), escala: f64, segundos: f64, reducidos: bool) -> Vec<Punto> {
    // En efectos reducidos, una aparición quieta por minuto, en su momento
    // de máxima presencia.
    let (n, fase) = if reducidos {
        ((segundos / 60.0).floor() as u64, APARICION / 2.0)
    } else {
        let n = (segundos / APARICION).floor();
        (n as u64, segundos - n * APARICION)
    };
    let suave = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let visible =
        suave(fase / ENTRADA) * (1.0 - suave((fase - (APARICION - PAUSA - SALIDA)) / SALIDA));
    if visible <= 0.01 {
        return Vec::new();
    }
    let unidad = f64::from(w.min(h)) / 1080.0;
    let margen = RADIO_GRUPO * unidad;
    let x0 = margen + azar(n, 0) * (f64::from(w) - 2.0 * margen).max(0.0);
    let y0 = margen + azar(n, 1) * (f64::from(h) - 2.0 * margen).max(0.0);
    let giro = (0.04 + 0.03 * azar(n, 5)) * if n.is_multiple_of(2) { 1.0 } else { -1.0 };
    let angulo = std::f64::consts::PI * azar(n, 2) + giro * (fase - APARICION / 2.0);
    let (seno, coseno) = angulo.sin_cos();
    let desfase = azar(n, 3) * std::f64::consts::TAU;
    // El ángulo áureo separa las apariciones consecutivas y recorre todos los
    // sentidos. Cambiarlo durante el negro evita que la cresta salte de golpe.
    let direccion = 5.8_f64.atan2(2.4)
        + n as f64 * std::f64::consts::TAU * 0.381_966_011_250_105_1
        + 0.30 * (azar(n, 4) - 0.5);
    let (onda_y, onda_x) = direccion.sin_cos();
    // El giro curva la trayectoria y los 8 px/s hacen avanzar los puntos de
    // verdad, conservando el margen hasta el borde durante toda la aparición.
    let avance = 8.0 * (fase - APARICION / 2.0) * unidad;
    let cx = x0 + avance * (onda_x * coseno - onda_y * seno);
    let cy = y0 + avance * (onda_x * seno + onda_y * coseno);
    let frecuencia = 2.4_f64.hypot(5.8);
    let degradado = DEGRADADOS[n as usize % DEGRADADOS.len()];
    let mut puntos = Vec::with_capacity(HILERAS * MAX_POR_HILERA);
    for i in 0..HILERAS {
        let d = (i as f64 - (HILERAS - 1) as f64 / 2.0) * SEPARACION;
        let v = d / 172.0;
        for j in 0..MAX_POR_HILERA {
            let a = (j as f64 - (MAX_POR_HILERA - 1) as f64 / 2.0) * PASO;
            let u = a / DISCO;
            let distancia = u.hypot(v);
            if distancia >= 1.0 {
                continue;
            }
            let borde = suave((1.0 - distancia) / 0.22);
            // El desfase entre filas hace viajar la cresta por la malla; una
            // sola sinusoide temporal haría latir todo el conjunto a la vez.
            let frente =
                frecuencia * (onda_x * u + onda_y * v) + 0.65 * u * u - 1.65 * fase + desfase;
            let onda = (0.5 + 0.5 * frente.cos()).powi(2);
            // Con 18 px entre columnas, 6 px de radio dejan separadas las crestas.
            // Antes el valle quedaba en 0,5 px y 4 % de opacidad: sobre negro
            // desaparecía. El mínimo conserva el punteado entre las crestas.
            let radio = (1.25 + 4.75 * onda) * borde.sqrt();
            let tono = 0.74 + 0.32 * (0.5 * frente + 0.6 * u + 1.1).sin();
            let color = color_degradado(degradado, suave(onda / 0.35) * tono);
            // La curva local acompaña al avance: incluso los puntos pequeños
            // conservan su hueco en la malla.
            let local_x = a * (1.0 - 0.10 * v * v);
            let curva = 65.0 * (0.45 * fase + 1.0).cos();
            let local_y =
                d + curva * (u * u - 0.35) + 12.0 * (1.4 * u + 0.4 * v - 0.40 * fase).sin();
            puntos.push(Punto {
                x: cx + (local_x * coseno - local_y * seno) * unidad,
                y: cy + (local_x * seno + local_y * coseno) * unidad,
                radio: radio * unidad / escala,
                color,
                alfa: ((0.28 + 0.72 * onda) * borde * visible) as f32,
                hueco: i * MAX_POR_HILERA + j,
            });
        }
    }
    puntos
}

fn color_degradado(paradas: [[f32; 3]; 5], t: f64) -> [f32; 3] {
    let posicion = t.clamp(0.0, 1.0) * (paradas.len() - 1) as f64;
    let tramo = (posicion.floor() as usize).min(paradas.len() - 2);
    let mezcla = (posicion - tramo as f64) as f32;
    [0, 1, 2].map(|c| paradas[tramo][c] + (paradas[tramo + 1][c] - paradas[tramo][c]) * mezcla)
}

/// Onda triangular: posición dentro de `[0, libre]` y paredes tocadas.
fn rebote(recorrido: f64, libre: f64) -> (f64, usize) {
    if libre <= 0.0 {
        return (0.0, 0);
    }
    let vueltas = (recorrido / libre).floor();
    let resto = recorrido - vueltas * libre;
    let posicion = if (vueltas as usize).is_multiple_of(2) {
        resto
    } else {
        libre - resto
    };
    (posicion, vueltas as usize)
}

fn rasterizar(color: &str) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    let svg = include_str!("../../bookos-shell/assets/bookos.svg").replace("white", color);
    let arbol = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())?;
    let [_, _, book, os] = arbol.root().children() else {
        anyhow::bail!("el logo debe conservar el círculo, la onda y los dos trazados de texto");
    };
    let mut imagen = resvg::tiny_skia::Pixmap::new(ANCHO as u32, ALTO as u32)
        .expect("textura del salvapantallas de 392 × 114");
    // El original coloca «Book» y «OS» en dos líneas y con tamaños distintos.
    // Igualar su altura conserva los trazados de la marca en una sola palabra.
    let cajas = [book.abs_bounding_box(), os.abs_bounding_box()];
    let alto = 112.0;
    let anchos = cajas.map(|caja| caja.width() * alto / caja.height());
    let mut x = (ANCHO as f32 - anchos.iter().sum::<f32>() - 8.0) / 2.0;
    for ((nodo, caja), ancho) in [book, os].into_iter().zip(cajas).zip(anchos) {
        let escala = alto / caja.height();
        let transformacion = resvg::tiny_skia::Transform::from_scale(escala, escala)
            .post_translate(x, (ALTO as f32 - alto) / 2.0);
        resvg::render_node(nodo, transformacion, &mut imagen.as_mut());
        x += ancho + 8.0;
    }
    anyhow::ensure!(
        x - 8.0 <= ANCHO as f32,
        "las letras no caben en la textura: miden {} px",
        x - 8.0
    );
    anyhow::ensure!(
        imagen.pixels().iter().any(|p| p.alpha() != 0),
        "los trazados de BookOS deben producir texto visible"
    );
    Ok(imagen)
}

pub fn previsualizar(state: &mut BookosComp, estilo: Estilo) -> Result<(), String> {
    if state.modo.kiosko()
        || !state.dpms_encendido
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return Err(
            "la vista previa necesita el escritorio desbloqueado y la pantalla encendida".into(),
        );
    }
    if state.salvapantallas.activo() {
        return Err("el salvapantallas ya está activo".into());
    }
    activar(state, Origen::VistaPrevia, estilo);
    if !state.salvapantallas.activo() {
        return Err("no se pudo preparar el salvapantallas".into());
    }
    Ok(())
}

/// Las miniaturas usan el mismo modelo que la escena: Settings no mantiene
/// otra versión de las ondas. SVG interpola muestras a 8 Hz entre sí; al
/// retirar la imagen de la página también termina la animación.
pub fn miniatura(estilo: Estilo, reducidos: bool) -> anyhow::Result<String> {
    match estilo {
        Estilo::Estela => {
            let base = estela((1920, 1080), 1.0, 2.0, reducidos);
            let fotogramas: Vec<_> = if reducidos {
                Vec::new()
            } else {
                (0..=96)
                    .map(|i| {
                        estela(
                            (1920, 1080),
                            1.0,
                            (2.0 + f64::from(i) / 8.0) % APARICION,
                            false,
                        )
                    })
                    .collect()
            };
            // Una malla vertical necesita más alto que una horizontal: medir
            // todo el ciclo impide que la miniatura corte las filas al curvarse.
            let (izquierda, arriba, derecha, abajo) =
                base.iter().chain(fotogramas.iter().flatten()).fold(
                    (
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ),
                    |(x0, y0, x1, y1), p| {
                        let margen = p.radio * 1.8 + 20.0;
                        (
                            x0.min(p.x - margen),
                            y0.min(p.y - margen),
                            x1.max(p.x + margen),
                            y1.max(p.y + margen),
                        )
                    },
                );
            let ancho = 660.0_f64
                .max(derecha - izquierda)
                .max((abajo - arriba) * 1.6);
            let alto = ancho / 1.6;
            let mut svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{:.2} {:.2} {ancho:.2} {alto:.2}">"#,
                (izquierda + derecha - ancho) / 2.0,
                (arriba + abajo - alto) / 2.0,
            );
            for punto in &base {
                let color = |p: &Punto| {
                    let [r, g, b] = p.color.map(|c| (c * 255.0).round() as u8);
                    format!("#{r:02x}{g:02x}{b:02x}")
                };
                svg.push_str(&format!(
                    r#"<circle cx="{:.2}" cy="{:.2}" r="{:.2}" fill="{}" opacity="{:.3}">"#,
                    punto.x,
                    punto.y,
                    punto.radio,
                    color(punto),
                    punto.alfa,
                ));
                if !reducidos {
                    let mut valores: [Vec<String>; 5] = std::array::from_fn(|_| Vec::new());
                    for foto in &fotogramas {
                        let actual = foto.iter().find(|p| p.hueco == punto.hueco);
                        let p = actual.unwrap_or(punto);
                        valores[0].push(format!("{:.2}", p.x));
                        valores[1].push(format!("{:.2}", p.y));
                        valores[2].push(format!("{:.2}", p.radio));
                        valores[3].push(color(p));
                        valores[4].push(format!("{:.3}", actual.map_or(0.0, |p| p.alfa)));
                    }
                    for (atributo, valores) in ["cx", "cy", "r", "fill", "opacity"]
                        .into_iter()
                        .zip(valores)
                    {
                        svg.push_str(&format!(
                            r#"<animate attributeName="{atributo}" values="{}" dur="12s" repeatCount="indefinite"/>"#,
                            valores.join(";"),
                        ));
                    }
                }
                svg.push_str("</circle>");
            }
            svg.push_str("</svg>");
            Ok(svg)
        }
        Estilo::Rebote => {
            let png = rasterizar(COLORES[0])?.encode_png()?;
            let imagen = base64::engine::general_purpose::STANDARD.encode(png);
            let (x, y, ancho, alto, _) = geometria((640, 400), 1.0, 0.0, false);
            let mut svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 400"><image href="data:image/png;base64,{imagen}" x="{x}" y="{y}" width="{ancho}" height="{alto}">"#,
            );
            if !reducidos {
                for (eje, libre, desfase) in [
                    ("x", 640.0 - f64::from(ancho), 0.3),
                    ("y", 400.0 - f64::from(alto), 0.7),
                ] {
                    let inicio = libre * desfase;
                    svg.push_str(&format!(
                        r#"<animate attributeName="{eje}" values="{inicio};{libre};0;{inicio}" keyTimes="0;{};{};1" dur="{}s" repeatCount="indefinite"/>"#,
                        (1.0 - desfase) / 2.0, 1.0 - desfase / 2.0, 2.0 * libre / VELOCIDAD,
                    ));
                }
            }
            svg.push_str("</image></svg>");
            Ok(svg)
        }
    }
}

/// Sale también sobre el bloqueo: es el escalón que va entre bloquear y
/// apagar la pantalla, y lo que tapa no deja de estar bloqueado debajo.
pub fn activar(state: &mut BookosComp, origen: Origen, estilo: Estilo) {
    if state.modo.kiosko() || !state.dpms_encendido || state.salvapantallas.activo() {
        return;
    }
    let escena = match estilo {
        Estilo::Rebote => {
            let letras = COLORES
                .iter()
                .map(|color| {
                    rasterizar(color).map(|imagen| {
                        MemoryRenderBuffer::from_slice(
                            imagen.data(),
                            Fourcc::Abgr8888,
                            (ANCHO, ALTO),
                            1,
                            Transform::Normal,
                            None,
                        )
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>();
            match letras {
                Ok(letras) => Escena::Rebote { letras },
                Err(err) => {
                    tracing::warn!("no se pudo preparar el salvapantallas: {err}");
                    return;
                }
            }
        }
        Estilo::Estela => Escena::Estela { puntos: None },
    };
    crate::keybinds::liberar_puntero_para_capa(state);
    state.meta_sola = false;
    state.salvapantallas.modo = Modo::Salvapantallas {
        origen,
        desde: Instant::now(),
        escena,
        negro: Id::new(),
    };
    state.needs_redraw = true;
    let resultado = state
        .loop_handle
        .insert_source(Timer::immediate(), |_, _, state| {
            if !state.salvapantallas.activo() {
                state.salvapantallas.fotograma = None;
                return TimeoutAction::Drop;
            }
            if state.dpms_encendido {
                state.needs_redraw = true;
            }
            TimeoutAction::ToDuration(if bookos_shell::tema::efectos_reducidos() {
                Duration::from_secs(60)
            } else {
                // Al ritmo del panel de 120 Hz: no hay nada más que pintar y
                // a 60 se notaban los ~5 px de salto por fotograma. Con 8 ms
                // salen 125 peticiones y el vblank las recorta a 120.
                Duration::from_millis(8)
            })
        });
    match resultado {
        Ok(token) => state.salvapantallas.fotograma = Some(token),
        Err(err) => tracing::error!("no se pudo animar el salvapantallas: {err}"),
    }
}

pub fn retirar(state: &mut BookosComp) -> bool {
    if !state.salvapantallas.activo() {
        return false;
    }
    state.salvapantallas.modo = Modo::Escritorio;
    if let Some(token) = state.salvapantallas.fotograma.take() {
        state.loop_handle.remove(token);
    }
    state.needs_redraw = true;
    programar(state);
    true
}

/// Devuelve si el salvapantallas se queda el evento. Ver [`Origen`]: puesto
/// por inactividad se retira, y la entrada que lo quita no sigue hacia nadie,
/// como el primer toque que enciende una pantalla.
pub fn actividad(state: &mut BookosComp) -> bool {
    if let Modo::Salvapantallas { origen, desde, .. } = state.salvapantallas.modo {
        let cerrar = match origen {
            Origen::Atajo => false,
            Origen::Inactividad => true,
            // El clic de «Probar» aún puede estar soltándose cuando D-Bus
            // activa la capa; no debe cerrar la vista antes de verla.
            Origen::VistaPrevia => desde.elapsed() >= Duration::from_millis(350),
        };
        if cerrar {
            retirar(state);
        }
        return true;
    }
    if state.salvapantallas.pantalla_apagada() && state.dpms_encendido {
        state.salvapantallas.modo = Modo::Escritorio;
    }
    if state.salvapantallas.alarma.is_none() || state.salvapantallas.alarma_apagado.is_none() {
        programar(state);
    }
    false
}

pub fn apagar_pantalla(state: &mut BookosComp) {
    if state.aplicar_dpms.is_none() {
        tracing::warn!("el apagado físico de pantalla requiere la sesión DRM de BookOS");
        return;
    }
    retirar(state);
    crate::backend::apagar_dpms(state);
    if !state.dpms_encendido {
        state.salvapantallas.modo = Modo::PantallaApagada;
        crate::backend::cancelar_suspension_inactividad(state);
    }
}

pub fn programar(state: &mut BookosComp) {
    programar_apagado(state);
    if let Some(token) = state.salvapantallas.alarma.take() {
        state.loop_handle.remove(token);
    }
    if state.salvapantallas.inactividad.is_zero()
        || state.modo.kiosko()
        || !state.dpms_encendido
        || state.salvapantallas.activo()
        || !state.idle_inhibidores.is_empty()
        || bookos_shell::mantener_despierto()
    {
        return;
    }
    let espera = state
        .salvapantallas
        .inactividad
        .saturating_sub(state.last_input.unwrap_or(state.start_time).elapsed())
        .max(Duration::from_millis(50));
    let resultado = state
        .loop_handle
        .insert_source(Timer::from_duration(espera), |_, _, state| {
            state.salvapantallas.alarma = None;
            if state.idle_inhibidores.is_empty()
                && !bookos_shell::mantener_despierto()
                && state.last_input.unwrap_or(state.start_time).elapsed()
                    >= state.salvapantallas.inactividad
            {
                let estilo = state.salvapantallas.estilo;
                activar(state, Origen::Inactividad, estilo);
            } else {
                programar(state);
            }
            TimeoutAction::Drop
        });
    match resultado {
        Ok(token) => state.salvapantallas.alarma = Some(token),
        Err(err) => tracing::error!("no se pudo programar el salvapantallas: {err}"),
    }
}

/// Último escalón antes de la suspensión: apaga el panel. No pasa por
/// [`apagar_pantalla`], que es la de Meta+Alt+O y cancela la suspensión; aquí
/// la suspensión tiene que seguir contando.
fn programar_apagado(state: &mut BookosComp) {
    if let Some(token) = state.salvapantallas.alarma_apagado.take() {
        state.loop_handle.remove(token);
    }
    if state.salvapantallas.apagado.is_zero()
        || state.modo.kiosko()
        || state.aplicar_dpms.is_none()
        || !state.dpms_encendido
        || !state.idle_inhibidores.is_empty()
        || bookos_shell::mantener_despierto()
    {
        return;
    }
    let espera = state
        .salvapantallas
        .apagado
        .saturating_sub(state.last_input.unwrap_or(state.start_time).elapsed())
        .max(Duration::from_millis(50));
    let resultado = state
        .loop_handle
        .insert_source(Timer::from_duration(espera), |_, _, state| {
            state.salvapantallas.alarma_apagado = None;
            if !state.idle_inhibidores.is_empty() || bookos_shell::mantener_despierto() {
                return TimeoutAction::Drop;
            }
            if state.last_input.unwrap_or(state.start_time).elapsed()
                >= state.salvapantallas.apagado
            {
                tracing::info!(
                    segundos = state.salvapantallas.apagado.as_secs(),
                    "pantalla apagada por inactividad"
                );
                // Primero apagar: así `retirar` ya no vuelve a programar el
                // salvapantallas sobre un panel a oscuras.
                crate::backend::apagar_dpms(state);
                retirar(state);
            } else {
                programar_apagado(state);
            }
            TimeoutAction::Drop
        });
    match resultado {
        Ok(token) => state.salvapantallas.alarma_apagado = Some(token),
        Err(err) => tracing::error!("no se pudo programar el apagado de pantalla: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_texto_permanece_dentro_de_cada_pantalla() {
        for pantalla in [(1920, 1080), (2880, 1800), (720, 1280), (320, 200)] {
            for escala in [1.0, 1.75, 2.0] {
                for segundo in 0..1200 {
                    let (x, y, w, h, _) =
                        geometria(pantalla, escala, f64::from(segundo) / 4.0, false);
                    assert!(x >= 0.0 && y >= 0.0);
                    assert!(x + f64::from(w) * escala <= f64::from(pantalla.0));
                    assert!(y + f64::from(h) * escala <= f64::from(pantalla.1));
                }
            }
        }
    }

    #[test]
    fn los_efectos_reducidos_mueven_el_texto_solo_cada_minuto() {
        assert_eq!(
            geometria((1920, 1080), 1.0, 0.0, true),
            geometria((1920, 1080), 1.0, 59.0, true)
        );
        assert_ne!(
            geometria((1920, 1080), 1.0, 0.0, true),
            geometria((1920, 1080), 1.0, 60.0, true)
        );
    }

    #[test]
    fn cambia_de_color_al_tocar_una_pared() {
        // 1920 − 240 = 1680 libres en x; arranca en 504, toca a los 7,35 s.
        let antes = geometria((1920, 1080), 1.0, 7.2, false).4;
        let despues = geometria((1920, 1080), 1.0, 7.5, false).4;
        assert_eq!(despues, antes + 1);
        let (x, ..) = geometria((1920, 1080), 1.0, 7.5, false);
        assert!(x < 1680.0, "tras el rebote vuelve hacia la izquierda");
    }

    #[test]
    fn la_estela_no_se_sale_de_ninguna_pantalla() {
        for pantalla in [(1920, 1080), (2880, 1800), (720, 1280), (320, 200)] {
            for escala in [1.0, 1.75, 2.0] {
                for paso in 0..2400 {
                    let t = f64::from(paso) * 0.5;
                    for p in estela(pantalla, escala, t, false) {
                        let margen = p.radio * escala * 1.8 + 2.0 * escala;
                        assert!(
                            p.x - margen >= 0.0 && p.y - margen >= 0.0,
                            "{pantalla:?} t={t}: {p:?}"
                        );
                        assert!(p.x + margen <= f64::from(pantalla.0));
                        assert!(p.y + margen <= f64::from(pantalla.1));
                    }
                }
            }
        }
    }

    #[test]
    fn cada_aparicion_cambia_de_sitio_y_entre_medias_hay_negro() {
        let centro = |t| {
            let p = estela((1920, 1080), 1.0, t, false);
            let n = p.len() as f64;
            (
                p.iter().map(|p| p.x).sum::<f64>() / n,
                p.iter().map(|p| p.y).sum::<f64>() / n,
            )
        };
        // El avance debe apreciarse dentro de una aparición sin cruzar la pantalla.
        let (a, b) = (centro(3.0), centro(8.0));
        let recorrido = (a.0 - b.0).hypot(a.1 - b.1);
        assert!(
            (25.0..60.0).contains(&recorrido),
            "{a:?} {b:?}: {recorrido}"
        );
        let saltos = (0..20)
            .filter(|n| {
                let t = f64::from(*n) * APARICION + 5.0;
                let (a, b) = (centro(t), centro(t + APARICION));
                (a.0 - b.0).hypot(a.1 - b.1) > 150.0
            })
            .count();
        assert!(
            saltos >= 15,
            "solo {saltos} de 20 apariciones cambian de sitio"
        );
        assert!(estela((1920, 1080), 1.0, APARICION - 0.5, false).is_empty());
        let entrando = estela((1920, 1080), 1.0, 0.3, false);
        assert!(entrando.iter().all(|p| p.alfa < 0.5), "entra con fundido");
    }

    /// Una hilera completa permite medir la curva sin confundirla con la onda de luz.
    fn hilera_central(t: f64) -> Vec<Punto> {
        let centro = HILERAS / 2;
        estela((1920, 1080), 1.0, t, false)
            .into_iter()
            .filter(|p| p.hueco / MAX_POR_HILERA == centro)
            .collect()
    }

    #[test]
    fn la_onda_cambia_tamanos_en_ambos_sentidos_a_la_vez() {
        let a = estela((1920, 1080), 1.0, 3.0, false);
        let b = estela((1920, 1080), 1.0, 4.0, false);
        assert_eq!(a.len(), b.len());
        let creciendo = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| b.radio - a.radio > 1.0)
            .count();
        let menguando = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| a.radio - b.radio > 1.0)
            .count();
        assert!(
            creciendo > 15 && menguando > 15,
            "crecen {creciendo}, menguan {menguando}"
        );
        assert!(a.iter().zip(&b).any(|(a, b)| a.color != b.color));
    }

    #[test]
    fn la_cresta_de_luz_recorre_distintas_hileras() {
        let mut crestas = std::collections::HashSet::new();
        for instante in 0..24 {
            let puntos = estela((1920, 1080), 1.0, 2.0 + f64::from(instante) / 8.0, false);
            let energias: Vec<f32> = (0..HILERAS)
                .map(|fila| {
                    puntos
                        .iter()
                        .filter(|p| p.hueco / MAX_POR_HILERA == fila)
                        .map(|p| p.alfa * p.radio.powi(2) as f32)
                        .sum()
                })
                .collect();
            let fila = energias
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .expect("la malla tiene hileras")
                .0;
            crestas.insert(fila);
        }
        assert!(crestas.len() >= 4, "la cresta solo recorre {crestas:?}");
    }

    #[test]
    fn las_crestas_viajan_en_todas_las_direcciones() {
        let mut sentidos = std::collections::HashSet::new();
        for aparicion in 0..24 {
            let t = f64::from(aparicion) * APARICION + 4.0;
            let a = estela((1920, 1080), 1.0, t, false);
            let b = estela((1920, 1080), 1.0, t + 0.03, false);
            let buscar = |hueco| {
                a.iter()
                    .find(|p| p.hueco == hueco)
                    .expect("vecinos de la zona central dentro de la malla")
            };
            let (mut dx, mut dy) = (0.0, 0.0);
            // La correlación espacio-tiempo distingue una onda que viaja de
            // una malla que solo cambia de sitio o gira con la misma pulsación.
            for (p, siguiente) in a.iter().zip(&b) {
                let fila = p.hueco / MAX_POR_HILERA;
                let columna = p.hueco % MAX_POR_HILERA;
                if fila.abs_diff(HILERAS / 2) > 2 || columna.abs_diff(MAX_POR_HILERA / 2) > 5 {
                    continue;
                }
                let tiempo = siguiente.radio - p.radio;
                dx += (buscar(p.hueco + 1).radio - buscar(p.hueco - 1).radio) * tiempo / PASO;
                dy += (buscar(p.hueco + MAX_POR_HILERA).radio
                    - buscar(p.hueco - MAX_POR_HILERA).radio)
                    * tiempo
                    / SEPARACION;
            }
            assert!(
                dx.hypot(dy) > 0.001,
                "la onda no avanza: aparición {aparicion}"
            );
            sentidos.insert((dx.is_sign_positive(), dy.is_sign_positive()));
        }
        assert_eq!(
            sentidos.len(),
            4,
            "faltan sentidos de propagación: {sentidos:?}"
        );
    }

    #[test]
    fn los_puntos_pequenos_conectan_las_crestas_sin_desaparecer() {
        for aparicion in 0..12 {
            let t = f64::from(aparicion) * APARICION + 4.0;
            let puntos = estela((1920, 1080), 1.0, t, false);
            let pequenos: Vec<_> = puntos
                .iter()
                .filter(|p| {
                    let fila = p.hueco / MAX_POR_HILERA;
                    let columna = p.hueco % MAX_POR_HILERA;
                    fila.abs_diff(HILERAS / 2) <= 2
                        && columna.abs_diff(MAX_POR_HILERA / 2) <= 5
                        && p.radio < 1.8
                })
                .collect();
            assert!(
                pequenos.len() >= 8,
                "faltan puntos entre crestas: aparición {aparicion}"
            );
            assert!(
                pequenos.iter().all(|p| p.radio >= 1.2 && p.alfa >= 0.25),
                "el valle pierde sus puntos: aparición {aparicion}"
            );
        }
    }

    #[test]
    fn hay_mas_puntos_sin_agrandar_el_conjunto_ni_unir_las_crestas() {
        let puntos = estela((1920, 1080), 1.0, 4.0, false);
        assert!(
            puntos.len() > 240,
            "faltan puntos intermedios: {}",
            puntos.len()
        );
        let cx = puntos.iter().map(|p| p.x).sum::<f64>() / puntos.len() as f64;
        let cy = puntos.iter().map(|p| p.y).sum::<f64>() / puntos.len() as f64;
        for p in puntos {
            assert!((p.x - cx).hypot(p.y - cy) < 300.0);
            assert!(p.radio * 2.0 < PASO.min(SEPARACION));
        }
    }

    #[test]
    fn la_onda_es_continua_entre_fotogramas() {
        for instante in 0..480 {
            let t = 2.0 + f64::from(instante) / 60.0;
            let a = estela((1920, 1080), 1.0, t, false);
            let b = estela((1920, 1080), 1.0, t + 1.0 / 60.0, false);
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.hueco, b.hueco);
                assert!((a.radio - b.radio).abs() < 0.20, "radio: {a:?} {b:?}");
                assert!((a.x - b.x).hypot(a.y - b.y) < 1.5, "posición: {a:?} {b:?}");
                assert!((a.alfa - b.alfa).abs() < 0.04);
            }
        }
    }

    #[test]
    fn la_proporcion_visual_no_depende_de_la_densidad_del_monitor() {
        let a = estela((1920, 1080), 1.0, 4.0, false);
        for escala in [1.25, 1.75, 2.0] {
            let b = estela((1920, 1080), escala, 4.0, false);
            for (a, b) in a.iter().zip(&b) {
                assert_eq!((a.x, a.y), (b.x, b.y));
                assert!((a.radio - b.radio * escala).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn las_hileras_de_fuera_son_mas_cortas() {
        let puntos = estela((1920, 1080), 1.0, 5.0, false);
        let cuenta = |i| {
            puntos
                .iter()
                .filter(|p| p.hueco / MAX_POR_HILERA == i)
                .count()
        };
        assert!(cuenta(0) < cuenta(HILERAS / 2));
        assert_eq!(cuenta(0), cuenta(HILERAS - 1));
    }

    #[test]
    fn el_degradado_va_de_parada_a_parada() {
        let paradas = DEGRADADOS[1];
        assert_eq!(color_degradado(paradas, 0.0), paradas[0]);
        assert_eq!(color_degradado(paradas, 1.0), paradas[4]);
    }

    /// Distancia con signo del centro de la hilera central a la recta que une
    /// sus extremos: cero si la hilera fuese recta.
    fn flecha(t: f64) -> f64 {
        let puntos = hilera_central(t);
        let centro = |p: &Punto| (p.x, p.y);
        let (a, b) = (centro(&puntos[0]), centro(&puntos[puntos.len() - 1]));
        let m = centro(&puntos[puntos.len() / 2]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        (dx * (m.1 - a.1) - dy * (m.0 - a.0)) / (dx * dx + dy * dy).sqrt()
    }

    #[test]
    fn las_hileras_se_curvan_hacia_los_dos_lados() {
        let flechas: Vec<f64> = (0..30).map(|n| flecha(2.0 + f64::from(n) / 4.0)).collect();
        assert!(flechas.iter().any(|&f| f > 10.0), "{flechas:?}");
        assert!(flechas.iter().any(|&f| f < -10.0), "{flechas:?}");
    }

    #[test]
    fn la_superficie_se_comba_dentro_de_una_aparicion() {
        let (a, b) = (flecha(2.0), flecha(5.5));
        assert!((a - b).abs() > 2.0, "{a} {b}");
    }

    #[test]
    fn con_efectos_reducidos_cambia_solo_cada_minuto() {
        let puntos = |t| estela((1920, 1080), 1.0, t, true);
        assert_eq!(puntos(1.0), puntos(59.0));
        assert_ne!(puntos(1.0), puntos(61.0));
    }

    #[test]
    fn las_miniaturas_se_ven_sin_esperar_a_la_animacion() {
        for estilo in [Estilo::Estela, Estilo::Rebote] {
            for reducidos in [false, true] {
                let svg = miniatura(estilo, reducidos).expect("modelo de la miniatura válido");
                assert!(
                    svg.len() < 1_000_000,
                    "miniatura demasiado grande: {}",
                    svg.len()
                );
                assert_eq!(svg.contains("<animate "), !reducidos);
                let arbol = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
                    .expect("la miniatura es un SVG válido");
                let mut imagen =
                    resvg::tiny_skia::Pixmap::new(640, 400).expect("miniatura de 640 × 400");
                let escala = 640.0 / arbol.size().width();
                resvg::render(
                    &arbol,
                    resvg::tiny_skia::Transform::from_scale(escala, escala),
                    &mut imagen.as_mut(),
                );
                let visibles = imagen.pixels().iter().filter(|p| p.alpha() > 30).count();
                assert!(
                    visibles > 200,
                    "miniatura vacía: {estilo:?}, reducidos={reducidos}"
                );
                if let Some(carpeta) = std::env::var_os("BOOKOS_MINIATURAS") {
                    let ruta = std::path::Path::new(&carpeta)
                        .join(format!("{}-{reducidos}.svg", estilo.nombre()));
                    std::fs::write(ruta, &svg).expect("carpeta de miniaturas accesible");
                }
            }
        }
    }

    #[test]
    fn la_marca_se_rasteriza_y_se_puede_inspeccionar() {
        let imagen = rasterizar(COLORES[0]).expect("trazados de texto incluidos en la marca");
        assert!(imagen.pixels().iter().any(|p| p.alpha() > 200));
        if let Some(ruta) = std::env::var_os("BOOKOS_SALVAPANTALLAS_PNG") {
            imagen
                .save_png(ruta)
                .expect("ruta de la previsualización accesible");
        }
    }
}
