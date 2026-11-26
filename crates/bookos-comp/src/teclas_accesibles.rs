//! Teclas especiales y teclas lentas: un filtro **delante** de `input::tecla`.
//!
//! Va delante y no dentro porque las dos cosas deciden *si* y *cuándo* llega
//! una pulsación, y todo lo demás (atajos, conmutador, cliente con foco) tiene
//! que verla ya decidida. Lo que sale de aquí entra por `entregar_tecla`, que
//! es la entrada de siempre.
//!
//! - **Teclas lentas**: la pulsación espera `teclas_lentas` ms. Si se suelta
//!   antes, no ha existido: ni la pulsación ni la suelta llegan a nadie.
//! - **Teclas especiales**: Mayús, Ctrl y Alt (los dos lados, y AltGr) se
//!   pulsan de una en una. Soltar uno sin haber tocado otra tecla lo deja
//!   puesto para la siguiente; pulsarlo dos veces seguidas lo bloquea. Meta
//!   queda fuera: su pulsación sola abre el launchpad al soltarla, y retener
//!   esa suelta lo rompería.

use std::collections::HashMap;
use std::time::Duration;

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::state::BookosComp;

/// Los códigos de xkb (evdev + 8) de los modificadores que se pegan.
const MAYUS_IZQ: u32 = 42 + 8;
const MAYUS_DCHA: u32 = 54 + 8;
const CTRL_IZQ: u32 = 29 + 8;
const CTRL_DCHA: u32 = 97 + 8;
const ALT_IZQ: u32 = 56 + 8;
const ALT_GR: u32 = 100 + 8;

fn es_pegajosa(codigo: u32) -> bool {
    matches!(
        codigo,
        MAYUS_IZQ | MAYUS_DCHA | CTRL_IZQ | CTRL_DCHA | ALT_IZQ | ALT_GR
    )
}

/// Dónde está un modificador pegajoso. Sin entrada en el mapa es que no se
/// ha tocado.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pegada {
    /// La tecla está abajo. Si llega otra mientras tanto es un atajo normal
    /// (`con_otra`) y la suelta pasa sin más.
    Abajo { con_otra: bool },
    /// Soltada sin más: sigue puesta para la próxima tecla.
    Puesta,
    /// Se pulsó estando puesta: pasará a bloqueada al soltarla.
    AbajoParaBloquear,
    /// Bloqueada hasta que se pulse otra vez.
    Bloqueada,
    /// Se pulsó estando bloqueada: la suelta la libera.
    AbajoParaLiberar,
}

#[derive(Default)]
pub struct Estado {
    pub especiales: bool,
    /// Milisegundos; 0 apaga las teclas lentas.
    pub lentas: u32,
    /// Pulsaciones que esperan a cumplir el retardo.
    esperando: HashMap<u32, RegistrationToken>,
    /// Teclas ya entregadas por las lentas: su suelta tiene que pasar.
    entregadas: std::collections::HashSet<u32>,
    pegadas: HashMap<u32, Pegada>,
}

impl Estado {
    pub fn nuevo(especiales: bool, lentas: u32) -> Self {
        Self {
            especiales,
            lentas,
            ..Self::default()
        }
    }
}

/// La entrada de teclado de verdad (libinput, winit, autotests).
pub fn tecla(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
    let lentas = state.teclas_accesibles.lentas;
    if lentas == 0 && state.teclas_accesibles.esperando.is_empty() {
        pegajosas(state, codigo, estado, time);
        return;
    }
    let raw = codigo.raw();
    match estado {
        KeyState::Pressed => {
            // Apagadas mientras quedaba una esperando: esta ya no espera.
            if lentas == 0 {
                pegajosas(state, codigo, estado, time);
                return;
            }
            // Una pulsación que ya espera no se vuelve a armar.
            if state.teclas_accesibles.esperando.contains_key(&raw) {
                return;
            }
            let retardo = Duration::from_millis(u64::from(lentas));
            let token = state
                .loop_handle
                .insert_source(Timer::from_duration(retardo), move |_, _, state| {
                    state.teclas_accesibles.esperando.remove(&raw);
                    state.teclas_accesibles.entregadas.insert(raw);
                    // Con la hora de la pulsación más lo esperado: así la
                    // marca sigue la base de tiempo de libinput.
                    pegajosas(state, codigo, KeyState::Pressed, time.wrapping_add(lentas));
                    TimeoutAction::Drop
                })
                .expect("el bucle acepta un temporizador");
            state.teclas_accesibles.esperando.insert(raw, token);
        }
        KeyState::Released => {
            if let Some(token) = state.teclas_accesibles.esperando.remove(&raw) {
                // Se soltó antes de tiempo: no ha existido.
                state.loop_handle.remove(token);
                return;
            }
            state.teclas_accesibles.entregadas.remove(&raw);
            pegajosas(state, codigo, estado, time);
        }
    }
}

fn marcar_con_otra(pegadas: &mut HashMap<u32, Pegada>) {
    for p in pegadas.values_mut() {
        if let Pegada::Abajo { con_otra } = p {
            *con_otra = true;
        }
    }
}

/// Teclas especiales: qué teclas salen hacia el teclado por una pulsación o
/// una suelta, en orden. Separado de la entrega para poder probarlo sin un
/// compositor entero.
fn decidir(pegadas: &mut HashMap<u32, Pegada>, raw: u32, estado: KeyState) -> Vec<(u32, KeyState)> {
    let pulsada = estado == KeyState::Pressed;
    if !es_pegajosa(raw) {
        if pulsada {
            marcar_con_otra(pegadas);
        }
        let mut salida = vec![(raw, estado)];
        // Lo puesto vale para **una** tecla: se suelta después de entregarla,
        // para que el cliente la vea ya con el modificador.
        if pulsada {
            salida.extend(gastar(pegadas).into_iter().map(|c| (c, KeyState::Released)));
        }
        return salida;
    }
    let actual = pegadas.get(&raw).copied();
    // `true` = la tecla sale hacia el cliente; `false` = se queda aquí, y
    // para el teclado sigue pulsada.
    let entregar = match (estado, actual) {
        (KeyState::Pressed, None) => {
            pegadas.insert(raw, Pegada::Abajo { con_otra: false });
            true
        }
        (KeyState::Pressed, Some(Pegada::Puesta)) => {
            pegadas.insert(raw, Pegada::AbajoParaBloquear);
            false
        }
        (KeyState::Pressed, Some(Pegada::Bloqueada)) => {
            pegadas.insert(raw, Pegada::AbajoParaLiberar);
            false
        }
        // Una repetición de la tecla ya abajo.
        (KeyState::Pressed, Some(_)) => false,
        (KeyState::Released, Some(Pegada::Abajo { con_otra: false })) => {
            pegadas.insert(raw, Pegada::Puesta);
            false
        }
        (KeyState::Released, Some(Pegada::AbajoParaBloquear)) => {
            pegadas.insert(raw, Pegada::Bloqueada);
            false
        }
        (KeyState::Released, Some(Pegada::AbajoParaLiberar | Pegada::Abajo { .. })) => {
            pegadas.remove(&raw);
            true
        }
        // La suelta de una que ya estaba puesta o bloqueada y no se pulsó:
        // no hay nada que soltar.
        (KeyState::Released, Some(Pegada::Puesta | Pegada::Bloqueada)) => false,
        (KeyState::Released, None) => true,
    };
    if entregar {
        vec![(raw, estado)]
    } else {
        Vec::new()
    }
}

/// Los modificadores que estaban puestos para una sola tecla, ya olvidados.
/// Los bloqueados se quedan.
fn gastar(pegadas: &mut HashMap<u32, Pegada>) -> Vec<u32> {
    let puestas: Vec<u32> = pegadas
        .iter()
        .filter(|(_, p)| **p == Pegada::Puesta)
        .map(|(c, _)| *c)
        .collect();
    for raw in &puestas {
        pegadas.remove(raw);
    }
    puestas
}

fn pegajosas(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
    if !state.teclas_accesibles.especiales {
        crate::input::entregar_tecla(state, codigo, estado, time);
        return;
    }
    for (raw, estado) in decidir(&mut state.teclas_accesibles.pegadas, codigo.raw(), estado) {
        crate::input::entregar_tecla(state, Keycode::new(raw), estado, time);
    }
}

fn decidir_tras_clic(pegadas: &mut HashMap<u32, Pegada>) -> Vec<u32> {
    marcar_con_otra(pegadas);
    gastar(pegadas)
}

/// Un clic con un modificador puesto lo gasta, como una tecla.
// añadido en la 0.2.3
pub fn tras_clic(state: &mut BookosComp, time: u32) {
    if !state.teclas_accesibles.especiales {
        return;
    }
    for raw in decidir_tras_clic(&mut state.teclas_accesibles.pegadas) {
        crate::input::entregar_tecla(state, Keycode::new(raw), KeyState::Released, time);
    }
}

/// Aplica la configuración. Al apagar las teclas especiales se sueltan los
/// modificadores que quedaron pegados: el teclado los cree pulsados y, sin
/// esto, Mayús se quedaría puesta hasta reiniciar la sesión.
// añadido en la 0.2.3
pub fn configurar(state: &mut BookosComp, especiales: bool, lentas: u32) {
    if !especiales && state.teclas_accesibles.especiales {
        let tiempo = state.start_time.elapsed().as_millis() as u32;
        let pegadas: Vec<u32> = state
            .teclas_accesibles
            .pegadas
            .drain()
            // Las que siguen físicamente abajo se sueltan solas al levantar
            // el dedo: soltarlas aquí también sería una suelta doble.
            .filter(|(_, p)| matches!(p, Pegada::Puesta | Pegada::Bloqueada))
            .map(|(c, _)| c)
            .collect();
        for raw in pegadas {
            crate::input::entregar_tecla(state, Keycode::new(raw), KeyState::Released, tiempo);
        }
    }
    if lentas == 0 && !state.teclas_accesibles.esperando.is_empty() {
        for (_, token) in state.teclas_accesibles.esperando.drain() {
            state.loop_handle.remove(token);
        }
    }
    state.teclas_accesibles.especiales = especiales;
    state.teclas_accesibles.lentas = lentas;
}

#[cfg(test)]
mod pruebas {
    use super::*;

    use KeyState::{Pressed as P, Released as R};
    const A: u32 = 30 + 8;

    fn pasa(m: &mut HashMap<u32, Pegada>, raw: u32, e: KeyState) -> Vec<(u32, KeyState)> {
        decidir(m, raw, e)
    }

    #[test]
    fn un_toque_de_mayus_vale_para_la_tecla_siguiente() {
        let mut m = HashMap::new();
        assert_eq!(pasa(&mut m, MAYUS_IZQ, P), [(MAYUS_IZQ, P)]);
        // La suelta se queda aquí: para el teclado Mayús sigue pulsada.
        assert_eq!(pasa(&mut m, MAYUS_IZQ, R), []);
        // La A sale con Mayús puesta y, detrás, se suelta Mayús.
        assert_eq!(pasa(&mut m, A, P), [(A, P), (MAYUS_IZQ, R)]);
        assert_eq!(pasa(&mut m, A, R), [(A, R)]);
        // Y la siguiente ya va sola.
        assert_eq!(pasa(&mut m, A, P), [(A, P)]);
    }

    #[test]
    fn dos_toques_la_bloquean_y_un_tercero_la_libera() {
        let mut m = HashMap::new();
        pasa(&mut m, CTRL_IZQ, P);
        pasa(&mut m, CTRL_IZQ, R);
        assert_eq!(pasa(&mut m, CTRL_IZQ, P), []);
        assert_eq!(pasa(&mut m, CTRL_IZQ, R), []);
        // Bloqueada: dos letras seguidas y Ctrl no se suelta.
        assert_eq!(pasa(&mut m, A, P), [(A, P)]);
        assert_eq!(pasa(&mut m, A, R), [(A, R)]);
        assert_eq!(pasa(&mut m, A, P), [(A, P)]);
        // El tercer toque: la pulsación no sale, la suelta libera.
        assert_eq!(pasa(&mut m, CTRL_IZQ, P), []);
        assert_eq!(pasa(&mut m, CTRL_IZQ, R), [(CTRL_IZQ, R)]);
        assert!(m.is_empty());
    }

    #[test]
    fn un_atajo_con_la_tecla_mantenida_no_pega_nada() {
        let mut m = HashMap::new();
        assert_eq!(pasa(&mut m, MAYUS_IZQ, P), [(MAYUS_IZQ, P)]);
        assert_eq!(pasa(&mut m, A, P), [(A, P)]);
        assert_eq!(pasa(&mut m, A, R), [(A, R)]);
        // Se usó con otra tecla: la suelta sale y no queda puesta.
        assert_eq!(pasa(&mut m, MAYUS_IZQ, R), [(MAYUS_IZQ, R)]);
        assert!(m.is_empty());
    }

    #[test]
    fn varios_modificadores_puestos_se_sueltan_con_una_tecla() {
        let mut m = HashMap::new();
        for c in [CTRL_IZQ, ALT_IZQ] {
            pasa(&mut m, c, P);
            pasa(&mut m, c, R);
        }
        let salida = pasa(&mut m, A, P);
        assert_eq!(salida[0], (A, P));
        assert_eq!(salida.len(), 3);
        assert!(salida[1..].iter().all(|(_, e)| *e == R));
    }

    #[test]
    fn solo_se_pegan_mayus_ctrl_y_alt() {
        assert!(es_pegajosa(MAYUS_IZQ) && es_pegajosa(ALT_GR) && es_pegajosa(CTRL_DCHA));
        // Super_L (evdev 125) abre el launchpad al soltarse: no se pega.
        assert!(!es_pegajosa(125 + 8));
        // Una letra (A, evdev 30).
        assert!(!es_pegajosa(30 + 8));
    }

    #[test]
    fn ctrl_mantenido_con_clic_no_queda_pegado_al_soltar() {
        let mut m = HashMap::new();
        assert_eq!(pasa(&mut m, CTRL_IZQ, P), [(CTRL_IZQ, P)]);
        assert!(decidir_tras_clic(&mut m).is_empty());
        // Como ya se usó en Ctrl+clic, al soltar Ctrl debe salir el Released normal
        // y no quedarse pegado para la siguiente letra.
        assert_eq!(pasa(&mut m, CTRL_IZQ, R), [(CTRL_IZQ, R)]);
        assert!(m.is_empty());
    }
}
