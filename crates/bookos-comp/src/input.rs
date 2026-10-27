//! Tratamiento de la entrada, común a los dos backends.
//!
//! Está escrito contra `InputBackend` en vez de contra un backend concreto, así
//! que el mismo código sirve para winit (anidado) y para libinput (sesión real).
//! Es deliberado: un fallo de coordenadas o de foco arreglado aquí queda
//! arreglado en los dos sitios, y ya nos ha mordido una vez.
//!
//! La diferencia entre ambos es solo cómo llega el puntero: winit da posición
//! absoluta normalizada y libinput da desplazamientos relativos. Los dos
//! terminan en el mismo sitio, [`set_pointer`].

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
    GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
    TouchSlot,
};
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::RelativeMotionEvent;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::input::touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};

use bookos_shell::TeclaPulsada;

use crate::gestos::Gesto;
use crate::state::{ArrastreEscritorio, BookosComp};

/// Botones del ratón, según linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Los modificadores pulsados ahora mismo.
///
/// Un evento de botón no los trae: hay que preguntárselos al teclado, que es
/// quien lleva la cuenta.
fn modificadores(state: &BookosComp) -> smithay::input::keyboard::ModifiersState {
    state
        .seat
        .get_keyboard()
        .map(|kbd| kbd.modifier_state())
        .unwrap_or_default()
}

fn modificador_conmutador_suelto(
    modo: bookos_shell::conmutador::Modo,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    match modo {
        bookos_shell::conmutador::Modo::Aplicaciones => !modifiers.alt,
        bookos_shell::conmutador::Modo::Ventanas => !modifiers.logo,
    }
}

/// Una tecla, ya traducida por `teclas.conf`, camino de xkb y del cliente.
///
/// `codigo` es el de xkb (evdev + 8). Se conserva el código y no solo el keysym
/// porque algunas teclas Fn llegan sin keysym aunque el kernel sí haya
/// identificado correctamente su botón físico.
/// Una tecla, venga de libinput, de winit o de un autotest.
pub fn tecla(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
    let Some(kbd) = state.seat.get_keyboard() else {
        return;
    };
    let serial = SERIAL_COUNTER.next_serial();
    let pulsada = estado == KeyState::Pressed;
    let tapado = pulsada && crate::salvapantallas::actividad(state);
    // Cualquier tecla, pulsada o suelta, corta la repetición: si sigue
    // Retroceso mantenido, la pulsación siguiente la vuelve a armar.
    if let Some(token) = state.repeticion_retroceso.take() {
        state.loop_handle.remove(token);
    }
    if let Some(token) = state.repeticion_multimedia.take() {
        state.loop_handle.remove(token);
    }
    crate::multimedia::soltar_chasquido(state);
    let mut repetir = false;
    // Hay un menú abierto que aún no tiene el teclado: se le da ahora, antes
    // de que la tecla salga hacia el cliente. El grab de popups ignora los
    // cambios de foco mientras está puesto, así que se quita y se vuelve a
    // poner con el mismo serial.
    if pulsada && let Some(grab) = state.menu_sin_foco.take().filter(|g| !g.has_ended()) {
        kbd.unset_grab(state);
        kbd.set_focus(state, grab.current_grab(), serial);
        kbd.set_grab(
            state,
            smithay::desktop::PopupKeyboardGrab::new(&grab),
            grab.serial(),
        );
    }
    // El filtro corre con las teclas **antes** de reenviarlas, así que
    // los atajos del compositor funcionan aunque el cliente con foco
    // esté colgado. Esa es justamente la propiedad que hace que el
    // cambio de TTY sea una salida de emergencia fiable.
    let accion = kbd.input(
        state,
        codigo,
        estado,
        serial,
        time,
        |state, modifiers, handle| {
            let _ = modifiers;
            // Con el salvapantallas puesto solo pasa su propio atajo, que lo
            // quita; el resto de teclas se las queda la capa.
            if tapado {
                let accion =
                    crate::keybinds::resolver(modifiers, &handle, codigo.raw()).filter(|a| {
                        matches!(
                            a,
                            crate::keybinds::Accion::Salvapantallas
                                | crate::keybinds::Accion::SalvapantallasOtro
                        )
                    });
                return FilterResult::Intercept(accion);
            }
            let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
            // Con la capa de captura abierta, el teclado es suyo: Esc
            // la cierra, Intro captura y las flechas cambian de modo.
            // Nada de eso puede llegar a la ventana de debajo.
            if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
                if !pulsada {
                    return FilterResult::Intercept(None);
                }
                let accion = traducir_tecla(&handle, modifiers.ctrl)
                    .and_then(|t| state.shell.as_mut().map(|s| s.captura_tecla(t)))
                    .and_then(|(_, accion)| accion);
                // Escape cierra la capa aquí mismo.
                cursor_de_captura(state);
                state.needs_redraw = true;
                // Consumida o no, la tecla se queda aquí: mientras
                // encuadras, el escritorio no responde a nada más.
                return FilterResult::Intercept(accion.map(crate::keybinds::Accion::DelShell));
            }
            // Solo la pulsación: interceptar también la suelta le
            // dejaría la tecla trabada al cliente. Bloqueado, tampoco
            // la suelta sale de aquí.
            if !pulsada {
                if bloqueado {
                    return FilterResult::Intercept(None);
                }
                // Con una excepción: soltar el modificador **resuelve**
                // el conmutador de Alt+Tab. Es lo que lo hace el de
                // macOS y no una lista que se queda abierta: eliges
                // mientras mantienes la tecla y confirmas al soltarla.
                //
                // La acción **no** se devuelve por `Intercept`, que es
                // lo que hacía antes: `Intercept` se queda la tecla y no
                // la reenvía (`KeyboardHandle::input` de Smithay), así
                // que el cliente al que llegas nunca ve que Alt se ha
                // soltado y se le queda trabada para siempre. Se marca
                // aquí, se devuelve `Forward` para que la suelta salga
                // hacia el cliente que la esperaba, y se resuelve al
                // volver de `kbd.input`.
                // Smithay actualiza el estado de modificadores antes de
                // llamar al filtro. Mirarlo evita depender del keysym
                // concreto de Alt/Meta en cada mapa de teclado.
                let cerrando = state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
                    && !state.conmutador_pegado
                    && state
                        .conmutador_modo
                        .is_some_and(|modo| modificador_conmutador_suelto(modo, modifiers));
                if cerrando {
                    state.conmutador_resolver = true;
                }
                // Meta a secas: se soltó sin que llegara nada más por el
                // medio, así que era la tecla y no un modificador. El
                // launchpad se abre al volver de `kbd.input`, igual que
                // el conmutador y por lo mismo: la suelta tiene que
                // salir hacia el cliente o se le queda trabada.
                let meta = matches!(
                    handle.modified_sym().raw(),
                    keysyms::KEY_Super_L | keysyms::KEY_Super_R
                );
                if meta && std::mem::take(&mut state.meta_sola) {
                    state.abrir_launchpad = true;
                }
                return FilterResult::Forward;
            }
            // Cada tecla que llega, con sus modificadores. Es la única
            // forma de distinguir "el atajo no funciona" de "la tecla no
            // llega": anidado, el compositor de debajo se queda con
            // Alt+Tab y aquí no aparece nada.
            tracing::debug!(
                codigo = codigo.raw(),
                sym = handle.modified_sym().raw(),
                nombre = ?handle.modified_sym().name(),
                alt = modifiers.alt,
                logo = modifiers.logo,
                ctrl = modifiers.ctrl,
                shift = modifiers.shift,
                "tecla"
            );
            // Meta empieza a contar como «sola» solo si no hay nada más
            // pulsado; cualquier otra tecla la descarta hasta que se
            // vuelva a pulsar.
            state.meta_sola = matches!(
                handle.modified_sym().raw(),
                keysyms::KEY_Super_L | keysyms::KEY_Super_R
            ) && !modifiers.alt
                && !modifiers.ctrl
                && !modifiers.shift;
            // Escape cancela el conmutador sin ir a ninguna parte, y no
            // llega al cliente: es la salida del propio conmutador.
            if state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
                && handle.modified_sym().raw() == keysyms::KEY_Escape
            {
                return FilterResult::Intercept(Some(crate::keybinds::Accion::ConmutarCancelar));
            }
            let buscador = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("buscador"));
            let emojis = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("emojis"));
            if let Some(accion) = crate::keybinds::resolver(modifiers, &handle, codigo.raw()) {
                // En el kiosko solo se cambia de TTY, para poder mirar el
                // journal si el asistente se atasca. Todo lo demás se traga:
                // Meta+Return o Ctrl+Alt+T abrirían un terminal de root, y
                // Meta+Alt+Retroceso saldría sin terminar la configuración.
                if state.modo.kiosko() {
                    return FilterResult::Intercept(
                        matches!(accion, crate::keybinds::Accion::CambiarVt(_)).then_some(accion),
                    );
                }
                // Con el buscador abierto, un atajo que mueve ventanas o
                // cambia de escritorio actuaría sobre algo que no se ve y
                // dejaría la paleta flotando encima del resultado. Solo
                // pasan Meta+Espacio, que la cierra, y las de emergencia.
                if buscador
                    && !matches!(accion, crate::keybinds::Accion::Buscador)
                    && !accion.es_emergencia()
                {
                    return FilterResult::Intercept(None);
                }
                if emojis
                    && !matches!(accion, crate::keybinds::Accion::Emojis)
                    && !accion.es_emergencia()
                {
                    return FilterResult::Intercept(None);
                }
                // Con el bloqueo echado valen las salidas de emergencia
                // y apagar la pantalla. Que el cambio de TTY siga funcionando no
                // es un descuido: es la puerta trasera con la que se
                // recupera la máquina si el bloqueo falla, y la tiene
                // cualquier escritorio. Lo demás —cerrar ventanas,
                // maximizar, el launchpad— se traga.
                if !bloqueado
                    || accion.es_emergencia()
                    || matches!(accion, crate::keybinds::Accion::ApagarPantalla)
                {
                    return FilterResult::Intercept(Some(accion));
                }
                return FilterResult::Intercept(None);
            }
            if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
                // La capa es modal: mientras el usuario mantiene Alt o
                // Meta, ninguna tecla suelta debe acabar escribiéndose
                // en la ventana que hay debajo.
                return FilterResult::Intercept(None);
            }
            if bloqueado {
                // La tecla se compara por **keysym** y no por código de
                // tecla: `key_code` devuelve el de xkb, que es el de
                // evdev más ocho, y comparar con el 1 de evdev no
                // acertaba nunca. Ese fue el fallo que dejaba el
                // bloqueo sin salida.
                let sym = handle.modified_sym();
                let tecla = match sym.raw() {
                    keysyms::KEY_Escape => bookos_shell::TeclaPulsada::Escape,
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                        bookos_shell::TeclaPulsada::Intro
                    }
                    keysyms::KEY_Left => bookos_shell::TeclaPulsada::Izquierda,
                    keysyms::KEY_Right => bookos_shell::TeclaPulsada::Derecha,
                    keysyms::KEY_Tab => bookos_shell::TeclaPulsada::Tabulador,
                    _ => bookos_shell::TeclaPulsada::Caracter(' '),
                };
                if let Some(shell) = state.shell.as_mut() {
                    let (consumida, peticion) = shell.bloqueo_confirmar_tecla(tecla);
                    if consumida {
                        state.needs_redraw = true;
                        return FilterResult::Intercept(
                            peticion.map(crate::keybinds::Accion::Energia),
                        );
                    }
                }
                let accion = match sym.raw() {
                    keysyms::KEY_F9 => Some(crate::keybinds::Accion::BloqueoHuella),
                    keysyms::KEY_Escape => Some(crate::keybinds::Accion::BloqueoLimpiar),
                    keysyms::KEY_BackSpace => Some(crate::keybinds::Accion::BloqueoBorrar),
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                        Some(crate::keybinds::Accion::BloqueoComprobar)
                    }
                    _ => sym
                        .key_char()
                        // Los caracteres de control —tabulador, avance
                        // de línea— tienen `key_char` y no son parte de
                        // ninguna contraseña.
                        .filter(|c| !c.is_control())
                        .map(crate::keybinds::Accion::BloqueoEscribir),
                };
                // Ninguna tecla llega a los clientes, se entienda o no:
                // es la propiedad que define un bloqueo.
                return FilterResult::Intercept(accion);
            }
            // Supr sobre el escritorio: a la papelera lo seleccionado. Solo
            // si nadie tiene el teclado —pulsar el escritorio se lo quita a
            // las ventanas— y sin una emergente abierta, que tiene sus propias
            // teclas; si no, Supr es de quien esté escribiendo.
            if matches!(
                handle.modified_sym().raw(),
                keysyms::KEY_Delete | keysyms::KEY_KP_Delete
            ) && state
                .seat
                .get_keyboard()
                .is_some_and(|k| k.current_focus().is_none())
                && state.shell.as_ref().is_some_and(|s| {
                    s.emergente_nombre().is_none() && s.escritorio_seleccion().contains(&true)
                })
            {
                return FilterResult::Intercept(Some(crate::keybinds::Accion::DelShell(
                    bookos_shell::Accion::EscritorioEliminarSeleccion,
                )));
            }
            // Una superficie emergente abierta se queda con las teclas
            // que entiende: es lo que permite cerrar el menú con Esc y
            // recorrerlo con las flechas sin que el cliente con foco
            // llegue a verlas. `Intercept(None)` es "consumida, pero no
            // es un atajo del compositor".
            if let Some(tecla) = traducir_tecla(&handle, modifiers.ctrl) {
                let es_retroceso = matches!(tecla, TeclaPulsada::Retroceso);
                let (consumida, accion) = state
                    .shell
                    .as_mut()
                    .map(|s| s.tecla(tecla))
                    .unwrap_or((false, None));
                if consumida {
                    state.needs_redraw = true;
                    repetir = es_retroceso;
                    // La acción no se ejecuta aquí: seguimos dentro del
                    // filtro de `kbd.input`, con el estado prestado.
                    return FilterResult::Intercept(accion.map(crate::keybinds::Accion::DelShell));
                }
            }
            // Ctrl+C, Ctrl+V y compañía no se traducen; con el buscador
            // abierto tampoco deben llegar a la ventana que hay debajo.
            if buscador || emojis {
                return FilterResult::Intercept(None);
            }
            FilterResult::Forward
        },
    );
    if let Some(Some(accion)) = accion {
        crate::keybinds::ejecutar(state, accion);
    }
    if repetir {
        repetir_retroceso(state);
    }
    // El orden importa y es este: la suelta de Alt ya ha salido hacia el
    // cliente **antiguo** —que es quien vio la pulsación y la espera— y
    // el cambio de foco ocurre ahora, así que el `enter` del cliente
    // nuevo llega con el juego de teclas pulsadas ya sin Alt.
    if std::mem::take(&mut state.conmutador_resolver) {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
    }
    if std::mem::take(&mut state.abrir_launchpad) {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::Launchpad);
    }
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        let caps_lock = kbd.modifier_state().caps_lock;
        if let Some(shell) = state.shell.as_mut() {
            shell.bloqueo_caps_lock(caps_lock);
        }
        state.needs_redraw = true;
    }
}

/// Repite Retroceso mientras se mantiene, con el ritmo del teclado del seat.
///
/// Las repeticiones las genera el **cliente** a partir de la pulsación que
/// recibe; una tecla que el filtro consume para el shell no le llega a nadie,
/// así que en el launchpad mantener Retroceso borraba un solo carácter.
fn repetir_retroceso(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let (retardo, velocidad) = state.teclado_repeticion;
    if velocidad == 0 {
        return;
    }
    let espera = std::time::Duration::from_millis(retardo.into());
    let cada = std::time::Duration::from_millis((1000 / velocidad).into());
    let token =
        state
            .loop_handle
            .insert_source(Timer::from_duration(espera), move |_, _, state| {
                let seguir = state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.tecla(TeclaPulsada::Retroceso).0);
                if !seguir {
                    state.repeticion_retroceso = None;
                    return TimeoutAction::Drop;
                }
                state.needs_redraw = true;
                TimeoutAction::ToDuration(cada)
            });
    match token {
        Ok(token) => state.repeticion_retroceso = Some(token),
        Err(err) => tracing::warn!("sin repetición de Retroceso: {err}"),
    }
}

/// La acción de un remapeo. Con el bloqueo echado no hace nada: una tecla
/// cambiada no puede ser una forma de saltárselo.
fn remapeada(state: &mut BookosComp, accion: crate::keybinds::Accion) {
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return;
    }
    // La tecla no pasa por xkb ni por el filtro, que es quien descarta el
    // «Meta sola»: sin esto, soltar Meta después abriría el launchpad.
    state.meta_sola = false;
    crate::keybinds::ejecutar(state, accion);
}

/// Si la app de teclas espera una tecla, se queda con esta pulsación y la
/// anuncia por `KeyCaptured`.
///
/// Los modificadores no se capturan: son la primera mitad del acorde de
/// Copilot, y al pulsarlos no se sabe si viene algo detrás. Quien quiera
/// remapear uno lo escribe a mano en `teclas.conf`.
fn capturar(state: &mut BookosComp, evdev: u32) -> bool {
    const MODIFICADORES: [u32; 8] = [29, 97, 42, 54, 56, 100, 125, 126];
    if !state.captura_tecla
        || MODIFICADORES.contains(&evdev)
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return false;
    }
    state.captura_tecla = false;
    let copilot = state.traductor.es_copilot(evdev);
    crate::ajustes::avisar_tecla_capturada(state, evdev, copilot);
    state.traductor.tragar_suelta(evdev);
    true
}

pub fn handle<B: InputBackend>(state: &mut BookosComp, event: InputEvent<B>) {
    state.last_input = Some(std::time::Instant::now());
    crate::atenuacion::actividad(state);
    state.idle_notifier_state.notify_activity(&state.seat);
    crate::backend::despertar_dpms(state);
    // Mientras está el salvapantallas, el puntero y el tacto no llegan a nadie.
    // El teclado se filtra dentro de `tecla`, que deja pasar Meta+Alt+S.
    if matches!(
        event,
        InputEvent::PointerMotion { .. }
            | InputEvent::PointerMotionAbsolute { .. }
            | InputEvent::PointerButton { .. }
            | InputEvent::PointerAxis { .. }
            | InputEvent::TouchDown { .. }
            | InputEvent::TouchMotion { .. }
    ) && crate::salvapantallas::actividad(state)
    {
        return;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|shell| shell.esta_bloqueado())
    {
        crate::backend::programar_suspension_inactividad(state);
    }

    // Los gestos del touchpad abren el conmutador, la vista de escritorios y
    // el launchpad, y cambian de escritorio: con el bloqueo echado, lo que
    // abrían enseñaba ventanas vivas por encima del candado.
    let es_gesto = matches!(
        event,
        InputEvent::GestureSwipeBegin { .. }
            | InputEvent::GestureSwipeUpdate { .. }
            | InputEvent::GestureSwipeEnd { .. }
            | InputEvent::GesturePinchBegin { .. }
            | InputEvent::GesturePinchUpdate { .. }
            | InputEvent::GesturePinchEnd { .. }
            | InputEvent::GestureHoldBegin { .. }
            | InputEvent::GestureHoldEnd { .. }
    );
    if es_gesto && state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return;
    }

    match event {
        InputEvent::Keyboard { event } => {
            if state.salvapantallas.activo() && event.state() == KeyState::Pressed {
                tecla(state, event.key_code(), event.state(), event.time_msec());
                return;
            }
            // `teclas.conf` habla en evdev, que es lo que dicen
            // `input-event-codes.h` y `libinput debug-events`; smithay da el
            // código de xkb, ocho más.
            let evdev = event.key_code().raw() - 8;
            let pulsada = event.state() == KeyState::Pressed;
            if pulsada {
                crate::teclado::retirar(state);
            }
            let time = event.time_msec();
            let eventos = if pulsada && capturar(state, evdev) {
                Vec::new()
            } else {
                state.traductor.traducir(&state.teclas, evdev, pulsada)
            };
            for evento in eventos {
                match evento {
                    crate::teclas::Evento::Tecla(evdev, pulsada) => {
                        let estado = if pulsada {
                            KeyState::Pressed
                        } else {
                            KeyState::Released
                        };
                        tecla(state, Keycode::new(evdev + 8), estado, time);
                    }
                    crate::teclas::Evento::Accion(accion) => remapeada(state, accion.accion()),
                    crate::teclas::Evento::Comando(cmd) => {
                        remapeada(state, crate::keybinds::Accion::Lanzar(cmd));
                    }
                }
            }
        }

        // libinput: desplazamiento relativo respecto de donde estaba el cursor.
        InputEvent::PointerMotion { event } => {
            let delta = (event.delta_x(), event.delta_y());
            let destino = state.pointer_location + Point::from(delta);
            set_pointer(state, destino, event.time_msec());
        }

        // winit: posición absoluta, normalizada al tamaño de la ventana.
        InputEvent::PointerMotionAbsolute { event } => {
            let (w, h) = logical_size(state);
            // El backend anidado entrega coordenadas relativas a su única
            // salida. Sumar su origen mantiene correcto el camino si la salida
            // fue recolocada desde el editor de pantallas.
            let origen = state
                .space
                .outputs()
                .next()
                .and_then(|o| state.space.output_geometry(o))
                .map(|r| r.loc)
                .unwrap_or_default();
            let destino = (
                origen.x as f64 + event.x_transformed(w),
                origen.y as f64 + event.y_transformed(h),
            )
                .into();
            set_pointer(state, destino, event.time_msec());
        }

        InputEvent::PointerButton { event } => {
            state.teclado.tactil = false;
            boton(state, event.button_code(), event.state(), event.time_msec());
        }

        InputEvent::PointerAxis { event } => {
            // Con una emergente abierta, el gesto es suyo: en el launchpad pasa
            // de página, como el swipe horizontal del plasmoide. `amount` viene
            // en píxeles lógicos con el touchpad y en muescas con la rueda, y a
            // las dos se les da el mismo trato porque el umbral está en píxeles.
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            if (dx != 0.0 || dy != 0.0) && state.shell.as_mut().is_some_and(|s| s.desplazar(dx, dy))
            {
                state.needs_redraw = true;
                return;
            }

            // No todo desplazamiento es una rueda. Firefox, entre otros,
            // mantiene estados distintos para una rueda (pasos) y un touchpad
            // (gesto continuo): etiquetar siempre como `Wheel` hace que un
            // gesto de dos dedos pueda quedarse abierto y el siguiente deje
            // de llegar. Se conserva la fuente real que da libinput.
            let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
            for axis in [Axis::Horizontal, Axis::Vertical] {
                if let Some(discreto) = event.amount_v120(axis) {
                    frame = frame.v120(axis, discreto as i32);
                }
                match event.amount(axis) {
                    Some(cantidad) if cantidad != 0.0 => frame = frame.value(axis, cantidad),
                    // Un 0 explícito significa "aquí se acabó el gesto": hay que
                    // reenviarlo o el cliente sigue creyendo que se desplaza.
                    Some(_) => frame = frame.stop(axis),
                    None => {}
                }
            }
            let pointer = state.pointer.clone();
            pointer.axis(state, frame);
            pointer.frame(state);
        }

        // --- Gestos de touchpad ---------------------------------------------
        // Solo llegan desde libinput: winit no los reenvía, así que esto está
        // muerto en el backend anidado. La lógica vive en `crate::gestos`, que
        // sí se puede probar sin touchpad.
        InputEvent::GestureSwipeBegin { event } => {
            state.gestos.deslizamiento_inicio(event.fingers());
        }

        // Los gestos mueven escritorios y ventanas: en el kiosko no hay de eso.
        InputEvent::GestureSwipeUpdate { .. } if state.modo.kiosko() => {}

        InputEvent::GestureSwipeUpdate { event } => {
            if let Some(gesto) = state
                .gestos
                .deslizamiento_avance(event.delta_x(), event.delta_y())
            {
                tracing::debug!(?gesto, "gesto de touchpad");
                match gesto {
                    Gesto::Escritorio(pasos) => {
                        let destino = crate::escritorios::destino(
                            crate::escritorios::activo_aqui(state),
                            pasos,
                            state.escritorios.cuantos(),
                        );
                        crate::escritorios::cambiar_aqui(state, destino);
                    }
                    // Con la vista abierta, bajar es deshacer el subir que la
                    // abrió. Antes despejaba por debajo de la vista: las
                    // ventanas quedaban escondidas sin que se viera, y ni el
                    // gesto de enseñarlas las traía hasta cambiar de escritorio.
                    Gesto::DespejarEscritorio
                        if state
                            .shell
                            .as_ref()
                            .is_some_and(|s| s.vista_escritorios_abierta()) =>
                    {
                        crate::keybinds::ejecutar(state, crate::keybinds::Accion::VistaEscritorios);
                    }
                    Gesto::DespejarEscritorio => crate::escritorios::despejar(state),
                    // Subir con el escritorio despejado devuelve las ventanas;
                    // subir estando en una aplicación cualquiera enseña los
                    // escritorios, que es lo que hace el mismo gesto en macOS.
                    Gesto::SubirCuatroDedos => {
                        if state.escritorios.despejado() {
                            crate::escritorios::recuperar(state);
                        } else {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::VistaEscritorios,
                            );
                        }
                    }
                    // La exposición es el conmutador de ventanas sin tecla que
                    // sostener: se abre y se queda, y se cierra al elegir una
                    // miniatura, con Esc o con el gesto contrario. El `0` es que
                    // no mueva la selección al abrir —no hay Tab que aplicar—.
                    Gesto::Exponer => exponer(state),
                    Gesto::CerrarExposicion => {
                        if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::ConmutarCancelar,
                            );
                        }
                    }
                    // El pellizco no sale de un deslizamiento.
                    Gesto::AbrirLaunchpad | Gesto::CerrarLaunchpad => {}
                }
            }
        }

        InputEvent::GestureSwipeEnd { .. } => state.gestos.deslizamiento_fin(),

        InputEvent::GesturePinchBegin { event } => {
            state.gestos.pellizco_inicio(event.fingers());
            if state.gestos.pellizco_de_la_ventana() {
                let pointer = state.pointer.clone();
                pointer.gesture_pinch_begin(
                    state,
                    &smithay::input::pointer::GesturePinchBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        fingers: event.fingers(),
                    },
                );
            }
        }

        InputEvent::GesturePinchUpdate { event } if state.gestos.pellizco_de_la_ventana() => {
            let pointer = state.pointer.clone();
            pointer.gesture_pinch_update(
                state,
                &smithay::input::pointer::GesturePinchUpdateEvent {
                    time: event.time_msec(),
                    delta: (event.delta_x(), event.delta_y()).into(),
                    scale: event.scale(),
                    rotation: event.rotation(),
                },
            );
        }

        InputEvent::GesturePinchUpdate { event } => {
            let abierto = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("launchpad"));
            if let Some(gesto) = state.gestos.pellizco_avance(event.scale(), abierto) {
                tracing::debug!(?gesto, escala = event.scale(), "gesto de touchpad");
                // Los dos gestos alternan lo mismo; `pellizco_avance` ya se ha
                // encargado de que solo salga el que corresponde al estado
                // actual, así que aquí no hay nada que distinguir.
                if let Some(shell) = state.shell.as_mut() {
                    shell.alternar_launchpad();
                }
                state.needs_redraw = true;
            }
        }

        InputEvent::GesturePinchEnd { event } => {
            if state.gestos.pellizco_de_la_ventana() {
                let pointer = state.pointer.clone();
                pointer.gesture_pinch_end(
                    state,
                    &smithay::input::pointer::GesturePinchEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled(),
                    },
                );
            }
            state.gestos.pellizco_fin();
        }

        // «Mantener» —dedos quietos sobre el touchpad— no es de nadie del
        // escritorio. Los navegadores lo usan para frenar el scroll con inercia.
        InputEvent::GestureHoldBegin { event } => {
            let pointer = state.pointer.clone();
            pointer.gesture_hold_begin(
                state,
                &smithay::input::pointer::GestureHoldBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    fingers: event.fingers(),
                },
            );
        }

        InputEvent::GestureHoldEnd { event } => {
            let pointer = state.pointer.clone();
            pointer.gesture_hold_end(
                state,
                &smithay::input::pointer::GestureHoldEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    cancelled: event.cancelled(),
                },
            );
        }

        InputEvent::TouchDown { event } => {
            let punto = punto_tactil(state, &event);
            toque_abajo(state, event.slot(), punto, event.time_msec());
        }
        InputEvent::TouchMotion { event } => {
            let punto = punto_tactil(state, &event);
            toque_mueve(state, event.slot(), punto, event.time_msec());
        }
        InputEvent::TouchUp { event } => toque_arriba(state, event.slot(), event.time_msec()),
        InputEvent::TouchFrame { .. } => {
            if let Some(touch) = state.seat.get_touch() {
                touch.frame(state);
            }
        }
        InputEvent::TouchCancel { event } => toque_cancelar(state, event.time_msec()),

        _ => {}
    }
}

/// Dónde cae un toque en el escritorio.
///
/// La pantalla táctil es la del panel interno, no la que tenga el puntero: con
/// un monitor externo enchufado, el dedo sigue estando sobre el portátil. En
/// el anidado no hay `eDP` y se usa la única salida, como `PointerMotionAbsolute`.
/// La rotación de la salida no se aplica: `output_geometry` ya da el tamaño
/// girado, pero el eje del digitalizador no gira con él.
fn punto_tactil<B: InputBackend>(
    state: &BookosComp,
    event: &impl AbsolutePositionEvent<B>,
) -> Point<f64, Logical> {
    let salida = state
        .space
        .outputs()
        .find(|o| {
            ["eDP", "LVDS", "DSI"]
                .iter()
                .any(|p| o.name().starts_with(p))
        })
        .or_else(|| state.space.outputs().next());
    let Some(geo) = salida.and_then(|o| state.space.output_geometry(o)) else {
        return state.pointer_location;
    };
    (
        geo.loc.x as f64 + event.x_transformed(geo.size.w),
        geo.loc.y as f64 + event.y_transformed(geo.size.h),
    )
        .into()
}

/// Un dedo apoyado, venga de libinput, de winit o de un autotest.
///
/// Sobre una superficie de cliente el toque es `wl_touch`, el que las apps
/// entienden como tal (desplazar con inercia, pulsación larga). Sobre lo que
/// dibuja el compositor —panel, dock, emergentes, decoraciones, bloqueo— se
/// emula el ratón: todo eso ya sabe de clics y arrastres, y enseñarle un
/// segundo idioma solo duplicaría la lógica de `boton`. Esa emulación es de un
/// solo dedo; los demás se ignoran hasta que se levanta.
pub fn toque_abajo(state: &mut BookosComp, slot: TouchSlot, punto: Point<f64, Logical>, time: u32) {
    crate::teclado::iniciar_toque(state, slot, punto, time);
    if crate::teclado::pulsar(state, punto, crate::teclado::Dedo::Toque(slot)) {
        return;
    }
    let Some(touch) = state.seat.get_touch() else {
        return;
    };
    // La captura y el conmutador se quedan la entrada aunque haya una ventana
    // debajo, igual que hacen con el ratón en `boton`.
    let del_shell = state.toque_emulado.is_some()
        || state
            .shell
            .as_ref()
            .is_some_and(|s| s.hay_captura() || s.hay_conmutador());
    let foco = if del_shell {
        None
    } else {
        state.surface_under(punto)
    };
    match foco {
        Some(foco) => {
            crate::teclado::tocado(state, &foco.0);
            if let Some((window, _)) = state.ventana_bajo(punto) {
                state.enfocar(&window);
            }
            let evento = DownEvent {
                slot,
                location: punto,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            };
            touch.down(state, Some(foco), &evento);
        }
        None if state.toque_emulado.is_none() => {
            state.toque_emulado = Some(slot);
            set_pointer(state, punto, time);
            boton(state, BTN_LEFT, ButtonState::Pressed, time);
        }
        None => {}
    }
}

pub fn toque_mueve(state: &mut BookosComp, slot: TouchSlot, punto: Point<f64, Logical>, time: u32) {
    crate::teclado::mover_toque(state, slot, punto);
    if state.toque_emulado == Some(slot) {
        set_pointer(state, punto, time);
        return;
    }
    let Some(touch) = state.seat.get_touch() else {
        return;
    };
    // El foco no cambia en un movimiento (lo fija `down`); lo de debajo solo
    // le sirve a Smithay para el destino de un arrastrar y soltar.
    let foco = state.surface_under(punto);
    let evento = TouchMotionEvent {
        slot,
        location: punto,
        time,
    };
    touch.motion(state, foco, &evento);
}

pub fn toque_arriba(state: &mut BookosComp, slot: TouchSlot, time: u32) {
    crate::teclado::terminar_toque(state, slot, time);
    if crate::teclado::soltar(state, crate::teclado::Dedo::Toque(slot)) {
        return;
    }
    if state.toque_emulado == Some(slot) {
        state.toque_emulado = None;
        boton(state, BTN_LEFT, ButtonState::Released, time);
        return;
    }
    let Some(touch) = state.seat.get_touch() else {
        return;
    };
    let evento = UpEvent {
        slot,
        serial: SERIAL_COUNTER.next_serial(),
        time,
    };
    touch.up(state, &evento);
}

fn toque_cancelar(state: &mut BookosComp, time: u32) {
    crate::teclado::cancelar_toques(state);
    // Un botón pulsado sin su soltar deja al shell creyendo que se sigue
    // arrastrando: se suelta aunque el toque se haya anulado.
    if state.toque_emulado.take().is_some() {
        boton(state, BTN_LEFT, ButtonState::Released, time);
    }
    if let Some(touch) = state.seat.get_touch() {
        touch.cancel(state);
    }
}

/// Traduce un keysym de xkb a la tecla que entiende el shell.
///
/// El shell no conoce xkb a propósito: su enum es una docena de variantes sin
/// ninguna dependencia, así que la frontera aguanta aunque debajo cambie la
/// capa de teclado.
fn traducir_tecla(
    handle: &smithay::input::keyboard::KeysymHandle<'_>,
    ctrl: bool,
) -> Option<TeclaPulsada> {
    use smithay::input::keyboard::keysyms;
    let sym = handle.modified_sym();
    let tecla = match sym.raw() {
        keysyms::KEY_Escape => TeclaPulsada::Escape,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => TeclaPulsada::Intro,
        keysyms::KEY_Up => TeclaPulsada::Arriba,
        keysyms::KEY_Down => TeclaPulsada::Abajo,
        keysyms::KEY_Left => TeclaPulsada::Izquierda,
        keysyms::KEY_Right => TeclaPulsada::Derecha,
        keysyms::KEY_Home | keysyms::KEY_KP_Home => TeclaPulsada::Inicio,
        keysyms::KEY_End | keysyms::KEY_KP_End => TeclaPulsada::Fin,
        keysyms::KEY_Prior | keysyms::KEY_KP_Prior => TeclaPulsada::PaginaArriba,
        keysyms::KEY_Next | keysyms::KEY_KP_Next => TeclaPulsada::PaginaAbajo,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => TeclaPulsada::Tabulador,
        keysyms::KEY_BackSpace if ctrl => TeclaPulsada::BorrarTodo,
        // Con Ctrl, `key_char` da el carácter de control (0x01) y caería en
        // el descarte de abajo: por eso se mira el keysym.
        keysyms::KEY_a | keysyms::KEY_A if ctrl => TeclaPulsada::SeleccionarTodo,
        keysyms::KEY_BackSpace => TeclaPulsada::Retroceso,
        // Lo demás solo interesa si escribe algo: es lo que alimentará la
        // búsqueda del launchpad. Los controles se descartan para que un
        // Ctrl+C no acabe metiendo un carácter raro en un campo de texto.
        _ => {
            let c = sym.key_char()?;
            if c.is_control() {
                return None;
            }
            TeclaPulsada::Caracter(c)
        }
    };
    Some(tecla)
}

/// Un botón del ratón, venga de libinput, de winit o de un autotest: todos
/// pasan por aquí para que el clic de prueba recorra el mismo camino.
pub fn boton(state: &mut BookosComp, button: u32, pulsado: ButtonState, time: u32) {
    // Meta+clic mueve y redimensiona ventanas: si hubo clic, la tecla
    // era un modificador y no debe abrir nada al soltarla.
    state.meta_sola = false;
    // Bloqueo y captura pueden cancelar el arrastre mientras sigue pulsado.
    // Su suelta debe liberarlo antes de que esas capas intercepten el botón.
    if button == BTN_LEFT
        && pulsado == ButtonState::Released
        && state.shell.as_ref().is_some_and(|s| s.dock_agarrado())
    {
        if let Some(accion) = state.shell.as_mut().and_then(|s| s.soltar()) {
            crate::keybinds::hacer(state, accion);
        }
        state.needs_redraw = true;
        return;
    }
    // Bloqueado, los clics no salen hacia los clientes: lo único que
    // atienden es el botón de apagado de la propia pantalla de bloqueo.
    // La capa de captura se queda con el ratón mientras está: se está
    // encuadrando una foto y un clic no puede llegar a la ventana de
    // debajo. Va antes que el bloqueo porque son excluyentes y esta es
    // la que puede estar abierta con el escritorio en uso.
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        let (x, y) = (state.pointer_location.x, state.pointer_location.y);
        let accion = match pulsado {
            ButtonState::Pressed => state.shell.as_mut().and_then(|s| s.captura_pulsar(x, y)),
            ButtonState::Released => {
                if let Some(shell) = state.shell.as_mut() {
                    shell.captura_soltar();
                }
                None
            }
        };
        if let Some(accion) = accion {
            crate::keybinds::hacer(state, accion);
        }
        cursor_de_captura(state);
        state.needs_redraw = true;
        return;
    }
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        if pulsado == ButtonState::Pressed {
            let (x, y) = (state.pointer_location.x, state.pointer_location.y);
            let peticion = state
                .shell
                .as_mut()
                .and_then(|shell| shell.bloqueo_pulsado(x, y));
            if let Some(peticion) = peticion {
                crate::keybinds::ejecutar(state, crate::keybinds::Accion::Energia(peticion));
            }
            state.needs_redraw = true;
        }
        return;
    }
    // Pulsar lo que asoma de una ventana apartada las trae todas, como en
    // macOS. Antes que el resto: no hay cliente debajo que deba enterarse.
    if button == BTN_LEFT
        && pulsado == ButtonState::Pressed
        && crate::escritorios::asomada_en(state, state.pointer_location)
    {
        crate::escritorios::recuperar(state);
        state.needs_redraw = true;
        return;
    }
    // El teclado en pantalla también se puede usar con el ratón: en un
    // portátil sin pantalla táctil es la única forma de probarlo.
    if button == BTN_LEFT {
        let del_teclado = match pulsado {
            ButtonState::Pressed => {
                crate::teclado::pulsar(state, state.pointer_location, crate::teclado::Dedo::Raton)
            }
            ButtonState::Released => crate::teclado::soltar(state, crate::teclado::Dedo::Raton),
        };
        if del_teclado {
            return;
        }
    }
    let serial = SERIAL_COUNTER.next_serial();

    if pulsado == ButtonState::Released && std::mem::take(&mut state.conmutador_clic) {
        return;
    }
    if pulsado == ButtonState::Pressed && state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        state.conmutador_clic = true;
        let elegida = (button == BTN_LEFT)
            .then(|| {
                state.shell.as_ref().and_then(|s| {
                    s.conmutador_en(state.pointer_location.x, state.pointer_location.y)
                })
            })
            .flatten();
        if let Some(i) = elegida {
            if let Some(shell) = state.shell.as_mut() {
                shell.conmutador_elegir(i);
            }
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
        } else {
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarCancelar);
        }
        return;
    }

    // Con la vista de escritorios abierta, una ventana de una miniatura se
    // coge para llevarla a otro escritorio. Va antes que el shell porque la
    // vista cambia de escritorio al pulsar, y eso acabaría el arrastre antes
    // de empezar.
    if button == BTN_LEFT
        && crate::escritorios::boton_en_vista(state, pulsado == ButtonState::Pressed)
    {
        return;
    }

    // Soltar cierra el arrastre y encaja la ventana si el cursor estaba
    // en un borde. Que el evento llegue o no al cliente depende de quién
    // empezó: con Meta+ratón el clic nunca existió para él, pero si fue
    // él quien pidió el movimiento, ya vio la pulsación y **necesita**
    // ver el soltar — sin eso se queda creyendo que sigues pulsando y no
    // vuelve a pedir otro movimiento nunca más.
    if pulsado == ButtonState::Released {
        if (button == BTN_LEFT || button == BTN_RIGHT) && crate::multitarea::soltar(state) {
            return;
        }
        if button == BTN_RIGHT && std::mem::take(&mut state.menu_ventana_suelta) {
            return;
        }
        // La banda elástica o los iconos agarrados se cierran aquí, y
        // ese soltar no es de nadie más: empezó sobre el escritorio.
        if soltar_escritorio(state) {
            return;
        }
        // El shell primero: si tenía un deslizador agarrado, el soltar
        // es suyo aunque el puntero esté ya fuera de su superficie.
        if button == BTN_LEFT
            && let Some(accion) = state.shell.as_mut().and_then(|s| s.soltar())
        {
            crate::keybinds::hacer(state, accion);
        }
        // Un botón de la barra de título se dispara al soltar y sobre
        // el mismo botón donde se pulsó, como cualquier botón del
        // sistema: bajar el ratón en la ✕ y salirse antes de soltar no
        // cierra nada.
        if let Some((window, boton)) = crate::decoracion::soltar(state, state.pointer_location) {
            crate::decoracion::accionar(state, &window, boton);
            return;
        }
        if state.soltar_arrastre() {
            return;
        }
    }

    // Meta + botón mueve o redimensiona sin necesidad de acertarle a un
    // borde. Es el gesto de KWin y de casi todos los gestores de
    // ventanas de Wayland, y aquí es además la **única** forma de mover
    // una ventana que se dibuje sin decoración cliente.
    // Sobre el shell no: con el launchpad, un menú o la vista de escritorios
    // abiertos, la ventana de debajo está tapada o se dibuja en otro sitio, y
    // se movía o redimensionaba una ventana que no se veía.
    if pulsado == ButtonState::Pressed
        && modificadores(state).logo
        && !state.shell_contiene(state.pointer_location)
    {
        let modo = match button {
            BTN_LEFT => Some(crate::ventanas::Modo::Mover),
            BTN_RIGHT => Some(crate::ventanas::Modo::Redimensionar {
                izquierda: false,
                arriba: false,
            }),
            _ => None,
        };
        if let Some(modo) = modo
            && state.empezar_arrastre(modo)
        {
            return;
        }
    }

    // El shell se dibuja por encima de las ventanas, así que se mira
    // primero: un clic en el dock es del dock, no de lo que haya
    // debajo.
    let punto = state.pointer_location;
    // Clic derecho en el dock: menú contextual del icono, como en
    // Plasma. Fijar y cerrar salen de ahí, no del propio clic.
    // Y en el launchpad: el botón derecho saca las ✕ para quitar
    // aplicaciones de la rejilla, que es el mismo gesto de «esto tiene
    // más opciones» que en el dock.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && !modificadores(state).logo
        && state.shell.as_mut().is_some_and(|s| s.launchpad_menu())
    {
        state.needs_redraw = true;
        return;
    }
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && !modificadores(state).logo
        && state
            .shell
            .as_ref()
            .is_some_and(|s| s.en_el_dock(punto.x, punto.y))
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.menu_dock_en(punto.x, punto.y))
    {
        state.needs_redraw = true;
        return;
    }
    if pulsado == ButtonState::Pressed
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.recoger_actividad_fuera(punto.x, punto.y))
    {
        state.needs_redraw = true;
    }
    if state.shell_contiene(punto) {
        // Ni la pulsación ni el soltar llegan al cliente: para él este
        // clic no ha existido. Reenviar solo el soltar le dejaría un
        // botón que se levanta sin haberse pulsado nunca.
        if pulsado == ButtonState::Pressed {
            if button == BTN_RIGHT && state.shell.as_mut().is_some_and(|s| s.editar_launchpad()) {
                state.needs_redraw = true;
                return;
            }
            // Solo el botón principal activa controles. Antes cualquier
            // botón caía por este camino cuando su menú contextual no
            // encontraba un item; en el dock eso convertía un clic
            // derecho en «activar» y minimizaba la app con la lámpara.
            if button == BTN_LEFT
                && let Some(accion) = state
                    .shell
                    .as_mut()
                    .and_then(|s| s.pulsar(punto.x, punto.y))
            {
                crate::keybinds::hacer(state, accion);
            }
        }
        return;
    }

    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && crate::multitarea::empezar_separador(state)
    {
        return;
    }

    // La barra de título que dibujamos nosotros: sus clics no son de
    // nadie más. Va después del shell —el panel manda sobre la barra de
    // una ventana que llegue hasta él— y antes del reenvío, porque para
    // el cliente este clic no existe.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && let Some((window, _)) = crate::decoracion::barra_en(state, punto)
    {
        state.menu_ventana_suelta = true;
        crate::keybinds::menu_ventana(state, window);
        return;
    }
    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && let Some((window, boton)) = crate::decoracion::barra_en(state, punto)
    {
        if boton.is_none()
            && let Some(rect) = crate::decoracion::barra_rect(state, &window)
            && crate::multitarea::pulsar_pestana(state, &window, punto.x - rect.loc.x as f64)
        {
            return;
        }
        state.enfocar(&window);
        match boton {
            Some(boton) => crate::decoracion::pulsar(state, &window, boton),
            // Fuera de los botones, la barra es para agarrar la
            // ventana; y dos clics seguidos, para maximizarla.
            None if doble_clic_en_barra(state) => {
                state.alternar_maximizada(&window);
            }
            None => {
                state.arrastrar_ventana(&window, crate::ventanas::Modo::Mover, false);
            }
        }
        state.needs_redraw = true;
        return;
    }

    // El escritorio pelado: aquí no hay ventana ni barra ni shell, así
    // que el clic es de los iconos. Va lo último de todo porque el
    // escritorio está **detrás** de todo lo demás.
    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && state.space.element_under(punto).is_none()
        && pulsar_escritorio(state, punto)
    {
        state.needs_redraw = true;
        return;
    }
    // Y el mismo escritorio con el botón derecho: el menú de «Nueva
    // carpeta» y «Eliminar». El soltar que sigue se traga con el mismo
    // `menu_ventana_suelta` que usa el menú de la barra de título, dos
    // líneas más abajo de este bloque.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && state.space.element_under(punto).is_none()
        && pulsar_derecho_escritorio(state, punto)
    {
        state.menu_ventana_suelta = true;
        return;
    }

    // Las barras del cliente procesan su propio doble clic. Interceptar la
    // segunda pulsación dejaba a GTK con un clic pendiente: el siguiente
    // gesto alternaba una vez en Firefox y otra aquí, y no cambiaba el tamaño.
    // Pulsar sobre una ventana le da también el foco de teclado y la
    // eleva: es lo que espera cualquiera al hacer clic.
    if pulsado == ButtonState::Pressed {
        focus_under_pointer(state);
    }

    let pointer = state.pointer.clone();
    pointer.button(
        state,
        &ButtonEvent {
            button,
            state: pulsado,
            serial,
            time,
        },
    );
    pointer.frame(state);
}

/// Mueve el cursor a un punto lógico, lo confina al escritorio y reenvía el
/// movimiento al cliente que haya debajo.
pub fn set_pointer(state: &mut BookosComp, destino: Point<f64, Logical>, time: u32) {
    // No se confina al rectángulo envolvente: con una pantalla arriba y otra a
    // la derecha ese rectángulo contiene una esquina vacía donde el cursor
    // desaparecería. Se proyecta sobre la salida real más cercana y por eso
    // también funcionan coordenadas negativas (monitores a izquierda/arriba).
    let geometrías: Vec<_> = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o))
        .collect();
    let anterior = state.pointer_location;
    let delta_fisico = destino - anterior;
    let mut location = confinar_a_salidas(destino, &geometrías);

    // Juegos, máquinas virtuales y escritorios remotos usan
    // pointer-constraints junto con relative-pointer. Publicar esos globales y
    // activar la restricción no basta: el compositor tiene que obedecerla al
    // calcular la nueva posición. Antes el protocolo contestaba «locked» pero
    // el cursor seguía escapando de la ventana, dejando a VirtualBox con una
    // mitad de la captura (teclado o ratón) en un estado incoherente.
    let foco_anterior = state.pointer.current_focus().and_then(|surface| {
        state
            .surface_under(anterior)
            .filter(|(bajo, _)| *bajo == surface)
    });
    let pointer = state.pointer.clone();
    let mut bloqueado = false;
    if let Some((surface, origen)) = foco_anterior.as_ref() {
        let decision = with_pointer_constraint(surface, &pointer, |constraint| {
            let constraint = constraint.filter(|c| c.is_active())?;
            match &*constraint {
                PointerConstraint::Locked(_) => Some((true, false)),
                PointerConstraint::Confined(_) => {
                    let dentro = match constraint.region() {
                        Some(region) => {
                            let local = location - *origen;
                            region.contains((local.x.floor() as i32, local.y.floor() as i32))
                        }
                        // Sin región explícita, la propia superficie es la
                        // región. El hit-test incluye su input_region.
                        None => state
                            .surface_under(location)
                            .is_some_and(|(bajo, _)| bajo == *surface),
                    };
                    Some((false, dentro))
                }
            }
        });
        match decision {
            Some((true, _)) => {
                bloqueado = true;
                location = anterior;
            }
            Some((false, false)) => location = anterior,
            _ => {}
        }
    }
    state.pointer_location = location;
    // Mover el cursor **es** un cambio en pantalla. Sin esto el puntero solo se
    // repinta cuando algo más provoca un frame, y se arrastra a tirones.
    state.needs_redraw = true;

    // Con una ventana agarrada, el movimiento es del compositor: ni el shell ni
    // el cliente lo ven. Reenviarlo mientras arrastras una terminal la deja
    // creyendo que estás seleccionando texto.
    if crate::multitarea::seguir_separador(state) || state.seguir_arrastre() {
        state.space.refresh();
        let fallback = state.output_scale();
        state.broadcast_preferred_scale(fallback);
        return;
    }

    if !state.shell_contiene(location) && crate::multitarea::separador_en(state, location).is_some()
    {
        use smithay::input::pointer::{CursorIcon, CursorImageStatus};
        state.cursor_status = CursorImageStatus::Named(CursorIcon::EwResize);
        let pointer = state.pointer.clone();
        pointer.motion(
            state,
            None,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(state);
        return;
    }

    // Y con la banda elástica o unos iconos agarrados, igual: el gesto es del
    // escritorio y no llega a ningún cliente.
    if seguir_arrastre_escritorio(state, location) {
        return;
    }

    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        let celda = state
            .shell
            .as_ref()
            .and_then(|s| s.conmutador_en(location.x, location.y));
        if let (Some(i), Some(shell)) = (celda, state.shell.as_mut()) {
            state.needs_redraw |= shell.conmutador_elegir(i);
        }
        // Es una capa modal: lo de debajo no recibe hover mientras elegimos.
        let pointer = state.pointer.clone();
        pointer.motion(
            state,
            None,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(state);
        return;
    }

    // Con la capa de captura abierta el ratón es suyo entero: se está
    // encuadrando, y ni el dock ni la ventana de debajo tienen nada que decir.
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        if let Some(shell) = state.shell.as_mut() {
            shell.captura_puntero(location.x, location.y);
        }
        cursor_de_captura(state);
        state.needs_redraw = true;
        return;
    }
    // Con la vista abierta el ✕ sigue al puntero de ventana en ventana.
    if crate::escritorios::mover_en_vista(state)
        || state
            .shell
            .as_ref()
            .is_some_and(|s| s.vista_escritorios_abierta())
    {
        state.needs_redraw = true;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.puntero(location.x, location.y);
    }
    // El cursor pegado a un borde trae de vuelta a la barra que se apartó. Va
    // en físicos porque la franja sensible se mide contra la pantalla.
    //
    // Dos cosas que antes estaban mal y son la misma: las barras están en la
    // **principal**, así que el punto hay que acotarlo a ella y escalarlo con
    // **su** escala. Aquí se usaba `output_scale()`, que es la del monitor bajo
    // el puntero, contra un `self.screen` que son físicos de la principal —con
    // dos escalas distintas la comparación es sencillamente falsa—, y no se
    // miraba la x, así que llevar el ratón al borde de **cualquier** monitor
    // sacaba la barra oculta de la principal.
    let mut repintar = false;
    if let Some(shell) = state.shell.as_ref() {
        let (ancho, alto) = shell.area_principal();
        let escala = shell.escala_principal();
        let dentro =
            location.x >= 0.0 && location.y >= 0.0 && location.x < ancho && location.y < alto;
        if dentro {
            let fisico = (location.x * escala, location.y * escala);
            for cual in [crate::shell::Barra::Panel, crate::shell::Barra::Dock] {
                if let Some(shell) = state.shell.as_mut() {
                    repintar |= shell.reclamada(cual, fisico);
                }
            }
        }
    }
    if repintar {
        state.needs_redraw = true;
    }

    // El botón de la barra de título bajo el cursor. Repinta solo cuando
    // cambia: mover el ratón por una barra sin salir del mismo botón no tiene
    // por qué costar un rasterizado.
    if crate::decoracion::señalar(state, location) {
        state.needs_redraw = true;
    }

    let focus = if bloqueado {
        foco_anterior
    } else {
        state.surface_under(location)
    };
    if focus.is_none() {
        state.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
    }
    let pointer = state.pointer.clone();
    // Un puntero bloqueado no cambia sus coordenadas absolutas; el cliente
    // recibe solo el movimiento relativo. Mandarle además `wl_pointer.motion`
    // contradice el bloqueo que acabamos de confirmar.
    if !bloqueado {
        pointer.motion(
            state,
            focus.clone(),
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        // Una restricción persistente se desactiva al perder el foco (por
        // ejemplo, al abrir la capa de captura). Al volver a entrar en la
        // superficie hay que activarla de nuevo; `new_constraint` no vuelve a
        // llamarse porque el objeto del cliente sigue siendo el mismo.
        if let Some((surface, origen)) = focus.as_ref() {
            with_pointer_constraint(surface, &pointer, |constraint| {
                let Some(constraint) = constraint.filter(|c| !c.is_active()) else {
                    return;
                };
                let local = location - *origen;
                let dentro = constraint.region().is_none_or(|region| {
                    region.contains((local.x.floor() as i32, local.y.floor() as i32))
                });
                if dentro {
                    constraint.activate();
                }
            });
        }
    }
    pointer.relative_motion(
        state,
        focus,
        &RelativeMotionEvent {
            // La posición puede quedarse quieta por un lock, un borde o una
            // región confinada; el movimiento físico no. Ese delta es justo lo
            // que consumen una VM o un juego para mover su cursor interno.
            delta: delta_fisico,
            delta_unaccel: delta_fisico,
            utime: u64::from(time) * 1_000,
        },
    );
    pointer.frame(state);
}

fn confinar_a_salidas(
    destino: Point<f64, Logical>,
    salidas: &[smithay::utils::Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    let dentro = |r: &smithay::utils::Rectangle<i32, Logical>| {
        destino.x >= r.loc.x as f64
            && destino.y >= r.loc.y as f64
            && destino.x < (r.loc.x + r.size.w) as f64
            && destino.y < (r.loc.y + r.size.h) as f64
    };
    if salidas.iter().any(dentro) {
        return destino;
    }

    salidas
        .iter()
        .filter(|r| r.size.w > 0 && r.size.h > 0)
        .map(|r| {
            let x = destino
                .x
                .clamp(r.loc.x as f64, (r.loc.x + r.size.w - 1) as f64);
            let y = destino
                .y
                .clamp(r.loc.y as f64, (r.loc.y + r.size.h - 1) as f64);
            let distancia = (destino.x - x).powi(2) + (destino.y - y).powi(2);
            (distancia, Point::from((x, y)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
        .unwrap_or_else(|| Point::from((0.0, 0.0)))
}

/// Cuánto puede tardar el segundo clic para seguir siendo un doble clic.
///
/// 400 ms es el valor por defecto de Qt y de GTK, o sea el que el usuario ya
/// tiene en los dedos del resto del sistema.
const DOBLE_CLIC: std::time::Duration = std::time::Duration::from_millis(400);

/// Cuánto se puede mover el ratón entre los dos clics.
///
/// Sin este margen, un temblor de un píxel entre pulsaciones convierte el doble
/// clic en dos clics sueltos y la ventana no hace nada.
const DOBLE_CLIC_RADIO: f64 = 6.0;

/// ¿Este clic llega lo bastante seguido y lo bastante cerca del anterior?
///
/// Apunta el clic actual pase lo que pase: dos clics lentos no son un doble,
/// pero el segundo sí puede ser el primero de otro par.
fn es_doble_clic(state: &mut BookosComp) -> bool {
    let ahora = std::time::Instant::now();
    let punto = state.pointer_location;
    let previo = state.ultimo_clic.replace((ahora, punto));
    previo.is_some_and(|(t, p)| {
        ahora.duration_since(t) <= DOBLE_CLIC
            && (p.x - punto.x).abs() <= DOBLE_CLIC_RADIO
            && (p.y - punto.y).abs() <= DOBLE_CLIC_RADIO
    })
}

/// El doble clic sobre **nuestra** barra: no hace falta mirar dónde cae dentro
/// de la ventana, porque la barra entera es zona de título.
fn doble_clic_en_barra(state: &mut BookosComp) -> bool {
    if !es_doble_clic(state) {
        return false;
    }
    // El tercer clic no vuelve a disparar: se olvida el par ya usado.
    state.ultimo_clic = None;
    true
}

/// Lo que hay que alejarse del punto de pulsación para que agarrar un icono
/// cuente como arrastrarlo, en píxeles lógicos.
///
/// Cuatro es el mismo umbral que usa el gestor de ventanas para distinguir un
/// clic de un tirón. Sin umbral, una mano poco firme recoloca el icono cada vez
/// que lo selecciona, y esa posición se guarda en disco.
const UMBRAL_ARRASTRE: f64 = 4.0;

/// Un clic sobre el escritorio pelado: abrir, seleccionar o empezar un gesto.
///
/// Devuelve `true` si el clic era suyo, que lo es siempre que haya shell: el
/// escritorio es el último que mira, y por debajo de él no hay nadie.
fn pulsar_escritorio(state: &mut BookosComp, punto: Point<f64, Logical>) -> bool {
    // Los iconos viven en la principal, igual que el resto del shell. El
    // hit-test de un icono ya se acota solo —ningún rectángulo cae fuera—, pero
    // el hueco vacío no: sin esto, un clic en el escritorio pelado de la
    // pantalla de al lado arrancaba una banda elástica en la principal.
    if !state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_alcanza(punto.x, punto.y))
    {
        return false;
    }
    // El escritorio se queda el teclado, como en cualquier gestor de
    // archivos: sin esto la ventana de antes seguía con el foco y Supr le
    // llegaba a ella en vez de borrar lo seleccionado aquí.
    state.desenfocar();
    // Se apunta el clic pase lo que pase, aunque no sea doble: el siguiente
    // puede serlo con este de pareja.
    let doble = es_doble_clic(state);
    let ctrl = modificadores(state).ctrl;
    let icono = state
        .shell
        .as_ref()
        .and_then(|s| s.escritorio_en(punto.x, punto.y));

    match icono {
        Some(i) if doble => {
            // El tercer clic no vuelve a abrir lo mismo.
            state.ultimo_clic = None;
            if let Some(accion) = state.shell.as_ref().and_then(|s| s.escritorio_abrir(i)) {
                crate::keybinds::hacer(state, accion);
            }
        }
        Some(i) => {
            if let Some(shell) = state.shell.as_mut() {
                // Pulsar sobre algo que **ya** estaba seleccionado no deshace la
                // selección: es lo que permite arrastrar varios iconos de una
                // vez, como en cualquier gestor de archivos.
                let ya = shell
                    .escritorio_seleccion()
                    .get(i)
                    .copied()
                    .unwrap_or(false);
                if ctrl || !ya {
                    shell.escritorio_seleccionar(Some(i), ctrl);
                }
            }
            state.arrastre_escritorio = Some(ArrastreEscritorio::Iconos {
                origen: punto,
                movido: false,
            });
        }
        None => {
            let previa = match ctrl {
                true => state
                    .shell
                    .as_ref()
                    .map(|s| s.escritorio_seleccion())
                    .unwrap_or_default(),
                false => Vec::new(),
            };
            if !ctrl && let Some(shell) = state.shell.as_mut() {
                shell.escritorio_seleccionar(None, false);
            }
            state.arrastre_escritorio = Some(ArrastreEscritorio::Banda {
                origen: punto,
                previa,
            });
        }
    }
    true
}

/// El clic derecho sobre el escritorio: sobre un icono, abrirlo o mandarlo a
/// la papelera; en hueco vacío, carpeta nueva, fondo y pantalla.
///
/// Reutiliza `MenuDock` —el menú del dock, con sus filas compactas— anclado
/// al punto del clic, en vez de montar una emergente propia para cuatro
/// entradas.
///
/// `true` si el clic era del escritorio y no debe llegar a nadie más.
fn pulsar_derecho_escritorio(state: &mut BookosComp, punto: Point<f64, Logical>) -> bool {
    if !state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_alcanza(punto.x, punto.y))
    {
        return false;
    }
    // Clic derecho sobre un icono que no estaba seleccionado: lo selecciona
    // solo, como el clic izquierdo. Sobre uno que ya lo estaba, se deja la
    // selección tal cual, para poder eliminar varios a la vez. En hueco vacío
    // se limpia, también como el izquierdo: si no, lo seleccionado antes
    // seguía mandando y el menú ofrecía la papelera en vez del del escritorio.
    let icono = state
        .shell
        .as_ref()
        .and_then(|s| s.escritorio_en(punto.x, punto.y));
    if icono.is_none()
        && let Some(shell) = state.shell.as_mut()
    {
        shell.escritorio_seleccionar(None, false);
    }
    if let Some(i) = icono {
        let ya = state
            .shell
            .as_ref()
            .map(|s| s.escritorio_seleccion())
            .and_then(|sel| sel.get(i).copied())
            .unwrap_or(false);
        if !ya && let Some(shell) = state.shell.as_mut() {
            shell.escritorio_seleccionar(Some(i), false);
        }
    }
    let hay_seleccion = state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_seleccion().iter().any(|&s| s));
    use bookos_shell::Accion;
    let opciones = if hay_seleccion {
        // «Abrir» solo con un icono debajo del clic: con varios seleccionados
        // por banda no hay uno que abrir, y abrirlos todos de golpe no es lo
        // que nadie espera de un clic derecho.
        let abrir = icono
            .and_then(|i| state.shell.as_ref().and_then(|s| s.escritorio_abrir(i)))
            .map(|accion| Some(("Abrir".to_string(), "abrir", accion)));
        let divisor = abrir.is_some().then_some(None);
        abrir
            .into_iter()
            .chain(divisor)
            .chain([Some((
                "Mover a la papelera".to_string(),
                "papelera",
                Accion::EscritorioEliminarSeleccion,
            ))])
            .collect()
    } else {
        let visibles = state.shell.as_ref().is_some_and(|s| s.escritorio_visible());
        // Con los iconos ocultos, «Nueva carpeta» crearía algo que no se ve:
        // sale solo cuando se ven.
        let nueva = visibles.then_some(Some((
            "Nueva carpeta".to_string(),
            "carpeta-nueva",
            Accion::EscritorioNuevaCarpeta,
        )));
        let iconos = match visibles {
            true => ("Ocultar iconos".to_string(), "ojo-tachado"),
            false => ("Mostrar iconos".to_string(), "ojo"),
        };
        nueva
            .into_iter()
            .chain(visibles.then_some(None))
            .chain([
                Some((
                    "Cambiar fondo…".to_string(),
                    "fondo",
                    Accion::Lanzar("bookos-settings --page fondos".into()),
                )),
                Some((
                    "Ajustes de pantalla".to_string(),
                    "pantalla",
                    Accion::Lanzar("bookos-settings --page pantalla".into()),
                )),
                None,
                Some((iconos.0, iconos.1, Accion::EscritorioIconos(!visibles))),
            ])
            .collect()
    };
    // Sobre un icono, el menú sale debajo de él, no encima tapando lo que se
    // acaba de seleccionar, y con el aire de cualquier emergente: pegado a la
    // celda realzada parecía parte de ella. En hueco vacío, junto al clic.
    let origen = icono
        .and_then(|i| state.shell.as_ref().and_then(|s| s.escritorio_rect(i)))
        .map(|(rx, ry, _, rh)| (rx, ry + rh + crate::shell::SEPARACION as f32))
        .unwrap_or((punto.x as f32, punto.y as f32));
    if let Some(shell) = state.shell.as_mut() {
        shell.menu_escritorio(opciones, origen);
    }
    state.needs_redraw = true;
    true
}

/// Mueve la banda elástica o los iconos agarrados. `true` si el movimiento era
/// del escritorio y no debe llegar a nadie más.
fn seguir_arrastre_escritorio(state: &mut BookosComp, location: Point<f64, Logical>) -> bool {
    // Se saca y se vuelve a poner porque el gesto y el shell viven los dos en
    // `state`: con el gesto prestado no se puede tocar el shell.
    let Some(arrastre) = state.arrastre_escritorio.take() else {
        return false;
    };
    let arrastre = match arrastre {
        ArrastreEscritorio::Banda { origen, previa } => {
            let rect = (
                origen.x.min(location.x),
                origen.y.min(location.y),
                (location.x - origen.x).abs(),
                (location.y - origen.y).abs(),
            );
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_banda(rect, &previa);
            }
            state.needs_redraw = true;
            ArrastreEscritorio::Banda { origen, previa }
        }
        ArrastreEscritorio::Iconos { origen, movido } => {
            let delta = (location.x - origen.x, location.y - origen.y);
            let movido = movido || delta.0.abs().max(delta.1.abs()) >= UMBRAL_ARRASTRE;
            if movido {
                if let Some(shell) = state.shell.as_mut() {
                    shell.escritorio_arrastrar(delta);
                }
                state.needs_redraw = true;
            }
            ArrastreEscritorio::Iconos { origen, movido }
        }
    };
    state.arrastre_escritorio = Some(arrastre);
    true
}

/// Cierra el gesto del escritorio: quita la banda o deja los iconos en su celda.
pub(crate) fn soltar_escritorio(state: &mut BookosComp) -> bool {
    let Some(arrastre) = state.arrastre_escritorio.take() else {
        return false;
    };
    if let Some(shell) = state.shell.as_mut() {
        match arrastre {
            ArrastreEscritorio::Banda { .. } => {
                shell.escritorio_quitar_banda();
            }
            ArrastreEscritorio::Iconos { .. } => {
                shell.escritorio_soltar();
            }
        }
    }
    state.needs_redraw = true;
    true
}

/// Da el foco de teclado a la ventana bajo el cursor y la sube del todo.
fn focus_under_pointer(state: &mut BookosComp) {
    let bajo = state.ventana_bajo(state.pointer_location).map(|(w, _)| w);
    if let Some(window) = bajo {
        // Los menús X11 reciben el clic con su grab y conservan el foco de
        // teclado de la aplicación que los abrió.
        if window
            .x11_surface()
            .is_some_and(|x| x.is_override_redirect())
        {
            return;
        }
        state.enfocar(&window);
    }
}

/// Tamaño de la pantalla en píxeles **lógicos**.
///
/// El modo de la salida está en físicos, pero el puntero y la escena razonan en
/// lógicos: sin dividir por la escala, a 1,75 los clics caen desplazados un 75 %.
fn logical_size(state: &BookosComp) -> (i32, i32) {
    state
        .space
        .outputs()
        .next()
        .and_then(|o| Some((o.current_mode()?, o.current_scale().fractional_scale())))
        .map(|(mode, scale)| {
            (
                (mode.size.w as f64 / scale).round().max(1.0) as i32,
                (mode.size.h as f64 / scale).round().max(1.0) as i32,
            )
        })
        .unwrap_or((1, 1))
}

/// Pone el cursor que toca sobre la capa de captura, o la flecha si ya no está.
///
/// Lo pone el compositor y no un cliente porque la capa es suya, y mientras
/// está abierta nadie tiene el foco del puntero. Al cerrarla hay que devolver
/// la flecha a mano: sobre el escritorio vacío ningún cliente va a pedir otro
/// cursor y se quedaría la cruz.
pub fn cursor_de_captura(state: &mut BookosComp) {
    use bookos_shell::captura::Forma;
    use smithay::input::pointer::{CursorIcon, CursorImageStatus};
    let (x, y) = (state.pointer_location.x, state.pointer_location.y);
    let forma = state.shell.as_ref().and_then(|s| s.captura_forma(x, y));
    state.cursor_status = match forma {
        None => CursorImageStatus::default_named(),
        Some(forma) => CursorImageStatus::Named(match forma {
            Forma::Cruz => CursorIcon::Crosshair,
            Forma::Mover => CursorIcon::Move,
            Forma::Horizontal => CursorIcon::EwResize,
            Forma::Vertical => CursorIcon::NsResize,
            Forma::DiagonalBajando => CursorIcon::NwseResize,
            Forma::DiagonalSubiendo => CursorIcon::NeswResize,
            Forma::Flecha => CursorIcon::Default,
        }),
    };
}

/// Tres dedos hacia arriba: el conmutador de ventanas, abierto hasta elegir.
/// Aparte porque el guion de los autotests la llama igual: en el anidado no
/// llegan gestos.
pub fn exponer(state: &mut BookosComp) {
    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        return;
    }
    // Con el escritorio despejado no hay ventanas mapeadas que exponer: se
    // devuelven primero, que es lo que pide quien quiere verlas.
    crate::escritorios::recuperar(state);
    crate::keybinds::ejecutar(
        state,
        crate::keybinds::Accion::Conmutar(bookos_shell::conmutador::Modo::Ventanas, 0),
    );
    state.conmutador_pegado = state.shell.as_ref().is_some_and(|s| s.hay_conmutador());
    state.conmutador_entrada = state.conmutador_pegado.then(std::time::Instant::now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookos_shell::conmutador::Modo;
    use smithay::input::keyboard::ModifiersState;

    #[test]
    fn el_cursor_recorre_salidas_con_coordenadas_negativas() {
        let salidas = [
            smithay::utils::Rectangle::new((-1280, 0).into(), (1280, 1024).into()),
            smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into()),
        ];
        assert_eq!(
            confinar_a_salidas((-400.0, 500.0).into(), &salidas),
            (-400.0, 500.0).into()
        );
        assert_eq!(
            confinar_a_salidas((-2000.0, 500.0).into(), &salidas),
            (-1280.0, 500.0).into()
        );
    }

    #[test]
    fn el_cursor_no_cae_en_huecos_del_escritorio_virtual() {
        let salidas = [
            smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into()),
            smithay::utils::Rectangle::new((1920, 500).into(), (1280, 1024).into()),
        ];
        // A la derecha de la principal pero por encima de la secundaria hay un
        // hueco; se conserva el cursor en el borde real más próximo.
        assert_eq!(
            confinar_a_salidas((2000.0, 100.0).into(), &salidas),
            (1919.0, 100.0).into()
        );
    }

    #[test]
    fn cada_selector_se_cierra_solo_con_su_modificador() {
        let alt = ModifiersState {
            alt: true,
            ..Default::default()
        };
        assert!(!modificador_conmutador_suelto(Modo::Aplicaciones, &alt));
        assert!(modificador_conmutador_suelto(Modo::Ventanas, &alt));

        let meta = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert!(modificador_conmutador_suelto(Modo::Aplicaciones, &meta));
        assert!(!modificador_conmutador_suelto(Modo::Ventanas, &meta));
    }
}
