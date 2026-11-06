//! La luz nocturna: teñir la pantalla de cálido por la noche.
//!
//! ## Por qué la curva de gamma del CRTC y no un sombreador
//!
//! El tinte se aplica en el último paso del hardware: la tabla de gamma que el
//! controlador de pantalla aplica a cada píxel ya compuesto, camino del panel.
//! Cuesta **cero** por fotograma —no hay una pasada más de GPU a 2880×1800 y
//! 120 Hz—, no rompe el escaneo directo de una ventana a pantalla completa y
//! tiñe también el cursor, que va en su propio plano y un sombreador no vería.
//! Y como no toca lo compuesto, las capturas y la pantalla compartida salen con
//! sus colores de verdad, que es lo que se espera de ellas.
//!
//! El precio: solo existe en la sesión real. Anidado, la pantalla es del
//! compositor anfitrión y no hay curva que tocar; ahí la configuración se
//! guarda y se ve en las tarjetas, pero no tiñe nada.
//!
//! ## El horario: un despertar por cambio
//!
//! Como el tema automático (ver [`crate::apariencia`]): se calcula cuánto falta
//! para el próximo borde del horario y se programa **ese** instante.
//!
//! ## Encenderla a mano con el horario puesto
//!
//! La baldosa no cambia el modo: quien tiene un horario y la apaga una noche no
//! quiere perderlo. Se apunta como forzada y el horario vuelve a mandar en el
//! siguiente borde —apagarla a las once deja de valer a las siete—, que es lo
//! que hace Night Shift.

use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use bookos_shell::{LuzNocturna, ModoLuzNocturna};

use crate::state::BookosComp;

/// Aplica la curva a todas las pantallas encendidas. La pone el backend real.
pub type AplicarGamma = Box<dyn FnMut([f32; 3]) -> Result<(), String>>;

/// Lo que dura el fundido al encender o apagar.
///
/// Corto y en pocos pasos a propósito: cada paso es un `SETGAMMA`, que el
/// kernel resuelve con un commit atómico **bloqueante** y espera al vblank
/// —hasta 8 ms a 120 Hz— en el hilo del compositor. Ocho pasos son ocho
/// fotogramas que pueden llegar tarde en medio segundo; un fundido largo y fino
/// serían cientos.
const PASOS: u32 = 8;
const PASO: std::time::Duration = std::time::Duration::from_millis(60);

/// Por lo mismo que en el tema: el cálculo va en minutos y el despertar cae
/// siempre después del minuto en punto.
const MINIMO: std::time::Duration = std::time::Duration::from_secs(70);

pub struct Estado {
    pub config: LuzNocturna,
    /// La temperatura que hay puesta en la curva. [`LuzNocturna::NEUTRA`] es
    /// sin teñir.
    aplicada: u32,
    /// Encendida o apagada a mano con el horario puesto; vale hasta el próximo
    /// borde del horario.
    forzada: Option<bool>,
    /// El despertar del próximo borde del horario.
    tick: Option<RegistrationToken>,
    /// El fundido en marcha.
    fundido: Option<RegistrationToken>,
}

impl Estado {
    // 0.1.0
    pub fn nuevo(config: LuzNocturna) -> Self {
        Self {
            config,
            aplicada: LuzNocturna::NEUTRA,
            forzada: None,
            tick: None,
            fundido: None,
        }
    }

    // 0.1.0
    fn encendida(&self, ahora: bookos_shell::tema::HoraDelDia) -> bool {
        self.forzada.unwrap_or_else(|| self.config.toca(ahora))
    }

    /// La temperatura a la que hay que ir ahora.
    // 0.1.0
    fn objetivo(&self, ahora: bookos_shell::tema::HoraDelDia) -> u32 {
        if self.encendida(ahora) {
            self.config.temperatura
        } else {
            LuzNocturna::NEUTRA
        }
    }
}

/// Lo que se multiplica cada canal a esa temperatura, normalizado para que
/// 6500 K sea `[1, 1, 1]`.
///
/// Es la aproximación de Tanner Helland a la curva del cuerpo negro: tres
/// tramos ajustados sobre los datos de Mitchell Charity, con un error de un par
/// de unidades sobre 255 entre 1000 y 40 000 K. Para teñir una pantalla es de
/// sobra; la tabla exacta de redshift serían doscientas líneas de números para
/// una diferencia que no se ve.
// 0.1.0
pub fn multiplicadores(kelvin: u32) -> [f32; 3] {
    // 0.1.0
    fn crudo(kelvin: u32) -> [f32; 3] {
        let t = kelvin as f32 / 100.0;
        let r = if t <= 66.0 {
            255.0
        } else {
            329.698_73 * (t - 60.0).powf(-0.133_204_76)
        };
        let g = if t <= 66.0 {
            99.470_8 * t.ln() - 161.119_57
        } else {
            288.122_17 * (t - 60.0).powf(-0.075_514_85)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_73 * (t - 10.0).ln() - 305.044_8
        };
        [r, g, b].map(|c| c.clamp(0.0, 255.0) / 255.0)
    }
    // A 6500 K la fórmula no da el blanco exacto (verde 0,997 y azul 0,980):
    // sin normalizar, «sin teñir» ya amarillearía un poco.
    let base = crudo(LuzNocturna::NEUTRA);
    let c = crudo(kelvin);
    [0, 1, 2].map(|i| (c[i] / base[i]).clamp(0.0, 1.0))
}

/// Una rampa de gamma de `n` entradas para un canal: la identidad escalada.
// 0.1.0
pub fn rampa(n: usize, multiplicador: f32) -> Vec<u16> {
    let ultimo = n.saturating_sub(1).max(1) as f32;
    (0..n)
        .map(|i| (i as f32 / ultimo * multiplicador * 65535.0).round() as u16)
        .collect()
}

/// Lee la configuración y va a lo que toque. Se llama al arrancar y al recargar
/// los ajustes. Cambiar la configuración desde Ajustes descarta lo forzado a
/// mano: quien acaba de elegir un modo quiere ver ese modo.
// 0.1.0
pub fn recargar(state: &mut BookosComp, config: LuzNocturna) {
    state.luz_nocturna.config = config;
    state.luz_nocturna.forzada = None;
    ir_al_objetivo(state);
    programar(state);
}

/// Encender o apagar desde la baldosa o la tarjeta de brillo.
// 0.1.0
pub fn alternar(state: &mut BookosComp, encender: bool) {
    let luz = &mut state.luz_nocturna;
    match luz.config.modo {
        // Con horario se fuerza hasta el próximo borde; ver la cabecera.
        ModoLuzNocturna::Programada => luz.forzada = Some(encender),
        ModoLuzNocturna::Apagada | ModoLuzNocturna::Encendida => {
            luz.config.modo = if encender {
                ModoLuzNocturna::Encendida
            } else {
                ModoLuzNocturna::Apagada
            };
            if let Err(err) = bookos_shell::guardar_luz_nocturna(luz.config.modo) {
                tracing::warn!("no se pudo guardar la luz nocturna: {err}");
            }
        }
    }
    ir_al_objetivo(state);
}

/// Vuelve a poner la curva que había, sin fundido. Hace falta después de que
/// otro haya podido tocarla: al volver de otro TTY —el maestro de allí la deja
/// como quiere— y al encender una pantalla nueva, que nace con la identidad.
// 0.1.0
pub fn reponer(state: &mut BookosComp) {
    let aplicada = state.luz_nocturna.aplicada;
    if aplicada != LuzNocturna::NEUTRA {
        poner(state, aplicada);
    }
}

/// Funde de lo que hay a lo que toca ahora.
// 0.1.0
fn ir_al_objetivo(state: &mut BookosComp) {
    let ahora = bookos_shell::hora_local_ahora();
    let objetivo = state.luz_nocturna.objetivo(ahora);
    bookos_shell::marcar_luz_nocturna(state.luz_nocturna.encendida(ahora));
    // Las tarjetas abiertas enseñan el estado: que se repinten.
    state.needs_redraw = true;
    if let Some(token) = state.luz_nocturna.fundido.take() {
        state.loop_handle.remove(token);
    }
    let desde = state.luz_nocturna.aplicada;
    if desde == objetivo {
        return;
    }
    // Sin animaciones no hay fundido: el cambio entero en un paso.
    if bookos_shell::tema::efectos_reducidos() {
        poner(state, objetivo);
        return;
    }
    let mut paso = 0;
    let token = state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            paso += 1;
            let f = paso as f32 / PASOS as f32;
            let k = desde as f32 + (objetivo as f32 - desde as f32) * f;
            poner(state, k.round() as u32);
            if paso >= PASOS {
                state.luz_nocturna.fundido = None;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(PASO)
            }
        });
    match token {
        Ok(token) => state.luz_nocturna.fundido = Some(token),
        Err(err) => {
            tracing::warn!("no se pudo programar el fundido de la luz nocturna: {err}");
            poner(state, objetivo);
        }
    }
}

// 0.1.0
fn poner(state: &mut BookosComp, kelvin: u32) {
    state.luz_nocturna.aplicada = kelvin;
    let Some(aplicar) = state.aplicar_gamma.as_mut() else {
        return;
    };
    if let Err(err) = aplicar(multiplicadores(kelvin)) {
        tracing::warn!(kelvin, "no se pudo aplicar la luz nocturna: {err}");
    }
}

/// Programa el despertar del próximo borde del horario, o lo quita.
// 0.1.0
fn programar(state: &mut BookosComp) {
    if let Some(token) = state.luz_nocturna.tick.take() {
        state.loop_handle.remove(token);
    }
    let Some(minutos) = state
        .luz_nocturna
        .config
        .minutos_al_cambio(bookos_shell::hora_local_ahora())
    else {
        return;
    };
    let falta = std::time::Duration::from_secs(minutos as u64 * 60).max(MINIMO);
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(falta), |_, _, state| {
            state.luz_nocturna.tick = None;
            // En el borde el horario vuelve a mandar.
            state.luz_nocturna.forzada = None;
            ir_al_objetivo(state);
            programar(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(token) => state.luz_nocturna.tick = Some(token),
        Err(err) => tracing::error!("no se pudo programar la luz nocturna: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_6500_no_tine_nada() {
        assert_eq!(multiplicadores(LuzNocturna::NEUTRA), [1.0, 1.0, 1.0]);
    }

    /// Cálida quita azul y algo de verde, nunca rojo; fría quita rojo.
    #[test]
    fn calida_quita_azul_y_fria_quita_rojo() {
        let [r, g, b] = multiplicadores(4000);
        assert_eq!(r, 1.0);
        assert!(g < 1.0 && b < g, "a 4000 K: {g} {b}");
        let [r, g, b] = multiplicadores(LuzNocturna::MAXIMA);
        assert!(r < g && g < 1.0 && b == 1.0, "a 8000 K: {r} {g} {b}");
    }

    /// Más cálida, menos azul: el deslizador tiene que ir siempre en el mismo
    /// sentido o el usuario no sabe hacia dónde moverlo.
    #[test]
    fn el_azul_baja_con_la_temperatura() {
        let azules: Vec<f32> = (LuzNocturna::MINIMA..=LuzNocturna::NEUTRA)
            .step_by(100)
            .map(|k| multiplicadores(k)[2])
            .collect();
        assert!(azules.windows(2).all(|p| p[0] <= p[1]), "{azules:?}");
        assert!(azules[0] > 0.0, "a 2500 K queda algo de azul");
    }

    #[test]
    fn la_rampa_va_de_cero_al_multiplicador() {
        let r = rampa(256, 1.0);
        assert_eq!((r[0], r[255]), (0, 65535));
        let r = rampa(1024, 0.5);
        assert_eq!(r[1023], 32768);
        assert!(r.windows(2).all(|p| p[0] <= p[1]));
    }
}
