//! Atenuación de la pantalla: bajar el brillo antes de que pase algo.
//!
//! Dos motivos, los dos con el brillo real del panel (la retroiluminación) y no
//! con un velo negro: un velo no ahorra un vatio y bajar el brillo sí, que es
//! de lo que va una batería a punto de acabarse.
//!
//! - **Inactividad**: a los `atenuacion_inactividad` segundos (cinco minutos
//!   por defecto) el brillo baja a un [`FRACCION_INACTIVIDAD`] del que había,
//!   como las teles que se oscurecen antes del salvapantallas. Es el primer
//!   escalón; luego vienen el bloqueo, el salvapantallas y el apagado. El
//!   primer movimiento lo devuelve de inmediato.
//! - **Batería baja**: al cruzar [`UMBRAL_BATERIA`] sin cargador, a la mitad.
//!   No vuelve con la actividad —el usuario no se ha ido, la batería sigue
//!   igual de baja— sino al enchufar o al subir de [`UMBRAL_BATERIA`] más
//!   [`HISTERESIS_BATERIA`].
//!
//! Lo que se baja es lo que haya puesto el usuario o el sensor de luz; al
//! volver se escribe ese mismo nivel. Si el usuario toca la tecla de brillo
//! mientras está atenuada se da por terminada: lo que acaba de elegir manda
//! sobre lo que había antes (ver [`olvidar`]).

use std::time::Duration;

use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::state::BookosComp;

const FRACCION_INACTIVIDAD: f64 = 0.3;
const FRACCION_BATERIA: f64 = 0.5;
/// Porcentaje de batería al que se atenúa, y lo que tiene que subir para soltar.
const UMBRAL_BATERIA: u8 = 10;
const HISTERESIS_BATERIA: u8 = 5;
/// El suelo de la tecla de brillo (`multimedia::brillo`): por debajo, la
/// pantalla se ve apagada y el usuario cree que se ha colgado.
const SUELO: u8 = 5;
/// La bajada es una rampa, no un salto. Un segundo, en pasos de 40 ms;
/// subir no lleva rampa porque quien vuelve quiere ver.
const RAMPA: Duration = Duration::from_secs(1);
const PASO_RAMPA: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Motivo {
    Inactividad,
    Bateria,
}

struct Atenuada {
    motivo: Motivo,
    /// El brillo, en tanto por ciento, al que hay que volver.
    previo: u8,
}

pub struct Estado {
    pub inactividad: Duration,
    atenuada: Option<Atenuada>,
    alarma: Option<RegistrationToken>,
    rampa: Option<RegistrationToken>,
}

impl Estado {
    pub fn nuevo(segundos: u64) -> Self {
        Self {
            inactividad: Duration::from_secs(segundos),
            atenuada: None,
            alarma: None,
            rampa: None,
        }
    }

    pub fn activa(&self) -> bool {
        self.atenuada.is_some()
    }
}

/// Programa la alarma de la atenuación por inactividad, con el mismo criterio
/// que el bloqueo (`backend::programar_bloqueo_inactividad`, que la llama): una
/// alarma única que al vencer comprueba la edad de la última entrada en vez de
/// reiniciarse con cada movimiento del ratón.
pub fn programar(state: &mut BookosComp) {
    if let Some(token) = state.atenuacion.alarma.take() {
        state.loop_handle.remove(token);
    }
    let plazo = state.atenuacion.inactividad;
    tracing::debug!(?plazo, "programando la atenuación");
    if plazo.is_zero()
        || !state.idle_inhibidores.is_empty()
        || bookos_shell::mantener_despierto()
        || state.atenuacion.atenuada.is_some()
        || state.shell.as_ref().is_none_or(|s| s.esta_bloqueado())
    {
        return;
    }
    let desde_entrada = state.last_input.map(|t| t.elapsed()).unwrap_or_default();
    let espera = plazo
        .saturating_sub(desde_entrada)
        .max(Duration::from_millis(50));
    let token =
        state
            .loop_handle
            .insert_source(Timer::from_duration(espera), move |_, _, state| {
                state.atenuacion.alarma = None;
                tracing::debug!(
                    inactivo = ?state.last_input.map(|t| t.elapsed()),
                    ?plazo,
                    "alarma de atenuación vencida"
                );
                if !state.idle_inhibidores.is_empty()
                    || bookos_shell::mantener_despierto()
                    || state.shell.as_ref().is_none_or(|s| s.esta_bloqueado())
                {
                    return TimeoutAction::Drop;
                }
                if state.last_input.is_some_and(|t| t.elapsed() >= plazo) {
                    atenuar(state, Motivo::Inactividad);
                } else {
                    programar(state);
                }
                TimeoutAction::Drop
            });
    match token {
        Ok(token) => state.atenuacion.alarma = Some(token),
        Err(err) => tracing::error!("no se pudo programar la atenuación: {err}"),
    }
}

fn atenuar(state: &mut BookosComp, motivo: Motivo) {
    if state.atenuacion.atenuada.is_some() || !state.dpms_encendido {
        return;
    }
    let (Some(previo), Some((dispositivo, maximo))) =
        (bookos_shell::brillo_actual(), bookos_shell::backlight())
    else {
        return;
    };
    let fraccion = match motivo {
        Motivo::Inactividad => FRACCION_INACTIVIDAD,
        Motivo::Bateria => FRACCION_BATERIA,
    };
    let objetivo = ((f64::from(previo) * fraccion).round() as u8).max(SUELO);
    // Ya en el suelo o por debajo: no hay nada que bajar, y apuntarlo como
    // atenuada devolvería un brillo que el usuario ya tenía.
    if objetivo >= previo {
        return;
    }
    tracing::info!(
        previo,
        objetivo,
        motivo = if motivo == Motivo::Bateria {
            "batería"
        } else {
            "inactividad"
        },
        "pantalla atenuada"
    );
    state.atenuacion.atenuada = Some(Atenuada { motivo, previo });
    crate::brillo_auto::fijar(state, objetivo);

    let pasos = (RAMPA.as_millis() / PASO_RAMPA.as_millis()).max(1) as u32;
    let mut paso = 0;
    let token = state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            paso += 1;
            let nivel = f64::from(previo)
                + (f64::from(objetivo) - f64::from(previo)) * f64::from(paso) / f64::from(pasos);
            let crudo = (f64::from(maximo) * nivel / 100.0).round() as u32;
            bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
            if paso >= pasos {
                state.atenuacion.rampa = None;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(PASO_RAMPA)
            }
        });
    state.atenuacion.rampa = token.ok();
}

/// Devuelve el brillo de antes. Sin rampa y sin comprobar nada más: la
/// escritura es la misma que la de la rampa de bajada y la última gana.
fn restaurar(state: &mut BookosComp) {
    let Some(atenuada) = state.atenuacion.atenuada.take() else {
        return;
    };
    if let Some(token) = state.atenuacion.rampa.take() {
        state.loop_handle.remove(token);
    }
    if let Some((dispositivo, maximo)) = bookos_shell::backlight() {
        let crudo = (f64::from(maximo) * f64::from(atenuada.previo) / 100.0).round() as u32;
        bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
    }
    crate::brillo_auto::fijar(state, atenuada.previo);
    tracing::info!(previo = atenuada.previo, "pantalla restaurada");
    crate::brillo_auto::evaluar(state);
    programar(state);
}

/// Cualquier entrada del usuario. Es lo primero que hace `input::handle`, así
/// que la comprobación de reposo tiene que costar un `match`: llega a 1 kHz.
pub fn actividad(state: &mut BookosComp) {
    if matches!(&state.atenuacion.atenuada, Some(a) if a.motivo == Motivo::Inactividad) {
        restaurar(state);
    }
}

/// El usuario ha elegido un brillo con la tecla: lo atenuado ya no es lo que
/// había que devolver.
pub fn olvidar(state: &mut BookosComp) {
    if state.atenuacion.atenuada.take().is_some() {
        if let Some(token) = state.atenuacion.rampa.take() {
            state.loop_handle.remove(token);
        }
        programar(state);
    }
}

/// Se mira en cada aviso de hardware, junto al cargador: la batería cambia de
/// porcentaje y de adaptador por udev y no hay que sondearla.
pub fn bateria(state: &mut BookosComp) {
    let Some((enchufado, porciento)) = bookos_shell::cargador() else {
        return;
    };
    let por_bateria = matches!(&state.atenuacion.atenuada, Some(a) if a.motivo == Motivo::Bateria);
    if por_bateria {
        if enchufado || porciento > UMBRAL_BATERIA + HISTERESIS_BATERIA {
            restaurar(state);
        }
    } else if !enchufado && porciento <= UMBRAL_BATERIA {
        atenuar(state, Motivo::Bateria);
    }
}
