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
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{IsAlive, Logical, Point, SERIAL_COUNTER};
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
// 0.1.0
fn modificadores(state: &BookosComp) -> smithay::input::keyboard::ModifiersState {
    state
        .seat
        .get_keyboard()
        .map(|kbd| kbd.modifier_state())
        .unwrap_or_default()
}

// 0.1.0
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
// modified 0.1.1 -- con el cuentagotas, Esc o su atajo lo quitan y nada más pasa
// modificado en la 0.2.0 -- mantener Meta abre la hoja de atajos
pub fn tecla(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
    crate::teclas_accesibles::tecla(state, codigo, estado, time);
}

/// La tecla que ya pasó por las teclas lentas y las especiales.
// añadido en la 0.2.3
pub(crate) fn entregar_tecla(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
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
    let mut repetir: Option<TeclaPulsada> = None;
    let mut pegar_shell = false;
    let mut tecla_vista = None;
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
            // Con el cuentagotas, igual que con la captura: Esc lo quita, su
            // propio atajo también, y nada más llega a la ventana de debajo.
            if state.cuentagotas {
                let salir = pulsada
                    && (handle.modified_sym().raw() == keysyms::KEY_Escape
                        || crate::keybinds::resolver(modifiers, &handle, codigo.raw())
                            == Some(crate::keybinds::Accion::Cuentagotas));
                return FilterResult::Intercept(
                    salir.then_some(crate::keybinds::Accion::Cuentagotas),
                );
            }
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
            if state.vista_tactil.is_some() && !bloqueado {
                if pulsada
                    && (modifiers.logo || modifiers.ctrl || modifiers.alt)
                    && let Some(accion) =
                        crate::keybinds::resolver(modifiers, &handle, codigo.raw())
                {
                    return FilterResult::Intercept(Some(accion));
                }
                if pulsada {
                    tecla_vista = Some((handle.modified_sym().raw(), modifiers.shift));
                }
                return FilterResult::Intercept(None);
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
                if meta {
                    state.meta_desde = None;
                    // La hoja que abrió mantener Meta se va al soltarla. La
                    // que se abrió desde el buscador no: se cierra con Esc.
                    if std::mem::take(&mut state.hoja_por_meta)
                        && let Some(shell) = state.shell.as_mut()
                        && shell.emergente_nombre() == Some("atajos")
                    {
                        shell.cerrar_emergente();
                        state.needs_redraw = true;
                    }
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
            // pulsado ni una capa modal o bloqueo impidiendo abrir el launchpad.
            let bloqueada_meta = bloqueado
                || state.cuentagotas
                || state.modo.kiosko()
                || state.shell.as_ref().is_some_and(|s| {
                    s.hay_captura()
                        || matches!(
                            s.emergente_nombre(),
                            Some(
                                "buscador"
                                    | "emojis"
                                    | "respuesta"
                                    | "polkit"
                                    | "acceso"
                                    | "elegir-app"
                            )
                        )
                });
            let ya_contaba = state.meta_sola;
            state.meta_sola = !bloqueada_meta
                && matches!(
                    handle.modified_sym().raw(),
                    keysyms::KEY_Super_L | keysyms::KEY_Super_R
                )
                && !modifiers.alt
                && !modifiers.ctrl
                && !modifiers.shift;
            // Solo la primera pulsación arranca la cuenta: si llega la
            // autorrepetición de Meta, cada repetición la reiniciaría y la
            // hoja no saldría nunca mientras se mantiene.
            if state.meta_sola && !ya_contaba {
                contar_meta_mantenida(state);
            }
            // Escape cancela el conmutador sin ir a ninguna parte, y no
            // llega al cliente: es la salida del propio conmutador.
            if state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
                && handle.modified_sym().raw() == keysyms::KEY_Escape
            {
                return FilterResult::Intercept(Some(crate::keybinds::Accion::ConmutarCancelar));
            }
            if handle.modified_sym().raw() == keysyms::KEY_Escape
                && state
                    .miniatura_captura
                    .as_mut()
                    .is_some_and(|m| m.cancelar_apertura(std::time::Instant::now()))
            {
                state.needs_redraw = true;
                return FilterResult::Intercept(None);
            }
            let buscador = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("buscador"));
            let respuesta = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("respuesta"));
            let emojis = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("emojis"));
            let portapapeles = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("portapapeles"));
            let polkit = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("polkit"));
            let acceso = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("acceso"));
            let elegir_app = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("elegir-app"));
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
                if portapapeles
                    && !matches!(accion, crate::keybinds::Accion::Portapapeles)
                    && !accion.es_emergencia()
                {
                    return FilterResult::Intercept(None);
                }
                if (polkit || acceso || elegir_app) && !accion.es_emergencia() {
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
            // modificado en la 0.2.3 -- también los atajos que enseña el menú
            // contextual: Ctrl+C/V, F2, Espacio y Alt+Intro. Mismas reglas que
            // Supr: nadie con el teclado y sin emergente abierta.
            let libre = state
                .seat
                .get_keyboard()
                .is_some_and(|k| k.current_focus().is_none())
                && state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.emergente_nombre().is_none());
            if libre {
                let hay = state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.escritorio_seleccion().contains(&true));
                let tecla = handle.modified_sym().raw();
                let (ctrl, alt) = (modifiers.ctrl, modifiers.alt);
                use bookos_shell::Accion as A;
                let accion = match tecla {
                    keysyms::KEY_Delete | keysyms::KEY_KP_Delete if hay => {
                        Some(A::EscritorioEliminarSeleccion)
                    }
                    keysyms::KEY_c | keysyms::KEY_C if ctrl && hay => Some(A::EscritorioCopiar),
                    keysyms::KEY_v | keysyms::KEY_V
                        if ctrl && crate::archivos::hay_archivos(state) =>
                    {
                        Some(A::EscritorioPegar)
                    }
                    keysyms::KEY_F2 if hay => Some(A::EscritorioRenombrar),
                    keysyms::KEY_space if hay && !ctrl && !alt => Some(A::EscritorioVistaRapida),
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter if alt && hay => {
                        Some(A::EscritorioInformacion)
                    }
                    _ => None,
                };
                if let Some(accion) = accion {
                    return FilterResult::Intercept(Some(crate::keybinds::Accion::DelShell(accion)));
                }
            }
            if (buscador || respuesta || polkit || elegir_app)
                && modifiers.ctrl
                && matches!(
                    handle.modified_sym().raw(),
                    keysyms::KEY_v | keysyms::KEY_V
                )
            {
                state.tecla_muerta = None;
                pegar_shell = true;
                return FilterResult::Intercept(None);
            }
            if buscador || portapapeles || respuesta || polkit || elegir_app {
                let raw = handle.modified_sym().raw();
                if es_tecla_muerta(raw) {
                    state.tecla_muerta = Some(raw);
                    return FilterResult::Intercept(None);
                }
            } else {
                state.tecla_muerta = None;
            }
            // Una superficie emergente abierta se queda con las teclas
            // que entiende: es lo que permite cerrar el menú con Esc y
            // recorrerlo con las flechas sin que el cliente con foco
            // llegue a verlas. `Intercept(None)` es "consumida, pero no
            // es un atajo del compositor".
            if let Some(mut tecla) = traducir_tecla(&handle, modifiers.ctrl) {
                if let Some(muerta) = state.tecla_muerta.take()
                    && let TeclaPulsada::Caracter(c) = tecla
                    && let Some(combinada) = combinar_muerta(muerta, c)
                {
                    tecla = TeclaPulsada::Caracter(combinada);
                }
                let repetible = matches!(
                    tecla,
                    TeclaPulsada::Retroceso
                        | TeclaPulsada::Suprimir
                        | TeclaPulsada::Izquierda
                        | TeclaPulsada::Derecha
                        | TeclaPulsada::Arriba
                        | TeclaPulsada::Abajo
                )
                .then(|| tecla.clone());
                let (consumida, accion) = state
                    .shell
                    .as_mut()
                    .map(|s| s.tecla(tecla))
                    .unwrap_or((false, None));
                if consumida {
                    state.needs_redraw = true;
                    repetir = repetible;
                    // La acción no se ejecuta aquí: seguimos dentro del
                    // filtro de `kbd.input`, con el estado prestado.
                    return FilterResult::Intercept(accion.map(crate::keybinds::Accion::DelShell));
                }
            }
            // Ctrl+C, Ctrl+V y compañía no se traducen; con el buscador
            // abierto tampoco deben llegar a la ventana que hay debajo.
            if buscador || emojis || portapapeles || respuesta || polkit || acceso || elegir_app {
                return FilterResult::Intercept(None);
            }
            FilterResult::Forward
        },
    );
    if let Some((tecla, mayus)) = tecla_vista {
        crate::multitarea_tactil::tecla(state, tecla, mayus);
    }
    if let Some(Some(accion)) = accion {
        crate::multitarea_tactil::cancelar(state);
        crate::keybinds::ejecutar(state, accion);
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.emergente_tiene_alarma())
    {
        crate::backend::schedule_panel_tick(state);
    }
    if pegar_shell {
        pegar_en_shell(state);
    }
    if let Some(tecla_rep) = repetir {
        repetir_tecla(state, tecla_rep);
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
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.emergente_nombre() == Some("polkit"))
    {
        let caps_lock = kbd.modifier_state().caps_lock;
        if let Some(shell) = state.shell.as_mut() {
            shell.polkit_caps_lock(caps_lock);
        }
        state.needs_redraw = true;
    }
}

/// Repite Retroceso mientras se mantiene, con el ritmo del teclado del seat.
///
/// Las repeticiones las genera el **cliente** a partir de la pulsación que
/// recibe; una tecla que el filtro consume para el shell no le llega a nadie,
/// así que en el launchpad mantener Retroceso borraba un solo carácter.
/// Mantener Meta sola este tiempo abre la hoja de atajos. Un segundo, como
/// CheatSheet: pulsarla y soltarla tiene que seguir abriendo el launchpad, y
/// quien la mantiene mientras piensa qué letra va con ella es justo quien
/// necesita la hoja.
const MANTENER_META: std::time::Duration = std::time::Duration::from_secs(1);

/// Mantener la tecla del asistente más que esto es pulsar para hablar. Un
/// toque dura unos 100 ms; 400 deja margen a quien pulsa despacio, y no se
/// nota la espera porque Zenith ya está escuchando desde que se pulsó.
// añadido en la 0.2.0
const MANTENER_ASISTENTE: std::time::Duration = std::time::Duration::from_millis(400);

/// Si la tecla sigue abajo pasado `MANTENER_ASISTENTE`, Zenith deja de cortar
/// en los silencios hasta que se suelte.
// añadido en la 0.2.0
fn contar_asistente_mantenido(state: &mut BookosComp, evdev: u32) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let pulsada = std::time::Instant::now();
    state.asistente_tecla = Some((evdev, pulsada));
    let programado = state.loop_handle.insert_source(
        Timer::from_duration(MANTENER_ASISTENTE),
        move |_, _, state| {
            // La misma pulsación, aún sin soltar.
            if state.asistente_tecla == Some((evdev, pulsada)) {
                crate::keybinds::lanzar(state, "bookos-assistant --mantener");
            }
            TimeoutAction::Drop
        },
    );
    if let Err(err) = programado {
        tracing::warn!("no se pudo contar la pulsación del asistente: {err}");
    }
}

// añadido en la 0.2.0
fn contar_meta_mantenida(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let pulsada = std::time::Instant::now();
    state.meta_desde = Some(pulsada);
    let programado =
        state
            .loop_handle
            .insert_source(Timer::from_duration(MANTENER_META), move |_, _, state| {
                // Sigue sola y es la misma pulsación: nada la ha convertido en
                // un atajo ni se ha soltado y vuelto a pulsar entre medias.
                if state.meta_sola && state.meta_desde == Some(pulsada) {
                    // Ya no es «Meta a secas»: al soltarla no abre el launchpad.
                    state.meta_sola = false;
                    if let Some(shell) = state.shell.as_mut()
                        && !shell.hay_emergente()
                    {
                        shell.abrir_atajos();
                        state.hoja_por_meta = true;
                        state.needs_redraw = true;
                    }
                }
                TimeoutAction::Drop
            });
    if let Err(err) = programado {
        tracing::warn!("no se pudo contar la pulsación de Meta: {err}");
    }
}

// 0.1.0
fn repetir_tecla(state: &mut BookosComp, tecla: TeclaPulsada) {
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
                    .is_some_and(|s| s.tecla(tecla.clone()).0);
                if !seguir {
                    state.repeticion_retroceso = None;
                    return TimeoutAction::Drop;
                }
                state.needs_redraw = true;
                TimeoutAction::ToDuration(cada)
            });
    match token {
        Ok(token) => state.repeticion_retroceso = Some(token),
        Err(err) => tracing::warn!("sin repetición de tecla: {err}"),
    }
}

fn es_tecla_muerta(sym: u32) -> bool {
    matches!(
        sym,
        keysyms::KEY_dead_acute
            | keysyms::KEY_dead_grave
            | keysyms::KEY_dead_diaeresis
            | keysyms::KEY_dead_circumflex
            | keysyms::KEY_dead_tilde
    )
}

fn combinar_muerta(muerta: u32, c: char) -> Option<char> {
    match (muerta, c) {
        (keysyms::KEY_dead_acute, 'a') => Some('á'),
        (keysyms::KEY_dead_acute, 'e') => Some('é'),
        (keysyms::KEY_dead_acute, 'i') => Some('í'),
        (keysyms::KEY_dead_acute, 'o') => Some('ó'),
        (keysyms::KEY_dead_acute, 'u') => Some('ú'),
        (keysyms::KEY_dead_acute, 'A') => Some('Á'),
        (keysyms::KEY_dead_acute, 'E') => Some('É'),
        (keysyms::KEY_dead_acute, 'I') => Some('Í'),
        (keysyms::KEY_dead_acute, 'O') => Some('Ó'),
        (keysyms::KEY_dead_acute, 'U') => Some('Ú'),
        (keysyms::KEY_dead_acute, ' ') => Some('´'),
        (keysyms::KEY_dead_grave, 'a') => Some('à'),
        (keysyms::KEY_dead_grave, 'e') => Some('è'),
        (keysyms::KEY_dead_grave, 'i') => Some('ì'),
        (keysyms::KEY_dead_grave, 'o') => Some('ò'),
        (keysyms::KEY_dead_grave, 'u') => Some('ù'),
        (keysyms::KEY_dead_grave, 'A') => Some('À'),
        (keysyms::KEY_dead_grave, 'E') => Some('È'),
        (keysyms::KEY_dead_grave, 'I') => Some('Ì'),
        (keysyms::KEY_dead_grave, 'O') => Some('Ò'),
        (keysyms::KEY_dead_grave, 'U') => Some('Ù'),
        (keysyms::KEY_dead_grave, ' ') => Some('`'),
        (keysyms::KEY_dead_diaeresis, 'a') => Some('ä'),
        (keysyms::KEY_dead_diaeresis, 'e') => Some('ë'),
        (keysyms::KEY_dead_diaeresis, 'i') => Some('ï'),
        (keysyms::KEY_dead_diaeresis, 'o') => Some('ö'),
        (keysyms::KEY_dead_diaeresis, 'u') => Some('ü'),
        (keysyms::KEY_dead_diaeresis, 'A') => Some('Ä'),
        (keysyms::KEY_dead_diaeresis, 'E') => Some('Ë'),
        (keysyms::KEY_dead_diaeresis, 'I') => Some('Ï'),
        (keysyms::KEY_dead_diaeresis, 'O') => Some('Ö'),
        (keysyms::KEY_dead_diaeresis, 'U') => Some('Ü'),
        (keysyms::KEY_dead_diaeresis, ' ') => Some('¨'),
        (keysyms::KEY_dead_circumflex, 'a') => Some('â'),
        (keysyms::KEY_dead_circumflex, 'e') => Some('ê'),
        (keysyms::KEY_dead_circumflex, 'i') => Some('î'),
        (keysyms::KEY_dead_circumflex, 'o') => Some('ô'),
        (keysyms::KEY_dead_circumflex, 'u') => Some('û'),
        (keysyms::KEY_dead_circumflex, 'A') => Some('Â'),
        (keysyms::KEY_dead_circumflex, 'E') => Some('Ê'),
        (keysyms::KEY_dead_circumflex, 'I') => Some('Î'),
        (keysyms::KEY_dead_circumflex, 'O') => Some('Ô'),
        (keysyms::KEY_dead_circumflex, 'U') => Some('Û'),
        (keysyms::KEY_dead_circumflex, ' ') => Some('^'),
        (keysyms::KEY_dead_tilde, 'a') => Some('ã'),
        (keysyms::KEY_dead_tilde, 'n') => Some('ñ'),
        (keysyms::KEY_dead_tilde, 'o') => Some('õ'),
        (keysyms::KEY_dead_tilde, 'A') => Some('Ã'),
        (keysyms::KEY_dead_tilde, 'N') => Some('Ñ'),
        (keysyms::KEY_dead_tilde, 'O') => Some('Õ'),
        (keysyms::KEY_dead_tilde, ' ') => Some('~'),
        _ => None,
    }
}

fn canal_pegado(state: &mut BookosComp) -> smithay::reexports::calloop::channel::Sender<String> {
    use smithay::reexports::calloop::channel;
    if let Some(tx) = &state.pegado_canal {
        return tx.clone();
    }
    let (tx, rx) = channel::channel::<String>();
    let insertado = state.loop_handle.insert_source(rx, |evento, _, state| {
        if let channel::Event::Msg(texto) = evento
            && state
                .shell
                .as_ref()
                .is_some_and(|s| matches!(s.emergente_nombre(), Some("buscador" | "respuesta" | "polkit" | "elegir-app")))
            && let Some(shell) = state.shell.as_mut()
            && shell.tecla(TeclaPulsada::Pegar(texto)).0
        {
            state.needs_redraw = true;
        }
    });
    if let Err(err) = insertado {
        tracing::error!("no se pudo registrar el canal de pegado del shell: {err}");
    }
    state.pegado_canal = Some(tx.clone());
    tx
}

fn pegar_en_shell(state: &mut BookosComp) {
    use smithay::wayland::selection::data_device::{
        current_data_device_selection_userdata, request_data_device_client_selection,
    };
    const M_TEXTO: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
    let propio =
        current_data_device_selection_userdata(&state.seat).and_then(|datos| match &*datos {
            crate::handlers::Seleccion::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
            crate::handlers::Seleccion::PorTipo(tipos) => M_TEXTO
                .iter()
                .find_map(|m| tipos.iter().find(|(t, _)| t == m))
                .map(|(_, b)| String::from_utf8_lossy(b).into_owned()),
            crate::handlers::Seleccion::X11 => None,
        });
    if let Some(texto) = propio {
        if let Some(shell) = state.shell.as_mut()
            && shell.tecla(TeclaPulsada::Pegar(texto)).0
        {
            state.needs_redraw = true;
        }
        return;
    }
    let Some(tipo) = M_TEXTO
        .iter()
        .find(|m| state.archivos.tipos.iter().any(|t| t == *m))
        .copied()
    else {
        return;
    };
    let Ok((lectura, escritura)) = smithay::reexports::rustix::pipe::pipe() else {
        return;
    };
    if let Err(err) = request_data_device_client_selection(&state.seat, tipo.into(), escritura) {
        tracing::warn!(?err, "no se pudo pedir texto del portapapeles");
        return;
    }
    let tx = canal_pegado(state);
    let _ = std::thread::Builder::new()
        .name("pegar-shell".into())
        .spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let mut f = std::fs::File::from(lectura);
            // Tope razonable para no leer megabytes en un buscador de una línea.
            let mut limitado = (&mut f).take(16 * 1024);
            if limitado.read_to_end(&mut buf).is_ok() {
                let texto = String::from_utf8_lossy(&buf).into_owned();
                if !texto.is_empty() {
                    let _ = tx.send(texto);
                }
            }
        });
}

/// La acción de un remapeo. Con el bloqueo echado no hace nada: una tecla
/// cambiada no puede ser una forma de saltárselo.
// 0.1.0
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
// 0.1.0
fn capturar(state: &mut BookosComp, evdev: u32) -> bool {
    const MODIFICADORES: [u32; 8] = [29, 97, 42, 54, 56, 100, 125, 126];
    if MODIFICADORES.contains(&evdev) || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return false;
    }
    let Some(destino) = state.captura_tecla.take() else {
        return false;
    };
    let copilot = state.traductor.es_copilot(evdev);
    crate::ajustes::avisar_tecla_capturada(state, &destino, evdev, copilot);
    state.traductor.tragar_suelta(evdev);
    true
}

// 0.1.0
// modified 0.1.1 -- el salvapantallas se traga puntero y tacto, la pantalla táctil entra por toques y el kiosko ignora los gestos
pub fn handle<B: InputBackend>(state: &mut BookosComp, event: InputEvent<B>) {
    // añadido en la 0.2.0 -- se asocia al próximo fotograma compuesto, nunca al que ya está en vuelo.
    let ahora = std::time::Instant::now();
    state.last_input = Some(ahora);
    state.metricas.entrada(ahora);
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
                if state.cursor_oculto_por_toque || state.teclado.tactil {
                    crate::extras::toque_fuera(state);
                }
                crate::teclado::retirar(state);
            }
            // añadido en la 0.2.0 -- soltar la tecla del asistente mantenida envía lo dicho.
            if !pulsada
                && let Some((_, desde)) = state.asistente_tecla.take_if(|(k, _)| *k == evdev)
                && desde.elapsed() >= MANTENER_ASISTENTE
            {
                crate::keybinds::lanzar(state, "bookos-assistant --soltar");
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
            // Aquí y no en `keybinds`: por Meta+Alt+A o por una tecla
            // remapeada (Copilot), la que hay que vigilar es esta física.
            if pulsada && std::mem::take(&mut state.asistente_armado) {
                contar_asistente_mantenido(state, evdev);
            }
        }

        // libinput: desplazamiento relativo respecto de donde estaba el cursor.
        InputEvent::PointerMotion { event } => {
            mostrar_cursor_raton(state);
            let delta = (event.delta_x(), event.delta_y());
            let destino = state.pointer_location + Point::from(delta);
            if state
                .agitar_cursor
                .mover(destino.x, destino.y, std::time::Instant::now())
            {
                state.needs_redraw = true;
            }
            mover_miniatura(state, destino);
            set_pointer(state, destino, event.time_msec());
        }

        // winit: posición absoluta, normalizada al tamaño de la ventana.
        InputEvent::PointerMotionAbsolute { event } => {
            mostrar_cursor_raton(state);
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
            let destino: Point<f64, Logical> = (
                origen.x as f64 + event.x_transformed(w),
                origen.y as f64 + event.y_transformed(h),
            )
                .into();
            if state
                .agitar_cursor
                .mover(destino.x, destino.y, std::time::Instant::now())
            {
                state.needs_redraw = true;
            }
            mover_miniatura(state, destino);
            set_pointer(state, destino, event.time_msec());
        }

        InputEvent::PointerButton { event } => {
            mostrar_cursor_raton(state);
            state.teclado.tactil = false;
            boton(state, event.button_code(), event.state(), event.time_msec());
            // Ctrl+clic con Ctrl pegado: el clic lo gasta, como una tecla.
            if event.state() == ButtonState::Pressed {
                crate::teclas_accesibles::tras_clic(state, event.time_msec());
            }
        }

        InputEvent::PointerAxis { event } => {
            mostrar_cursor_raton(state);
            // Con una emergente abierta, el gesto es suyo: en el launchpad pasa
            // de página, como el swipe horizontal del plasmoide. `amount` viene
            // en píxeles lógicos con el touchpad y en muescas con la rueda, y a
            // las dos se les da el mismo trato porque el umbral está en píxeles.
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            let punto = state.pointer_location;
            if dx != 0.0
                && let Some(accion) = state
                    .shell
                    .as_mut()
                    .and_then(|s| s.rueda_aviso(punto.x, punto.y, dx))
            {
                if let Some(accion) = accion {
                    crate::keybinds::hacer(state, accion);
                }
                state.needs_redraw = true;
                return;
            }
            if (dx != 0.0 || dy != 0.0) && state.shell.as_mut().is_some_and(|s| s.desplazar(dx, dy))
            {
                state.needs_redraw = true;
                return;
            }
            if entrada_cliente_bloqueada(state) {
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
            if !gestos_touchpad_permitidos(state) {
                return;
            }
            // añadido en la 0.2.0 -- agarrar no espera al primer desplazamiento reconocido.
            match event.fingers() {
                4 => crate::escritorios::agarrar_dedos(state),
                3 => agarrar_exposicion(state),
                _ => {}
            }
            let ancho = state
                .salida_del_puntero()
                .and_then(|o| state.space.output_geometry(o))
                .map_or(1.0, |r| r.size.w as f64);
            state
                .gestos
                .deslizamiento_inicio(event.fingers(), ancho, event.time());
            // añadido en la 0.2.0 -- cuatro dedos en vertical siguen a la mano con la vista de escritorios: subir la abre, abierta, bajar la cierra.
            if event.fingers() == 4 {
                let abierta = state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.vista_escritorios_abierta());
                state.gestos.vista_continua(abierta);
                if !abierta {
                    state
                        .gestos
                        .despeje_continuo(if state.escritorios.apartando() {
                            -1.0
                        } else {
                            1.0
                        });
                }
            }
        }

        // Los gestos mueven escritorios y ventanas: si se perdió el permiso a mitad, se cancela.
        InputEvent::GestureSwipeUpdate { event } if !gestos_touchpad_permitidos(state) => {
            finalizar_deslizamiento_touchpad(state, true, event.time());
        }

        InputEvent::GestureSwipeUpdate { event } => {
            if let Some(gesto) =
                state
                    .gestos
                    .deslizamiento_avance(event.delta_x(), event.delta_y(), event.time())
            {
                tracing::debug!(?gesto, "gesto de touchpad");
                match gesto {
                    Gesto::Continuo(crate::gestos::Movimiento::Escritorio, progreso) => {
                        crate::escritorios::seguir_dedos(state, progreso);
                    }
                    Gesto::Continuo(crate::gestos::Movimiento::Exposicion, progreso) => {
                        seguir_exposicion(state, progreso);
                    }
                    Gesto::Continuo(crate::gestos::Movimiento::Despeje, progreso) => {
                        crate::escritorios::soltar_agarre_cancelado(state);
                        crate::escritorios::seguir_despeje(state, progreso);
                    }
                    Gesto::Continuo(crate::gestos::Movimiento::Vista, progreso) => {
                        crate::escritorios::soltar_agarre_cancelado(state);
                        let activo = crate::escritorios::activo_aqui(state);
                        let nombres = state.escritorios.nombres().to_vec();
                        if let Some(shell) = state.shell.as_mut() {
                            shell.seguir_vista(progreso, activo, nombres);
                        }
                        state.revisar_barras();
                        state.needs_redraw = true;
                    }
                    Gesto::Continuo(crate::gestos::Movimiento::Launchpad, _) => {}
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
                        crate::escritorios::soltar_agarre_cancelado(state);
                        crate::keybinds::ejecutar(state, crate::keybinds::Accion::VistaEscritorios);
                    }
                    Gesto::DespejarEscritorio => {
                        crate::escritorios::soltar_agarre_cancelado(state);
                        crate::escritorios::despejar(state);
                    }
                    // Subir con el escritorio despejado devuelve las ventanas;
                    // subir estando en una aplicación cualquiera enseña los
                    // escritorios, que es lo que hace el mismo gesto en macOS.
                    Gesto::SubirCuatroDedos => {
                        crate::escritorios::soltar_agarre_cancelado(state);
                        if state.escritorios.despejado() {
                            crate::escritorios::recuperar(state);
                        } else {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::VistaEscritorios,
                            );
                        }
                    }
                }
            }
        }

        InputEvent::GestureSwipeEnd { event } => {
            let cancelada = event.cancelled() || !gestos_touchpad_permitidos(state);
            finalizar_deslizamiento_touchpad(state, cancelada, event.time());
        }

        InputEvent::GesturePinchBegin { event } => {
            if !gestos_touchpad_permitidos(state) {
                return;
            }
            state.gestos.pellizco_inicio(event.fingers(), event.time());
            // añadido en la 0.2.0 -- una apertura puede devolverse desde el fotograma que estaba mostrando.
            // Solo los pellizcos del compositor (5 dedos) agarran el launchpad; con 4 dedos el fin
            // devuelve `None` y dejaría `launchpad_gesto` colgado para siempre.
            if !state.gestos.pellizco_de_la_ventana() {
                if let Some(shell) = state.shell.as_mut() {
                    shell.agarrar_launchpad();
                }
            } else if !entrada_cliente_bloqueada(state) {
                state.pinch_cliente_activo = true;
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
            if !state.pinch_cliente_activo {
                return;
            }
            let pointer = state.pointer.clone();
            if entrada_cliente_bloqueada(state) {
                state.pinch_cliente_activo = false;
                pointer.gesture_pinch_end(
                    state,
                    &smithay::input::pointer::GesturePinchEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: true,
                    },
                );
                return;
            }
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
            if !gestos_touchpad_permitidos(state) {
                if let Some(suelta) = state.gestos.pellizco_fin(true, event.time())
                    && let Some(shell) = state.shell.as_mut()
                {
                    shell.soltar_launchpad(suelta);
                    state.needs_redraw = true;
                }
            } else if let Some(progreso) = state.gestos.pellizco_avance(event.scale(), event.time())
                && let Some(shell) = state.shell.as_mut()
            {
                shell.seguir_launchpad(progreso);
                state.needs_redraw = true;
            }
        }

        InputEvent::GesturePinchEnd { event } => {
            if state.pinch_cliente_activo {
                state.pinch_cliente_activo = false;
                let pointer = state.pointer.clone();
                pointer.gesture_pinch_end(
                    state,
                    &smithay::input::pointer::GesturePinchEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled() || entrada_cliente_bloqueada(state),
                    },
                );
            }
            let cancelada = event.cancelled() || !gestos_touchpad_permitidos(state);
            if let Some(suelta) = state.gestos.pellizco_fin(cancelada, event.time()) {
                if let Some(shell) = state.shell.as_mut() {
                    shell.soltar_launchpad(suelta);
                }
                state.needs_redraw = true;
            }
        }

        // «Mantener» —dedos quietos sobre el touchpad— no es de nadie del
        // escritorio. Los navegadores lo usan para frenar el scroll con inercia.
        InputEvent::GestureHoldBegin { event } => {
            if entrada_cliente_bloqueada(state) {
                return;
            }
            state.hold_cliente_activo = true;
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
            if !std::mem::take(&mut state.hold_cliente_activo) {
                return;
            }
            let pointer = state.pointer.clone();
            pointer.gesture_hold_end(
                state,
                &smithay::input::pointer::GestureHoldEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                    cancelled: event.cancelled() || entrada_cliente_bloqueada(state),
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

pub(crate) fn gestos_touchpad_permitidos(state: &BookosComp) -> bool {
    !state.modo.kiosko()
        && !state.cuentagotas
        && state.vista_tactil.is_none()
        && !state.shell.as_ref().is_some_and(|s| {
            s.esta_bloqueado()
                || s.hay_captura()
                || matches!(
                    s.emergente_nombre(),
                    Some("polkit" | "acceso" | "elegir-app")
                )
        })
}

fn entrada_cliente_bloqueada(state: &BookosComp) -> bool {
    !gestos_touchpad_permitidos(state)
        || state.shell.as_ref().is_some_and(|s| {
            s.hay_conmutador() || s.vista_escritorios_abierta() || s.emergente_tapa_la_pantalla()
        })
}

fn finalizar_deslizamiento_touchpad(state: &mut BookosComp, cancelada: bool, tiempo: u64) {
    let Some(suelta) = state.gestos.deslizamiento_fin(cancelada, tiempo) else {
        return;
    };
    if suelta.movimiento != crate::gestos::Movimiento::Escritorio {
        crate::escritorios::soltar_agarre_cancelado(state);
    }
    match suelta.movimiento {
        crate::gestos::Movimiento::Escritorio => crate::escritorios::soltar_dedos(state, suelta),
        crate::gestos::Movimiento::Exposicion => soltar_exposicion(state, suelta),
        crate::gestos::Movimiento::Despeje => crate::escritorios::soltar_despeje(state, suelta),
        crate::gestos::Movimiento::Vista => {
            if let Some(shell) = state.shell.as_mut() {
                shell.soltar_vista(suelta);
            }
            state.revisar_barras();
            state.needs_redraw = true;
        }
        crate::gestos::Movimiento::Launchpad => {}
    }
}

/// Dónde cae un toque en el escritorio.
///
/// La pantalla táctil es la del panel interno, no la que tenga el puntero: con
/// un monitor externo enchufado, el dedo sigue estando sobre el portátil. En
/// el anidado no hay `eDP` y se usa la única salida, como `PointerMotionAbsolute`.
/// La rotación de la salida no se aplica: `output_geometry` ya da el tamaño
/// girado, pero el eje del digitalizador no gira con él.
// 0.1.1
pub(crate) fn salida_tactil(state: &BookosComp) -> Option<&smithay::output::Output> {
    state
        .space
        .outputs()
        .find(|o| {
            ["eDP", "LVDS", "DSI"]
                .iter()
                .any(|p| o.name().starts_with(p))
        })
        .or_else(|| state.space.outputs().next())
}

fn punto_tactil<B: InputBackend>(
    state: &BookosComp,
    event: &impl AbsolutePositionEvent<B>,
) -> Point<f64, Logical> {
    let salida = salida_tactil(state);
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
// 0.1.1
fn ocultar_cursor_tactil(state: &mut BookosComp) {
    if !state.cursor_oculto_por_toque {
        state.cursor_oculto_por_toque = true;
        state.needs_redraw = true;
    }
}

pub(crate) fn mostrar_cursor_raton(state: &mut BookosComp) {
    if state.cursor_oculto_por_toque {
        state.cursor_oculto_por_toque = false;
        crate::extras::toque_fuera(state);
        state.needs_redraw = true;
    }
}

fn borde_redimension_tactil(
    state: &BookosComp,
    punto: Point<f64, Logical>,
) -> Option<(smithay::desktop::Window, crate::ventanas::Modo)> {
    use smithay::utils::Rectangle;
    for w in state
        .space
        .elements()
        .rev()
        .filter(|w| crate::multitarea::visible(state, w))
    {
        let Some(loc) = state.space.element_location(w) else {
            continue;
        };
        let size = w.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let decorada = crate::decoracion::decorada(w);
        let top_y = if decorada {
            loc.y - crate::decoracion::ALTO
        } else {
            loc.y
        };
        let anillo = Rectangle::new(
            (loc.x - 22, top_y - 22).into(),
            (size.w + 44, loc.y - top_y + size.h + 44).into(),
        );
        if !anillo.to_f64().contains(punto) {
            continue;
        }
        let dentro = Rectangle::new(
            (loc.x, top_y).into(),
            (size.w, loc.y - top_y + size.h).into(),
        );
        if !crate::ventanas::completa(w)
            && !crate::ventanas::encajada(w)
            && !crate::multitarea::agrupada(state, w)
            && let Some(modo) = arista_tactil(dentro.to_f64(), punto)
        {
            return Some((w.clone(), modo));
        }
        if Rectangle::new(loc, size).to_f64().contains(punto)
            || crate::decoracion::barra_rect(state, w).is_some_and(|r| r.to_f64().contains(punto))
        {
            return None;
        }
    }
    None
}

fn arista_tactil(
    rect: smithay::utils::Rectangle<f64, Logical>,
    punto: Point<f64, Logical>,
) -> Option<crate::ventanas::Modo> {
    let izquierda = punto.x - rect.loc.x;
    let derecha = rect.size.w - izquierda;
    let arriba = punto.y - rect.loc.y;
    let abajo = rect.size.h - arriba;
    // Las esquinas inferiores dan 44 puntos de agarre (−22..+22); en las aristas
    // rectas se acota a 10 px por fuera y 6 px por dentro para no robar toques
    // lejos de la ventana ni invadir contenido o scroll.
    let esquina_inferior = (-22.0..22.0).contains(&abajo)
        && ((-22.0..22.0).contains(&izquierda) || (-22.0..22.0).contains(&derecha));
    let dentro_vertical = (-10.0..).contains(&arriba) && (-10.0..).contains(&abajo);
    let dentro_horizontal = (-10.0..).contains(&izquierda) && (-10.0..).contains(&derecha);
    let horizontal = if esquina_inferior {
        Some(izquierda < derecha)
    } else if dentro_vertical && (-10.0..6.0).contains(&izquierda) {
        Some(true)
    } else if dentro_vertical && (-10.0..6.0).contains(&derecha) {
        Some(false)
    } else {
        None
    };
    let vertical = if esquina_inferior {
        Some(false)
    } else if dentro_horizontal && (-10.0..6.0).contains(&arriba) {
        Some(true)
    } else if dentro_horizontal && (-10.0..6.0).contains(&abajo) {
        Some(false)
    } else {
        None
    };
    (horizontal.is_some() || vertical.is_some()).then_some(
        crate::ventanas::Modo::RedimensionarBorde {
            horizontal,
            vertical,
        },
    )
}

// modificado en la 0.2.0 -- el apoyo resalta; la acción espera únicamente a la liberación del dedo.
pub fn toque_abajo(state: &mut BookosComp, slot: TouchSlot, punto: Point<f64, Logical>, time: u32) {
    ocultar_cursor_tactil(state);
    if state.contactos.apoyar(slot, punto, time)
        || crate::extras::interceptar_toque_abajo(state, slot, punto, time)
    {
        return;
    }
    toque_abajo_normal(state, slot, punto, time);
}

pub(crate) fn toque_abajo_normal(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Point<f64, Logical>,
    time: u32,
) {
    if crate::multitarea_tactil::pulsar(state, Some(slot), punto, time) {
        return;
    }
    crate::tactil::renovar_barra(state, punto);
    crate::teclado::iniciar_toque(state, slot, punto, time);
    if crate::teclado::pulsar(state, punto, crate::teclado::Dedo::Toque(slot)) {
        return;
    }
    state.pointer_location = punto;
    let Some(touch) = state.seat.get_touch() else {
        return;
    };
    let capa_modal = state
        .shell
        .as_ref()
        .is_some_and(|s| s.esta_bloqueado() || s.hay_captura() || s.hay_conmutador());
    if state.toque_emulado.is_none() && !capa_modal && !state.shell_contiene(punto) {
        if crate::multitarea::empezar_separador_tactil(state, punto) {
            state.toque_emulado = Some(ToqueEmulado::Separador {
                slot,
                origen: punto,
                movido: false,
            });
            return;
        }
        if !crate::decoracion::barra_en(state, punto).is_some_and(|(_, boton)| boton.is_some())
            && let Some((window, modo)) = borde_redimension_tactil(state, punto)
        {
            state.enfocar(&window);
            state.arrastrar_ventana(&window, modo, false);
            state.toque_emulado = Some(ToqueEmulado::ArrastreVentana(slot));
            return;
        }
    }
    // La captura y el conmutador se quedan la entrada aunque haya una ventana
    // debajo, igual que hacen con el ratón en `boton`.
    let pantalla = state.pantalla();
    let en_miniatura = state
        .miniatura_captura
        .as_ref()
        .is_some_and(|m| m.contiene(punto, pantalla));
    let del_shell = state.toque_emulado.is_some()
        || en_miniatura
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
            // Si el toque va a una ventana normal (no a una layer-surface de extras),
            // avisamos a `bookos-extras` por si tiene la cortina abierta.
            let es_capa = state
                .space
                .layer_for_surface(&foco.0, smithay::desktop::WindowSurfaceType::ALL)
                .is_some();
            if !es_capa {
                crate::extras::toque_fuera(state);
            }
            crate::teclado::tocado(state, &foco.0);
            if let Some((window, loc)) = state.ventana_bajo(punto) {
                state.enfocar(&window);
                if state.toque_titulo_pendiente.is_some() {
                    // Un segundo dedo invalida el posible arrastre de título (p. ej. pellizco o scroll).
                    state.toque_titulo_pendiente = None;
                } else if state.arrastre.is_none()
                    && !es_capa
                    && !crate::ventanas::completa(&window)
                    && !crate::decoracion::decorada(&window)
                {
                    let w = window.geometry().size.w as f64;
                    let lx = punto.x - loc.x as f64;
                    let ly = punto.y - loc.y as f64;
                    if (0.0..=38.0).contains(&ly) && w > 0.0 && lx >= w * 0.18 && lx <= w * 0.82 {
                        state.toque_titulo_pendiente = Some((slot, window, punto));
                    }
                }
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
            crate::extras::toque_fuera(state);
            set_pointer(state, punto, time);
            let del_shell = state.shell_contiene(punto)
                || en_miniatura
                || state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.esta_bloqueado() || s.hay_captura() || s.hay_conmutador());
            let en_conmutador = state
                .shell
                .as_ref()
                .is_some_and(|s| s.hay_conmutador());
            // added 0.2.3 -- los mandos de la música del bloqueo se aprietan
            // al apoyar el dedo, como con el ratón; el resto del bloqueo
            // (apagar, la contraseña) sigue esperando a levantarlo.
            let directo = state
                .shell
                .as_ref()
                .is_some_and(|s| s.bloqueo_toque_directo(punto.x, punto.y));
            let titulo = !del_shell
                && crate::decoracion::barra_en(state, punto)
                    .is_some_and(|(_, boton)| boton.is_none());
            let escritorio =
                !capa_modal && !del_shell && !titulo && state.space.element_under(punto).is_none();
            state.toque_emulado = Some(if !directo && !en_conmutador {
                ToqueEmulado::Pendiente {
                    slot,
                    origen: punto,
                    tiempo: time,
                    arrastrable: escritorio
                        || en_miniatura
                        || titulo
                        || state
                            .shell
                            .as_ref()
                            .is_some_and(|s| s.toque_arrastrable(punto.x, punto.y)),
                }
            } else {
                boton(state, BTN_LEFT, ButtonState::Pressed, time);
                ToqueEmulado::Arrastrando(slot)
            });
            let isla = del_shell
                && state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.sobre_actividad(punto.x, punto.y));
            let dock = del_shell
                && state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.en_el_dock(punto.x, punto.y));
            if !capa_modal && (isla || dock || escritorio || titulo) {
                contar_toque_largo(state, slot, punto, time, isla);
            }
        }
        None => {}
    }
}

/// Abre el mismo menú del puntero si el contacto se mantiene quieto.
fn contar_toque_largo(
    state: &mut BookosComp,
    slot: TouchSlot,
    origen: Point<f64, Logical>,
    time: u32,
    isla: bool,
) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let programado = state.loop_handle.insert_source(
        Timer::from_duration(crate::shell::MANTENER_ISLA),
        move |_, _, state| {
            let sigue = matches!(
                state.toque_emulado,
                Some(ToqueEmulado::Pendiente { slot: s, origen: o, tiempo, .. })
                    if s == slot && o == origen && tiempo == time
            );
            if !sigue
                || state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.esta_bloqueado() || s.hay_captura())
            {
                return TimeoutAction::Drop;
            }
            state.toque_emulado = Some(ToqueEmulado::Cancelado(slot));
            let posicion = state.pointer_location;
            set_pointer(state, origen, time);
            if isla {
                if !state.shell.as_ref().is_some_and(|s| s.actividad_abierta()) {
                    boton(state, BTN_LEFT, ButtonState::Pressed, time);
                    boton(state, BTN_LEFT, ButtonState::Released, time);
                }
                if state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.menu_actividad_en(origen.x, origen.y))
                {
                    state.needs_redraw = true;
                }
            } else {
                boton(state, BTN_RIGHT, ButtonState::Pressed, time);
                boton(state, BTN_RIGHT, ButtonState::Released, time);
            }
            set_pointer(state, posicion, time);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = programado {
        tracing::warn!("no se pudo contar el toque largo: {err}");
    }
}

// 0.1.1
// modificado en la 0.2.0 -- superar el margen elige arrastre o cancelación sin activar botones.
pub fn toque_mueve(state: &mut BookosComp, slot: TouchSlot, punto: Point<f64, Logical>, time: u32) {
    ocultar_cursor_tactil(state);
    if crate::tactil::mover_escritorio(state, slot, punto, time)
        || crate::extras::interceptar_toque_mueve(state, slot, punto, time)
    {
        return;
    }
    toque_mueve_normal(state, slot, punto, time);
}

pub(crate) fn toque_mueve_normal(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Point<f64, Logical>,
    time: u32,
) {
    if crate::multitarea_tactil::mover(state, Some(slot), punto, time) {
        return;
    }
    crate::teclado::mover_toque(state, slot, punto);
    if let Some(toque) = state.toque_emulado {
        if toque.slot() != slot {
            return;
        }
        if let ToqueEmulado::ArrastreVentana(_) = toque {
            set_pointer(state, punto, time);
            return;
        }
        if let ToqueEmulado::Separador { origen, movido, .. } = toque {
            if movido || (punto.x - origen.x).abs() >= 10.0 {
                state.toque_emulado = Some(ToqueEmulado::Separador {
                    slot,
                    origen,
                    movido: true,
                });
                set_pointer(state, punto, time);
            } else if (punto.y - origen.y).abs() >= 10.0 {
                crate::multitarea::soltar(state);
                state.toque_emulado = Some(ToqueEmulado::Cancelado(slot));
            }
            return;
        }
        if let ToqueEmulado::Pendiente {
            origen,
            arrastrable,
            ..
        } = toque
            && (punto.x - origen.x).hypot(punto.y - origen.y) > 10.0
        {
            if arrastrable {
                set_pointer(state, origen, time);
                boton(state, BTN_LEFT, ButtonState::Pressed, time);
                state.toque_emulado = Some(ToqueEmulado::Arrastrando(slot));
            } else {
                state.toque_emulado = Some(ToqueEmulado::Cancelado(slot));
            }
        }
        set_pointer(state, punto, time);
        return;
    }
    state.pointer_location = punto;
    if crate::multitarea::seguir_separador(state) || state.seguir_arrastre() {
        state.toque_titulo_pendiente = None;
        state.space.refresh();
        let fallback = state.output_scale();
        state.broadcast_preferred_scale(fallback);
        return;
    }
    if let Some((pend_slot, _, origen)) = state.toque_titulo_pendiente.as_ref()
        && *pend_slot == slot
        && (punto.x - origen.x).hypot(punto.y - origen.y) > 12.0
    {
        let (_, window, origen) = state.toque_titulo_pendiente.take().unwrap();
        if let Some(touch) = state.seat.get_touch() {
            touch.cancel(state);
        }
        state.pointer_location = origen;
        state.arrastrar_ventana(&window, crate::ventanas::Modo::Mover, false);
        state.pointer_location = punto;
        state.seguir_arrastre();
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

// 0.1.1
// modificado en la 0.2.0 -- la holgura conserva el objetivo inicial al confirmar.
pub fn toque_arriba(state: &mut BookosComp, slot: TouchSlot, time: u32) {
    ocultar_cursor_tactil(state);
    if crate::tactil::soltar_escritorio(state, slot, time)
        || crate::extras::interceptar_toque_arriba(state, slot, time)
    {
        return;
    }
    toque_arriba_normal(state, slot, time);
}

pub(crate) fn toque_arriba_normal(state: &mut BookosComp, slot: TouchSlot, time: u32) {
    if crate::multitarea_tactil::soltar(state, Some(slot), time, false) {
        return;
    }
    crate::teclado::terminar_toque(state, slot, time);
    if crate::teclado::soltar(state, crate::teclado::Dedo::Toque(slot)) {
        return;
    }
    if state
        .toque_titulo_pendiente
        .as_ref()
        .is_some_and(|(s, _, _)| *s == slot)
    {
        state.toque_titulo_pendiente = None;
    }
    if let Some(toque) = state.toque_emulado {
        if toque.slot() != slot {
            return;
        }
        state.toque_emulado = None;
        match toque {
            ToqueEmulado::Pendiente { origen, .. } => {
                // añadido en la 0.2.0 -- los 10 puntos de holgura pertenecen al control de origen, nunca al vecino.
                let posicion = state.pointer_location;
                set_pointer(state, origen, time);
                boton(state, BTN_LEFT, ButtonState::Pressed, time);
                boton(state, BTN_LEFT, ButtonState::Released, time);
                set_pointer(state, posicion, time);
            }
            ToqueEmulado::Arrastrando(_) => boton(state, BTN_LEFT, ButtonState::Released, time),
            ToqueEmulado::ArrastreVentana(_) => {
                let _ = state.soltar_arrastre();
            }
            ToqueEmulado::Separador { origen, movido, .. } => {
                crate::multitarea::soltar(state);
                if !movido
                    && let Some(w) =
                        crate::multitarea::separador_en_con_holgura(state, origen, 24.0)
                {
                    crate::keybinds::menu_separador(state, w);
                }
            }
            ToqueEmulado::Cancelado(_) => {}
        }
        return;
    }
    let solto_sep = crate::multitarea::soltar(state);
    let solto_arrastre = state.soltar_arrastre();
    if solto_arrastre {
        return;
    }
    if solto_sep {
        if let Some(touch) = state.seat.get_touch() {
            touch.cancel(state);
        }
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

// 0.1.1
// modificado en la 0.2.0 -- perder el contacto no ejecuta la acción pendiente.
pub(crate) fn toque_cancelar(state: &mut BookosComp, time: u32) {
    ocultar_cursor_tactil(state);
    crate::tactil::cancelar_escritorio(state, time);
    cancelar_toques_cliente(state, time);
}

pub(crate) fn cancelar_toques_cliente(state: &mut BookosComp, time: u32) {
    state.toque_titulo_pendiente = None;
    state.arrastre_exposicion = None;
    crate::extras::interceptar_toque_cancelar(state, time);
    crate::multitarea_tactil::cancelar_toque(state, time);
    crate::teclado::cancelar_toques(state);
    let _ = crate::multitarea::soltar(state);
    if let Some(arrastre) = state.arrastre.as_mut() {
        arrastre.zona = None;
    }
    let _ = state.soltar_arrastre();
    if state
        .miniatura_captura
        .as_mut()
        .is_some_and(|m| m.cancelar_arrastre())
    {
        state.needs_redraw = true;
    }
    // Un botón pulsado sin su soltar deja al shell creyendo que se sigue
    // arrastrando: se suelta aunque el toque se haya anulado.
    if matches!(
        state.toque_emulado.take(),
        Some(ToqueEmulado::Arrastrando(_))
    ) {
        if let Some(shell) = state.shell.as_mut() {
            shell.cancelar_toque();
        }
        state.decoracion.pulsado = None;
        boton(state, BTN_LEFT, ButtonState::Released, time);
    }
    if let Some(touch) = state.seat.get_touch() {
        touch.cancel(state);
    }
}

// añadido en la 0.2.0 -- un toque anulado no puede reaparecer como clic al recibir TouchUp.
#[derive(Debug, Clone, Copy)]
pub enum ToqueEmulado {
    Pendiente {
        slot: TouchSlot,
        origen: Point<f64, Logical>,
        tiempo: u32,
        arrastrable: bool,
    },
    Separador {
        slot: TouchSlot,
        origen: Point<f64, Logical>,
        movido: bool,
    },
    Arrastrando(TouchSlot),
    ArrastreVentana(TouchSlot),
    Cancelado(TouchSlot),
}

impl ToqueEmulado {
    // añadido en la 0.2.0 -- los demás dedos no cambian la captura del primer contacto.
    fn slot(self) -> TouchSlot {
        match self {
            Self::Pendiente { slot, .. }
            | Self::Separador { slot, .. }
            | Self::Arrastrando(slot)
            | Self::ArrastreVentana(slot)
            | Self::Cancelado(slot) => slot,
        }
    }
}

/// Traduce un keysym de xkb a la tecla que entiende el shell.
///
/// El shell no conoce xkb a propósito: su enum es una docena de variantes sin
/// ninguna dependencia, así que la frontera aguanta aunque debajo cambie la
/// capa de teclado.
// 0.1.0
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
        keysyms::KEY_Delete | keysyms::KEY_KP_Delete => TeclaPulsada::Suprimir,
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

// añadido en la 0.2.4
fn mover_miniatura(state: &mut BookosComp, destino: Point<f64, Logical>) {
    let pantalla = state.pantalla();
    let ahora = std::time::Instant::now();
    if let Some((cambio, programar)) = state
        .miniatura_captura
        .as_mut()
        .map(|m| m.mover_en(destino, pantalla, ahora))
    {
        if cambio {
            state.needs_redraw = true;
        }
        if programar {
            let _ = state.loop_handle.insert_source(
                Timer::from_duration(std::time::Duration::from_secs(2)),
                |_, _, state| {
                    state.needs_redraw = true;
                    TimeoutAction::Drop
                },
            );
        }
    }
}

// añadido en la 0.2.4
fn ejecutar_accion_miniatura(state: &mut BookosComp, accion: crate::miniatura_captura::Accion) {
    match accion {
        crate::miniatura_captura::Accion::Abrir {
            generacion,
            recurso,
        } => {
            let visor = bookos_shell::visor_imagenes();
            if let Some(mini) = state.miniatura_captura.as_mut() {
                match visor.as_ref() {
                    Some(v) => mini.configurar_visor(
                        Some(v.app_id.clone()),
                        v.wm_class.clone(),
                        None,
                        false,
                    ),
                    None => mini.configurar_visor(None, None, None, true),
                }
            }
            let _ = state.loop_handle.insert_source(
                Timer::from_duration(
                    crate::miniatura_captura::ESPERA_MAX_VISOR
                        + std::time::Duration::from_millis(50),
                ),
                |_, _, state| {
                    state.needs_redraw = true;
                    TimeoutAction::Drop
                },
            );
            let mut cmd = match visor {
                Some(v) => crate::keybinds::comando_directo(state, &v.programa, &v.argumentos),
                None => crate::keybinds::comando_directo(state, "xdg-open", &[]),
            };
            if let Some(ruta) = recurso.ruta_ya_guardada() {
                cmd.arg(&ruta);
                match cmd.spawn() {
                    Ok(hijo) => {
                        let pid = hijo.id();
                        if let Some(mini) = state.miniatura_captura.as_mut() {
                            mini.registrar_pid_visor(pid);
                        }
                        crate::keybinds::registrar_hijo(state, hijo, None);
                    }
                    Err(err) => {
                        tracing::error!(?ruta, "no se pudo abrir el visor de capturas: {err}");
                        if let Some(mini) = state.miniatura_captura.as_mut() {
                            mini.cancelar_apertura(std::time::Instant::now());
                        }
                    }
                }
                return;
            }
            let envio = state.captura_envio.clone();
            let _ = std::thread::Builder::new()
                .name("visor-captura".into())
                .spawn(move || {
                    use crate::miniatura_captura::{EstadoRecurso, EventoCaptura};
                    let ruta = match recurso.esperar(std::time::Duration::from_secs(3)) {
                        EstadoRecurso::Guardado(r) => Ok(r),
                        EstadoRecurso::PngEnMemoria(png) => {
                            bookos_shell::captura::guardar_bytes_png(&png, None)
                        }
                        EstadoRecurso::Error(err) => Err(std::io::Error::other(err)),
                        EstadoRecurso::Pendiente => {
                            Err(std::io::Error::other("tiempo de espera agotado"))
                        }
                    };
                    match ruta {
                        Ok(ruta) => {
                            cmd.arg(&ruta);
                            match cmd.spawn() {
                                Ok(hijo) => {
                                    let _ = envio.send(EventoCaptura::HijoVisor { generacion, hijo });
                                }
                                Err(err) => {
                                    tracing::error!(
                                        ?ruta,
                                        "no se pudo lanzar el visor de capturas: {err}"
                                    );
                                    let _ = envio.send(EventoCaptura::FalloAbrirVisor { generacion });
                                }
                            }
                        }
                        Err(err) => {
                            tracing::error!("no se pudo preparar la captura para abrir: {err}");
                            let _ = envio.send(EventoCaptura::FalloAbrirVisor { generacion });
                        }
                    }
                });
        }
        crate::miniatura_captura::Accion::Descartar => {}
    }
}

/// Un botón del ratón, venga de libinput, de winit o de un autotest: todos
/// pasan por aquí para que el clic de prueba recorra el mismo camino.
// modified 0.1.1 -- con el cuentagotas, el clic izquierdo coge el color y otro lo cancela
pub fn boton(state: &mut BookosComp, button: u32, pulsado: ButtonState, time: u32) {
    // Meta+clic mueve y redimensiona ventanas: si hubo clic, la tecla
    // era un modificador y no debe abrir nada al soltarla.
    state.meta_sola = false;
    if state.vista_tactil.is_some() {
        if button == BTN_LEFT {
            match pulsado {
                ButtonState::Pressed => {
                    crate::multitarea_tactil::pulsar(state, None, state.pointer_location, time);
                }
                ButtonState::Released => {
                    crate::multitarea_tactil::soltar(state, None, time, false);
                }
            }
        }
        return;
    }
    // añadido en la 0.2.4 -- interacción continua con la miniatura flotante de captura.
    if pulsado == ButtonState::Pressed {
        let pantalla = state.pantalla();
        let pos = state.pointer_location;
        let ahora = std::time::Instant::now();
        if let Some(mini) = state.miniatura_captura.as_mut() {
            if mini.esperando_visor() {
                if button != BTN_LEFT {
                    mini.cancelar_apertura(ahora);
                    state.needs_redraw = true;
                    return;
                }
                if mini.contiene(pos, pantalla) {
                    return;
                }
                mini.cancelar_apertura(ahora);
                state.needs_redraw = true;
            } else {
                if let Some(accion) = mini.pulsar(pos, pantalla, button != BTN_LEFT, ahora) {
                    ejecutar_accion_miniatura(state, accion);
                    state.needs_redraw = true;
                    return;
                }
                if mini.arrastrando() {
                    state.needs_redraw = true;
                    return;
                }
            }
        }
    } else if button == BTN_LEFT
        && pulsado == ButtonState::Released
        && state
            .miniatura_captura
            .as_ref()
            .is_some_and(|m| m.arrastrando())
    {
        let pos = state.pointer_location;
        let area_util = state.work_area();
        let ahora = std::time::Instant::now();
        if let Some(accion) = state
            .miniatura_captura
            .as_mut()
            .and_then(|m| m.soltar(pos, area_util, ahora))
        {
            ejecutar_accion_miniatura(state, accion);
        }
        state.needs_redraw = true;
        return;
    }
    // Un aviso cogido: el soltar decide si fue un clic o un arrastre, y no
    // llega a nadie más (la pulsación tampoco llegó).
    // añadido en la 0.2.2
    if button == BTN_LEFT
        && pulsado == ButtonState::Released
        && let Some(accion) = state.shell.as_mut().and_then(|s| s.soltar_aviso())
    {
        if let Some(accion) = accion {
            crate::keybinds::hacer(state, accion);
        }
        state.needs_redraw = true;
        return;
    }
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
    // El clic izquierdo coge el color y cualquier otro lo cancela. La suelta
    // también se traga: la pulsación no llegó al cliente. Va con su propia
    // marca porque la pulsación ya quitó el cuentagotas y la suelta llega con
    // él apagado; sin ella, la ventana recibía una suelta sin pulsación.
    // modificado en la 0.2.0 -- la suelta del clic que elige ya no llega a la ventana
    if pulsado == ButtonState::Released && std::mem::take(&mut state.cuentagotas_clic) {
        return;
    }
    if state.cuentagotas {
        if pulsado == ButtonState::Pressed {
            state.cuentagotas_clic = true;
            if button == BTN_LEFT {
                crate::cuentagotas::elegir(state);
            } else {
                crate::cuentagotas::cancelar(state);
            }
        }
        return;
    }
    // Bloqueado, los clics no salen hacia los clientes: lo único que
    // atienden es el botón de apagado de la propia pantalla de bloqueo.
    // La capa de captura se queda con el ratón mientras está: se está
    // encuadrando una foto y un clic no puede llegar a la ventana de
    // debajo. Va antes que el bloqueo porque son excluyentes y esta es
    // la que puede estar abierta con el escritorio en uso.
    if state.shell.as_ref().is_some_and(|s| s.hay_grabacion_pill()) {
        let (x, y) = (state.pointer_location.x, state.pointer_location.y);
        match pulsado {
            ButtonState::Pressed if state.shell.as_ref().is_some_and(|s| s.grabacion_pill_contiene(x, y)) => {
                if let Some(accion) = state.shell.as_mut().and_then(|s| s.grabacion_pill_pulsar(x, y)) {
                    crate::keybinds::hacer(state, accion);
                }
                state.needs_redraw = true;
                return;
            }
            ButtonState::Released => {
                if let Some(shell) = state.shell.as_mut() {
                    shell.grabacion_pill_soltar();
                }
            }
            _ => {}
        }
    }
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
        } else if let Some(shell) = state.shell.as_mut() {
            // añadido en la 0.2.3 -- soltar la barra de la música salta ahí.
            shell.bloqueo_soltar();
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

    if pulsado == ButtonState::Released && button == BTN_LEFT
        && let Some(arr) = state.arrastre_exposicion.take()
    {
        if !arr.moviendo {
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
        } else if let Some(zona) = arr.zona_split {
            state.conmutador_pegado = false;
            state.conmutador_salida = None;
            state.conmutador_entrada = None;
            state.exposicion_gesto = None;
            state.exposicion_salida_escritorio = None;
            state.exposicion_escritorio_sujeto = false;
            state.exposicion_desplazamiento_x.fijar(0.0);
            let destinos = std::mem::take(&mut state.conmutador_destinos);
            state.conmutador_modo = None;
            if let Some(shell) = state.shell.as_mut() {
                shell.cancelar_conmutador();
            }
            if let Some(window) = destinos.get(arr.indice).cloned().filter(|w| w.alive()) {
                crate::multitarea::emparejar_en_lado(state, &window, zona);
            }
            state.needs_redraw = true;
        } else if arr.actual.y - arr.origen.y < -110.0 {
            let mut muelle =
                bookos_shell::tema::Muelle::nuevo(0.25, bookos_shell::tema::M_SUAVE);
            muelle.soltar_gesto(1.0, 2.5);
            state.exposicion_salida_escritorio = Some(muelle);
            state.exposicion_escritorio_sujeto = false;
            state.needs_redraw = true;
        } else {
            state.needs_redraw = true;
        }
        return;
    }
    if pulsado == ButtonState::Released && std::mem::take(&mut state.conmutador_clic) {
        return;
    }
    if pulsado == ButtonState::Pressed && state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        let elegida = (button == BTN_LEFT)
            .then(|| {
                crate::keybinds::celda_bajo(state, state.pointer_location.x, state.pointer_location.y)
            })
            .flatten();
        if let Some(i) = elegida {
            // Un segundo clic durante la vuelta no cambia lo ya elegido.
            if state.conmutador_salida.is_none()
                && let Some(shell) = state.shell.as_mut()
            {
                shell.conmutador_elegir(i);
            }
            if state.conmutador_pegado
                && state.conmutador_modo == Some(bookos_shell::conmutador::Modo::Ventanas)
                && state.conmutador_salida.is_none()
                && state.exposicion_salida_escritorio.is_none()
            {
                state.arrastre_exposicion = Some(crate::state::ArrastreExposicion {
                    indice: i,
                    origen: state.pointer_location,
                    actual: state.pointer_location,
                    moviendo: false,
                    zona_split: None,
                });
                state.needs_redraw = true;
                return;
            }
            state.conmutador_clic = true;
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
        } else {
            state.conmutador_clic = true;
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
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.menu_actividad_en(punto.x, punto.y))
    {
        state.needs_redraw = true;
        return;
    }
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
        && {
            // modificado en la 0.2.3 -- el menú lista las ventanas de la app.
            let foco = state.ventana_con_foco();
            let ventanas: Vec<(String, String, bool)> = state
                .space
                .elements()
                .filter_map(|w| {
                    let app_id = crate::handlers::app_id(w)?;
                    let titulo = crate::handlers::titulo(w).unwrap_or_else(|| app_id.clone());
                    Some((app_id, titulo, foco.as_ref() == Some(w)))
                })
                .collect();
            state
                .shell
                .as_mut()
                .is_some_and(|s| s.menu_dock_en(punto.x, punto.y, &ventanas))
        }
    {
        state.needs_redraw = true;
        return;
    }
    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && let Some(accion) = state
            .shell
            .as_mut()
            .and_then(|s| s.despedir_asistente_fuera(punto.x, punto.y))
    {
        crate::keybinds::hacer(state, accion);
    } else if pulsado == ButtonState::Pressed
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.recoger_actividad_fuera(punto.x, punto.y))
    {
        state.needs_redraw = true;
    }
    // Clic derecho en otro sitio del escritorio con su menú ya abierto: el menú
    // se muda ahí. Sin esto el clic se gastaba en cerrarlo y había que pulsar
    // dos veces para ver el menú donde se quería.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && state.space.element_under(punto).is_none()
        && state.shell.as_ref().is_some_and(|s| {
            s.emergente_nombre() == Some("menu-dock") && !s.emergente_contiene(punto.x, punto.y)
        })
        && pulsar_derecho_escritorio(state, punto)
    {
        state.menu_ventana_suelta = true;
        return;
    }
    if pulsado == ButtonState::Pressed
        && state.shell.as_mut().is_some_and(|s| s.soltar_foco_barra())
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
            if button == BTN_LEFT
                && state
                    .shell
                    .as_ref()
                    .is_some_and(|s| s.emergente_tiene_alarma())
            {
                crate::backend::schedule_panel_tick(state);
            }
            if state.shell.as_mut().is_some_and(|s| s.programar_mantener_isla()) {
                use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
                let programado = state.loop_handle.insert_source(
                    Timer::from_duration(crate::shell::MANTENER_ISLA),
                    |_, _, state| {
                        if state.shell.as_mut().is_some_and(|s| s.menu_si_sigue_mantenida()) {
                            state.needs_redraw = true;
                        }
                        TimeoutAction::Drop
                    },
                );
                if let Err(err) = programado {
                    tracing::warn!("no se pudo contar la pulsación larga de la isla: {err}");
                }
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
            Some(boton) => {
                state.ultimo_clic_barra = None;
                crate::decoracion::pulsar(state, &window, boton);
            }
            // Fuera de los botones, la barra es para agarrar la
            // ventana; y dos clics seguidos, para maximizarla.
            None if doble_clic_en_barra(state, &window) => {
                state.alternar_maximizada(&window);
            }
            None => {
                state.arrastrar_ventana(&window, crate::ventanas::Modo::Mover, false);
            }
        }
        state.needs_redraw = true;
        return;
    }
    if pulsado == ButtonState::Pressed && button == BTN_LEFT {
        state.ultimo_clic_barra = None;
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
// modified 0.1.1 -- con el cuentagotas, el puntero no llega a los clientes y lleva la cruz
// modificado en la 0.2.0 -- la cruz deja paso a la lupa
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

    // añadido en la 0.2.3 -- la barra de la música del bloqueo se arrastra y
    // engorda bajo el cursor; el shell solo repinta si cambia.
    if let Some(shell) = state.shell.as_mut()
        && shell.esta_bloqueado()
    {
        shell.bloqueo_mover(location.x, location.y);
    }

    if crate::multitarea_tactil::mover(state, None, location, time) {
        return;
    }

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
        if state.arrastre_exposicion.is_some() {
            let area = state.work_area();
            if let Some(arr) = state.arrastre_exposicion.as_mut() {
                arr.actual = location;
                if !arr.moviendo
                    && (location.x - arr.origen.x).hypot(location.y - arr.origen.y) > 12.0
                {
                    arr.moviendo = true;
                }
                if arr.moviendo {
                    arr.zona_split =
                        if location.x <= area.loc.x as f64 + area.size.w as f64 * 0.22 {
                            Some(crate::ventanas::Zona::Izquierda)
                        } else if location.x >= area.loc.x as f64 + area.size.w as f64 * 0.78 {
                            Some(crate::ventanas::Zona::Derecha)
                        } else {
                            None
                        };
                }
            }
            state.needs_redraw = true;
            return;
        }
        let celda = crate::keybinds::celda_bajo(state, location.x, location.y);
        // añadido en la 0.2.0 -- con la vuelta de la exposición en marcha la
        // elegida ya está decidida: seguir al cursor la cambiaba y, al reposar
        // el muelle, `conmutar_fin` enfocaba otra ventana (el «pop»).
        if state.conmutador_salida.is_none()
            && let (Some(i), Some(shell)) = (celda, state.shell.as_mut())
        {
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

    // Sin foco, como con el conmutador: la ventana de debajo no puede poner
    // su cursor encima de la cruz.
    if state.cuentagotas {
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
        state.cursor_status = smithay::input::pointer::CursorImageStatus::Hidden;
        crate::cuentagotas::muestrear(state);
        state.needs_redraw = true;
        return;
    }

    // Con la capa de captura abierta el ratón es suyo entero: se está
    // encuadrando, y ni el dock ni la ventana de debajo tienen nada que decir.
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        let ventana = state
            .space
            .element_under(location)
            .and_then(|(w, _)| state.space.element_geometry(&w))
            .map(|g| bookos_shell::captura::Recuadro {
                x: g.loc.x as f32,
                y: g.loc.y as f32,
                w: g.size.w as f32,
                h: g.size.h as f32,
            });
        if let Some(shell) = state.shell.as_mut() {
            shell.captura_establecer_ventana(ventana);
            shell.captura_puntero(location.x, location.y);
        }
        cursor_de_captura(state);
        state.needs_redraw = true;
        return;
    }
    if state
        .shell
        .as_mut()
        .is_some_and(|s| s.grabacion_pill_puntero(location.x, location.y))
    {
        state.needs_redraw = true;
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
    // añadido en la 0.2.2 -- el aviso se despliega con el puntero encima;
    // al dejarlo cambia su caducidad y hay que despertar para su salida.
    if state
        .shell
        .as_mut()
        .is_some_and(|s| s.arrastrar_aviso(location.x, location.y))
    {
        state.needs_redraw = true;
    }
    if state
        .shell
        .as_mut()
        .is_some_and(|s| s.toast_puntero(location.x, location.y))
    {
        state.needs_redraw = true;
        crate::notificaciones::despertar_para_la_salida(state);
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

// 0.1.0
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

fn mismo_doble_clic(
    previo: Option<(std::time::Instant, Point<f64, Logical>)>,
    ahora: std::time::Instant,
    punto: Point<f64, Logical>,
) -> bool {
    previo.is_some_and(|(t, p)| {
        ahora.duration_since(t) <= DOBLE_CLIC
            && (p.x - punto.x).abs() <= DOBLE_CLIC_RADIO
            && (p.y - punto.y).abs() <= DOBLE_CLIC_RADIO
    })
}

fn mismo_doble_clic_barra(
    previo: Option<(std::time::Instant, Point<f64, Logical>, u64)>,
    ahora: std::time::Instant,
    punto: Point<f64, Logical>,
    ventana_id: u64,
) -> bool {
    previo.is_some_and(|(t, p, id)| {
        id == ventana_id && mismo_doble_clic(Some((t, p)), ahora, punto)
    })
}

/// ¿Este clic llega lo bastante seguido y lo bastante cerca del anterior?
///
/// Apunta el clic actual pase lo que pase: dos clics lentos no son un doble,
/// pero el segundo sí puede ser el primero de otro par.
// 0.1.0
fn es_doble_clic(state: &mut BookosComp) -> bool {
    let ahora = std::time::Instant::now();
    let punto = state.pointer_location;
    let previo = state.ultimo_clic.replace((ahora, punto));
    mismo_doble_clic(previo, ahora, punto)
}

/// El doble clic sobre **nuestra** barra: exige que ambos clics caigan sobre
/// la barra de la **misma** ventana.
// 0.1.0
fn doble_clic_en_barra(state: &mut BookosComp, window: &smithay::desktop::Window) -> bool {
    let Some(ventana_id) = crate::decoracion::id(window) else {
        state.ultimo_clic_barra = None;
        return false;
    };
    let ahora = std::time::Instant::now();
    let punto = state.pointer_location;
    let previo = state.ultimo_clic_barra.replace((ahora, punto, ventana_id));
    if !mismo_doble_clic_barra(previo, ahora, punto, ventana_id) {
        return false;
    }
    // El tercer clic no vuelve a disparar: se olvida el par ya usado.
    state.ultimo_clic_barra = None;
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
// 0.1.0
// modified 0.1.1 -- el escritorio se queda el teclado al pulsarlo
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
// 0.1.0
// modified 0.1.1 -- menú propio del escritorio: abrir, papelera, nueva carpeta, fondo, pantalla y mostrar u ocultar iconos
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
    // Sobre un icono, el menú sale debajo de él, no encima tapando lo que se
    // acaba de seleccionar, y con el aire de cualquier emergente: pegado a la
    // celda realzada parecía parte de ella. En hueco vacío, junto al clic.
    let origen = icono
        .and_then(|i| state.shell.as_ref().and_then(|s| s.escritorio_rect(i)))
        .map(|(rx, ry, _, rh)| (rx, ry + rh + crate::shell::SEPARACION as f32))
        .unwrap_or((punto.x as f32, punto.y as f32));
    let pegable = crate::archivos::hay_archivos(state);
    if let Some(shell) = state.shell.as_mut() {
        shell.menu_escritorio(origen, pegable);
    }
    state.needs_redraw = true;
    true
}

/// Mueve la banda elástica o los iconos agarrados. `true` si el movimiento era
/// del escritorio y no debe llegar a nadie más.
// 0.1.0
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
// 0.1.0
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
// 0.1.0
// modified 0.1.1 -- los menús X11 no roban el foco a su aplicación
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
// 0.1.0
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
// 0.1.0
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

// añadido en la 0.2.0 -- el destino previo sobrevive al apoyo para poder cancelar una inversión.
fn agarrar_exposicion(state: &mut BookosComp) {
    if !state.conmutador_pegado {
        state.exposicion_gesto = Some(crate::gestos::Agarre::nuevo(0.0, 0.0));
        return;
    }
    if let Some(muelle) = state.conmutador_entrada.as_mut() {
        state.exposicion_gesto = Some(crate::gestos::Agarre::nuevo(
            muelle.valor(),
            muelle.destino(),
        ));
        muelle.fijar(muelle.valor());
        state.conmutador_salida = None;
    }
}

// modificado en la 0.2.0 -- el exceso en un extremo no se acumula como recorrido invisible al invertir.
pub(crate) fn seguir_exposicion(state: &mut BookosComp, progreso: f32) {
    if state.exposicion_gesto.is_none() {
        agarrar_exposicion(state);
    }
    let Some(agarre) = state.exposicion_gesto.as_mut() else {
        return;
    };
    let posicion = agarre.seguir(progreso, 0.0, 1.0);
    if !state.conmutador_pegado {
        if posicion == 0.0 {
            return;
        }
        let agarre = *agarre;
        exponer(state);
        if !state.conmutador_pegado {
            return;
        }
        state.exposicion_gesto = Some(agarre);
        state.conmutador_salida = None;
    }
    if let Some(muelle) = state.conmutador_entrada.as_mut() {
        muelle.fijar(posicion);
    }
    state.needs_redraw = true;
}

/// Versión 2D táctil: además de subir y reducir la app 1:1, acompaña el dedo en X.
pub(crate) fn seguir_exposicion_con_x(
    state: &mut BookosComp,
    progreso: f32,
    desplazamiento_x: f32,
) {
    state
        .exposicion_desplazamiento_x
        .fijar(desplazamiento_x.clamp(-1.0, 1.0));
    seguir_exposicion(state, progreso);
}

// añadido en la 0.2.0
pub(crate) fn soltar_exposicion(state: &mut BookosComp, suelta: crate::gestos::Suelta) {
    state.exposicion_desplazamiento_x.ir_a(0.0);
    let Some(agarre) = state.exposicion_gesto.take() else {
        return;
    };
    let Some(muelle) = state.conmutador_entrada.as_mut() else {
        return;
    };
    let abrir = agarre.destino(suelta) > 0.0;
    muelle.soltar_gesto(
        if abrir { 1.0 } else { 0.0 },
        if suelta.cancelada {
            0.0
        } else {
            suelta.velocidad
        },
    );
    if !abrir {
        state.conmutador_salida = Some(crate::keybinds::Accion::ConmutarCancelar);
    }
    state.needs_redraw = true;
}

/// Segundo gesto de abajo hacia arriba con la exposición en medio: las tarjetas
/// despegan hacia arriba siguiendo el dedo antes de despejar al escritorio.
pub(crate) fn seguir_exposicion_escritorio(
    state: &mut BookosComp,
    progreso: f32,
    desplazamiento_x: f32,
) {
    state.arrastre_exposicion = None;
    state.exposicion_escritorio_sujeto = true;
    state
        .exposicion_desplazamiento_x
        .fijar(desplazamiento_x.clamp(-1.0, 1.0));
    let muelle = state
        .exposicion_salida_escritorio
        .get_or_insert_with(|| bookos_shell::tema::Muelle::nuevo(0.0, bookos_shell::tema::M_SUAVE));
    muelle.fijar(progreso.clamp(0.0, 1.0));
    state.needs_redraw = true;
}

pub(crate) fn soltar_exposicion_escritorio(state: &mut BookosComp, suelta: crate::gestos::Suelta) {
    state.exposicion_escritorio_sujeto = false;
    state.exposicion_desplazamiento_x.ir_a(0.0);
    let llega = !suelta.cancelada
        && bookos_shell::tema::proyectar_impulso(suelta.progreso, suelta.velocidad) >= 0.35;
    let muelle = state
        .exposicion_salida_escritorio
        .get_or_insert_with(|| bookos_shell::tema::Muelle::nuevo(0.0, bookos_shell::tema::M_SUAVE));
    muelle.soltar_gesto(
        if llega { 1.0 } else { 0.0 },
        if suelta.cancelada {
            0.0
        } else {
            suelta.velocidad
        },
    );
    state.needs_redraw = true;
}

/// Tres dedos hacia arriba: el conmutador de ventanas, abierto hasta elegir.
/// Aparte porque el guion de los autotests la llama igual: en el anidado no
/// llegan gestos.
// 0.1.0
// modified 0.1.1 -- apunta cuándo empezó la exposición para animar la entrada
pub fn exponer(state: &mut BookosComp) {
    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        return;
    }
    // añadido en la 0.2.0 -- la tarjeta de un widget (batería, red…) es modal
    // y se quedaba abierta debajo de la exposición, que no la cerraba nadie.
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
    }
    // Con el escritorio despejado no hay ventanas mapeadas que exponer: se
    // devuelven primero, que es lo que pide quien quiere verlas.
    crate::escritorios::recuperar(state);
    crate::keybinds::ejecutar(
        state,
        crate::keybinds::Accion::Conmutar(bookos_shell::conmutador::Modo::Ventanas, 0),
    );
    state.conmutador_pegado = state.shell.as_ref().is_some_and(|s| s.hay_conmutador());
    state.conmutador_entrada = state.conmutador_pegado.then(|| {
        let mut muelle = bookos_shell::tema::Muelle::nuevo(0.0, bookos_shell::tema::M_SUAVE);
        muelle.ir_a(1.0);
        muelle
    });
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

    #[test]
    fn arista_tactil_acota_bordes_rectos_y_amplia_esquina_inferior() {
        let rect = smithay::utils::Rectangle::new((100.0, 100.0).into(), (400.0, 300.0).into());
        // Lejos por fuera en arista recta (>10 px): no roba toques del escritorio o barras.
        assert!(arista_tactil(rect, (84.0, 250.0).into()).is_none());
        assert!(arista_tactil(rect, (300.0, 84.0).into()).is_none());
        // Dentro del margen de 10 px por fuera en arista izquierda: solo eje horizontal.
        assert_eq!(
            arista_tactil(rect, (93.0, 250.0).into()),
            Some(crate::ventanas::Modo::RedimensionarBorde {
                horizontal: Some(true),
                vertical: None,
            })
        );
        // Dentro del cuerpo de la ventana: ninguno.
        assert!(arista_tactil(rect, (250.0, 200.0).into()).is_none());
        // Esquina inferior derecha con 44 pt de agarre (−22..+22).
        assert_eq!(
            arista_tactil(rect, (515.0, 415.0).into()),
            Some(crate::ventanas::Modo::RedimensionarBorde {
                horizontal: Some(false),
                vertical: Some(false),
            })
        );
    }

    #[test]
    fn doble_clic_en_barra_exige_la_misma_ventana() {
        let t0 = std::time::Instant::now();
        let t1 = t0 + std::time::Duration::from_millis(120);
        let p = Point::from((400.0, 60.0));
        assert!(!mismo_doble_clic_barra(Some((t0, p, 1)), t1, p, 2));
        assert!(mismo_doble_clic_barra(Some((t0, p, 1)), t1, p, 1));
    }
}
