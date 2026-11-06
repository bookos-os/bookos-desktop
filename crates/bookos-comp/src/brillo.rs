//! El brillo de borde de Zenith: la luz que recorre el contorno de la pantalla
//! mientras se le habla o trabaja.
//!
//! Secuencia y tiempos medidos cuadro a cuadro en una grabación de Gemini
//! (396×850, unos 60 fps): un filo casi blanco sube en todo el contorno, se
//! sostiene un instante y da paso a la luz de color, que entra ensanchándose.
//! Los colores son los del fondo, como allí, y aquí la luz se ancla arriba,
//! junto a la isla: allí mandaba abajo porque es donde vive su barra.
//!
//! Va con un shader, pero solo sobre cuatro franjas que miden lo que el halo de
//! este fotograma. El de pantalla completa se descartó por caro; una textura
//! pintada de antemano, que es lo que hubo aquí, no puede hacer correr el color.
//! A 1440×900 lógicos y en silencio se evalúa el 13 % de los píxeles.

use std::time::{Duration, Instant};

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesRenderer, GlesTexture, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::{ContextId, Renderer};
use smithay::output::Output;
use smithay::utils::Rectangle;

use crate::state::BookosComp;

/// Medido: el borde pasa de 13 a 197 en 330 ms con forma de S, aguanta 70 ms
/// y el blanco cede al color en 200 ms.
const SUBE: Duration = Duration::from_millis(330);
const SOSTIENE: Duration = Duration::from_millis(70);
const CRUCE: Duration = Duration::from_millis(200);
/// Medido: se apaga de 196 a 9 en unos 530 ms, casi en línea recta, sin el
/// corte rápido de un menú. `D_TEMA` es lo más cercano que ya hay.
const SALE: Duration = bookos_shell::tema::D_TEMA;

/// Hondo del halo en silencio, en lógicos. La forma sale del lateral del vídeo
/// (286, 155, 106, 50, 11 cada 4 px: caída cuadrática sobre 20), pero 20 en un
/// móvil es el 5 % del ancho y en este panel el 1,5 %: casi no se veía. 32 es a
/// ojo, mirando capturas a 1280×800 lógicos.
const HONDO: f32 = 32.0;
/// Lo que se ensancha con la voz a tope.
const HONDO_VOZ: f32 = 36.0;
/// `isla` del shader: arriba en el centro el halo llega a tanto más.
const ISLA: f32 = 0.35;
/// Vueltas de color por segundo. Escuchando, deriva lenta como en el vídeo;
/// pensando no hay referencia, porque la grabación no llega a ese estado: va
/// cuatro veces más rápido para que se note que trabaja.
const FLUJO_ESCUCHA: f32 = 0.08;
const FLUJO_PIENSA: f32 = 0.32;
/// Cuánto tarda en notarse el cambio de escuchar a pensar: sin esto el color
/// pegaría un acelerón de un fotograma a otro.
const CAMBIO_FASE: f32 = 0.25;
/// Un ciclo de respiración pensando, en segundos.
const RESPIRA: f32 = 3.2;

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

uniform vec2 origen;
uniform vec2 pantalla;
uniform float flujo;
uniform float hondo;
uniform float luz;
uniform float fogonazo;
uniform float isla;
uniform vec3 tono_a;
uniform vec3 tono_b;
uniform vec3 tono_c;

const float TAU = 6.2831853;

void main() {
    vec2 p = origen + v_coords * size;
    vec2 medio = pantalla * 0.5;
    vec2 q = abs(p - medio) - medio + vec2(14.0);
    // Distancia al contorno redondeado, para que el halo doble la esquina en
    // vez de hacer inglete. Fuera de la curva queda 0: la esquina va encendida.
    float d = max(14.0 - length(max(q, 0.0)) - min(max(q.x, q.y), 0.0), 0.0);

    // Dónde cae en la vuelta al contorno. Todas las frecuencias son enteras:
    // ni el ángulo ni `flujo` (que da la vuelta en 1) dejan costura.
    vec2 r = (p - medio) / pantalla;
    float vuelta = atan(r.y, r.x) / TAU;
    float o1 = 0.5 + 0.5 * sin(TAU * (2.0 * vuelta + flujo));
    float o2 = 0.5 + 0.5 * sin(TAU * (3.0 * vuelta - 2.0 * flujo + 0.3));
    float o3 = sin(TAU * (4.0 * vuelta + 3.0 * flujo));
    vec3 color = mix(tono_a, tono_b, smoothstep(0.15, 0.85, o1));
    color = mix(color, tono_c, smoothstep(0.35, 0.95, o2));
    // Entre azul y amarillo la mezcla da un gris apagado (se ve en el vídeo, y
    // allí dura poco). Devolverle el máximo la mantiene luminosa.
    color /= max(max(color.r, max(color.g, color.b)), 0.2);

    float arriba = 1.0 - smoothstep(0.0, 0.25, p.y / pantalla.y);
    float cx = (p.x - medio.x) / (pantalla.x * 0.25);
    float ancla = (1.0 - 0.35 * p.y / pantalla.y) + isla * arriba * exp(-cx * cx);
    float h = hondo * ancla * (0.78 + 0.22 * o3);
    float caida = clamp(1.0 - d / max(h, 0.001), 0.0, 1.0);
    // El filo es el canto del cristal: una línea más clara que el halo, teñida
    // hacia el blanco. Lo demás cae en cuadrático, como lo medido.
    float halo = caida * caida * 0.85;
    float filo = 1.0 - smoothstep(1.0, 2.5, d);
    vec3 canto = mix(color, vec3(1.0), 0.35);
    vec3 rgb = (color * halo + canto * filo * 0.5) * luz;
    float cobertura = max(halo, filo * 0.6) * luz;

    // El fogonazo: unos 2 lógicos de línea (234 en el borde y 61 a los 3 px).
    float linea = (1.0 - smoothstep(1.0, 2.5, d)) * fogonazo;
    rgb += vec3(1.0, 0.98, 0.93) * linea;
    cobertura = max(cobertura, linea);

    // Alfa menor que el color: es luz que se suma sobre lo de detrás, y no una
    // pintura que lo tape. Sale gratis y lo hace pasar por cristal iluminado.
    gl_FragColor = vec4(rgb, cobertura * 0.75) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        gl_FragColor = vec4(0.0, 0.3, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
"#;

pub struct Brillo {
    /// Para qué modo y escala valen los tonos y el contexto para el programa.
    clave: (i32, i32, i32),
    contexto: ContextId<GlesTexture>,
    /// Arriba, abajo, izquierda, derecha.
    franjas: [PixelShaderElement; 4],
    tonos: [[f32; 3]; 3],
    desde: Instant,
    /// Desde cuándo se está yendo y con qué luz y fogonazo empezó: la salida
    /// parte de lo que había, también si se cancela a medio fogonazo.
    apagando: Option<(Instant, f32, f32)>,
    ultimo: (f32, f32),
    /// Vuelta del color, en 0..1. Se acumula en vez de salir del reloj: así
    /// cambiar de velocidad no hace saltar los colores.
    flujo: f32,
    /// De 0 (escuchando) a 1 (pensando), suavizado.
    pensar: f32,
    ultimo_paso: Instant,
    /// Lo último subido al shader: con efectos reducidos está quieto y subirlo
    /// igual repintaría el contorno sin motivo.
    subido: Option<[f32; 4]>,
}

/// Lo que luce de cada capa a tanto tiempo de empezar: `(color, fogonazo)`.
// añadido en la 0.2.0
fn envolvente(t: Duration) -> (f32, f32) {
    use bookos_shell::tema::{C_TEMA, fraccion};
    let cruce = fraccion(t.saturating_sub(SUBE + SOSTIENE), CRUCE);
    let sube = C_TEMA.eval(fraccion(t, SUBE));
    (C_TEMA.eval(cruce), sube * (1.0 - C_TEMA.eval(cruce)))
}

/// Las cuatro franjas, en lógicos, para un halo que llega hasta `hondo`.
/// Arriba es más alta por el ensanche de la isla. Los lados van entre las dos
/// para que ningún píxel se pinte dos veces: dos halos encima se sumarían.
// añadido en la 0.2.0
fn franjas(w: i32, h: i32, hondo: f32) -> [(i32, i32, i32, i32); 4] {
    // El filo mide 2,5: no puede quedarse fuera aunque el halo esté recogido.
    let lado = (hondo.ceil() as i32 + 1).max(3).min(h / 2);
    let alto = ((hondo * (1.0 + ISLA)).ceil() as i32 + 1).max(3).min(h / 2);
    let medio = (h - alto - lado).max(1);
    [
        (0, 0, w, alto),
        (0, h - lado, w, lado),
        (0, alto, lado, medio),
        (w - lado, alto, lado, medio),
    ]
}

/// Los elementos del brillo para esta salida, o ninguno si Zenith no está
/// escuchando ni pensando. Libera el programa cuando deja de hacer falta.
// añadido en la 0.2.0
// modificado en la 0.2.0 -- de franjas pintadas a shader: fogonazo, color que corre y halo que respira.
pub fn elementos(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> Vec<PixelShaderElement> {
    use bookos_shell::actividad::{Clase, Fase};
    use bookos_shell::tema;
    let Some(shell) = state.shell.as_ref() else {
        return Vec::new();
    };
    let bloqueado = shell.esta_bloqueado();
    let fase = match shell.actividad_clase() {
        Some(Clase::Asistente(f)) if f.con_brillo() && !bloqueado => Some(f),
        _ => None,
    };
    let nivel = shell.actividad_nivel();
    let reducidos = tema::efectos_reducidos();
    let ahora = Instant::now();
    if fase.is_none() {
        // Con el bloqueo echado se va ya: fundirse encima del candado
        // enseñaría que detrás había algo escuchando.
        if bloqueado || reducidos {
            state.brillo.clear();
        }
        let Some((salida, _, _)) = state.brillo.values().find_map(|b| b.apagando) else {
            if state.brillo.is_empty() {
                return Vec::new();
            }
            for b in state.brillo.values_mut() {
                b.apagando = Some((ahora, b.ultimo.0, b.ultimo.1));
            }
            return elementos(state, renderer, output);
        };
        if salida.elapsed() >= SALE {
            state.brillo.clear();
            return Vec::new();
        }
    } else {
        // Vuelve a media salida: sigue desde la luz de ahora, sin otro fogonazo.
        for b in state.brillo.values_mut() {
            if b.apagando.take().is_some() {
                b.desde = ahora - (SUBE + SOSTIENE + CRUCE.mul_f32(b.ultimo.0));
            }
        }
    }
    let Some(modo) = output.current_mode() else {
        return Vec::new();
    };
    let escala = output.current_scale().fractional_scale();
    let tamano = output.current_transform().transform_size(modo.size);
    let (w, h) = (
        (f64::from(tamano.w) / escala).ceil() as i32,
        (f64::from(tamano.h) / escala).ceil() as i32,
    );
    let clave = (tamano.w, tamano.h, (escala * 100.0).round() as i32);
    let contexto = renderer.context_id();
    // Una por salida: pueden tener tamaños distintos.
    let nombre = output.name();
    let hay_que_crear = state
        .brillo
        .get(&nombre)
        .is_none_or(|b| b.clave != clave || b.contexto != contexto);
    if fase.is_some() && hay_que_crear {
        let desde = state.brillo.values().next().map_or(ahora, |b| b.desde);
        // Se lee aquí y no al pintar: leer el JSON en cada fotograma es ir al
        // disco 120 veces por segundo.
        let tonos = tema::roles_paleta(["blue", "accent-2", "accent-3"])
            .unwrap_or([tema::acento(); 3])
            .map(|c| [c.r, c.g, c.b]);
        let Some(nuevo) = crear(renderer, clave, contexto.clone(), tonos, desde, ahora) else {
            return Vec::new();
        };
        state.brillo.insert(nombre.clone(), nuevo);
    }
    // Saliendo, una salida que no tenía el brillo pintado no lo estrena.
    let Some(brillo) = state.brillo.get_mut(&nombre) else {
        return Vec::new();
    };

    let dt = ahora.duration_since(brillo.ultimo_paso).as_secs_f32();
    brillo.ultimo_paso = ahora;
    let objetivo = if fase == Some(Fase::Pensando) {
        1.0
    } else {
        0.0
    };
    brillo.pensar += (objetivo - brillo.pensar) * (1.0 - (-dt / CAMBIO_FASE).exp());
    brillo.flujo = (brillo.flujo
        + dt * (FLUJO_ESCUCHA + (FLUJO_PIENSA - FLUJO_ESCUCHA) * brillo.pensar))
        .fract();
    let (mut luz, mut fogonazo) = if reducidos {
        (1.0, 0.0)
    } else {
        envolvente(brillo.desde.elapsed())
    };
    if let Some((t, l, f)) = brillo.apagando {
        let queda = 1.0 - tema::C_TEMA.eval(tema::fraccion(t.elapsed(), SALE));
        (luz, fogonazo) = (l * queda, f * queda);
    }
    brillo.ultimo = (luz, fogonazo);

    let vivo = if reducidos {
        0.0
    } else {
        let t = brillo.desde.elapsed().as_secs_f32() / RESPIRA * std::f32::consts::TAU;
        // Escuchando manda la voz; pensando, sin voz que seguir, respira.
        (1.0 - brillo.pensar) * nivel + brillo.pensar * (0.25 + 0.25 * t.sin())
    };
    // El halo nace recogido en el filo y se abre según entra el color.
    let hondo = (HONDO + HONDO_VOZ * vivo) * (0.15 + 0.85 * luz);
    let intensidad = luz * (0.82 + 0.18 * vivo);

    let flujo = if reducidos { 0.0 } else { brillo.flujo };
    let valores = [flujo, hondo, intensidad, fogonazo];
    if brillo.subido != Some(valores) {
        let [a, b, c] = brillo.tonos;
        for (elemento, (x, y, fw, fh)) in brillo.franjas.iter_mut().zip(franjas(w, h, hondo)) {
            elemento.resize(Rectangle::new((x, y).into(), (fw, fh).into()), None);
            elemento.update_uniforms(vec![
                Uniform::new("origen", (x as f32, y as f32)),
                Uniform::new("pantalla", (w as f32, h as f32)),
                Uniform::new("flujo", flujo),
                Uniform::new("hondo", hondo),
                Uniform::new("luz", intensidad),
                Uniform::new("fogonazo", fogonazo),
                Uniform::new("isla", ISLA),
                Uniform::new("tono_a", (a[0], a[1], a[2])),
                Uniform::new("tono_b", (b[0], b[1], b[2])),
                Uniform::new("tono_c", (c[0], c[1], c[2])),
            ]);
        }
        brillo.subido = Some(valores);
    }
    brillo.franjas.to_vec()
}

/// `None` si el driver no compila el shader: Zenith sigue en la isla, sin borde.
// añadido en la 0.2.0
fn crear(
    renderer: &mut GlesRenderer,
    clave: (i32, i32, i32),
    contexto: ContextId<GlesTexture>,
    tonos: [[f32; 3]; 3],
    desde: Instant,
    ahora: Instant,
) -> Option<Brillo> {
    let programa = renderer
        .compile_custom_pixel_shader(
            FRAGMENTO,
            &[
                UniformName::new("origen", UniformType::_2f),
                UniformName::new("pantalla", UniformType::_2f),
                UniformName::new("flujo", UniformType::_1f),
                UniformName::new("hondo", UniformType::_1f),
                UniformName::new("luz", UniformType::_1f),
                UniformName::new("fogonazo", UniformType::_1f),
                UniformName::new("isla", UniformType::_1f),
                UniformName::new("tono_a", UniformType::_3f),
                UniformName::new("tono_b", UniformType::_3f),
                UniformName::new("tono_c", UniformType::_3f),
            ],
        )
        .inspect_err(|err| tracing::warn!("el brillo del asistente no compila: {err}"))
        .ok()?;
    let franja = || {
        PixelShaderElement::new(
            programa.clone(),
            Rectangle::default(),
            None,
            1.0,
            Vec::new(),
            Kind::Unspecified,
        )
    };
    Some(Brillo {
        clave,
        contexto,
        franjas: [franja(), franja(), franja(), franja()],
        tonos,
        desde,
        apagando: None,
        ultimo: (0.0, 0.0),
        flujo: 0.0,
        pensar: 0.0,
        ultimo_paso: ahora,
        subido: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_fogonazo_llega_antes_que_el_color_y_cede_ante_el() {
        assert_eq!(envolvente(Duration::ZERO), (0.0, 0.0));
        let (color, fogonazo) = envolvente(SUBE + SOSTIENE);
        assert_eq!((color, fogonazo), (0.0, 1.0));
        let (color, fogonazo) = envolvente(SUBE + SOSTIENE + CRUCE);
        assert_eq!((color, fogonazo), (1.0, 0.0));
    }

    #[test]
    fn las_franjas_cubren_el_contorno_sin_solaparse() {
        for hondo in [0.0, 3.0, 20.0, 46.0] {
            let [arriba, abajo, izq, der] = franjas(1440, 900, hondo);
            assert_eq!(arriba.1 + arriba.3, izq.1);
            assert_eq!(izq.1 + izq.3, abajo.1);
            assert_eq!(abajo.1 + abajo.3, 900);
            assert_eq!(der.0 + der.2, 1440);
            assert!(
                arriba.3 >= 3 && izq.2 >= 3,
                "el filo no cabe con hondo {hondo}"
            );
        }
    }

    #[test]
    fn franjas_en_una_salida_diminuta_no_salen_negativas() {
        let [_, abajo, izq, _] = franjas(10, 4, 46.0);
        assert!(abajo.1 >= 0 && izq.3 >= 1);
    }
}
