//! Atajos que atiende el compositor antes que nadie.
//!
//! Hasta ahora `input.rs` reenviaba **todas** las teclas al cliente con foco.
//! Eso deja la sesión en un callejón sin salida: sin cambio de TTY no se puede
//! volver a un terminal de texto ni a la sesión de KDE, y sin forma de lanzar
//! nada, cerrar la última ventana te deja delante de un escritorio que no
//! responde. Las dos cosas se arreglan aquí, en el único sitio que ve las
//! teclas antes de que salgan hacia un cliente.
//!
//! Criterio de qué entra: solo lo que **no se puede hacer de otra manera** una
//! vez el compositor tiene el KMS. El resto de atajos son cosa del escritorio y
//! llegarán con la configuración, no cableados en el compositor.

use smithay::desktop::Window;
use smithay::input::keyboard::{KeysymHandle, ModifiersState, keysyms};
use smithay::utils::{IsAlive, SERIAL_COUNTER};

use crate::state::BookosComp;

/// Terminal que abre Meta+Return si `BOOKOS_TERMINAL` no dice otra cosa: la
/// propia de BookOS, no la de KDE.
const TERMINAL: &str = "bookos-shell";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    MenuVentana,
    SiempreEncima,
    /// Ctrl+Alt+F1..F12 — saltar a otro terminal virtual.
    CambiarVt(i32),
    /// Meta+Return — abrir un terminal dentro de la sesión.
    Terminal,
    /// Meta+E — abrir el explorador de archivos de BookOS.
    Explorador,
    /// Meta+Q — cerrar la ventana con foco (petición educada al cliente).
    CerrarVentana,
    /// Meta+Alt+Retroceso — terminar el compositor y devolver el TTY.
    Salir,
    /// Meta+Espacio — abrir o cerrar el launchpad.
    Launchpad,
    /// El buscador de Meta+Espacio: aplicaciones, comandos y estados.
    Buscador,
    Emojis,
    /// Meta+W — vista general y gestión de escritorios.
    VistaEscritorios,
    /// Meta+F — maximizar la ventana con foco, o devolverla a su sitio.
    Maximizar,
    /// Meta+H — mandar la ventana con foco al dock. Vuelve pulsando su icono.
    Minimizar,
    /// Meta+Alt+B y Meta+Alt+D — alternar si la barra se aparta de las ventanas
    /// o está siempre a la vista.
    AlternarBarra(crate::shell::Barra),
    /// Impr, o Meta+Mayús+S — abrir la capa de captura de pantalla.
    Captura,
    /// Meta+L — echar la pantalla de bloqueo.
    Bloquear,
    ApagarPantalla,
    Salvapantallas,
    /// El estilo que no está configurado, para probarlo sin editar nada.
    SalvapantallasOtro,
    /// Quitar el bloqueo. La pide el ayudante de PAM cuando la contraseña vale.
    Desbloquear,
    /// Apagar, reiniciar o suspender desde el menú de la pantalla de bloqueo.
    Energia(bookos_shell::bloqueo::Peticion),
    /// Un carácter más en la contraseña del bloqueo.
    BloqueoEscribir(char),
    BloqueoBorrar,
    BloqueoLimpiar,
    BloqueoComprobar,
    BloqueoHuella,
    /// Las teclas de función del portátil: volumen, brillo, touchpad.
    Multimedia(crate::multimedia::Tecla),
    /// Meta+flechas — encajar la ventana con foco, como en KDE: la primera
    /// pulsación la manda a media pantalla y la siguiente, en perpendicular, a
    /// un cuarto. Es la única forma de llegar a un cuarto sin apuntar a una
    /// esquina con el ratón.
    Encajar(crate::ventanas::Direccion),
    /// Alt+Tab abre aplicaciones; Meta+Tab, ventanas con miniaturas. El entero
    /// es el sentido: `-1` con Mayús.
    Conmutar(bookos_shell::conmutador::Modo, i32),
    /// Soltar el modificador: se va a la aplicación elegida.
    ConmutarFin,
    /// Escape con el conmutador abierto: se cierra sin ir a ninguna parte.
    ConmutarCancelar,
    /// Ir a un escritorio concreto: Meta+1..5.
    Escritorio(usize),
    /// Moverse uno a la izquierda o a la derecha: Meta+Ctrl+flechas y el gesto
    /// de cuatro dedos a los lados. Va aparte de [`Accion::Escritorio`] porque
    /// el destino depende de dónde estés, y eso no se sabe al resolver la tecla.
    EscritorioRelativo(i32),
    /// Meta+Ctrl+Mayús+flechas: enviar la ventana con foco al escritorio
    /// vecino sin cambiar la vista.
    MoverEscritorioRelativo(i32),
    /// Meta+Ctrl+D y el gesto de cuatro dedos abajo: apartar todas las ventanas
    /// para ver el escritorio, o devolverlas.
    MostrarEscritorio,
    /// Lo que ha pedido una superficie emergente del shell. Llega por aquí y no
    /// se ejecuta en el sitio porque se decide dentro del filtro de
    /// `kbd.input`, con el estado del compositor ya prestado.
    DelShell(bookos_shell::Accion),
    /// El diálogo del botón de encendido: dormir, bloquear, salir, reiniciar,
    /// apagar.
    DialogoEnergia,
    /// Meta+Z — el menú del logo de BookOS (Acerca de, Ajustes…).
    MenuBookos,
    /// Meta+X — el centro de control del panel.
    CentroControl,
    /// Fn+F4 / XF86Display — selector rápido de proyección.
    Proyeccion,
    /// Meta+Alt+F — poner o quitar el panel de diagnóstico de fotogramas.
    Diagnostico,
    /// Abrir la isla de actividades si está compacta, o recogerla. No tiene
    /// atajo propio: llega desde un remapeo de `teclas.conf`.
    Actividades,
    /// Meta+Alt+A, o una tecla remapeada, activa la escucha del asistente.
    /// El proceso mantiene fuera del compositor el audio y la inferencia.
    Asistente,
    /// El comando de un remapeo de `teclas.conf`.
    Lanzar(String),
}

/// El conmutador de aplicaciones: `Alt+Tab` y `Meta+Tab`, con Mayús al revés.
///
/// Está aparte de [`resolver`] para poder probarlo: `resolver` necesita un
/// `KeysymHandle`, que envuelve el estado de xkb y no se construye en un test.
/// Sin esto, "el atajo no responde" y "la tecla no llega al compositor" son
/// indistinguibles desde fuera — y anidado dentro de Plasma pasa lo segundo, que
/// es lo que hay que poder descartar.
fn conmutador_de(sym: u32, modifiers: &ModifiersState) -> Option<Accion> {
    if !matches!(sym, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab) {
        return None;
    }
    // Con Ctrl no: `Ctrl+Alt+Tab` es de otra cosa en varios escritorios y no
    // conviene pisarlo.
    if modifiers.ctrl || modifiers.alt == modifiers.logo {
        return None;
    }
    // Con Mayús se recorre al revés, como en todas partes. Se mira el
    // modificador y no el keysym porque xkb ya convierte el tabulador en
    // `ISO_Left_Tab` al entrar Mayús, y los dos llegan aquí.
    let modo = if modifiers.alt {
        bookos_shell::conmutador::Modo::Aplicaciones
    } else {
        bookos_shell::conmutador::Modo::Ventanas
    };
    Some(Accion::Conmutar(modo, if modifiers.shift { -1 } else { 1 }))
}

/// Teclas que el firmware puede usar para el selector de pantallas.
///
/// La mayoría de portátiles emite `KEY_SWITCHVIDEOMODE` (227), que XKB
/// convierte en `XF86Display`. Otros emiten el más reciente
/// `KEY_DISPLAYTOGGLE` (431). Miramos también el código físico XKB (evdev + 8)
/// porque un mapa de teclado sin las reglas `inet` deja ambos como `NoSymbol`.
fn es_tecla_de_pantalla(sym: u32, codigo_xkb: u32) -> bool {
    const XF86_DISPLAY_TOGGLE: u32 = 0x1008_11AF;
    const SWITCH_VIDEO_MODE_XKB: u32 = 227 + 8;
    const DISPLAY_TOGGLE_XKB: u32 = 431 + 8;

    matches!(sym, keysyms::KEY_XF86Display | XF86_DISPLAY_TOGGLE)
        || matches!(codigo_xkb, SWITCH_VIDEO_MODE_XKB | DISPLAY_TOGGLE_XKB)
}

/// Abre el conmutador o avanza en él.
///
/// Alt toma la ventana más reciente de cada aplicación; Meta conserva todas
/// las ventanas. Por eso dos terminales no hacen nada con Alt —solo hay una
/// aplicación—, pero aparecen separadas y con título propio en Meta.
///
/// Solo entran las ventanas **de este escritorio**: las de los demás están
/// desmapeadas, y saltar a una cambiaría de escritorio sin avisar, que no es lo
/// que espera quien abre cualquiera de los dos selectores.
fn conmutar(state: &mut BookosComp, modo: bookos_shell::conmutador::Modo, sentido: i32) {
    // La vista de escritorios ya es un selector de ventanas, y modal: abrir
    // otro encima dejaba dos a la vez, cada uno con su idea de cuál es la
    // ventana elegida.
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.vista_escritorios_abierta())
    {
        return;
    }
    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        if state.conmutador_modo != Some(modo) {
            state.conmutador_destinos.clear();
            state.conmutador_modo = None;
            if let Some(shell) = state.shell.as_mut() {
                shell.cancelar_conmutador();
            }
        } else {
            if let Some(shell) = state.shell.as_mut() {
                shell.conmutador_mover(sentido);
            }
            state.needs_redraw = true;
            return;
        }
    }

    let ventanas: Vec<Window> = state
        .space
        .elements()
        .filter(|w| w.alive() && !w.x11_surface().is_some_and(|x| x.is_override_redirect()))
        .filter(|w| !state.minimizando.iter().any(|(m, _)| m == *w))
        .cloned()
        .collect();
    let sellos: Vec<_> = ventanas.iter().map(crate::ventanas::enfocada_en).collect();
    let app_ids: Vec<_> = ventanas
        .iter()
        .map(|w| crate::handlers::app_id(w).unwrap_or_default())
        .collect();
    let maximo = match modo {
        bookos_shell::conmutador::Modo::Aplicaciones => bookos_shell::conmutador::MAXIMO,
        bookos_shell::conmutador::Modo::Ventanas => usize::MAX,
    };
    let mut orden = Vec::new();
    for i in crate::conmutador::orden(&sellos, usize::MAX) {
        if !orden
            .iter()
            .any(|j: &usize| crate::multitarea::misma_unidad(state, &ventanas[i], &ventanas[*j]))
        {
            orden.push(i);
        }
    }
    orden.truncate(maximo);

    let entradas: Vec<_> = orden
        .iter()
        .map(|i| {
            let window = &ventanas[*i];
            let app_id = &app_ids[*i];
            // El título también en Alt: es lo único que distingue dos ventanas
            // de la misma aplicación, que ahora tienen celda propia.
            let titulo = crate::handlers::titulo(window).unwrap_or_default();
            bookos_shell::entrada_de_ventana(app_id, &titulo)
        })
        .collect();
    state.conmutador_destinos = orden.iter().map(|i| ventanas[*i].clone()).collect();

    let abierto = state
        .shell
        .as_mut()
        .is_some_and(|s| s.abrir_conmutador(modo, entradas));
    if !abierto {
        // Nunca en silencio: que este atajo fallara sin decir nada es lo que
        // hizo que diagnosticarlo costara una tarde.
        tracing::info!(
            ventanas = ventanas.len(),
            celdas = orden.len(),
            hay_shell = state.shell.is_some(),
            "el conmutador no se abre"
        );
        state.conmutador_destinos.clear();
        state.conmutador_modo = None;
        return;
    }
    state.conmutador_modo = Some(modo);
    // El modelo nace señalando la ventana actual (índice 0); esta primera
    // pulsación aplica el mismo paso que las repeticiones. Hacia atrás cae así
    // directamente en la última celda.
    if let Some(shell) = state.shell.as_mut() {
        shell.conmutador_mover(sentido);
    }
    tracing::debug!(celdas = orden.len(), "conmutador abierto");
    state.needs_redraw = true;
}

/// Cierra la exposición de tres dedos con el viaje de vuelta y deja la acción
/// para cuando las ventanas han llegado. `true` si se encarga ella.
///
/// Solo la exposición: Alt+Tab se resuelve al soltar la tecla y esperar 260 ms
/// para cambiar de ventana se notaría como retraso, no como animación.
fn salir_de_la_exposicion(state: &mut BookosComp, accion: &Accion) -> bool {
    // Ya saliendo, una segunda orden —Esc con el gesto, un clic— no hace nada:
    // el temporizador ya va a resolver la primera.
    if state.conmutador_salida.is_some() {
        return true;
    }
    if !state.conmutador_pegado
        || state.conmutador_modo != Some(bookos_shell::conmutador::Modo::Ventanas)
        || bookos_shell::tema::efectos_reducidos()
        || !state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
    {
        return false;
    }
    state.conmutador_salida = Some(std::time::Instant::now());
    state.needs_redraw = true;
    let accion = accion.clone();
    state
        .loop_handle
        .insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(bookos_shell::tema::D_VENTANA),
            move |_, _, state| {
                state.conmutador_salida = None;
                state.conmutador_pegado = false;
                ejecutar(state, accion.clone());
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        )
        .expect("temporizador de salida de la exposición insertado");
    true
}

/// Se ha soltado el modificador: a la ventana elegida.
fn conmutar_fin(state: &mut BookosComp) {
    state.conmutador_pegado = false;
    let elegida = state.shell.as_mut().and_then(|s| s.cerrar_conmutador());
    let destinos = std::mem::take(&mut state.conmutador_destinos);
    state.conmutador_modo = None;
    // Repintar pase lo que pase: la tarjeta ya no está y alguien tiene que
    // borrarla de la pantalla, se haya elegido algo o no.
    state.needs_redraw = true;
    let Some(window) = elegida.and_then(|i| destinos.get(i).cloned()) else {
        return;
    };
    // La ventana pudo cerrarse con el conmutador abierto: el `Window` clonado
    // sigue vivo aquí aunque su cliente se haya ido, así que enfocarlo sin
    // comprobarlo daría el foco a un fantasma. Antes esto no hacía falta porque
    // se rebuscaba en el `Space`, que ya solo tiene ventanas vivas.
    if !window.alive() {
        tracing::warn!("la ventana elegida se cerró mientras se elegía");
        return;
    }
    state.enfocar(&window);
}

/// Decide si una tecla es un atajo del compositor.
///
/// Se llama **solo con la pulsación**, no con la suelta: interceptar también la
/// suelta dejaría al cliente con la tecla marcada como pulsada para siempre.
pub fn resolver(
    modifiers: &ModifiersState,
    handle: &KeysymHandle<'_>,
    codigo_xkb: u32,
) -> Option<Accion> {
    let sym = handle.modified_sym().raw();

    // Ctrl+Alt+Fn. Con la configuración de xkb por defecto la propia capa de
    // teclado ya traduce esa combinación a XF86Switch_VT_n, así que no hay que
    // mirar los modificadores: si llega este keysym, es que la quería.
    const VT1: u32 = keysyms::KEY_XF86Switch_VT_1;
    const VT12: u32 = keysyms::KEY_XF86Switch_VT_12;
    if (VT1..=VT12).contains(&sym) {
        return Some(Accion::CambiarVt((sym - VT1 + 1) as i32));
    }

    // Red de seguridad para distribuciones de teclado que no traen la regla
    // `srvr_ctrl` y por tanto nunca emiten XF86Switch_VT_n. Ahí Ctrl+Alt+F3
    // llega como una F3 normal y hay que reconocerla a mano, mirando el símbolo
    // **sin** modificadores: con Ctrl+Alt aplicados podría no ser ya una F.
    if modifiers.ctrl && modifiers.alt {
        let base = handle.raw_syms().first().map(|s| s.raw()).unwrap_or(sym);
        const F1: u32 = keysyms::KEY_F1;
        const F12: u32 = keysyms::KEY_F12;
        if (F1..=F12).contains(&base) {
            return Some(Accion::CambiarVt((base - F1 + 1) as i32));
        }
        // Ctrl+Alt+T, el atajo de terminal de GNOME y de Plasma. Va aquí y no
        // en el brazo de Meta porque es el gesto que traen los dedos de quien
        // viene de otro escritorio.
        if base == keysyms::KEY_t || base == keysyms::KEY_T {
            return Some(Accion::Terminal);
        }
        if base == keysyms::KEY_Delete {
            return Some(Accion::DelShell(bookos_shell::Accion::CerrarSesion));
        }
    }

    // El botón de encendido y `Meta+Esc`, los dos al mismo diálogo. El segundo
    // existe porque en un sobremesa el botón está en la caja, debajo de la
    // mesa, y porque en un portátil puede que logind se quede la tecla antes
    // de que llegue aquí (`HandlePowerKey`).
    if sym == XF86_ENCENDIDO || (modifiers.logo && sym == keysyms::KEY_Escape) {
        return Some(Accion::DialogoEnergia);
    }
    if es_tecla_de_pantalla(sym, codigo_xkb)
        || handle
            .raw_syms()
            .iter()
            .any(|raw| es_tecla_de_pantalla(raw.raw(), codigo_xkb))
    {
        return Some(Accion::Proyeccion);
    }

    // Las teclas de función no llevan modificador y valen aunque el foco lo
    // tenga una aplicación a pantalla completa: subir el volumen en un vídeo
    // tiene que funcionar sin salir de él.
    if let Some(tecla) = crate::multimedia::resolver(sym) {
        return Some(Accion::Multimedia(tecla));
    }

    // Alt+Tab, el de toda la vida. Estaba solo en Meta+Tab y eso es pedirle al
    // usuario que desaprenda el atajo que usa desde hace veinte años: aquí lo
    // que hay que teclear tiene que ser lo que ya sabe.
    //
    // Con Alt pulsada, xkb entrega el tabulador como ISO_Left_Tab en cuanto
    // entra Mayús, así que se aceptan los dos. Va **antes** que el brazo de
    // Ctrl+Alt para que no lo intercepte el cambio de VT.
    if let Some(accion) = conmutador_de(sym, modifiers) {
        return Some(accion);
    }

    // Meta+Ctrl, para los escritorios: va antes que Meta a secas por lo mismo
    // que Meta+Alt —si no, `Meta+Ctrl+Izquierda` encajaría la ventana a medio
    // lado en vez de cambiar de escritorio—. Son los atajos de KDE.
    if modifiers.logo && modifiers.ctrl {
        match sym {
            keysyms::KEY_Left if modifiers.shift => {
                return Some(Accion::MoverEscritorioRelativo(-1));
            }
            keysyms::KEY_Right if modifiers.shift => {
                return Some(Accion::MoverEscritorioRelativo(1));
            }
            keysyms::KEY_Left => return Some(Accion::EscritorioRelativo(-1)),
            keysyms::KEY_Right => return Some(Accion::EscritorioRelativo(1)),
            keysyms::KEY_d | keysyms::KEY_D => return Some(Accion::MostrarEscritorio),
            _ => {}
        }
    }

    // Impr, en sus varias formas. Va fuera de los brazos de Meta porque no
    // lleva ninguno.
    //
    // Son tres keysyms y no uno porque **Fn no llega hasta aquí**: la resuelve
    // el firmware del teclado, así que `Fn+Impr` puede aparecer como `Print` a
    // secas —lo normal en un portátil donde Impr comparte tecla— o como
    // `Sys_Req`, que es lo que da xkb cuando la combinación lleva Alt. El
    // tercero es el de los terminales 3270 y no cuesta nada.
    //
    // Si en alguna máquina Fn+Impr manda otra cosa, la traza de más abajo lo
    // dice: no hay forma de saberlo desde el código y adivinar no vale.
    if matches!(
        sym,
        keysyms::KEY_Print | keysyms::KEY_Sys_Req | keysyms::KEY_3270_PrintScreen
    ) {
        return Some(Accion::Captura);
    }
    // Las teclas sin nombre imprimible que no ha reclamado nadie: se apuntan a
    // nivel de traza para poder averiguar qué manda una tecla rara —Fn+algo—
    // sin tener que instalar herramientas.
    if sym >= 0xff00 && !modifiers.logo && !modifiers.ctrl && !modifiers.alt {
        tracing::trace!(
            keysym = format_args!("{sym:#06x}"),
            "tecla especial sin atajo"
        );
    }
    // Y Meta+Mayús+S, que es lo que tienen en los dedos los que vienen de
    // Windows o de GNOME. Antes que el brazo de Meta a secas por lo mismo que
    // Meta+Alt: si no, entraría por él y no llegaría aquí.
    // Con Alt también, el salvapantallas en el otro estilo. Va antes que la
    // captura: Meta+Mayús+S entraría también con Alt pulsado.
    if modifiers.logo
        && modifiers.shift
        && modifiers.alt
        && matches!(sym, keysyms::KEY_s | keysyms::KEY_S)
    {
        return Some(Accion::SalvapantallasOtro);
    }
    if modifiers.logo && modifiers.shift && matches!(sym, keysyms::KEY_s | keysyms::KEY_S) {
        return Some(Accion::Captura);
    }

    // Meta+Alt va antes que Meta a secas: si no, `Meta+Alt+D` entraría por el
    // brazo de Meta y no llegaría nunca aquí.
    if modifiers.logo && modifiers.alt {
        // Salida de emergencia. Antes era Ctrl+Alt+Retroceso, pero Ctrl+Retroceso
        // es «borrar la palabra» en cualquier campo y con Alt rozado se cerraba
        // la sesión. Se mira el símbolo sin modificadores por lo mismo que arriba.
        if handle.raw_syms().first().map(|s| s.raw()).unwrap_or(sym) == keysyms::KEY_BackSpace {
            return Some(Accion::Salir);
        }
        match sym {
            keysyms::KEY_o | keysyms::KEY_O => return Some(Accion::ApagarPantalla),
            keysyms::KEY_s | keysyms::KEY_S => return Some(Accion::Salvapantallas),
            keysyms::KEY_a | keysyms::KEY_A => return Some(Accion::Asistente),
            keysyms::KEY_t | keysyms::KEY_T => return Some(Accion::SiempreEncima),
            keysyms::KEY_b | keysyms::KEY_B => {
                return Some(Accion::AlternarBarra(crate::shell::Barra::Panel));
            }
            keysyms::KEY_d | keysyms::KEY_D => {
                return Some(Accion::AlternarBarra(crate::shell::Barra::Dock));
            }
            // F de fotogramas. Va en el brazo de Meta+Alt, así que no le quita
            // nada a Meta+F, que sigue maximizando.
            keysyms::KEY_f | keysyms::KEY_F => return Some(Accion::Diagnostico),
            _ => {}
        }
    }

    if modifiers.alt && !modifiers.logo && !modifiers.ctrl && sym == keysyms::KEY_F3 {
        return Some(Accion::MenuVentana);
    }
    if modifiers.logo {
        match sym {
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => return Some(Accion::Terminal),
            keysyms::KEY_q | keysyms::KEY_Q => return Some(Accion::CerrarVentana),
            // Meta+Espacio abre el **buscador**, que es donde lo tiene KRunner
            // y donde lo busca cualquiera que venga de Plasma o de macOS. El
            // launchpad se quedó con Meta sola, que ya funcionaba: son dos
            // gestos distintos para dos cosas distintas —ver todo lo instalado
            // frente a escribir lo que quieres.
            keysyms::KEY_space => return Some(Accion::Buscador),
            keysyms::KEY_period if !modifiers.alt && !modifiers.ctrl && !modifiers.shift => {
                return Some(Accion::Emojis);
            }
            keysyms::KEY_z | keysyms::KEY_Z => return Some(Accion::MenuBookos),
            keysyms::KEY_x | keysyms::KEY_X => return Some(Accion::CentroControl),
            keysyms::KEY_e | keysyms::KEY_E => return Some(Accion::Explorador),
            keysyms::KEY_w | keysyms::KEY_W => return Some(Accion::VistaEscritorios),
            keysyms::KEY_f | keysyms::KEY_F => return Some(Accion::Maximizar),
            // H de *hide*, como en macOS: Meta+M es «minimizar» en Windows pero
            // aquí Meta+M ya no está libre en cuanto haya un menú.
            keysyms::KEY_h | keysyms::KEY_H => return Some(Accion::Minimizar),
            keysyms::KEY_l | keysyms::KEY_L => {
                return Some(Accion::DelShell(bookos_shell::Accion::Bloquear));
            }
            // Alternativa deliberada para equipos cuyo firmware se queda Fn
            // antes de crear un evento de teclado. No sustituye Fn+F4: permite
            // abrir el mismo selector con un atajo que siempre llega.
            keysyms::KEY_p | keysyms::KEY_P => return Some(Accion::Proyeccion),
            keysyms::KEY_Left => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Izquierda));
            }
            keysyms::KEY_Right => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Derecha));
            }
            keysyms::KEY_Up => return Some(Accion::Encajar(crate::ventanas::Direccion::Arriba)),
            keysyms::KEY_Down => return Some(Accion::Encajar(crate::ventanas::Direccion::Abajo)),
            // Meta+1..5, el atajo directo a cada escritorio. Los keysyms de los
            // dígitos son consecutivos desde KEY_1, así que la cuenta sale sola
            // y no hace falta un brazo por escritorio.
            _ if (keysyms::KEY_1..keysyms::KEY_1 + crate::escritorios::MAXIMO as u32)
                .contains(&sym) =>
            {
                return Some(Accion::Escritorio((sym - keysyms::KEY_1) as usize));
            }
            _ => {}
        }
    }

    None
}

impl Accion {
    /// ¿Es una salida de emergencia? Son las únicas que siguen funcionando con
    /// el bloqueo echado: sin ellas, un fallo del bloqueo deja la máquina
    /// inservible hasta apagarla a lo bruto.
    pub fn es_emergencia(&self) -> bool {
        matches!(self, Accion::CambiarVt(_) | Accion::Salir)
    }
}

pub fn menu_ventana(state: &mut BookosComp, window: Window) {
    crate::escritorios::terminar_todos(state);
    let salida = crate::escritorios::salida_de(state, &window);
    let mut opciones = vec![(
        if crate::ventanas::siempre_encima(&window) {
            "Dejar de mantener encima"
        } else {
            "Siempre encima"
        }
        .into(),
        bookos_shell::Accion::VentanaEncima,
    )];
    for (n, nombre) in state.escritorios.nombres().iter().enumerate() {
        if salida
            .as_ref()
            .is_some_and(|s| state.escritorios.activo_en(s) == n)
        {
            continue;
        }
        opciones.push((
            format!("Mover a {nombre}"),
            bookos_shell::Accion::VentanaEscritorio(n),
        ));
    }
    for output in state.space.outputs() {
        let nombre = output.name();
        if salida.as_ref() == Some(&nombre) {
            continue;
        }
        let visible =
            crate::pantallas::nombre_visible(&nombre, &output.physical_properties().model);
        opciones.push((
            format!("Mover a {visible}"),
            bookos_shell::Accion::VentanaMonitor(nombre),
        ));
    }
    if crate::multitarea::agrupada(state, &window) {
        opciones.push((
            "Ver como pestañas".into(),
            bookos_shell::Accion::VentanaDisposicion(true),
        ));
        if crate::multitarea::admite_dos_paneles(state, &window) {
            opciones.push((
                "Ver lado a lado".into(),
                bookos_shell::Accion::VentanaDisposicion(false),
            ));
        }
        opciones.push(("Separar grupo".into(), bookos_shell::Accion::VentanaSeparar));
    }
    state.menu_ventana_companera = crate::multitarea::candidata(state, &window);
    if let Some(otra) = state.menu_ventana_companera.as_ref() {
        let titulo = crate::handlers::titulo(otra).unwrap_or_else(|| "otra aplicación".into());
        opciones.push((
            format!("Unir lado a lado con «{titulo}»"),
            bookos_shell::Accion::VentanaUnir(false),
        ));
        opciones.push((
            format!("Unir en pestañas con «{titulo}»"),
            bookos_shell::Accion::VentanaUnir(true),
        ));
    }
    state.menu_ventana = Some(window);
    if let Some(shell) = state.shell.as_mut() {
        shell.menu_ventana(opciones);
    }
    state.needs_redraw = true;
}

pub fn ejecutar(state: &mut BookosComp, accion: Accion) {
    match accion {
        Accion::MenuVentana => {
            if let Some(window) = state.ventana_con_foco() {
                menu_ventana(state, window);
            }
        }
        Accion::SiempreEncima => {
            if let Some(window) = state.ventana_con_foco() {
                state.alternar_encima(&window);
            }
        }
        Accion::CambiarVt(vt) => cambiar_vt(state, vt),
        Accion::Terminal => {
            let cmd = std::env::var("BOOKOS_TERMINAL").unwrap_or_else(|_| TERMINAL.to_string());
            lanzar(state, &cmd);
        }
        Accion::Explorador => lanzar(state, "bookos-explorer"),
        Accion::CerrarVentana => cerrar_ventana(state),
        Accion::Salir => {
            // Dos veces en menos de dos segundos. Una sola cerraba la sesión
            // entera sin preguntar, y en un portátil Ctrl y Alt van pegados:
            // al borrar con Ctrl+Retroceso se rozaba Alt y adiós a todo lo
            // abierto. Sigue sin diálogo a propósito: es la salida de
            // emergencia y tiene que funcionar aunque el shell no responda.
            const VENTANA: std::time::Duration = std::time::Duration::from_secs(2);
            if state.salida_armada.is_some_and(|t| t.elapsed() < VENTANA) {
                tracing::info!("salida pedida con Meta+Alt+Retroceso");
                state.loop_signal.stop();
                return;
            }
            state.salida_armada = Some(std::time::Instant::now());
            if let Some(shell) = state.shell.as_mut() {
                shell.mostrar_osd(
                    "salir",
                    None,
                    Some("Pulsa otra vez para cerrar la sesión".into()),
                );
            }
            state.needs_redraw = true;
        }
        Accion::Escritorio(n) => crate::escritorios::cambiar_aqui(state, n),
        Accion::EscritorioRelativo(pasos) => {
            let destino = crate::escritorios::destino(
                crate::escritorios::activo_aqui(state),
                pasos,
                state.escritorios.cuantos(),
            );
            crate::escritorios::cambiar_aqui(state, destino);
        }
        Accion::MoverEscritorioRelativo(pasos) => {
            let Some(window) = state.ventana_con_foco() else {
                return;
            };
            let Some(salida) = crate::escritorios::salida_de(state, &window) else {
                return;
            };
            let destino = crate::escritorios::destino(
                state.escritorios.activo_en(&salida),
                pasos,
                state.escritorios.cuantos(),
            );
            crate::escritorios::mover_ventana(state, &window, destino);
        }
        Accion::MostrarEscritorio => crate::escritorios::alternar_despejado(state),
        Accion::Launchpad => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_launchpad();
            }
            state.needs_redraw = true;
        }
        Accion::Buscador => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_buscador();
            }
            state.needs_redraw = true;
        }
        Accion::Emojis => crate::emojis::alternar(state),
        Accion::VistaEscritorios => alternar_vista_escritorios(state),
        Accion::AlternarBarra(cual) => {
            let modo = state.shell.as_mut().map(|s| s.alternar_visibilidad(cual));
            if let Some(modo) = modo {
                tracing::info!(?cual, ?modo, "visibilidad de la barra");
                let clave = match cual {
                    crate::shell::Barra::Panel => "panel_esquiva",
                    crate::shell::Barra::Dock => "dock_esquiva",
                };
                let esquiva = modo == crate::shell::Visibilidad::Esquivando;
                if let Err(err) = bookos_shell::guardar_esquiva(clave, esquiva) {
                    tracing::warn!("no se pudo guardar la visibilidad de la barra: {err}");
                }
                // Al pasar a esquivar, el área útil crece; al volver a fija,
                // mengua. Las ventanas encajadas hay que recolocarlas o se
                // quedan con el hueco del panel de más o de menos.
                state.recolocar_encajadas();
                state.revisar_barras();
                state.needs_redraw = true;
            }
        }
        Accion::Captura => {
            // Pulsar otra vez con la capa abierta la cierra, como cualquier
            // otro conmutador del escritorio.
            let abierta = state.shell.as_ref().is_some_and(|s| s.hay_captura());
            if abierta {
                if let Some(shell) = state.shell.as_mut() {
                    shell.cerrar_captura();
                }
                crate::input::cursor_de_captura(state);
            } else {
                // Una VM suele tener el puntero bloqueado y el cursor del
                // compositor oculto. La captura es una capa modal del sistema:
                // antes de enseñarla se retira el foco de puntero al cliente.
                // Smithay libera con ello el constraint y recupera el cursor
                // normal, para poder ver y mover el selector de la captura.
                liberar_puntero_para_capa(state);
                if let Some(shell) = state.shell.as_mut() {
                    shell.abrir_captura();
                }
                crate::input::cursor_de_captura(state);
            }
            state.needs_redraw = true;
        }
        Accion::Bloquear => bloquear(state),
        Accion::ApagarPantalla => crate::salvapantallas::apagar_pantalla(state),
        Accion::Salvapantallas | Accion::SalvapantallasOtro => {
            if !crate::salvapantallas::retirar(state) {
                let estilo = state.salvapantallas.estilo;
                let estilo = if matches!(accion, Accion::SalvapantallasOtro) {
                    estilo.otro()
                } else {
                    estilo
                };
                crate::salvapantallas::activar(state, crate::salvapantallas::Origen::Atajo, estilo);
            }
        }
        Accion::DialogoEnergia => alternar_dialogo_energia(state),
        Accion::MenuBookos => alternar_tarjeta_del_panel(state, "menu", "menu"),
        Accion::CentroControl => alternar_tarjeta_del_panel(state, "centro", "control"),
        Accion::Diagnostico => diagnostico(state),
        Accion::Actividades => {
            if state.shell.as_mut().is_some_and(|s| s.alternar_actividad()) {
                state.needs_redraw = true;
            } else {
                tracing::info!("no hay actividad publicada que abrir");
            }
        }
        Accion::Asistente => lanzar(state, "bookos-assistant --toggle"),
        Accion::Lanzar(cmd) => lanzar(state, &cmd),
        Accion::Proyeccion => {
            let conectadas = state.pantallas.compartido.salidas().len();
            tracing::info!(conectadas, "selector de proyección abierto");
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_proyeccion(conectadas);
            }
            state.needs_redraw = true;
        }
        Accion::Desbloquear => {
            state.bloqueo_generacion = state.bloqueo_generacion.wrapping_add(1);
            marcar_bloqueo_sesion(false);
            crate::backend::cancelar_suspension_inactividad(state);
            // Antes de soltarlo, no después: `Default::default()` reemplaza
            // el `String` entero y lo que había se limita a quedar libre, sin
            // que nadie lo haya puesto a cero.
            state.bloqueo.olvidar_escrito();
            state.bloqueo = Default::default();
            if let Some(shell) = state.shell.as_mut() {
                shell.desbloquear();
            }
            crate::multimedia::sonar_interfaz(state, "screen-unlock");
            state.last_input = Some(std::time::Instant::now());
            crate::backend::programar_bloqueo_inactividad(state);
            crate::backend::despertar_dpms(state);
            state.needs_redraw = true;
        }
        Accion::BloqueoEscribir(c) => {
            // Mientras se comprueba no se admite nada más: el campo está en
            // "comprobando" y aceptar teclas ahí deja escrito lo que se teclee
            // encima del intento anterior.
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.fallo = false;
                state.bloqueo.escrito.push(c);
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoBorrar => {
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.fallo = false;
                state.bloqueo.borrar_ultimo();
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoLimpiar => {
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.olvidar_escrito();
                state.bloqueo.fallo = false;
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoComprobar => comprobar_bloqueo(state),
        Accion::BloqueoHuella => comprobar_huella(state),
        Accion::Multimedia(tecla) => {
            if matches!(tecla, crate::multimedia::Tecla::Volumen(_)) {
                crate::multimedia::retener_chasquido(state);
            }
            crate::multimedia::ejecutar(state, tecla);
            crate::multimedia::repetir_al_mantener(state, tecla);
        }
        Accion::Maximizar => {
            if let Some(window) = state.ventana_con_foco() {
                state.alternar_maximizada(&window);
            }
        }
        // Con `-i` para ignorar inhibidores, igual que el menú del panel: desde
        // la pantalla de bloqueo no hay forma de contestarle a un diálogo de
        // «hay una aplicación que impide apagar».
        Accion::Energia(peticion) => {
            use bookos_shell::bloqueo::Peticion;
            let orden = match peticion {
                // Sin `systemctl`: es el compositor el que se va, como al
                // cerrar sesión desde el menú del panel.
                Peticion::CerrarSesion => {
                    state.loop_signal.stop();
                    return;
                }
                // Respetar los inhibidores de logind: una aplicación que está
                // guardando datos o una presentación activa debe poder
                // retrasar la transición. Forzar desde la UI escondería ese
                // estado y puede cortar trabajo del usuario.
                Peticion::Apagar => "systemctl poweroff",
                Peticion::Reiniciar => "systemctl reboot",
                Peticion::Suspender => "systemctl suspend",
                Peticion::Hibernar => "systemctl hibernate",
            };
            lanzar(state, orden);
        }
        Accion::Minimizar => {
            if let Some(window) = state.ventana_con_foco() {
                crate::ventanas::minimizar(state, window);
            }
        }
        Accion::Encajar(hacia) => {
            if let Some(window) = state.ventana_con_foco() {
                let actual = crate::ventanas::zona_de(&window);
                match crate::ventanas::empujar(actual, hacia) {
                    Some(zona) => state.encajar(&window, zona),
                    None => state.desencajar(&window),
                }
            }
        }
        Accion::Conmutar(modo, sentido) => conmutar(state, modo, sentido),
        Accion::ConmutarFin | Accion::ConmutarCancelar
            if salir_de_la_exposicion(state, &accion) => {}
        Accion::ConmutarFin => conmutar_fin(state),
        Accion::ConmutarCancelar => {
            state.conmutador_destinos.clear();
            state.conmutador_modo = None;
            state.conmutador_pegado = false;
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cancelar_conmutador())
            {
                state.needs_redraw = true;
            }
        }
        Accion::DelShell(accion) => hacer(state, accion),
    }
}

/// Quita temporalmente el foco de puntero a las aplicaciones al abrir una
/// capa modal del compositor.
pub(crate) fn liberar_puntero_para_capa(state: &mut BookosComp) {
    use smithay::input::pointer::MotionEvent;

    let pointer = state.pointer.clone();
    let location = state.pointer_location;
    let time = state.start_time.elapsed().as_millis() as u32;
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
}

/// Ejecuta lo que ha pedido el shell: lanzar un programa, o traer al frente el
/// que ya está abierto.
pub fn hacer(state: &mut BookosComp, accion: bookos_shell::Accion) {
    if let Some(energia) = bookos_shell::confirmacion::Energia::de_accion(&accion) {
        if let Some(shell) = state.shell.as_mut() {
            shell.confirmar_energia(energia);
        }
        state.needs_redraw = true;
        return;
    }
    match accion {
        bookos_shell::Accion::CerrarEmergente => {
            if let Some(shell) = state.shell.as_mut() {
                shell.cerrar_emergente();
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::EnergiaConfirmada(energia) => {
            use bookos_shell::confirmacion::Energia;
            match energia {
                Energia::Bloquear => bloquear(state),
                Energia::CerrarSesion => state.loop_signal.stop(),
                // Bloqueada **antes** de dormir, no al despertar: al volver lo
                // primero que se pinta tiene que ser el bloqueo. Pero no se
                // enseña: `systemctl` pasa por logind y tarda en dormir, y ese
                // rato se veía el candado. Con la pantalla apagada ya, el
                // bloqueo no llega a pintarse hasta que se enciende al volver
                // (`escuchar_reanudacion`).
                Energia::Suspender => {
                    bloquear(state);
                    crate::backend::apagar_dpms(state);
                    lanzar(state, "systemctl suspend");
                }
                // Igual que dormir: al volver lo primero tiene que ser el
                // bloqueo, y hasta que el equipo se apaga no hay nada que ver.
                Energia::Hibernar => {
                    bloquear(state);
                    crate::backend::apagar_dpms(state);
                    lanzar(state, "systemctl hibernate");
                }
                Energia::Reiniciar => lanzar(state, "systemctl reboot"),
                Energia::Apagar => lanzar(state, "systemctl poweroff"),
            }
        }
        bookos_shell::Accion::Lanzar(cmd) => lanzar(state, &cmd),
        bookos_shell::Accion::InsertarEmoji(texto) => crate::emojis::insertar(state, &texto),
        bookos_shell::Accion::Launchpad => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_launchpad();
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Escritorio(n) => crate::escritorios::cambiar_aqui(state, n),
        bookos_shell::Accion::VistaEscritorios => alternar_vista_escritorios(state),
        bookos_shell::Accion::CrearEscritorio => {
            if crate::escritorios::crear(state) {
                guardar_y_actualizar_escritorios(state);
            }
        }
        bookos_shell::Accion::EliminarEscritorio(i) => {
            if crate::escritorios::eliminar(state, i) {
                guardar_y_actualizar_escritorios(state);
            }
        }
        bookos_shell::Accion::RenombrarEscritorio { indice, nombre } => {
            if crate::escritorios::renombrar(state, indice, &nombre) {
                guardar_y_actualizar_escritorios(state);
            }
        }
        bookos_shell::Accion::Emergente(widget) => {
            if let Some(shell) = state.shell.as_mut() {
                shell.abrir_de_widget(widget);
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Proyeccion(modo) => {
            if let Some(shell) = state.shell.as_mut() {
                shell.cerrar_emergente();
            }
            if let Err(err) = crate::pantallas::aplicar_modo_rapido(state, modo) {
                tracing::warn!("no se pudo aplicar el modo de proyección: {err}");
                if let Some(shell) = state.shell.as_mut() {
                    shell.mostrar_osd("pantalla", None, Some(err));
                }
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Compartir { sesion, pantalla } => {
            crate::portal::responder(state, sesion, pantalla);
        }
        bookos_shell::Accion::Acerca => {
            if let Some(shell) = state.shell.as_mut() {
                shell.abrir_acerca();
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Apariencia { modo, acento } => {
            // La tarjeta manda lo **elegido**; qué tema toca con «automático»
            // depende de la hora y lo resuelve el compositor, que es quien
            // lleva el reloj y el temporizador del cambio.
            state.modo_tema = modo;
            bookos_shell::tema::aplicar_modo(modo);
            let tema = state.tema_que_toca();
            // Si el tema cambia de verdad, el fondo se va con él: la pareja
            // clara y la oscura son imágenes distintas, y quedarse con la
            // oscura sobre un escritorio claro es justo lo que no se quiere.
            // Se mira **antes** de aplicar, que es cuando `tema::actual()`
            // todavía dice el de ahora.
            let cambia_el_tema = bookos_shell::tema::actual() != tema;
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            if shell.aplicar_apariencia(tema, acento) {
                // Solo si la imagen depende del tema: con un `fondo` fijo y sin
                // pareja, recargar decodificaría 20 MB para poner lo mismo.
                if cambia_el_tema && state.fondo_config.depende_del_tema() {
                    state.recargar_fondo = true;
                }
                // Se guarda **después** de aplicarlo: si escribir falla —disco
                // lleno, `$HOME` de solo lectura— el escritorio ya ha cambiado
                // de color y lo que se pierde es que se recuerde, que es el
                // fallo menos malo de los dos.
                if let Err(err) = bookos_shell::guardar_apariencia(modo, acento) {
                    tracing::warn!("no se pudo guardar la apariencia: {err}");
                }
            }
            // Elegir «automático» —o cambiar de hora— reprograma el despertar
            // del próximo cambio; elegir uno fijo lo cancela.
            crate::apariencia::programar_cambio(state);
            // Y las aplicaciones de fuera se enteran por el portal: sin esto,
            // una ventana GTK ya abierta se queda con el tema de antes.
            crate::portal::apariencia_cambiada(state.bus_portal.as_ref());
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Capturar {
            x,
            y,
            ancho,
            alto,
            guardar,
        } => {
            // La capa se cierra **antes** de capturar: si no, el velo y la
            // barra de modos saldrían en la foto.
            if let Some(shell) = state.shell.as_mut() {
                shell.cerrar_captura();
            }
            crate::input::cursor_de_captura(state);
            state.captura_pedida = Some(crate::state::CapturaPedida {
                x,
                y,
                ancho,
                alto,
                guardar,
            });
            crate::multimedia::sonar_interfaz(state, "screen-capture");
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Fondo { claro, oscuro } => {
            // Las dos rutas de golpe: es lo que hace que el cambio de tema
            // pueda llevarse el fondo con él. `fondo` a secas se borra, o
            // seguiría mandando sobre la pareja al reiniciar la sesión.
            state.fondo_config = crate::fondo::Eleccion {
                ambos: None,
                claro: Some(claro.to_string_lossy().into_owned()),
                oscuro: Some(oscuro.to_string_lossy().into_owned()),
            };
            bookos_shell::fondos::poner_elegida(bookos_shell::fondos::familia_de(&claro));
            state.recargar_fondo = true;
            // Se guarda después de aplicarlo, igual que la apariencia: si el
            // disco falla, lo que se pierde es que se recuerde, no el cambio.
            if let Err(err) = bookos_shell::guardar_fondo(&claro, &oscuro) {
                tracing::warn!("no se pudo guardar el fondo: {err}");
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::BrilloAutomatico(elegido) => {
            crate::brillo_auto::elegir(state, elegido);
            if let Err(err) = bookos_shell::guardar_brillo_automatico(elegido) {
                tracing::warn!("no se pudo guardar el brillo automático: {err}");
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::PerfilEnergia(perfil) => lanzar(
            state,
            &format!(
                "busctl --system set-property net.hadess.PowerProfiles \
                 /net/hadess/PowerProfiles net.hadess.PowerProfiles ActiveProfile s {perfil}",
            ),
        ),
        bookos_shell::Accion::MantenerDespierto => {
            state.idle_notifier_state.set_is_inhibited(
                !state.idle_inhibidores.is_empty() || bookos_shell::mantener_despierto(),
            );
            crate::backend::programar_bloqueo_inactividad(state);
            crate::backend::programar_suspension_inactividad(state);
        }
        bookos_shell::Accion::AbrirCaptura => ejecutar(state, Accion::Captura),
        bookos_shell::Accion::LuzNocturna(encender) => {
            crate::luz_nocturna::alternar(state, encender);
        }
        bookos_shell::Accion::EscritorioNuevaCarpeta => {
            let pantalla = state.pantalla_logica();
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_nueva_carpeta(pantalla);
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::EscritorioEliminarSeleccion => {
            let pantalla = state.pantalla_logica();
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_eliminar_seleccion(pantalla);
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::EscritorioIconos(visibles) => {
            let pantalla = state.pantalla_logica();
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_mostrar(visibles, pantalla);
            }
            if let Err(err) = bookos_shell::guardar_iconos_escritorio(visibles) {
                tracing::warn!("no se pudo guardar si se ven los iconos: {err}");
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::EscritorioColor(color) => {
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_colorear(color.as_deref());
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::AlternarEfectos => {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            let elegido = shell.alternar_efectos();
            // Se guarda después de aplicarlo, igual que la apariencia: si el
            // disco falla, lo que se pierde es que se recuerde, no el cambio.
            if let Err(err) = bookos_shell::guardar_efectos(elegido) {
                tracing::warn!("no se pudieron guardar los efectos: {err}");
            }
            tracing::info!(
                reducidos = bookos_shell::tema::efectos_reducidos(),
                "efectos visuales"
            );
            // Las aplicaciones de fuera lo leen como `reduced-motion`, así que
            // calmar el escritorio también calma sus animaciones.
            crate::portal::apariencia_cambiada(state.bus_portal.as_ref());
            crate::backend::programar_fondo_animado(state);
            // No hace falta repintar el panel ni el dock: su buffer lleva su
            // color y el cristal es un elemento aparte de la escena. Basta con
            // componer otra vez sin él.
            state.needs_redraw = true;
        }
        bookos_shell::Accion::CerrarNotificacion(id) => {
            let cerrada = state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_notificacion(id));
            if cerrada {
                // La aplicación que la mandó espera saber que ya no está: es lo
                // que le permite no repetirla y limpiar lo suyo.
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        // «Responder» no es una acción que la app atienda al pulsarla: pide
        // texto primero. La notificación se queda hasta que se envíe.
        bookos_shell::Accion::NotificacionAccion { id, clave } if clave == "inline-reply" => {
            if let Some(shell) = state.shell.as_mut() {
                shell.abrir_respuesta(id);
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::NotificacionRespuesta { id, texto } => {
            crate::notificaciones::respondida(state.bus_notificaciones.as_ref(), id, &texto);
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_notificacion(id))
            {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::NotificacionAccion { id, clave } => {
            crate::notificaciones::accion_invocada(state.bus_notificaciones.as_ref(), id, &clave);
            // Atendida: la especificación pide cerrarla tras la acción, y
            // hay apps que esperan `NotificationClosed` para limpiar lo suyo.
            // Antes se quedaba en la lista de la campana como sin leer.
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_notificacion(id))
            {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::BorrarNotificaciones => {
            let ids = state
                .shell
                .as_mut()
                .map(|s| s.borrar_notificaciones())
                .unwrap_or_default();
            for id in ids {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Actividad(accion) => {
            crate::ajustes::accion_actividad(state, &accion);
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Bloquear => bloquear(state),
        bookos_shell::Accion::VentanaEncima
        | bookos_shell::Accion::VentanaEscritorio(_)
        | bookos_shell::Accion::VentanaMonitor(_)
        | bookos_shell::Accion::VentanaUnir(_)
        | bookos_shell::Accion::VentanaDisposicion(_)
        | bookos_shell::Accion::VentanaSeparar => {
            let Some(window) = state.menu_ventana.take().filter(|w| w.alive()) else {
                return;
            };
            match accion {
                bookos_shell::Accion::VentanaUnir(pestanas) => {
                    if let Some(otra) = state.menu_ventana_companera.take().filter(|w| w.alive()) {
                        crate::multitarea::unir(state, &window, &otra, pestanas);
                    }
                }
                bookos_shell::Accion::VentanaDisposicion(pestanas) => {
                    crate::multitarea::cambiar_disposicion(state, &window, pestanas)
                }
                bookos_shell::Accion::VentanaSeparar => crate::multitarea::separar(state, &window),
                bookos_shell::Accion::VentanaEncima => state.alternar_encima(&window),
                bookos_shell::Accion::VentanaEscritorio(n) => {
                    crate::escritorios::mover_ventana(state, &window, n)
                }
                bookos_shell::Accion::VentanaMonitor(n) => state.mover_a_monitor(&window, &n),
                _ => unreachable!(),
            }
        }
        bookos_shell::Accion::CerrarSesion => {
            tracing::info!("cierre de sesión pedido desde el diálogo de energía");
            state.loop_signal.stop();
        }
        bookos_shell::Accion::Anclar {
            app_id,
            exec,
            icono,
            fijar,
        } => {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            if let Some(lista) = shell.anclar(&app_id, &exec, &icono, fijar)
                && let Err(err) = bookos_shell::guardar_dock(&lista)
            {
                tracing::warn!("no se pudo guardar el dock: {err}");
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::GuardarDock(lista) => {
            if let Err(err) = bookos_shell::guardar_dock(&lista) {
                tracing::warn!("no se pudo guardar el orden del dock: {err}");
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Cerrar { app_id } => {
            // Se pide el cierre a todas sus ventanas, no solo a la primera: el
            // "Cerrar" del dock cierra la aplicación, y una que tenga tres
            // ventanas abiertas dejaría dos si se cerrara solo una.
            //
            // Las minimizadas y las apartadas por «mostrar el escritorio»
            // también: el dock las cuenta como abiertas —por eso sale
            // «Cerrar»—, y con solo las del `Space` una aplicación minimizada
            // ofrecía cerrarse y no hacía nada.
            let mut ventanas = state.ventanas_de_app(&app_id);
            ventanas.extend(
                state
                    .minimizadas
                    .iter()
                    .map(|(w, _)| w)
                    .chain(state.escritorios.escondidas_activas())
                    .filter(|w| {
                        crate::handlers::app_id(w)
                            .is_some_and(|suyo| bookos_shell::mismo_programa(&app_id, &suyo))
                    })
                    .cloned(),
            );
            for window in ventanas {
                cerrar(&window);
            }
        }
        bookos_shell::Accion::Mostrar { app_id, exec } => {
            let delante = state.ventana_de_app(&app_id).is_some_and(|w| {
                state.ventana_con_foco().as_ref() == Some(&w)
                    && !crate::ventanas::hay_minimizada(state, &app_id)
            });
            if !delante {
                hacer(state, bookos_shell::Accion::Activar { app_id, exec });
            }
        }
        bookos_shell::Accion::Activar { app_id, exec } => {
            // El icono funciona como alternador: minimizada vuelve con la
            // lámpara inversa; abierta se va al dock con la lámpara normal.
            // Primero las minimizadas para no esconder otra ventana de la misma
            // aplicación que se haya quedado abierta detrás.
            if crate::ventanas::hay_minimizada(state, &app_id) {
                crate::ventanas::restaurar_minimizada(state, &app_id);
                return;
            }
            if crate::escritorios::recuperar_app(state, &app_id) {
                return;
            }
            // Mientras sigue viajando aún figura en el `Space`; una segunda
            // orden duplicaría su entrada en la lista de minimización.
            if crate::ventanas::esta_minimizando(state, &app_id) {
                return;
            }
            match state.ventana_de_app(&app_id) {
                // Solo se minimiza la que ya estaba delante con el foco. Si
                // estás en otra aplicación, el clic la trae: minimizarla y
                // tener que pulsar otra vez para recuperarla es un paso tonto.
                Some(window) if state.ventana_con_foco().as_ref() == Some(&window) => {
                    crate::ventanas::minimizar(state, window)
                }
                Some(window) => state.enfocar(&window),
                // El dock creía que estaba abierta y no la encontramos: pasa si
                // el cliente declara un `app_id` distinto del de su `.desktop`.
                // Lanzar es mejor que quedarse quieto — un icono que no responde
                // se lee como que el dock está roto.
                // Vacío es el item que el dock creó desde un `app_id` que no
                // vale como orden; ahí no hay nada que lanzar. Ver el respaldo
                // de `Dock::set_abiertas`.
                None if exec.is_empty() => {
                    tracing::debug!(app_id, "sin ventana que activar y sin orden que lanzar");
                }
                None => {
                    tracing::debug!(app_id, "sin ventana que activar; se lanza");
                    lanzar(state, &exec);
                }
            }
        }
    }
}

fn alternar_vista_escritorios(state: &mut BookosComp) {
    let activo = crate::escritorios::activo_aqui(state);
    let nombres = state.escritorios.nombres().to_vec();
    if let Some(shell) = state.shell.as_mut() {
        shell.alternar_vista_escritorios(activo, nombres);
    }
    // Cuelga del panel como las demás: ver `alternar_tarjeta_del_panel`.
    state.revisar_barras();
    state.needs_redraw = true;
}

fn guardar_y_actualizar_escritorios(state: &mut BookosComp) {
    let activo = crate::escritorios::activo_aqui(state);
    let nombres = state.escritorios.nombres().to_vec();
    if let Err(err) = bookos_shell::guardar_escritorios(&nombres) {
        tracing::warn!("no se pudieron guardar los escritorios: {err}");
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.actualizar_vista_escritorios(activo, nombres);
    }
    state.needs_redraw = true;
}

fn cambiar_vt(state: &mut BookosComp, vt: i32) {
    let Some(cambiar) = state.cambiar_vt.as_ref() else {
        // Backend anidado: el TTY lo manda el compositor de debajo.
        tracing::debug!(vt, "cambio de TTY ignorado: no hay sesión propia");
        return;
    };
    tracing::info!(vt, "cambiando de TTY");
    cambiar(vt);
}

/// Le cuenta al shell cuántos puntos dibujar y en qué estado está.
fn refrescar_bloqueo(state: &mut BookosComp) {
    use bookos_shell::bloqueo::Estado;
    let espera = state.bloqueo.reintentar_desde.and_then(|hasta| {
        hasta
            .checked_duration_since(std::time::Instant::now())
            .map(|d| d.as_secs().saturating_add(u64::from(d.subsec_nanos() > 0)) as u32)
    });
    let estado = match (&state.bloqueo.comprobando, espera, state.bloqueo.fallo) {
        (Some(_), _, _) => Estado::Comprobando,
        (None, Some(segundos), _) => Estado::Espera(segundos.max(1)),
        (None, None, true) => Estado::Fallo,
        (None, None, false) => Estado::Escribiendo,
    };
    let escritos = state.bloqueo.escrito.chars().count();
    if let Some(shell) = state.shell.as_mut() {
        shell.bloqueo_estado(escritos, estado);
    }
    state.needs_redraw = true;
}

/// Manda la contraseña al ayudante de PAM y programa la recogida.
///
/// No se espera al hijo: `pam_unix` tarda del orden de segundos cuando la
/// contraseña falla —es su defensa contra la fuerza bruta— y el compositor no
/// puede quedarse parado ahí. Mientras tanto el campo enseña "comprobando".
fn comprobar_bloqueo(state: &mut BookosComp) {
    if state.bloqueo.comprobando.is_some() || state.bloqueo.escrito.is_empty() {
        return;
    }
    if state
        .bloqueo
        .reintentar_desde
        .is_some_and(|hasta| hasta > std::time::Instant::now())
    {
        refrescar_bloqueo(state);
        return;
    }
    state.bloqueo.reintentar_desde = None;
    let usuario = crate::autenticar::usuario();
    let Some(comprobacion) =
        crate::autenticar::Comprobacion::lanzar(&usuario, &state.bloqueo.escrito)
    else {
        // Sin PAM en el sistema no hay forma de comprobar nada. Se dice en
        // el log y el campo se pone en rojo: colar a quien sea porque falte un
        // binario es lo contrario de lo que hace un bloqueo.
        tracing::error!("no se pudo iniciar PAM: el bloqueo no puede autenticar");
        state.bloqueo.olvidar_escrito();
        state.bloqueo.fallo = true;
        refrescar_bloqueo(state);
        return;
    };
    state.bloqueo.comprobando = Some(comprobacion);
    refrescar_bloqueo(state);
    recoger_comprobacion(state);
}

fn mensaje_huella(state: &mut BookosComp, huella: bookos_shell::bloqueo::Huella) {
    if let Some(shell) = state.shell.as_mut() {
        shell.bloqueo_huella(huella);
    }
    state.needs_redraw = true;
}

/// A partir de cuánto un «no disponible» de la huella es que se agotó la
/// espera y no que falte el lector. La mitad del `timeout` de PAM: lejos de
/// los dos casos medidos (0,26 s y 15,4 s).
const HUELLA_AGOTADA: std::time::Duration = std::time::Duration::from_secs(7);

fn comprobar_huella(state: &mut BookosComp) {
    use bookos_shell::bloqueo::Huella;
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    if !state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
        || !bookos_shell::Config::cargar().bloqueo_huella
        || state.bloqueo.huella_sin_lector
        || state.bloqueo.huella.is_some()
        || state
            .bloqueo
            .huella_reintentar
            .is_some_and(|t| t > std::time::Instant::now())
    {
        return;
    }
    let Some(worker) = crate::autenticar::Comprobacion::huella(&crate::autenticar::usuario())
    else {
        mensaje_huella(
            state,
            Huella::Aviso("Huella no disponible. Usa la contraseña o pulsa F9 para reintentar."),
        );
        return;
    };
    state.bloqueo.huella = Some(worker);
    mensaje_huella(
        state,
        Huella::Aviso("Coloca el dedo en el lector o introduce tu contraseña."),
    );
    // Cada 25 ms. Eran 100, pero con el driver egismoc parcheado el lector
    // casa en ~53 ms (medido): con 100 el sondeo tardaba de media lo mismo que
    // el sensor. Solo corre mientras PAM espera el dedo, como mucho 15 s.
    const CADA: std::time::Duration = std::time::Duration::from_millis(25);
    let generacion = state.bloqueo_generacion;
    let reanudaciones = state.bloqueo.huella_reanudaciones;
    let lanzada = std::time::Instant::now();
    let resultado = state.loop_handle.insert_source(
        Timer::from_duration(CADA),
        move |_, _, state: &mut BookosComp| {
            if state.bloqueo_generacion != generacion { return TimeoutAction::Drop; }
            let Some(worker) = state.bloqueo.huella.as_mut() else { return TimeoutAction::Drop; };
            use crate::autenticar::Veredicto;
            match worker.resultado() {
                None => {
                    // Un dedo que no casa no acaba la conversación —PAM da tres
                    // intentos—, así que no llega como veredicto: llega como
                    // aviso por el camino, y es lo único que distingue en
                    // pantalla «no te he leído» de «te he leído y no eres tú».
                    if worker.dedo_no_casa() {
                        mensaje_huella(state, Huella::NoCoincide);
                    }
                    TimeoutAction::ToDuration(CADA)
                }
                Some(Veredicto::Correcto) => {
                    // Se dice antes de irse: la salida del bloqueo lleva la
                    // última imagen pintada, y así lo que se desvanece es el
                    // «reconocida» y no el «coloca el dedo».
                    mensaje_huella(state, Huella::Reconocida);
                    ejecutar(state, Accion::Desbloquear);
                    TimeoutAction::Drop
                }
                // No hay lector, o fprintd no tiene nada inscrito. **No es un
                // fallo del bloqueo** y no puede anunciarse como tal: en una
                // máquina sin sensor, eso abría el bloqueo diciendo «no se pudo
                // verificar la huella» y parecía que la autenticación estaba
                // rota cuando lo único que pasaba es que sobraba el intento.
                // Se calla, se deja el aviso de siempre y no se reintenta.
                // `pam_fprintd` contesta **lo mismo** cuando se le acaba el
                // `timeout=15` de `session/bookos-fingerprint.pam` sin que
                // nadie toque el lector. Lo que los separa es el tiempo:
                // medido, sin huellas inscritas contesta en 0,26 s, y por
                // tiempo agotado a los 15,4. Tomarlo por «no hay lector»
                // apagaba la huella a los 15 s de bloquear, o sea justo al
                // volver al portátil. Se vuelve a esperar el dedo.
                // Y hay un tercer «no disponible» que el tiempo no separa:
                // «Suspender» bloquea y duerme seguido, fprintd suelta el
                // lector al recibir `PrepareForSleep` y PAM contesta en 0,52 s
                // (medido: bloqueo 13:53:21,55, veredicto 13:53:22,07). El
                // veredicto se recoge al despertar, y `Instant` no cuenta el
                // rato dormido, así que parecía una máquina sin lector y la
                // huella quedaba apagada hasta desbloquear con contraseña.
                Some(Veredicto::NoDisponible)
                    if lanzada.elapsed() >= HUELLA_AGOTADA
                        || state.bloqueo.huella_reanudaciones != reanudaciones =>
                {
                    state.bloqueo.huella = None;
                    comprobar_huella(state);
                    TimeoutAction::Drop
                }
                Some(Veredicto::NoDisponible) => {
                    state.bloqueo.huella = None;
                    state.bloqueo.huella_sin_lector = true;
                    tracing::info!("sin lector de huellas: solo contraseña");
                    // Vacío y no un mensaje: el campo ya dice «Introduce tu
                    // contraseña para desbloquear» un poco más arriba, y
                    // repetirlo aquí debajo salía como dos avisos idénticos
                    // apilados, siempre, en cualquier máquina sin sensor.
                    mensaje_huella(state, Huella::Callada);
                    TimeoutAction::Drop
                }
                Some(Veredicto::Rechazado) => {
                    state.bloqueo.huella = None;
                    state.bloqueo.huella_reintentar = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    mensaje_huella(state, Huella::Aviso("No se pudo verificar la huella. Usa la contraseña o reintenta con F9 en 3 s."));
                    TimeoutAction::Drop
                }
            }
        });
    if let Err(err) = resultado {
        tracing::error!("no se pudo recoger la huella: {err}");
        state.bloqueo.huella = None;
        mensaje_huella(state, Huella::Callada);
    }
}

/// Vuelve a pedir la huella al despertar con el bloqueo echado.
///
/// El veredicto de antes de dormir puede haberse recogido ya —y haber dejado
/// `huella_sin_lector`— o seguir pendiente: lo primero se deshace aquí, lo
/// segundo lo reconoce el sondeo por el contador.
pub fn reanudar_huella(state: &mut BookosComp) {
    state.bloqueo.huella_reanudaciones = state.bloqueo.huella_reanudaciones.wrapping_add(1);
    state.bloqueo.huella_sin_lector = false;
    comprobar_huella(state);
}

/// Demora local frente a intentos repetidos. PAM conserva su propia política;
/// esta capa evita crear trabajadores sin límite desde la interfaz.
fn demora_reintento(fallos: u32) -> u64 {
    if fallos < 3 {
        0
    } else {
        (1_u64 << (fallos - 3).min(5)).min(30)
    }
}

/// Mira cada poco si el ayudante ya contestó.
///
/// Cada 10 ms. Eran 100, pero PAM contesta en unos 50 ms (medido, ver
/// `autenticar.rs`): con 100 la espera media del sondeo era tan larga como la
/// propia comprobación. Son unos cinco despertares por intento, no en reposo.
fn recoger_comprobacion(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    const CADA: std::time::Duration = std::time::Duration::from_millis(10);
    let generacion = state.bloqueo_generacion;
    let result = state.loop_handle.insert_source(
        Timer::from_duration(CADA),
        move |_, _, state: &mut BookosComp| {
            if state.bloqueo_generacion != generacion {
                return TimeoutAction::Drop;
            }
            let Some(comprobacion) = state.bloqueo.comprobando.as_mut() else {
                return TimeoutAction::Drop;
            };
            match comprobacion.resultado().map(|v| v.correcto()) {
                None => TimeoutAction::ToDuration(CADA),
                Some(true) => {
                    ejecutar(state, Accion::Desbloquear);
                    TimeoutAction::Drop
                }
                Some(false) => {
                    // Lo escrito se borra al fallar, como en cualquier pantalla
                    // de acceso: reintentar con la mitad de una contraseña mal
                    // escrita solo alarga el fallo.
                    state.bloqueo.comprobando = None;
                    state.bloqueo.olvidar_escrito();
                    state.bloqueo.fallo = true;
                    state.bloqueo.fallos = state.bloqueo.fallos.saturating_add(1);
                    let demora = demora_reintento(state.bloqueo.fallos);
                    if demora > 0 {
                        state.bloqueo.reintentar_desde = Some(
                            std::time::Instant::now() + std::time::Duration::from_secs(demora),
                        );
                        let generacion = state.bloqueo_generacion;
                        let resultado = state.loop_handle.insert_source(
                            Timer::from_duration(std::time::Duration::from_secs(demora)),
                            move |_, _, state| {
                                if state.bloqueo_generacion == generacion {
                                    state.bloqueo.reintentar_desde = None;
                                    refrescar_bloqueo(state);
                                }
                                TimeoutAction::Drop
                            },
                        );
                        if let Err(err) = resultado {
                            tracing::warn!("no se pudo programar la espera del bloqueo: {err}");
                        }
                    }
                    refrescar_bloqueo(state);
                    TimeoutAction::Drop
                }
            }
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la recogida del bloqueo: {err}");
    }
}

/// Pone o quita el panel de diagnóstico, y con él su despertar por segundo.
///
/// El temporizador es la única fuente de sondeo que hay en el compositor y
/// existe **solo mientras el panel está a la vista**: sin él los números se
/// congelarían en cuanto el escritorio se quedara quieto, que es justo cuando
/// hace falta ver que están a cero. Al quitarlo se suelta.
fn diagnostico(state: &mut BookosComp) {
    let puesto = state
        .shell
        .as_mut()
        .map(|shell| shell.alternar_diagnostico())
        .unwrap_or(false);
    state.needs_redraw = true;
    if let Some(token) = state.tick_diagnostico.take() {
        state.loop_handle.remove(token);
    }
    if !puesto {
        tracing::info!("panel de diagnóstico quitado");
        return;
    }
    tracing::info!("panel de diagnóstico puesto (Meta+Alt+F)");
    let timer =
        smithay::reexports::calloop::timer::Timer::from_duration(std::time::Duration::from_secs(1));
    // El callback no hace nada por sí mismo: basta con que el bucle dé una
    // vuelta para que `post_dispatch` cierre la ventana de medida y le pase los
    // números al panel, que solo repinta si cambiaron.
    let r = state.loop_handle.insert_source(timer, |_, _, state| {
        if state
            .shell
            .as_ref()
            .is_some_and(|s| s.diagnostico_visible())
        {
            smithay::reexports::calloop::timer::TimeoutAction::ToDuration(
                std::time::Duration::from_secs(1),
            )
        } else {
            smithay::reexports::calloop::timer::TimeoutAction::Drop
        }
    });
    match r {
        Ok(token) => state.tick_diagnostico = Some(token),
        Err(err) => tracing::error!("no se pudo programar el tick del diagnóstico: {err}"),
    }
}

fn cerrar_ventana(state: &mut BookosComp) {
    let foco = state
        .seat
        .get_keyboard()
        .and_then(|kbd| kbd.current_focus())
        .and_then(|surface| state.window_for_surface(&surface));
    let Some(window) = foco else {
        return;
    };
    cerrar(&window);
}

/// Pide a una ventana que se cierre, sea nativa o de XWayland.
///
/// Es una petición, no una orden: el cliente puede enseñar un "¿guardar los
/// cambios?" y quedarse. Matar el proceso sería lo contrario de lo que espera
/// quien pulsa "Cerrar".
pub(crate) fn cerrar(window: &smithay::desktop::Window) {
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_close();
        return;
    }
    if let Some(x11) = window.x11_surface()
        && let Err(err) = x11.close()
    {
        tracing::warn!("no se pudo cerrar la ventana X11: {err}");
    }
}

/// Arranca un programa como cliente de esta sesión.
///
/// Centralizado aquí porque los dos backends lo necesitan para el cliente
/// inicial y el atajo del terminal quiere exactamente el mismo entorno: si se
/// escribiera dos veces, tarde o temprano una de las copias se olvidaría de
/// quitar `DISPLAY` y el programa acabaría en el X11 del anfitrión.
/// Cuánto vale un vale de activación.
///
/// Sin plazo, una aplicación puede guardarse el suyo y saltar al frente media
/// hora después, que es el robo de foco que el protocolo venía a impedir.
/// Treinta segundos es lo que tarda en arrancar la peor de esta máquina.
pub const VALE_VALIDO: std::time::Duration = std::time::Duration::from_secs(30);

pub fn lanzar(state: &mut BookosComp, cmd: &str) {
    // Los hijos ya terminados se recogen aquí, no con un manejador de SIGCHLD:
    // el bucle es de un solo hilo y esto no necesita señales ni dependencias.
    state
        .hijos
        .retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));

    match orden(state, cmd).spawn() {
        Ok(hijo) => {
            tracing::info!(cmd, pid = hijo.id(), "cliente lanzado");
            state.hijos.push(hijo);
            if state.shell.as_mut().is_some_and(|s| s.dock_lanzando(cmd)) {
                state.needs_redraw = true;
            }
            if let Err(err) = bookos_shell::sumar_uso(cmd) {
                tracing::warn!("no se pudo apuntar el uso de {cmd}: {err}");
            }
        }
        Err(err) => tracing::error!(cmd, "no se pudo lanzar: {err}"),
    }
}

/// Lanza el programa del kiosko y avisa al bucle cuando termine.
///
/// Un hilo esperándolo, y no `try_wait` en `post_dispatch` como el resto de
/// hijos: el bucle duerme en epoll y no hay SIGCHLD que lo despierte. Si el
/// programa terminara después del último fotograma, nadie volvería a mirar y
/// el arranque se quedaría parado delante de una pantalla vacía.
pub fn lanzar_kiosko(state: &mut BookosComp, cmd: &str) -> anyhow::Result<()> {
    use smithay::reexports::calloop::channel;

    let mut hijo = orden(state, cmd)
        .spawn()
        .map_err(|err| anyhow::anyhow!("no se pudo lanzar {cmd}: {err}"))?;
    tracing::info!(cmd, pid = hijo.id(), "programa del kiosko lanzado");
    let (aviso, fuente) = channel::channel::<i32>();
    state
        .loop_handle
        .insert_source(fuente, |evento, _, state| {
            if let channel::Event::Msg(codigo) = evento {
                state.modo = crate::state::Modo::Kiosko {
                    salida: Some(codigo),
                };
            }
        })
        .map_err(|err| anyhow::anyhow!("insert_source(kiosko): {err}"))?;
    std::thread::Builder::new()
        .name("bookos-kiosko".into())
        .spawn(move || {
            // Matado por una señal no hay código: cuenta como fallo, que es lo
            // que tiene que decir el journal de la unidad.
            let codigo = match hijo.wait() {
                Ok(estado) => estado.code().unwrap_or(1),
                Err(err) => {
                    tracing::error!("no se pudo esperar al programa del kiosko: {err}");
                    1
                }
            };
            let _ = aviso.send(codigo);
        })?;
    Ok(())
}

/// La orden para lanzar `cmd` dentro de la sesión, con el entorno que necesita
/// para encontrar este compositor.
pub fn orden(state: &mut BookosComp, cmd: &str) -> std::process::Command {
    // `sh -c` para que valga cualquier cosa que se escriba en BOOKOS_TERMINAL,
    // con sus argumentos; `exec` evita dejar un shell de más colgando.
    //
    // Pero **solo con un comando suelto**: `exec A || B` no ejecuta B si A no
    // existe, porque `exec` con un programa inexistente termina el shell antes
    // de llegar al `||`. Así se quedaron sin funcionar las entradas del menú que
    // llevan respaldo — el log decía "exec: bookos-about: no encontrado" y ahí
    // se acababa. Cuando hay encadenado, el shell se queda de padre.
    let linea = if cmd.contains("||") || cmd.contains("&&") || cmd.contains(';') {
        cmd.to_string()
    } else {
        format!("exec {cmd}")
    };
    // El tema de cursor va en el entorno porque en Wayland **lo dibuja cada
    // cliente**: Qt y GTK cargan el tema ellos mismos y mandan su propia
    // superficie de cursor. Sin esto, el puntero cambia de aspecto según sobre
    // qué ventana esté — el nuestro sobre el escritorio y el de Adwaita sobre
    // una ventana GTK.
    let (tema, tamano) = state
        .cursor_theme
        .as_ref()
        .map(|t| (t.nombre().to_string(), t.tamano_logico().to_string()))
        .unwrap_or_default();

    // El vale de `xdg-activation` para lo que se lanza desde aquí. Es lo que
    // separa «una ventana ha aparecido» de «el usuario ha pedido esta ventana»:
    // sin él, o se enfoca todo lo que se mapea —y una notificación que abre una
    // ventana te quita el teclado— o no se enfoca nada, y pulsar en el dock
    // deja una ventana muerta delante.
    //
    // Va sin `serial` ni `surface`: no lo pide un cliente por un clic suyo, lo
    // emite el compositor porque el clic lo ha recibido él. `create_external_token`
    // existe justo para este caso y no dispara `token_created`.
    let vale = {
        use smithay::wayland::xdg_activation::XdgActivationTokenData;
        // Los que nadie usó se tiran aquí y no con un temporizador: un vale
        // caduca solo por el paso del tiempo, así que basta con barrer cuando
        // se crea el siguiente. Una aplicación que no arranca, o que arranca y
        // no lo gasta, dejaría el suyo en el mapa para siempre.
        state
            .activacion_state
            .retain_tokens(|_, datos| datos.timestamp.elapsed() < VALE_VALIDO);
        let (token, _) = state
            .activacion_state
            .create_external_token(XdgActivationTokenData::default());
        token.as_str().to_string()
    };

    let mut orden = std::process::Command::new("/bin/sh");
    orden
        .arg("-c")
        .arg(linea)
        .env("WAYLAND_DISPLAY", &state.socket_name)
        .env("XCURSOR_THEME", tema)
        .env("XCURSOR_SIZE", tamano)
        // El nombre de la variable lo fija el protocolo; los toolkits la leen y
        // la borran del entorno para que no la hereden sus propios hijos.
        .env("XDG_ACTIVATION_TOKEN", &vale)
        // La de X11, que es la misma idea con otro nombre. XWayland la traduce,
        // así que una aplicación X11 lanzada del dock también llega enfocada.
        .env("DESKTOP_STARTUP_ID", &vale);
    // DISPLAY solo si XWayland ya contestó. Ponerlo antes es peor que no
    // ponerlo: una aplicación que sabe hablar los dos protocolos prefiere X11
    // en cuanto ve un DISPLAY, y acabaría yendo por el camino largo hacia un
    // servidor que todavía no escucha.
    match state.display_x11 {
        Some(n) => orden.env("DISPLAY", format!(":{n}")),
        None => orden.env_remove("DISPLAY"),
    };
    orden
}

/// El keysym de la tecla de encendido. Como los demás `XF86`, a mano: su valor
/// está fijado desde hace treinta años y no depende de cómo lo llame la versión
/// de turno de la biblioteca de teclado.
const XF86_ENCENDIDO: u32 = 0x1008FF2A;

/// Pide a logind que **no** atienda el botón de encendido mientras corra el
/// compositor. Sin esto logind lo ve antes que nadie y apaga la máquina en el
/// acto (`HandlePowerKey=poweroff` es su valor por defecto), y el diálogo de
/// energía no llega a salir. Con el inhibidor `block` la tecla llega aquí como
/// `XF86PowerOff`, y al terminar la sesión el descriptor se cierra y logind
/// recupera el botón, sin tocar `/etc` ni las demás sesiones.
///
/// El descriptor lo guarda un hilo que no acaba nunca: el inhibidor vive lo que
/// viva el descriptor, y el hilo evita tener que colgarlo del estado.
pub fn inhibir_boton_de_encendido() {
    let fd = zbus::blocking::Connection::system().and_then(|conexion| {
        conexion
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &(
                    "handle-power-key",
                    "BookOS",
                    "El botón de encendido abre el diálogo de energía",
                    "block",
                ),
            )
            .and_then(|respuesta| Ok(respuesta.body().deserialize::<zbus::zvariant::OwnedFd>()?))
    });
    match fd {
        Ok(fd) => {
            let _ = std::thread::Builder::new()
                .name("bookos-inhibidor".into())
                .spawn(move || {
                    let _fd = fd;
                    loop {
                        std::thread::park();
                    }
                });
        }
        Err(err) => tracing::warn!("logind no suelta el botón de encendido: {err}"),
    }
}

/// Abre el diálogo de energía, o lo cierra si ya estaba: pulsar dos veces el
/// botón de encendido tiene que dejar el escritorio como estaba, no apilar
/// diálogos.
fn alternar_dialogo_energia(state: &mut BookosComp) {
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    if shell.emergente_nombre() == Some("apagar") {
        shell.cerrar_emergente();
    } else {
        shell.abrir_de_widget("apagar");
    }
    state.needs_redraw = true;
}

/// Abre la tarjeta que cuelga de un widget del panel, o la cierra si ya estaba,
/// igual que un clic en el widget. `nombre` es el de la emergente y `widget` el
/// del panel que la abre.
///
/// Con el panel apartado por una ventana, el clic ya trae el cursor al borde y
/// eso lo vuelve a sacar; un atajo no, y la tarjeta salía colgando de una barra
/// que no se veía. Por eso se revisan las barras al abrir: una tarjeta abierta
/// retiene la suya (`retenida_por_emergente`).
fn alternar_tarjeta_del_panel(state: &mut BookosComp, nombre: &str, widget: &'static str) {
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    match shell.emergente_nombre() {
        Some(abierta) if abierta == nombre => {
            shell.cerrar_emergente();
        }
        Some(_) => shell.cambiar_a_widget(widget),
        None => shell.abrir_de_widget(widget),
    }
    state.revisar_barras();
    state.needs_redraw = true;
}

fn marcar_bloqueo_sesion(bloqueada: bool) {
    if let Some(path) = std::env::var_os("BOOKOS_SESSION_LOCK_FILE") {
        if bloqueada {
            // Eliminar el permiso de recuperación antes de bloquear. La
            // ausencia del fichero se interpreta como bloqueado (fail closed).
            if let Err(err) = std::fs::remove_file(path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!("no se pudo proteger la recuperación: {err}");
                // EX_NOPERM: el supervisor obliga a volver al login,
                // incluso si quedó un marcador «unlocked» antiguo.
                std::process::exit(77);
            }
        } else if let Err(err) = std::fs::write(path, "unlocked") {
            tracing::warn!("la recuperación requerirá iniciar sesión: {err}");
        }
    }
}

fn bloquear(state: &mut BookosComp) {
    crate::salvapantallas::retirar(state);
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return;
    }
    marcar_bloqueo_sesion(true);
    // Lo que estuviera abierto se cierra antes de echar el bloqueo. El
    // conmutador y la vista de escritorios componen ventanas vivas, y la
    // captura se queda el teclado: por inactividad, o al cerrar la tapa con
    // uno de ellos abierto, salían encima del candado y lo que se tecleaba
    // como contraseña iba a la captura. Un arrastre a medias seguía moviendo
    // la ventana bajo el candado, y al desbloquear la llevaba pegada al cursor.
    // Sin el viaje de vuelta: con el candado ya puesto no se vería.
    state.conmutador_pegado = false;
    state.conmutador_salida = None;
    ejecutar(state, Accion::ConmutarCancelar);
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
        shell.cerrar_captura();
    }
    crate::input::cursor_de_captura(state);
    state.soltar_arrastre();
    crate::input::soltar_escritorio(state);
    state.arrastre_vista = None;
    state.revisar_barras();
    if let Some(token) = state.tick_bloqueo.take() {
        state.loop_handle.remove(token);
    }
    let pantalla = state.pantalla_logica();
    let hora = bookos_shell::Shell::hora_bloqueo();
    let fecha = bookos_shell::Shell::fecha_bloqueo();
    if let Some(shell) = state.shell.as_mut() {
        shell.bloquear(hora, fecha, pantalla);
    }
    // Sin mover el ratón no hay `motion` que haga el `leave`, y la ventana
    // que estaba señalada conservaría su cursor sobre el bloqueo.
    liberar_puntero_para_capa(state);
    // El bloqueo ya está puesto antes de preguntar por MPRIS: un reproductor
    // lento nunca puede retrasar la barrera de seguridad ni el primer frame.
    state.bloqueo_generacion = state.bloqueo_generacion.wrapping_add(1);
    comprobar_huella(state);
    let generacion = state.bloqueo_generacion;
    let canal = state.medios_bloqueo.clone();
    std::thread::Builder::new()
        .name("bookos-lock-media".into())
        .spawn(move || {
            // Un móvil pausado se queda en el bus indefinidamente y la tarjeta
            // salía en cada bloqueo con la canción de hace horas.
            let sonando = bookos_shell::medios::Sonando::leer()
                .filter(|s| s.reproduciendo || !s.es_de_movil());
            let _ = canal.send((generacion, sonando));
        })
        .inspect_err(|err| tracing::warn!("no se pudo consultar MPRIS para el bloqueo: {err}"))
        .ok();
    tracing::info!("bloqueo echado");
    crate::multimedia::sonar_interfaz(state, "screen-lock");
    crate::backend::programar_dpms_bloqueo(state);
    crate::backend::programar_suspension_inactividad(state);
    state.needs_redraw = true;
}

/// Da el foco de teclado a la ventana que quede más arriba.
///
/// Se llama al destruirse un toplevel: sin esto el foco se queda apuntando a la
/// superficie muerta y las ventanas que siguen abiertas no reciben teclas.
pub fn refocalizar(state: &mut BookosComp) {
    // `alive()` porque la ventana destruida sigue mapeada en el espacio hasta
    // el siguiente `refresh()`: sin filtrarla se le devolvería el foco al
    // muerto.
    let heredera = state
        .space
        .elements()
        .rfind(|w| w.alive() && crate::multitarea::visible(state, w))
        .cloned();
    match heredera {
        // Por `enfocar` y no por `set_focus` a secas: la ventana que hereda el
        // foco también tiene que enterarse de que ahora está activa, o se queda
        // dibujándose con la barra de título apagada.
        Some(window) => state.enfocar(&window),
        None => {
            if let Some(kbd) = state.seat.get_keyboard() {
                kbd.set_focus(state, None, SERIAL_COUNTER.next_serial());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(alt: bool, logo: bool, shift: bool, ctrl: bool) -> ModifiersState {
        ModifiersState {
            alt,
            logo,
            shift,
            ctrl,
            ..Default::default()
        }
    }

    /// Alt+Tab y Meta+Tab abren el conmutador, y con Mayús va al revés.
    ///
    /// Si esto pasa y el atajo no responde en la máquina, la tecla no está
    /// llegando: anidado, el compositor de debajo se queda con Alt+Tab.
    #[test]
    fn el_tabulador_con_modificador_conmuta() {
        use bookos_shell::conmutador::Modo;
        let tab = keysyms::KEY_Tab;
        assert_eq!(
            conmutador_de(tab, &mods(true, false, false, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, 1)),
            "Alt+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(false, true, false, false)),
            Some(Accion::Conmutar(Modo::Ventanas, 1)),
            "Meta+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(true, false, true, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, -1)),
            "Alt+Mayús+Tab va hacia atrás"
        );
        // Y con Mayús, xkb entrega otro keysym: tiene que valer igual.
        assert_eq!(
            conmutador_de(keysyms::KEY_ISO_Left_Tab, &mods(true, false, true, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, -1)),
            "ISO_Left_Tab es el mismo gesto"
        );
    }

    /// Un tabulador sin modificador es del cliente —se escribe—, y con Ctrl es
    /// de otra cosa.
    #[test]
    fn el_tabulador_solo_no_es_del_compositor() {
        let tab = keysyms::KEY_Tab;
        assert_eq!(conmutador_de(tab, &mods(false, false, false, false)), None);
        assert_eq!(
            conmutador_de(tab, &mods(true, false, false, true)),
            None,
            "Ctrl+Alt+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(true, true, false, false)),
            None,
            "Alt+Meta+Tab"
        );
        assert_eq!(
            conmutador_de(keysyms::KEY_a, &mods(true, false, false, false)),
            None
        );
    }

    #[test]
    fn reconoce_las_variantes_de_la_tecla_de_pantalla() {
        assert!(es_tecla_de_pantalla(keysyms::KEY_XF86Display, 0));
        assert!(es_tecla_de_pantalla(0x1008_11AF, 0));
        assert!(es_tecla_de_pantalla(0, 227 + 8));
        assert!(es_tecla_de_pantalla(0, 431 + 8));
        assert!(!es_tecla_de_pantalla(keysyms::KEY_F4, 62));
    }

    #[test]
    fn los_reintentos_de_bloqueo_tienen_espera_acotada() {
        assert_eq!(demora_reintento(0), 0);
        assert_eq!(demora_reintento(2), 0);
        assert_eq!(demora_reintento(3), 1);
        assert_eq!(demora_reintento(4), 2);
        assert_eq!(demora_reintento(5), 4);
        assert_eq!(demora_reintento(8), 30);
        assert_eq!(demora_reintento(u32::MAX), 30);
    }
}
