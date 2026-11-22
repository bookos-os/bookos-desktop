//! El shell en el bus de accesibilidad (AT-SPI2), para Orca.
//!
//! El árbol lo describe `bookos_shell::accesible`; esto lo publica con
//! `accesskit_unix` y devuelve al bucle lo que pide el lector.
//!
//! Los manejadores de AccessKit corren en su propio hilo y el shell vive en
//! el de calloop, así que todo pasa por un canal: igual que el portal y el
//! servidor de Ajustes. El árbol solo se construye mientras hay un lector
//! escuchando (`activo`), y solo se manda si ha cambiado.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::{self, Event, Sender};

use crate::state::BookosComp;

enum Aviso {
    /// Un lector se ha conectado: hay que mandarle el árbol entero ya.
    Activar,
    Accion(ActionRequest),
}

pub struct Accesibilidad {
    adaptador: accesskit_unix::Adapter,
    activo: Arc<AtomicBool>,
    /// Lo último enviado: si el árbol nuevo es igual, no se manda nada, y de
    /// aquí salen los centros de los nodos que pulsa el lector.
    ultimo: Option<bookos_shell::accesible::Arbol>,
    /// Hay que mandar el árbol completo aunque no haya cambiado: lo acaba de
    /// pedir un lector.
    completo: bool,
    /// Cuándo se construyó el último árbol.
    construido: std::time::Instant,
    /// Hay un temporizador en marcha para la vuelta que se saltó por `CADA`.
    /// Sin él, el último cambio antes de quedarse quieta la pantalla no
    /// llegaba nunca: sin fotogramas nadie vuelve a llamar a `actualizar`.
    aplazado: bool,
}

/// Cada cuánto, como mucho, se rehace el árbol. Se llama en cada fotograma y
/// durante una animación son 120 por segundo: el brillo lee sysfs para su
/// valor. Una décima es más rápido de lo que Orca empieza a hablar.
const CADA: std::time::Duration = std::time::Duration::from_millis(100);

struct Activacion(Sender<Aviso>, Arc<AtomicBool>);
impl ActivationHandler for Activacion {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // El árbol solo se puede construir en el hilo del shell. AccessKit
        // admite `None` si llega antes del siguiente fotograma: lo mandamos
        // en cuanto el bucle procese el aviso.
        self.1.store(true, Ordering::Relaxed);
        let _ = self.0.send(Aviso::Activar);
        None
    }
}

struct Acciones(Sender<Aviso>);
impl ActionHandler for Acciones {
    fn do_action(&mut self, peticion: ActionRequest) {
        let _ = self.0.send(Aviso::Accion(peticion));
    }
}

struct Desactivacion(Arc<AtomicBool>);
impl DeactivationHandler for Desactivacion {
    fn deactivate_accessibility(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

// añadido en la 0.2.2
pub fn arrancar(loop_handle: &LoopHandle<'static, BookosComp>) -> Option<Accesibilidad> {
    let (emisor, receptor) = channel::channel::<Aviso>();
    let registrado = loop_handle.insert_source(receptor, |evento, _, state| {
        let Event::Msg(aviso) = evento else {
            return;
        };
        match aviso {
            Aviso::Activar => {
                if let Some(a) = state.accesibilidad.as_mut() {
                    a.completo = true;
                }
                actualizar(state);
            }
            Aviso::Accion(peticion) => accion(state, peticion),
        }
    });
    if let Err(err) = registrado {
        tracing::warn!("sin accesibilidad: {err}");
        return None;
    }
    let activo = Arc::new(AtomicBool::new(false));
    let adaptador = accesskit_unix::Adapter::new(
        Activacion(emisor.clone(), activo.clone()),
        Acciones(emisor),
        Desactivacion(activo.clone()),
    );
    Some(Accesibilidad {
        adaptador,
        activo,
        ultimo: None,
        completo: false,
        construido: std::time::Instant::now(),
        aplazado: false,
    })
}

/// Rehace el árbol si hay un lector escuchando y lo manda si cambió. Se llama
/// una vez por vuelta del bucle; sin lector cuesta una lectura atómica.
// añadido en la 0.2.2
pub fn actualizar(state: &mut BookosComp) {
    let Some(a) = state.accesibilidad.as_mut() else {
        return;
    };
    if !a.activo.load(Ordering::Relaxed) {
        a.ultimo = None;
        return;
    }
    if !a.completo && a.construido.elapsed() < CADA {
        if !std::mem::replace(&mut a.aplazado, true) {
            use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
            let falta = CADA.saturating_sub(a.construido.elapsed());
            let programado =
                state
                    .loop_handle
                    .insert_source(Timer::from_duration(falta), |_, _, state| {
                        if let Some(a) = state.accesibilidad.as_mut() {
                            a.aplazado = false;
                        }
                        actualizar(state);
                        TimeoutAction::Drop
                    });
            if let Err(err) = programado {
                tracing::warn!("árbol accesible sin aplazar: {err}");
            }
        }
        return;
    }
    a.construido = std::time::Instant::now();
    let Some(arbol) = state.shell.as_mut().and_then(|s| s.arbol_accesible()) else {
        return;
    };
    let Some(a) = state.accesibilidad.as_mut() else {
        return;
    };
    if !a.completo
        && a.ultimo
            .as_ref()
            .is_some_and(|u| u.actualizacion == arbol.actualizacion)
    {
        return;
    }
    a.completo = false;
    a.adaptador.update_window_focus_state(arbol.con_foco);
    let actualizacion = arbol.actualizacion.clone();
    a.ultimo = Some(arbol);
    a.adaptador.update_if_active(|| actualizacion);
}

/// Lo que pide el lector. Pulsar es un clic en el centro del nodo: el mismo
/// camino que el ratón, para que no haya una segunda forma de activar nada.
fn accion(state: &mut BookosComp, peticion: ActionRequest) {
    if peticion.action != accesskit::Action::Click {
        return;
    }
    let Some((x, y)) = state
        .accesibilidad
        .as_ref()
        .and_then(|a| a.ultimo.as_ref())
        .and_then(|arbol| arbol.centro(peticion.target_node))
    else {
        return;
    };
    if let Some(accion) = state.shell.as_mut().and_then(|s| s.pulsar(x, y)) {
        crate::keybinds::hacer(state, accion);
    }
    if let Some(accion) = state.shell.as_mut().and_then(|s| s.soltar()) {
        crate::keybinds::hacer(state, accion);
    }
    state.needs_redraw = true;
}
