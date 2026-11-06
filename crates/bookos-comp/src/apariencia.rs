//! El tema automático: claro de día y oscuro de noche.
//!
//! ## Un despertar por cambio, no un sondeo
//!
//! Lo evidente sería un temporizador de un minuto que mire la hora. Serían
//! **mil cuatrocientos cuarenta despertares al día para dos cambios**, y cada
//! uno saca a la CPU de su C-state: es justo lo que este proyecto no hace.
//!
//! Aquí se calcula cuánto falta para el próximo cambio y se programa **ese**
//! instante. Al despertar se aplica el tema y se vuelve a programar el
//! siguiente. Con un modo fijo no hay temporizador ninguno.
//!
//! ## Por qué dos horas y no el amanecer de verdad
//!
//! Calcular el amanecer pide una posición, y sacarla del sistema significa
//! geoclue —un servicio con su permiso y su agente— o pedirle al usuario que
//! escriba latitud y longitud en un fichero. Dos horas se entienden solas. Si
//! algún día hay posición, se calculan desde ella y no cambia nada más: todo lo
//! de aquí abajo habla solo de esas dos horas.

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::state::BookosComp;

/// Cuánto se espera como poco antes de volver a mirar.
///
/// El cálculo va en minutos, así que un cambio previsto para «dentro de un
/// minuto» puede caer unos segundos antes de la hora por el redondeo del reloj.
/// Con este suelo, el despertar cae siempre **después** del minuto en punto y
/// no hay que volver a programarlo por unos segundos.
const MINIMO: std::time::Duration = std::time::Duration::from_secs(70);

/// Programa el despertar del próximo cambio automático, o lo cancela.
///
/// Se llama al arrancar, al elegir en la tarjeta de Apariencia y cada vez que
/// el propio temporizador salta. Es idempotente: siempre suelta el anterior
/// antes de poner otro, así que llamarla de más no acumula temporizadores.
// 0.1.0
pub fn programar_cambio(state: &mut BookosComp) {
    if let Some(token) = state.tick_tema.take() {
        state.loop_handle.remove(token);
    }
    let Some(falta) = state.hasta_el_cambio_de_tema() else {
        // Modo fijo: no hay nada que esperar y no se deja ningún temporizador.
        return;
    };
    let falta = falta.max(MINIMO);
    tracing::debug!(
        minutos = falta.as_secs() / 60,
        "próximo cambio automático de tema"
    );
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(falta), |_, _, state| {
            aplicar_el_que_toca(state);
            // El siguiente se programa desde dentro y este se suelta: un timer
            // que se repitiera solo tendría que acertar el periodo, y el periodo
            // no es constante —de las 20:00 a las 07:00 hay once horas y al
            // revés trece—.
            state.tick_tema = None;
            programar_cambio(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(token) => state.tick_tema = Some(token),
        Err(err) => tracing::error!("no se pudo programar el cambio de tema: {err}"),
    }
}

/// Pone el tema que toca a esta hora, si no es el que ya está.
///
/// Es lo mismo que hace la tarjeta de Apariencia al pulsar, menos guardar: aquí
/// no ha elegido nadie nada, solo ha pasado el tiempo, y reescribir el fichero
/// en cada amanecer sería tocar el disco para no decir nada nuevo.
// 0.1.0
pub fn aplicar_el_que_toca(state: &mut BookosComp) {
    let tema = state.tema_que_toca();
    if bookos_shell::tema::actual() == tema {
        return;
    }
    let acento = bookos_shell::tema::acento_actual();
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    if !shell.aplicar_apariencia(tema, acento) {
        return;
    }
    if state.fondo_config.depende_del_tema() {
        state.recargar_fondo = true;
    }
    tracing::info!(?tema, "tema automático");
    // El cambio de las ocho de la tarde también tiene que llegarle a las
    // aplicaciones de fuera, no solo al escritorio.
    crate::portal::apariencia_cambiada(state.bus_portal.as_ref());
    state.needs_redraw = true;
}
