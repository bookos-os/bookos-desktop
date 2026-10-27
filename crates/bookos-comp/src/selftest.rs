//! Autotest del enrutado de entrada.
//!
//! El backend anidado depende de que el compositor anfitrión entregue eventos
//! de ratón, y eso no se puede provocar desde un script. Este módulo sintetiza
//! el mismo camino que recorre un clic real —buscar la superficie bajo el
//! cursor, mover el puntero, pulsar, dar el foco— y cuenta qué encontró en cada
//! paso, para poder distinguir dos fallos que se parecen mucho desde fuera:
//!
//! - "no llegan eventos" (problema del anfitrión o del backend), y
//! - "llegan pero no aciertan" (problema nuestro de coordenadas o de escena).
//!
//! Se activa con `BOOKOS_INPUT_SELFTEST=1` y no se compila fuera de depuración
//! de este binario: es una herramienta de diagnóstico, no una función del DE.

use std::time::Duration;

use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;

use crate::state::BookosComp;

/// Botón izquierdo, según linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

pub fn schedule(state: &mut BookosComp) {
    if std::env::var_os("BOOKOS_INPUT_SELFTEST").is_none() {
        return;
    }
    // Se espera a que el cliente de prueba se haya mapeado.
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(4)),
        |_, _, state| {
            run(state);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar el autotest: {err}");
    }
}

/// Comprueba que el hit-test del dock acierta, sin necesidad de ratón.
///
/// Es la mitad del camino que no se puede provocar desde un script: el dock no
/// es una superficie Wayland, así que ningún cliente puede decir si un clic le
/// llegó. Aquí se pregunta directamente por el centro del primer icono, que es
/// donde el usuario apuntaría.
/// Anclar y desanclar arrastrando entre el launchpad y el dock.
///
/// `BOOKOS_SELFTEST_ANCLAR=1`. Recorre el gesto entero por el mismo camino que
/// el ratón —`pulsar`, `puntero`, `soltar`— porque lo que se comprueba no es la
/// lógica de `Dock`, que ya tiene sus tests, sino el enrutado: con el launchpad
/// abierto la emergente ocupa la pantalla entera y se comía el clic del dock.
fn comprobar_anclar(state: &mut BookosComp) {
    let Some(shell) = state.shell.as_mut() else {
        tracing::error!("no hay shell");
        return;
    };
    shell.alternar_launchpad();
    if shell.emergente_nombre() != Some("launchpad") {
        tracing::error!("el launchpad no abrió");
        return;
    }
    tracing::info!(dock = shell.dock_visible(), rect = ?shell.dock_rect(), "launchpad abierto");
    if !shell.dock_visible() {
        tracing::error!("el dock no se ve con el launchpad abierto");
        return;
    }

    // `BOOKOS_SELFTEST_ANCLAR=captura` se queda aquí: guarda un PNG del
    // launchpad recién abierto y no toca nada más. Es la única forma de
    // comprobar que el dock se **dibuja** ahí —que su superficie esté en la
    // lista no dice nada del orden—, y hacerlo antes del arrastre evita
    // fotografiar el dock a medio rehacer, que es lo que pasa justo después
    // de anclar: la superficie se recrea vacía y se pinta en el refresco
    // siguiente.
    if std::env::var("BOOKOS_SELFTEST_ANCLAR").is_ok_and(|v| v == "captura") {
        let (w, h) = state
            .space
            .outputs()
            .next()
            .map(|o| {
                let g = state.space.output_geometry(o).unwrap_or_default();
                (g.size.w, g.size.h)
            })
            .unwrap_or((1280, 800));
        state.captura_pedida = Some(crate::state::CapturaPedida {
            x: 0,
            y: 0,
            ancho: w,
            alto: h,
            guardar: true,
        });
        state.needs_redraw = true;
        return;
    }

    let Some((cx, cy)) = shell.celda_launchpad(0) else {
        tracing::error!("la rejilla está vacía");
        return;
    };
    let (dx, dy, dw, dh) = shell.dock_rect();
    let (mx, my) = (dx + dw / 2.0, dy + dh / 2.0);

    // Arrastre de ida: del primer icono de la rejilla al centro del dock.
    shell.pulsar(cx, cy);
    shell.puntero(mx, my);
    let accion = shell.soltar();
    tracing::info!(?accion, celda = ?(cx, cy), dock = ?(mx, my), "soltado sobre el dock");
    let Some(bookos_shell::Accion::Anclar {
        app_id,
        exec,
        icono,
        fijar,
    }) = accion
    else {
        tracing::error!("soltar sobre el dock no ancló");
        return;
    };
    if fijar != Some(true) {
        tracing::error!(?fijar, "el arrastre tiene que fijar, no alternar");
    }
    let Some(lista) = shell.anclar(&app_id, &exec, &icono, fijar) else {
        tracing::error!("el anclado no llegó al dock");
        return;
    };
    tracing::info!(
        app_id,
        anclada = shell.esta_anclada(&app_id),
        ?lista,
        "anclada"
    );

    // Y de vuelta: se agarra su icono en el dock y se saca fuera.
    let Some((ix, iy, iw, ih)) = shell.icono_dock(&app_id) else {
        tracing::error!(app_id, "la anclada no tiene icono en el dock");
        return;
    };
    shell.pulsar(ix + iw / 2.0, iy + ih / 2.0);
    // Arriba del todo, que con el launchpad abierto sigue siendo rejilla.
    shell.puntero(ix + iw / 2.0, 60.0);
    let accion = shell.soltar();
    tracing::info!(?accion, desde = ?(ix + iw / 2.0, iy + ih / 2.0), "sacado del dock");
    match accion {
        Some(bookos_shell::Accion::Anclar {
            app_id,
            exec,
            icono,
            fijar: Some(false),
        }) => {
            shell.anclar(&app_id, &exec, &icono, Some(false));
            if shell.esta_anclada(&app_id) {
                tracing::error!(app_id, "sigue anclada después de sacarla");
            } else {
                tracing::info!(app_id, "desanclada al sacarla del dock");
            }
        }
        otra => tracing::error!(?otra, "sacar del dock no desancló"),
    }
    state.needs_redraw = true;
}

/// Crear una carpeta arrastrando un icono del launchpad sobre otro.
///
/// `BOOKOS_SELFTEST_CARPETA=1`. Recorre el gesto por el camino del ratón, que es
/// donde puede fallar: la lógica de `soltar_en` ya tiene su test en el shell.
fn comprobar_carpeta(state: &mut BookosComp) {
    let Some(shell) = state.shell.as_mut() else {
        tracing::error!("no hay shell");
        return;
    };
    shell.alternar_launchpad();
    let (Some(a), Some(b)) = (shell.celda_launchpad(0), shell.celda_launchpad(1)) else {
        tracing::error!("la rejilla no tiene dos celdas");
        return;
    };
    let antes = shell.primeros_del_launchpad(3);
    tracing::info!(?antes, origen = ?a, destino = ?b, "antes de arrastrar");
    shell.pulsar(a.0, a.1);
    // Un par de pasos intermedios, como haría la mano: con un solo salto el
    // umbral se pasa igual, pero así se comprueba también el señalado.
    shell.puntero((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    shell.puntero(b.0, b.1);
    // `BOOKOS_SELFTEST_CARPETA=captura` se queda **a mitad del arrastre**, con
    // el botón todavía pulsado: es el instante que hay que mirar, porque lo que
    // faltaba era la señal de que soltar ahí va a hacer algo.
    if std::env::var("BOOKOS_SELFTEST_CARPETA").is_ok_and(|v| v == "captura") {
        let (w, h) = state
            .space
            .outputs()
            .next()
            .map(|o| {
                let g = state.space.output_geometry(o).unwrap_or_default();
                (g.size.w, g.size.h)
            })
            .unwrap_or((1280, 800));
        state.captura_pedida = Some(crate::state::CapturaPedida {
            x: 0,
            y: 0,
            ancho: w,
            alto: h,
            guardar: true,
        });
        state.needs_redraw = true;
        return;
    }
    let accion = shell.soltar();
    let despues = shell.primeros_del_launchpad(3);
    tracing::info!(?accion, ?despues, "tras soltar encima del segundo");
    if despues.first().is_some_and(|n| n == "Carpeta") {
        tracing::info!("la carpeta se creó");
    } else {
        tracing::error!(?despues, "no nació ninguna carpeta");
    }
    state.needs_redraw = true;
}

fn comprobar_dock(state: &mut BookosComp) {
    let Some((dx, dy, dw, dh)) = state.shell.as_ref().map(|s| s.dock_rect()) else {
        tracing::error!("no hay shell: nada que comprobar en el dock");
        return;
    };
    tracing::info!(dx, dy, dw, dh, "dock (lógico)");

    let centro = (
        dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
        dy + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
    );
    let dentro = state
        .shell
        .as_ref()
        .is_some_and(|s| s.contiene(centro.0, centro.1, false));
    // Si esto falla, el dock se dibuja en un sitio y se pulsa en otro: casi
    // siempre es la escala, que convierte físicos en lógicos.
    tracing::info!(?centro, dentro, "centro del primer icono");

    let accion = state.shell.as_mut().and_then(|shell| {
        assert_eq!(shell.pulsar(centro.0, centro.1), None);
        shell.soltar()
    });
    match accion {
        Some(accion) => tracing::info!(?accion, "el primer icono responde al soltar"),
        None => tracing::error!("el centro del primer icono no devuelve acción al soltar"),
    }

    // Y un punto del hueco entre el primero y el segundo, que no debe ser de
    // nadie: si aquí sale acción, el hit-test se está comiendo los huecos.
    let hueco = (
        dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 + 7.0,
        centro.1,
    );
    match state
        .shell
        .as_mut()
        .and_then(|s| s.pulsar(hueco.0, hueco.1))
    {
        Some(accion) => tracing::error!(?accion, "el hueco entre iconos ha lanzado algo"),
        None => tracing::info!("el hueco entre iconos no es de nadie, correcto"),
    }
}

/// Usa el enrutado completo y el fichero real de la sesión de prueba: una
/// llamada directa al dock no detectaría un clic lanzado antes del arrastre.
fn comprobar_orden_dock(state: &mut BookosComp) {
    use smithay::backend::input::ButtonState;
    let config = bookos_shell::Config::cargar();
    assert!(
        config.dock.len() >= 5,
        "el autotest necesita cinco lanzadores"
    );
    let app = &config.dock[1].app_id;
    let destino = &config.dock[4].app_id;
    let shell = state.shell.as_ref().expect("shell de la sesión de prueba");
    let centro = |r: (f64, f64, f64, f64)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
    let inicio = centro(shell.icono_dock(app).expect("lanzador inicial visible"));
    let fin = centro(
        shell
            .icono_dock(destino)
            .expect("lanzador de destino visible"),
    );
    crate::input::set_pointer(state, inicio.into(), 100);
    crate::input::boton(state, BTN_LEFT, ButtonState::Pressed, 101);
    crate::input::set_pointer(state, fin.into(), 102);
    if std::env::var("BOOKOS_SELFTEST_ORDEN_DOCK").is_ok_and(|v| v == "captura") {
        crate::input::set_pointer(state, (fin.0, fin.1 - 35.0).into(), 103);
        let (w, h) = state
            .space
            .outputs()
            .next()
            .and_then(|o| state.space.output_geometry(o))
            .map(|g| (g.size.w, g.size.h))
            .expect("salida de la sesión de prueba");
        state.captura_pedida = Some(crate::state::CapturaPedida {
            x: 0,
            y: 0,
            ancho: w,
            alto: h,
            guardar: true,
        });
        state.needs_redraw = true;
        return;
    }
    crate::input::boton(state, BTN_LEFT, ButtonState::Released, 103);
    let guardado = bookos_shell::Config::cargar();
    let mut esperado: Vec<_> = config.dock.iter().map(|l| l.app_id.as_str()).collect();
    let movido = esperado.remove(1);
    esperado.insert(4, movido);
    assert_eq!(
        guardado
            .dock
            .iter()
            .map(|l| l.app_id.as_str())
            .collect::<Vec<_>>(),
        esperado
    );
    assert!(
        !state
            .shell
            .as_ref()
            .expect("shell de prueba")
            .dock_agarrado()
    );
    for captura in [true, false] {
        let shell = state.shell.as_ref().expect("shell de prueba");
        let origen = centro(
            shell
                .icono_dock(app)
                .expect("aplicación reordenada visible"),
        );
        let destino = centro(
            shell
                .icono_dock(&config.dock[0].app_id)
                .expect("launchpad visible"),
        );
        crate::input::set_pointer(state, origen.into(), 110);
        crate::input::boton(state, BTN_LEFT, ButtonState::Pressed, 111);
        crate::input::set_pointer(state, destino.into(), 112);
        let shell = state.shell.as_mut().expect("shell de prueba");
        if captura {
            shell.abrir_captura();
        } else {
            shell.bloquear("12:00".into(), "Prueba de arrastre".into(), (1280.0, 800.0));
        }
        crate::input::boton(state, BTN_LEFT, ButtonState::Released, 113);
        let shell = state.shell.as_mut().expect("shell de prueba");
        assert!(
            !shell.dock_agarrado(),
            "la suelta libera el arrastre cancelado por otra capa"
        );
        if captura {
            shell.cerrar_captura();
        } else {
            shell.desbloquear();
        }
    }
    tracing::info!(
        "autotest del orden del dock correcto: orden persistido y cancelación por captura y bloqueo"
    );
    state.loop_signal.stop();
}

/// Abre el menú y lo recorre con el puntero, midiendo lo que cuesta.
///
/// Es el caso que se siente lento y no se puede reproducir desde un script sin
/// esto: hace falta mover el ratón de verdad por encima de las filas, que es
/// cuando el shell repinta y el compositor recompone.
fn recorrer_menu(state: &mut BookosComp) {
    tracing::info!("--- recorrido del menú ---");
    if let Some(shell) = state.shell.as_mut() {
        // Pulsar el logo, arriba a la izquierda.
        shell.pulsar(20.0, 16.0);
    }
    // El movimiento va por `set_pointer`, que es el camino de verdad: marca
    // repintado y avisa al shell. Y se programa a 4 ms para que el bucle tenga
    // que dibujar entre paso y paso, que es lo que hace una mano moviendo el
    // ratón y lo que no se veía llamando al shell en un bucle cerrado.
    programar_paso(state, 0);
}

/// Un movimiento del puntero por el menú, y se programa el siguiente.
fn programar_paso(state: &mut BookosComp, i: u32) {
    const PASOS: u32 = 500;
    if i >= PASOS {
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        tracing::info!("--- fin del recorrido ---");
        return;
    }
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(4)),
        move |_, _, state| {
            // Zigzag por las filas del menú, que es donde cambia el resaltado.
            let y = 34.0 + (i % 40) as f64 * 5.0;
            let x = 60.0 + (i % 7) as f64;
            // Igual que `input::handle`: sin esto el bucle cree que no hay
            // nadie tocando nada y se duerme a 33 ms, y la medición sale
            // diciendo que el escritorio va a 30 fps cuando el problema era la
            // propia prueba.
            state.last_input = Some(std::time::Instant::now());
            crate::input::set_pointer(state, (x, y).into(), i);
            programar_paso(state, i + 1);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar el paso: {err}");
    }
}

/// Barre el puntero de izquierda a derecha y apunta qué cursor pide el cliente
/// en cada columna.
///
/// Sirve para ver **dónde cree el cliente que están sus bordes**: si pide
/// `EwResize` a doscientos píxeles del borde de verdad, su geometría y la
/// nuestra han dejado de coincidir. Cada paso va en su propio temporizador
/// porque el cliente contesta por el socket y no en la misma vuelta del bucle.
fn comprobar_bordes(state: &mut BookosComp, paso: u32) {
    const PASOS: u32 = 40;
    const ESPERA: Duration = Duration::from_millis(120);

    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(loc) = state.space.element_location(&window) else {
        return;
    };
    let tam = window.geometry().size;
    if paso == 0 {
        tracing::info!(?loc, ?tam, "--- bordes: barrido de izquierda a derecha ---");
    }
    if paso >= PASOS {
        tracing::info!("--- fin del barrido ---");
        return;
    }

    // A media altura, para no tropezar con la barra de título ni con el dock.
    let x = loc.x as f64 + (tam.w as f64 * paso as f64 / (PASOS - 1) as f64);
    let y = loc.y as f64 + tam.h as f64 / 2.0;
    crate::input::set_pointer(state, (x, y).into(), 500 + paso);

    let result =
        state
            .loop_handle
            .insert_source(Timer::from_duration(ESPERA), move |_, _, state| {
                tracing::info!(
                    x = format_args!("{x:.0}"),
                    desde_el_borde = format_args!("{:.0}", x - loc.x as f64),
                    cursor = ?state.cursor_status,
                    "columna"
                );
                comprobar_bordes(state, paso + 1);
                TimeoutAction::Drop
            });
    if let Err(err) = result {
        tracing::error!("no se pudo programar el barrido: {err}");
    }
}

/// Comprueba que del bloqueo se puede salir.
///
/// Es la prueba que faltaba y que costó un encierro: con el bloqueo echado
/// ninguna tecla llegaba a resolverse como atajo, así que ni Escape ni el
/// cambio de TTY funcionaban y la sesión se quedaba muerta.
fn comprobar_encierro(state: &mut BookosComp) {
    tracing::info!("--- salidas del bloqueo ---");
    crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
    let echado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    tracing::info!(echado, "bloqueo echado");

    // Las salidas de emergencia tienen que seguir reconociéndose. Se comprueba
    // sobre `Accion`, que es lo que decide si una tecla se traga o no.
    use crate::keybinds::Accion;
    for (accion, nombre) in [
        (Accion::CambiarVt(2), "Ctrl+Alt+F2"),
        (Accion::Salir, "Meta+Alt+Retroceso"),
    ] {
        if accion.es_emergencia() {
            tracing::info!(nombre, "sigue siendo salida de emergencia");
        } else {
            tracing::error!(nombre, "ha dejado de ser salida de emergencia");
        }
    }
    for (accion, nombre) in [
        (Accion::CerrarVentana, "Meta+Q"),
        (Accion::Launchpad, "Meta+Espacio"),
    ] {
        if accion.es_emergencia() {
            tracing::error!(nombre, "no debería funcionar con el bloqueo echado");
        } else {
            tracing::info!(nombre, "se traga, correcto");
        }
    }
    // Y así se quita. Esto **no** prueba la autenticación: la llamada salta
    // directamente a la acción, que en una sesión de verdad solo se alcanza
    // cuando PAM ha dicho que sí. Lo que se comprueba aquí es que el
    // desbloqueo deja el escritorio utilizable, no que el bloqueo proteja.
    crate::keybinds::ejecutar(state, Accion::Desbloquear);
    let sigue = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    if sigue {
        tracing::error!("el desbloqueo no quitó el bloqueo");
    } else {
        tracing::info!("el desbloqueo quita el bloqueo");
    }
    tracing::info!("--- fin ---");
}

/// Lanza una aplicación como lo hace el dock y mira si llega con el foco.
///
/// Es la comprobación de `xdg-activation` de punta a punta: el compositor emite
/// un vale, lo pone en el entorno del hijo, el toolkit lo lee y pide activación
/// con él, y aquí se mira quién tiene el teclado al final. Ninguna de esas
/// cuatro piezas se puede probar por separado —el vale no significa nada sin un
/// cliente que lo gaste—, y de las cuatro solo dos son código de aquí.
///
/// `BOOKOS_SELFTEST_ACTIVACION=1`, con un cliente en la línea de órdenes.
fn comprobar_activacion(state: &mut BookosComp) {
    tracing::info!("--- activación al lanzar ---");

    let vales_antes = state.activacion_state.tokens().count();
    let cliente = std::env::var("BOOKOS_TERMINAL").unwrap_or_else(|_| "konsole".into());
    crate::keybinds::lanzar(state, &cliente);
    let vales = state.activacion_state.tokens().count();
    if vales <= vales_antes {
        tracing::error!(vales, "lanzar no dejó ningún vale en el mapa");
        return;
    }
    tracing::info!(vales, "vale emitido");

    // El cliente tarda en arrancar y en mapear. Se mira al final, no ahora.
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(6)),
        move |_, _, state: &mut BookosComp| {
            let con_foco = state
                .ventana_con_foco()
                .and_then(|w| crate::handlers::app_id(&w));
            match con_foco {
                Some(id) => tracing::info!(app_id = %id, "la ventana lanzada tiene el foco"),
                None => tracing::error!("nadie tiene el foco tras lanzar"),
            }
            // Y el vale se ha gastado: `request_activation` lo retira. Si sigue
            // ahí es que el cliente no lo usó —no todos lo hacen— y entonces
            // esta prueba no dice nada del camino completo.
            let quedan = state.activacion_state.tokens().count();
            if quedan == 0 {
                tracing::info!("el vale se gastó: el cliente pidió activación con él");
            } else {
                tracing::warn!(
                    quedan,
                    "el vale sigue sin gastar; este cliente no usa el protocolo"
                );
            }
            tracing::info!("--- fin ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la comprobación: {err}");
    }
}

/// Recorre entero el permiso de compartir pantalla, sin D-Bus de por medio.
///
/// Es la mitad que no se puede provocar desde un script: `Start` bloquea al
/// hilo del portal hasta que alguien pulsa en la tarjeta, y pulsar en la
/// tarjeta pide un ratón dentro del compositor anidado. Aquí se sintetiza el
/// mismo [`crate::portal::Aviso`] que mandaría el hilo y se contesta con la
/// misma tecla que el usuario, así que lo único que queda sin ejercitar es el
/// empaquetado D-Bus —que sí se puede mirar desde fuera con `busctl`—.
///
/// Se ejecuta con `BOOKOS_SELFTEST_COMPARTIR=1`.
fn comprobar_compartir(state: &mut BookosComp) {
    tracing::info!("--- permiso de compartir pantalla ---");

    let (respuesta, espera) = crate::portal::promesa();
    let (nodo, espera_nodo) = crate::portal::promesa();
    crate::portal::recibir(
        state,
        crate::portal::Aviso::Consentir {
            sesion: 1,
            ruta: zbus::zvariant::OwnedObjectPath::try_from(
                "/org/freedesktop/portal/desktop/session/bookos/selftest",
            )
            .expect("ruta D-Bus fija válida"),
            app: "Autotest".into(),
            respuesta,
            nodo,
        },
    );

    let abierta = state.shell.as_ref().and_then(|s| s.emergente_nombre());
    if abierta != Some("compartir") {
        tracing::error!(?abierta, "la tarjeta del permiso no se abrió");
        return;
    }
    tracing::info!("la tarjeta se abrió");

    // Intro acepta la pantalla resaltada, que es la primera.
    let accion = state
        .shell
        .as_mut()
        .and_then(|s| s.tecla(bookos_shell::TeclaPulsada::Intro).1);
    match accion {
        Some(accion) => crate::keybinds::hacer(state, accion),
        None => {
            tracing::error!("Intro no produjo ninguna acción");
            return;
        }
    }

    // Ya está contestado: `responder` corre en este mismo hilo, así que el
    // buzón tiene el valor sin necesidad de esperar a nada.
    match futuro_ya_resuelto(espera) {
        Some((ancho, alto)) => tracing::info!(ancho, alto, "permiso concedido"),
        None => {
            tracing::error!("el permiso salió denegado");
            return;
        }
    }
    if !state.emisiones.hay() {
        tracing::error!("no quedó ninguna emisión viva");
        return;
    }

    // `BOOKOS_SELFTEST_COMPARTIR=queda` deja la emisión viva y mueve el puntero
    // sin parar: es la única forma de enchufar un consumidor de verdad
    // (`gst-launch-1.0 pipewiresrc`) y mirar los píxeles, porque sin daño en la
    // pantalla no sale ni un fotograma, que es justo el diseño.
    let queda = std::env::var("BOOKOS_SELFTEST_COMPARTIR").is_ok_and(|v| v == "queda");

    // El identificador del nodo lo asigna PipeWire en su propio hilo, así que
    // no está en esta misma vuelta del bucle. Se mira un segundo después, que
    // es también tiempo de sobra para que hayan salido varios fotogramas.
    // El `Timer` quiere un `FnMut` y leer la promesa la consume, así que va en
    // un `Option` que se vacía la primera vez. El temporizador se suelta acto
    // seguido, así que no hay una segunda.
    let mut espera_nodo = Some(espera_nodo);
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(1)),
        move |_, _, state: &mut BookosComp| {
            match espera_nodo.take().and_then(futuro_ya_resuelto) {
                Some(id) => tracing::info!(node_id = id, "nodo de PipeWire vivo"),
                None => tracing::error!("PipeWire no dio identificador de nodo"),
            }
            if queda {
                tracing::info!("la emisión se queda; muevo el puntero para dar daño");
                programar_paso(state, 0);
                return TimeoutAction::Drop;
            }
            state.emisiones.quitar(1);
            tracing::info!("--- fin ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la comprobación del nodo: {err}");
    }
    // Sin esto el escritorio está quieto y no sale ni un fotograma: la emisión
    // va pegada al dibujo, no a un temporizador.
    state.needs_redraw = true;
}

/// Lee una [`crate::portal::Promesa`] que ya tiene que estar resuelta.
///
/// El autotest corre en el hilo del bucle y no puede `.await`ear nada, pero
/// tampoco lo necesita: cuando llega aquí, quien tenía que contestar ya lo ha
/// hecho. Un `poll` con un waker que no hace nada lo saca sin bloquear, y si
/// sale `Pending` es que el fallo es justo que nadie contestó.
fn futuro_ya_resuelto<T>(promesa: crate::portal::Promesa<T>) -> Option<T> {
    use std::task::{Context, Poll};
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    match std::pin::pin!(promesa).poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => None,
    }
}

/// Comprueba que el panel y el dock se apartan de las ventanas y vuelven.
///
/// Es lo que hacen Meta+Alt+B y Meta+Alt+D. Se prueba aquí y no con un test
/// porque hace falta una ventana de verdad ocupando sitio: el solape se mide
/// contra las geometrías del espacio.
fn comprobar_barras(state: &mut BookosComp) {
    use crate::shell::{Barra, Visibilidad};

    tracing::info!("--- barras ---");
    let area_fija = state.work_area();
    tracing::info!(?area_fija, "área útil con el panel fijo");

    for cual in [Barra::Panel, Barra::Dock] {
        let modo = state
            .shell
            .as_mut()
            .map(|s| s.alternar_visibilidad(cual))
            .unwrap_or(Visibilidad::Siempre);
        tracing::info!(?cual, ?modo, "tras el atajo");
    }

    // Con el panel esquivando, el área útil llega hasta arriba del todo: si no,
    // ninguna ventana llegaría nunca a estorbarle.
    let area = state.work_area();
    if area.loc.y == 0 && area.size.h > area_fija.size.h {
        tracing::info!(?area, "el panel deja de reservar sitio");
    } else {
        tracing::error!(?area, ?area_fija, "el área útil no creció al esquivar");
    }

    // Maximizar la ventana la pone encima de las dos barras.
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ventana con la que tapar las barras");
        return;
    };
    state.encajar(&window, crate::ventanas::Zona::Maxima);
    state.revisar_barras();
    let escondidas = (
        !state
            .shell
            .as_ref()
            .is_some_and(|s| s.a_la_vista(Barra::Panel)),
        !state
            .shell
            .as_ref()
            .is_some_and(|s| s.a_la_vista(Barra::Dock)),
    );
    // La animación tarda, así que se mira pasado su tiempo.
    let queda = std::env::var("BOOKOS_SELFTEST_BARRAS").is_ok_and(|v| v == "queda");
    let ventana = window.clone();
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(400)),
        move |_, _, state| {
            let panel = state
                .shell
                .as_ref()
                .is_some_and(|s| s.a_la_vista(Barra::Panel));
            let dock = state
                .shell
                .as_ref()
                .is_some_and(|s| s.a_la_vista(Barra::Dock));
            if queda {
                let zonas = state
                    .shell
                    .as_ref()
                    .map(|s| s.zonas_barras())
                    .unwrap_or_default();
                let geos: Vec<_> = state
                    .space
                    .elements()
                    .filter_map(|w| {
                        state
                            .space
                            .element_location(w)
                            .map(|l| (l, w.geometry().size))
                    })
                    .collect();
                tracing::info!(
                    panel,
                    dock,
                    ?zonas,
                    ?geos,
                    "se queda con las barras apartadas"
                );
                return TimeoutAction::Drop;
            }
            if panel || dock {
                tracing::error!(
                    panel,
                    dock,
                    "una barra sigue a la vista con la ventana encima"
                );
            } else {
                tracing::info!("las dos barras se apartaron de la ventana");
            }

            // Antes de mover nada: el cursor pegado al borde de arriba tiene
            // que traer el panel de vuelta aunque la ventana siga ahí.
            crate::input::set_pointer(state, (400.0, 0.0).into(), 600);
            // La barra tarda lo que dura su animación en llegar; preguntar en
            // el mismo instante siempre diría que no ha vuelto.
            std::thread::sleep(Duration::from_millis(300));
            let vuelve = state
                .shell
                .as_ref()
                .is_some_and(|s| s.a_la_vista(Barra::Panel));
            if vuelve {
                tracing::info!("el cursor en el borde trae el panel de vuelta");
            } else {
                tracing::error!("el cursor en el borde no trae el panel");
            }
            // Con el cursor **sobre el panel** —no en el borde— tiene que
            // seguir a la vista: es lo que permite llegar a pulsar un icono.
            crate::input::set_pointer(state, (400.0, 20.0).into(), 602);
            std::thread::sleep(Duration::from_millis(120));
            if state
                .shell
                .as_ref()
                .is_some_and(|s| s.a_la_vista(Barra::Panel))
            {
                tracing::info!("el panel aguanta con el cursor encima");
            } else {
                tracing::error!("el panel se esconde al subir el cursor hacia sus iconos");
            }

            // Y al apartarlo del borde se vuelve a esconder.
            crate::input::set_pointer(state, (400.0, 400.0).into(), 601);
            std::thread::sleep(Duration::from_millis(300));
            let sigue = state
                .shell
                .as_ref()
                .is_some_and(|s| s.a_la_vista(Barra::Panel));
            if sigue {
                tracing::error!("el panel se queda tras apartar el cursor del borde");
            } else {
                tracing::info!("al apartar el cursor, el panel se vuelve a esconder");
            }

            // Y al quitar la ventana de en medio tienen que volver.
            state.desencajar(&ventana);
            state.encajar(&ventana, crate::ventanas::Zona::InfDer);
            state.revisar_barras();
            let result = state.loop_handle.insert_source(
                Timer::from_duration(Duration::from_millis(400)),
                |_, _, state| {
                    let panel = state
                        .shell
                        .as_ref()
                        .is_some_and(|s| s.a_la_vista(Barra::Panel));
                    tracing::info!(
                        panel,
                        "el panel tras apartar la ventana del borde de arriba"
                    );
                    if !panel {
                        tracing::error!("el panel no volvió cuando dejó de estorbar");
                    }
                    tracing::info!("--- fin del autotest de barras ---");
                    TimeoutAction::Drop
                },
            );
            if let Err(err) = result {
                tracing::error!("no se pudo programar la vuelta: {err}");
            }
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición: {err}");
    }
    tracing::debug!(?escondidas, "estado justo tras maximizar");
    // `BOOKOS_SELFTEST_BARRAS=queda` deja la ventana maximizada y las barras
    // apartadas, para poder capturarlo: el ciclo entero dura menos de un
    // segundo y no da tiempo.
}

/// Comprueba el doble clic en la barra de título: maximiza y restaura.
fn comprobar_doble_clic(state: &mut BookosComp) {
    tracing::info!("--- doble clic ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    if !crate::decoracion::decorada(&window) {
        tracing::warn!("el cliente gestiona su barra; usar konsole para probar la del compositor");
        return;
    }
    let antes = crate::ventanas::maximizada(&window);
    let Some(rect) = crate::decoracion::barra_rect(state, &window) else {
        tracing::error!("una ventana decorada sin rectángulo de barra");
        return;
    };
    let punto = (
        rect.loc.x as f64 + rect.size.w as f64 / 2.0,
        rect.loc.y as f64 + rect.size.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, punto.into(), 400);
    pulsar_dos_veces(state);
    if crate::ventanas::maximizada(&window) == antes {
        tracing::error!("el primer doble clic en la barra no alternó el tamaño");
    } else {
        tracing::info!(
            maximizada = !antes,
            "el primer doble clic en la barra alterna"
        );
    }

    // El cliente confirma el tamaño y el panel se aparta en otras vueltas del
    // bucle. Sin esperarlas, el segundo clic acierta al panel aún visible.
    let resultado = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(600)),
        move |_, _, state| {
            let Some(rect) = crate::decoracion::barra_rect(state, &window) else {
                tracing::error!("una ventana decorada sin rectángulo de barra");
                return TimeoutAction::Drop;
            };
            let punto = (
                rect.loc.x as f64 + rect.size.w as f64 / 2.0,
                rect.loc.y as f64 + rect.size.h as f64 / 2.0,
            );
            crate::input::set_pointer(state, punto.into(), 500);
            pulsar_dos_veces(state);
            if crate::ventanas::maximizada(&window) != antes {
                tracing::error!("el segundo doble clic en la barra no restauró el estado inicial");
            } else {
                tracing::info!(
                    maximizada = antes,
                    "el segundo doble clic en la barra alterna"
                );
            }

            let Some(loc) = state.space.element_location(&window) else {
                return TimeoutAction::Drop;
            };
            let tam = window.geometry().size;
            // Los primeros 40 píxeles de contenido también pertenecen al cliente:
            // ahí puede tener pestañas o botones propios.
            for y in [8.0, tam.h as f64 / 2.0] {
                let punto = (loc.x as f64 + tam.w as f64 / 2.0, loc.y as f64 + y);
                crate::input::set_pointer(state, punto.into(), 600);
                pulsar_dos_veces(state);
                if crate::ventanas::maximizada(&window) != antes {
                    tracing::error!(y, "el compositor cambió el tamaño al pulsar el contenido");
                } else {
                    tracing::info!(
                        y,
                        "el compositor deja el doble clic del contenido al cliente"
                    );
                }
            }
            tracing::info!("--- fin del autotest de doble clic ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = resultado {
        tracing::error!("no se pudo programar el segundo doble clic: {err}");
    }
}

/// Comprueba la barra de título que dibujamos: dónde cae, qué botón hay bajo
/// cada punto y que pulsarlos hace lo que dicen.
///
/// El cliente de prueba tiene que entender `xdg-decoration` —konsole sí, GTK
/// no—; con uno que se decore solo, esto solo puede decir que no lleva barra.
fn comprobar_decoracion(state: &mut BookosComp) {
    use bookos_shell::decoracion::Boton;

    tracing::info!("--- barra de título ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    if !crate::decoracion::decorada(&window) {
        tracing::warn!("el cliente se decora solo; con konsole esta comprobación sí corre");
        return;
    }
    let Some(rect) = crate::decoracion::barra_rect(state, &window) else {
        tracing::error!("una ventana decorada sin rectángulo de barra");
        return;
    };
    let panel = state.shell.as_ref().map_or(0, |s| s.panel_height());
    if rect.loc.y < panel {
        tracing::error!(?rect, panel, "la barra queda por debajo del panel");
    } else {
        tracing::info!(?rect, panel, "la barra cabe bajo el panel");
    }

    // Cada botón, por su centro. El orden es el de la pantalla: minimizar,
    // maximizar y cerrar pegado al borde.
    let medio = rect.loc.y as f64 + rect.size.h as f64 / 2.0;
    for (i, esperado) in [Boton::Cerrar, Boton::Maximizar, Boton::Minimizar]
        .into_iter()
        .enumerate()
    {
        let x = rect.loc.x as f64 + rect.size.w as f64 - 6.0 - i as f64 * 26.0 - 12.0;
        match crate::decoracion::barra_en(state, (x, medio).into()) {
            Some((_, Some(boton))) if boton == esperado => {
                tracing::info!(?esperado, x, "el botón está donde se dibuja")
            }
            otro => tracing::error!(?esperado, x, ?otro, "el hit-test falla en un botón"),
        }
    }

    // Y el maximizar, pulsado de verdad: el mismo camino que sigue un clic.
    let antes = crate::ventanas::maximizada(&window);
    let x = rect.loc.x as f64 + rect.size.w as f64 - 6.0 - 26.0 - 12.0;
    crate::input::set_pointer(state, (x, medio).into(), 700);
    crate::decoracion::pulsar(state, &window, Boton::Maximizar);
    match crate::decoracion::soltar(state, (x, medio).into()) {
        Some((w, boton)) => crate::decoracion::accionar(state, &w, boton),
        None => tracing::error!("soltar sobre el mismo botón no lo dio por pulsado"),
    }
    if crate::ventanas::maximizada(&window) == antes {
        tracing::error!(maximizada = antes, "el botón de maximizar no hizo nada");
    } else {
        tracing::info!(maximizada = !antes, "el botón de maximizar alterna");
    }

    // Soltar fuera del botón donde se pulsó no dispara nada, que es lo que
    // permite arrepentirse a mitad de un clic.
    crate::decoracion::pulsar(state, &window, Boton::Cerrar);
    let lejos = (rect.loc.x as f64 + 10.0, medio);
    if crate::decoracion::soltar(state, lejos.into()).is_some() {
        tracing::error!("soltar fuera del botón lo dio por pulsado");
    } else {
        tracing::info!("soltar fuera del botón no dispara nada");
    }
    tracing::info!("--- fin del autotest de la barra de título ---");
}

/// El detector debe recibir también las sueltas: llamar solo a su función
/// auxiliar no comprueba si el enrutado roba eventos al cliente.
fn pulsar_dos_veces(state: &mut BookosComp) {
    use smithay::backend::input::ButtonState;

    state.ultimo_clic = None;
    for tiempo in [400, 500] {
        crate::input::boton(state, BTN_LEFT, ButtonState::Pressed, tiempo);
        crate::input::boton(state, BTN_LEFT, ButtonState::Released, tiempo + 1);
    }
}

/// Comprueba que el cursor vuelve a ser el del escritorio al salir de una
/// ventana.
///
/// El fallo se veía así: pasabas el ratón por una terminal, que pide la barra
/// en I, salías al fondo y la barra en I seguía puesta por todo el escritorio.
/// El cliente que la pidió ya no tiene el puntero y no vuelve a pedir nada, así
/// que si el compositor no repone, no repone nadie.
fn comprobar_cursor(state: &mut BookosComp) {
    use smithay::input::pointer::CursorImageStatus;

    tracing::info!("--- cursor ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(loc) = state.space.element_location(&window) else {
        return;
    };
    let tam = window.geometry().size;
    let centro = (
        loc.x as f64 + tam.w as f64 / 2.0,
        loc.y as f64 + tam.h as f64 / 2.0,
    );

    // Dentro: el cliente pedirá lo suyo, pero tarda en contestar, así que la
    // comprobación de verdad va en el temporizador.
    crate::input::set_pointer(state, centro.into(), 300);
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(600)),
        move |_, _, state| {
            tracing::info!(cursor = ?state.cursor_status, "con el puntero dentro de la ventana");

            // Y ahora al fondo del escritorio, lejos de todo.
            crate::input::set_pointer(state, (4.0, 4.0).into(), 301);
            let fuera = matches!(state.cursor_status, CursorImageStatus::Named(_));
            if fuera {
                tracing::info!(cursor = ?state.cursor_status, "fuera vuelve al del escritorio");
            } else {
                tracing::error!(
                    cursor = ?state.cursor_status,
                    "el cursor del cliente se quedó puesto fuera de su ventana"
                );
            }
            tracing::info!("--- fin del autotest de cursor ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la comprobación: {err}");
    }
}

/// Cuánto se espera, desde que arranca el fundido, a que haya terminado.
///
/// El fundido dura `tema::D_PAGINA`; el margen de más cubre que la muestra se
/// tome unos milisegundos tarde y que el último fotograma tenga que llegar.
const FUNDIDO_MARGEN: Duration = Duration::from_millis(500);

/// La capa de captura, por el camino de verdad: se abre con el atajo, se marca
/// un recuadro arrastrando y se comprueba que el PNG acaba en el disco con las
/// medidas que se pidieron.
fn comprobar_captura(state: &mut BookosComp) {
    use crate::keybinds::{Accion, ejecutar};

    tracing::info!("--- captura de pantalla ---");
    ejecutar(state, Accion::Captura);
    if !state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        tracing::error!("el atajo no abrió la capa de captura");
        return;
    }
    tracing::info!("capa abierta");

    // Un recuadro a mano, por el mismo camino que el ratón: pulsar, mover y
    // soltar. Las coordenadas son lógicas de la pantalla. Soltar solo marca;
    // la captura la pide Intro, que es el atajo de «Copiar».
    let (x0, y0, x1, y1) = (200.0, 150.0, 600.0, 450.0);
    let accion = state.shell.as_mut().and_then(|s| {
        s.captura_pulsar(x0, y0);
        s.captura_puntero(x1, y1);
        s.captura_soltar();
        s.captura_tecla(bookos_shell::TeclaPulsada::Intro).1
    });
    let Some(accion) = accion else {
        tracing::error!("Intro con un recuadro marcado no pidió ninguna captura");
        return;
    };
    tracing::info!(?accion, "recuadro marcado");
    // El destino por defecto es el portapapeles, así que esta primera va ahí.
    // La del fichero se pide después, con la misma región: son dos caminos
    // distintos y hay que pasar por los dos.
    let region = match accion {
        bookos_shell::Accion::Capturar {
            x, y, ancho, alto, ..
        } => (x, y, ancho, alto),
        _ => {
            tracing::error!("el arrastre pidió algo que no era una captura");
            return;
        }
    };
    ejecutar(state, Accion::DelShell(accion));
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        tracing::error!("la capa sigue abierta: saldría en la propia foto");
    }
    if state.captura_pedida.is_none() {
        tracing::error!("no quedó ninguna captura pedida");
        return;
    }

    // La foto se hace al componer el siguiente fotograma.
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(400)),
        move |_, _, state: &mut BookosComp| {
            if state.captura_pedida.is_some() {
                tracing::error!("la captura se quedó pendiente");
                return TimeoutAction::Drop;
            }
            // Que el compositor sea el dueño del portapapeles y ofrezca una
            // imagen. Que los bytes lleguen de verdad no se puede comprobar
            // desde dentro —ver `captura::el_portapapeles_tiene_imagen`—: eso
            // pide un cliente que pegue.
            if crate::captura::el_portapapeles_tiene_imagen(state) {
                tracing::info!("el portapapeles ofrece la captura como image/png");
            } else {
                tracing::error!("el portapapeles no ofrece la captura");
            }

            // Y ahora la misma región al fichero, que es el otro camino.
            let (x, y, ancho, alto) = region;
            crate::keybinds::ejecutar(
                state,
                crate::keybinds::Accion::DelShell(bookos_shell::Accion::Capturar {
                    x,
                    y,
                    ancho,
                    alto,
                    guardar: true,
                }),
            );
            let _ = state.loop_handle.insert_source(
                Timer::from_duration(Duration::from_millis(400)),
                |_, _, state: &mut BookosComp| {
                    if state.captura_pedida.is_some() {
                        tracing::error!("la captura a fichero se quedó pendiente");
                        return TimeoutAction::Drop;
                    }
                    match bookos_shell::captura::ultima_guardada() {
                        Some(ruta) => match std::fs::metadata(&ruta) {
                            Ok(m) if m.len() > 0 => {
                                tracing::info!(?ruta, bytes = m.len(), "captura guardada")
                            }
                            _ => tracing::error!(?ruta, "el fichero está vacío o no está"),
                        },
                        None => tracing::error!("no se guardó ninguna captura"),
                    }
                    tracing::info!("--- fin del autotest de captura ---");
                    TimeoutAction::Drop
                },
            );
            TimeoutAction::Drop
        },
    );
}

/// Cambiar de tema tiene que llevarse el fondo con él.
///
/// No se puede comprobar desde el shell: el fondo lo carga el compositor y la
/// recarga pasa por el backend, que es quien tiene el `GlesRenderer`. Aquí se
/// pulsa por el camino de verdad —la misma `Accion` que manda la tarjeta de
/// apariencia— y se mira qué fichero acabó puesto.
fn comprobar_apariencia(state: &mut BookosComp) {
    use crate::keybinds::hacer;
    use bookos_shell::tema::{Tema, actual};

    tracing::info!("--- apariencia ---");
    // Lo que había, para dejarlo como estaba: este autotest va por el camino de
    // verdad y ese camino **escribe en `panel.conf`**. Sin devolverlo, correr
    // la batería de pruebas le cambiaba el tema al usuario.
    let modo_original = bookos_shell::tema::modo_actual();
    let acento_original = bookos_shell::tema::acento_actual();
    let antes_tema = actual();
    let antes = state.fondo.as_ref().map(|f| f.ruta().to_path_buf());
    tracing::info!(?antes_tema, ?antes, "de partida");

    // El modo contrario y **fijo**: lo que se comprueba es que cambiar de tema
    // se lleva el fondo, no la política del automático, que tiene sus propios
    // tests en `tema::tema_automatico`.
    let otro = match antes_tema {
        Tema::Claro => bookos_shell::tema::ModoTema::Oscuro,
        Tema::Oscuro => bookos_shell::tema::ModoTema::Claro,
    };
    hacer(
        state,
        bookos_shell::Accion::Apariencia {
            modo: otro,
            acento: bookos_shell::tema::acento_actual(),
        },
    );
    if !state.recargar_fondo {
        tracing::error!("el cambio de tema no pidió recargar el fondo");
    }

    // El fundido: se muestrea a los 120 ms, a mitad de los 280 que dura, y ahí
    // tienen que estar **los dos** fondos vivos. Sin esto solo se sabría que la
    // imagen acabó cambiando, que es justo lo que ya se comprobaba antes de que
    // hubiera transición ninguna.
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(120)),
        move |_, _, state: &mut BookosComp| {
            let Some((saliente, desde)) = state.fondo_saliente.as_ref() else {
                tracing::error!("a los 120 ms ya no hay fundido: el cambio fue un corte");
                return TimeoutAction::Drop;
            };
            tracing::info!(
                ms = desde.elapsed().as_millis(),
                saliente = ?saliente.ruta().file_name(),
                entrante = ?state.fondo.as_ref().and_then(|f| f.ruta().file_name()),
                "a mitad del fundido hay dos fondos"
            );
            // Y que termine y suelte los veinte megas del saliente: dejarlo
            // vivo sería una fuga por cada cambio de tema. Se cuenta desde
            // **aquí** y no desde la acción: el fundido no arranca hasta que la
            // imagen nueva está decodificada, que son otros ciento veinte
            // milisegundos, y contarlo desde antes daba por perdido un fundido
            // que solo iba por la mitad.
            let queda = FUNDIDO_MARGEN.saturating_sub(desde.elapsed());
            let _ = state.loop_handle.insert_source(
                Timer::from_duration(queda),
                move |_, _, state: &mut BookosComp| {
                    if state.fondo_saliente.is_some() {
                        tracing::error!("el fondo saliente sigue vivo pasado el fundido");
                    } else {
                        tracing::info!("el fundido terminó y soltó el fondo anterior");
                    }
                    // El selector va **detrás** y no en paralelo: cambia el
                    // fondo otra vez, y con los dos a la vez el segundo fundido
                    // estaría a medias cuando se comprueba el primero.
                    comprobar_selector_de_fondo(state, modo_original, acento_original);
                    TimeoutAction::Drop
                },
            );
            TimeoutAction::Drop
        },
    );

    // La recarga ocurre al componer el siguiente fotograma, no aquí: hay que
    // dejar pasar uno antes de mirar.
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(300)),
        move |_, _, state: &mut BookosComp| {
            let despues = state.fondo.as_ref().map(|f| f.ruta().to_path_buf());
            tracing::info!(tema = ?actual(), ?despues, "tras cambiar de tema");
            if state.recargar_fondo {
                tracing::error!("la recarga del fondo se quedó pendiente");
            }

            match (&antes, &despues) {
                (Some(a), Some(d)) if a == d => tracing::error!(
                    ?a,
                    "el fondo no cambió con el tema: sigue siendo el mismo fichero"
                ),
                (Some(_), Some(d)) => {
                    tracing::info!(?d, "el fondo se fue con el tema, correcto")
                }
                // Sin fondos instalados no hay nada que comprobar, y decirlo es
                // mejor que dar por bueno un `None` que no prueba nada.
                _ => tracing::warn!("no hay fondo cargado: no se puede comprobar"),
            }
            tracing::info!("--- fin del autotest de apariencia ---");
            TimeoutAction::Drop
        },
    );
}

/// El selector de fondo de la tarjeta de Apariencia, por el camino de verdad:
/// se abre la tarjeta, se pulsa una miniatura y se mira qué imagen acabó
/// puesta.
fn comprobar_selector_de_fondo(
    state: &mut BookosComp,
    modo_original: bookos_shell::tema::ModoTema,
    acento_original: bookos_shell::tema::Acento,
) {
    tracing::info!("--- selector de fondo ---");
    let familias = bookos_shell::fondos::instaladas(bookos_shell::tema::es_claro());
    if familias.len() < 2 {
        tracing::warn!(
            cuantas = familias.len(),
            "hacen falta dos familias instaladas para comprobar el cambio"
        );
        return;
    }
    let antes = state.fondo.as_ref().map(|f| f.ruta().to_path_buf());
    // Lo que había, para devolverlo: elegir un fondo escribe `fondo_claro` y
    // `fondo_oscuro` en `panel.conf`.
    let fondo_original = state.fondo_config.clone();
    // Una que **no** sea la puesta, o pulsarla no cambiaría nada y el test
    // daría por bueno que no pasa nada.
    let puesta = bookos_shell::fondos::elegida();
    let Some(otra) = familias
        .iter()
        .find(|f| Some(f.nombre.as_str()) != puesta.as_deref())
    else {
        tracing::error!("no hay ninguna familia distinta de la puesta");
        return;
    };
    tracing::info!(?antes, puesta = ?puesta, elegimos = %otra.nombre, "de partida");

    crate::keybinds::hacer(
        state,
        bookos_shell::Accion::Fondo {
            claro: otra.claro.clone(),
            oscuro: otra.oscuro.clone(),
        },
    );
    let esperada = otra.claro.clone();
    let esperada_oscura = otra.oscuro.clone();
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(300)),
        move |_, _, state: &mut BookosComp| {
            let despues = state.fondo.as_ref().map(|f| f.ruta().to_path_buf());
            tracing::info!(?despues, "tras elegir");
            let bien = despues.as_deref() == Some(esperada.as_path())
                || despues.as_deref() == Some(esperada_oscura.as_path());
            if bien {
                tracing::info!("el fondo elegido está puesto, correcto");
            } else {
                tracing::error!(?despues, ?esperada, "el fondo elegido no se aplicó");
            }
            restaurar_apariencia(state, modo_original, acento_original, &fondo_original);
            tracing::info!("--- fin del autotest del selector ---");
            TimeoutAction::Drop
        },
    );
}

/// Devuelve la apariencia y el fondo a lo que había antes del autotest, en el
/// disco además de en la pantalla.
///
/// Va por el mismo camino que el usuario para que lo guardado sea lo original,
/// no una mezcla: el autotest usó `Accion::Apariencia` y `Accion::Fondo`, y las
/// dos escriben. Sin esto, pasar las pruebas le dejaba al usuario otro tema y
/// otro fondo puestos.
fn restaurar_apariencia(
    state: &mut BookosComp,
    modo: bookos_shell::tema::ModoTema,
    acento: bookos_shell::tema::Acento,
    fondo: &crate::fondo::Eleccion,
) {
    use crate::keybinds::hacer;
    match (fondo.claro.as_deref(), fondo.oscuro.as_deref()) {
        (Some(claro), Some(oscuro)) => hacer(
            state,
            bookos_shell::Accion::Fondo {
                claro: claro.into(),
                oscuro: oscuro.into(),
            },
        ),
        // No había pareja elegida: se quitan las dos claves para dejar el
        // fichero como estaba, sin ninguna.
        _ => {
            state.fondo_config = fondo.clone();
            state.recargar_fondo = true;
            if let Err(err) = bookos_shell::olvidar_fondo() {
                tracing::warn!("no se pudo devolver el fondo: {err}");
            }
        }
    }
    hacer(state, bookos_shell::Accion::Apariencia { modo, acento });
    tracing::info!(?modo, "apariencia devuelta a como estaba");
}

/// Comprueba la pantalla completa: la ventana ocupa la pantalla entera —panel
/// incluido— y al salir vuelve exactamente a donde estaba.
///
/// Se provoca desde aquí y no esperando a que un cliente pulse F11 porque el
/// camino que hay que probar es el del compositor; que Firefox pida
/// `set_fullscreen` ya lo hace Firefox.
fn comprobar_pantalla_completa(state: &mut BookosComp) {
    tracing::info!("--- pantalla completa ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let antes = state.space.element_location(&window);
    let tamano_antes = window.geometry().size;
    let area = state.work_area();
    tracing::info!(?antes, ?area, "antes de nada");

    state.pantalla_completa(&window, true);
    let dentro = state.space.element_location(&window);
    tracing::info!(
        loc = ?dentro,
        oculta_el_shell = state.hay_pantalla_completa(),
        "puesta a pantalla completa"
    );

    // El tamaño se mide con retardo: el cliente tiene que recibir el configure,
    // repintar y hacer commit antes de que su geometría diga la verdad.
    let ventana = window.clone();
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(2)),
        move |_, _, state| {
            let tam = ventana.geometry().size;
            let loc = state.space.element_location(&ventana);
            let pantalla = state.work_area();
            tracing::info!(?tam, ?loc, "el cliente aceptó la pantalla completa");
            // Tapar el panel es justamente el punto: si la altura no pasa del
            // área útil, la ventana está maximizada, no a pantalla completa.
            if tam.h <= pantalla.size.h {
                tracing::error!(alto = tam.h, area = pantalla.size.h, "no tapa el panel");
            }

            // `BOOKOS_SELFTEST_COMPLETA=queda` deja la ventana dentro, que es
            // la única forma de mirar el resultado con calma: la ida y la
            // vuelta duran dos segundos y no da tiempo a capturar la pantalla.
            if std::env::var("BOOKOS_SELFTEST_COMPLETA").is_ok_and(|v| v == "queda") {
                tracing::info!("se queda a pantalla completa, como se pidió");
                return TimeoutAction::Drop;
            }

            if std::env::var("BOOKOS_SELFTEST_COMPLETA").is_ok_and(|v| v == "encaje") {
                comprobar_barras_tras_encajar(state, &ventana, tamano_antes);
                return TimeoutAction::Drop;
            }

            state.pantalla_completa(&ventana, false);
            let vuelta = state.space.element_location(&ventana);
            tracing::info!(?vuelta, ?antes, "tras salir de pantalla completa");
            if vuelta != antes {
                tracing::error!("no volvió a donde estaba");
            }
            tracing::info!(
                oculta_el_shell = state.hay_pantalla_completa(),
                "--- fin del autotest de pantalla completa ---"
            );
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición: {err}");
    }
}

/// El tamaño encajado y la marca de pantalla completa no pueden coexistir:
/// la escena ocultaría las barras aunque la ventana ya dejara su sitio libre.
fn comprobar_barras_tras_encajar(
    state: &mut BookosComp,
    ventana: &smithay::desktop::Window,
    tamano_antes: smithay::utils::Size<i32, smithay::utils::Logical>,
) {
    use crate::ventanas::{self, Zona};
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

    let salida = state
        .salida_del_puntero()
        .cloned()
        .expect("hay una salida anidada");
    let comprobar = |state: &BookosComp| {
        assert!(
            !ventanas::completa(ventana),
            "encajar quita pantalla completa"
        );
        assert!(
            !state.hay_pantalla_completa_en(&salida),
            "la ventana encajada no oculta el panel ni el dock"
        );
        if let Some(toplevel) = ventana.toplevel() {
            toplevel.with_pending_state(|s| {
                assert!(
                    !s.states.contains(xdg_toplevel::State::Fullscreen),
                    "el cliente también debe salir de pantalla completa"
                );
            });
        }
        if let Some(x11) = ventana.x11_surface() {
            assert!(
                !x11.is_fullscreen(),
                "X11 también sale de pantalla completa"
            );
        }
    };
    for zona in [Zona::Izquierda, Zona::Derecha, Zona::Maxima] {
        state.encajar(ventana, zona);
        comprobar(state);
        state.pantalla_completa(ventana, true);
    }
    state.desencajar(ventana);
    comprobar(state);
    if let Some(toplevel) = ventana.toplevel() {
        toplevel.with_pending_state(|s| {
            assert_eq!(s.size, Some(tamano_antes), "se conserva el tamaño flotante");
        });
    }

    state.pantalla_completa(ventana, true);
    state.encajar(ventana, Zona::Maxima);
    let activo = state.escritorios.activo_en(&salida.name());
    crate::escritorios::cambiar_a(state, &salida, (activo + 1) % 2);
    crate::escritorios::terminar_todos(state);
    assert!(
        !state.hay_pantalla_completa_en(&salida),
        "el escritorio vecino conserva sus barras"
    );
    crate::escritorios::cambiar_a(state, &salida, activo);
    crate::escritorios::terminar_todos(state);
    comprobar(state);
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_secs(2)),
            |_, _, state| {
                captura_multitarea(state);
                tracing::info!("BARRAS_TRAS_ENCAJE_OK: encajar, restaurar y volver al escritorio");
                TimeoutAction::Drop
            },
        )
        .expect("temporizador de la captura de regresión insertado");
}

/// Comprueba que una aplicación X11 llega hasta el escritorio.
///
/// No basta con que XWayland arranque: la ventana tiene que aparecer en el
/// espacio, traer su `WM_CLASS` —de ahí depende que el dock la reconozca— y
/// obedecer un encaje, que es donde X11 se separa de Wayland porque no hay
/// configure diferido. Se lanza el cliente aquí y no como cliente inicial
/// porque al arrancar el compositor todavía no hay `DISPLAY` que darle.
fn comprobar_xwayland(state: &mut BookosComp) {
    tracing::info!("--- xwayland ---");
    match state.display_x11 {
        Some(n) => tracing::info!(display = format!(":{n}"), "XWayland en pie"),
        None => {
            tracing::error!("XWayland no arrancó: no hay DISPLAY que ofrecer");
            return;
        }
    }

    let cliente = std::env::var("BOOKOS_SELFTEST_XWAYLAND")
        .ok()
        .filter(|v| v != "1")
        .unwrap_or_else(|| "xmessage -center hola".to_string());
    crate::keybinds::lanzar(state, &cliente);

    // Tres segundos: xmessage tarda en conectarse a X, pedir el mapeo y que el
    // xwm nos lo cuente. Menos daba falsos negativos al medir.
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(3)),
        |_, _, state| {
            revisar_x11(state);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la revisión de X11: {err}");
    }
}

fn revisar_x11(state: &mut BookosComp) {
    let x11: Vec<_> = state
        .space
        .elements()
        .filter(|w| w.x11_surface().is_some())
        .cloned()
        .collect();
    tracing::info!(n = x11.len(), "ventanas X11 en la escena");
    let Some(window) = x11.first() else {
        tracing::error!("ninguna ventana X11 se mapeó: el cliente no llegó al escritorio");
        return;
    };

    let loc = state.space.element_location(window);
    tracing::info!(
        clase = crate::xwayland::clase(window).unwrap_or_default(),
        app_id = crate::handlers::app_id(window).unwrap_or_default(),
        ?loc,
        tam = ?window.geometry().size,
        "ventana X11"
    );

    // El encaje es la prueba de fuego: si el cliente no obedece al `configure`
    // de X11, el `Space` lo dibuja en su zona pero el contenido sigue con el
    // tamaño viejo y se ve recortado.
    let zona = crate::ventanas::Zona::Izquierda;
    let esperado = zona.rect(state.work_area());
    state.encajar(&window.clone(), zona);
    let ventana = window.clone();
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(2)),
        move |_, _, state| {
            let real = ventana.geometry().size;
            let loc = state.space.element_location(&ventana);
            if real == esperado.size {
                tracing::info!(?real, "el cliente X11 obedeció el encaje");
            } else {
                tracing::error!(?real, esperado = ?esperado.size, "el cliente X11 no obedeció");
            }
            tracing::info!(?loc, esperado = ?esperado.loc, "posición tras el encaje");
            tracing::info!("--- fin del autotest de xwayland ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición del encaje: {err}");
    }
}

/// Mueve, redimensiona y maximiza la ventana sin tocar el ratón.
///
/// Es la otra mitad que no se puede provocar desde un script: mover una ventana
/// necesita una mano moviendo el ratón con Meta pulsada. Aquí se recorre el
/// mismo camino que un arrastre real —`empezar_arrastre`, varios movimientos del
/// puntero, soltar— y se comprueba dónde acabó la ventana.
/// El conmutador de Alt+Tab, por el camino de verdad: las acciones del atajo.
///
/// Lo que se comprueba es lo que se rompe sin avisar: que la lista sale por
/// **uso reciente** y no en el orden en que se abrieron las ventanas, y que
/// soltar el modificador enfoca de verdad la elegida.
fn comprobar_conmutador(state: &mut BookosComp) {
    comprobar_conmutador_tras(state, 0)
}

/// Igual, pero esperando a que haya dos aplicaciones.
///
/// El segundo cliente se lanza a mano desde fuera y una aplicación de KDE tarda
/// varios segundos en mapearse: sin esta espera, la prueba se ejecutaba siempre
/// antes de que llegara y no comprobaba nada.
fn comprobar_conmutador_tras(state: &mut BookosComp, intento: u32) {
    use crate::keybinds::{Accion, ejecutar};
    const INTENTOS: u32 = 8;

    // **Dos ventanas**, no dos aplicaciones: desde que el conmutador va por
    // ventana, dos terminales bastan — y son justo el caso que estuvo roto.
    let cuantas = state.space.elements().count();
    if cuantas < 2 && intento < INTENTOS {
        let result = state.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs(2)),
            move |_, _, state| {
                comprobar_conmutador_tras(state, intento + 1);
                TimeoutAction::Drop
            },
        );
        if let Err(err) = result {
            tracing::error!("no se pudo reprogramar: {err}");
        }
        return;
    }

    tracing::info!("--- conmutador ---");
    let ventanas: Vec<_> = state.space.elements().cloned().collect();
    let ids: Vec<_> = ventanas
        .iter()
        .filter_map(crate::handlers::app_id)
        .collect();
    let titulos: Vec<_> = ventanas
        .iter()
        .filter_map(crate::handlers::titulo)
        .collect();
    tracing::info!(?ids, ?titulos, "ventanas de partida");
    if ventanas.len() < 2 {
        tracing::error!(
            cuantas = ventanas.len(),
            "hacen falta dos ventanas; lanza el autotest con dos clientes"
        );
        return;
    }

    ejecutar(
        state,
        Accion::Conmutar(bookos_shell::conmutador::Modo::Ventanas, 1),
    );
    let abierto = state.shell.as_ref().is_some_and(|s| s.hay_conmutador());
    tracing::info!(abierto, "Alt+Tab");
    if !abierto {
        tracing::error!("el conmutador no se abrió");
        return;
    }

    // Soltar el modificador tiene que llevar el foco a la aplicación elegida,
    // que con un solo Tab es la **anterior** a la de ahora.
    // Por **identidad de ventana** y no por `app_id`: con dos ventanas del
    // mismo programa el `app_id` es el mismo y la comprobación no valdría nada.
    let antes = state.ventana_con_foco();
    ejecutar(state, Accion::ConmutarFin);
    let despues = state.ventana_con_foco();
    tracing::info!(
        antes = ?antes.as_ref().and_then(crate::handlers::titulo),
        despues = ?despues.as_ref().and_then(crate::handlers::titulo),
        "tras soltar"
    );
    if antes == despues {
        tracing::error!("el foco no cambió de ventana");
    }
    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        tracing::error!("el conmutador sigue abierto tras soltar");
    }

    // Y otra vez: tiene que volver a la primera. Ese ida y vuelta es todo el
    // valor del orden por uso reciente.
    ejecutar(
        state,
        Accion::Conmutar(bookos_shell::conmutador::Modo::Ventanas, 1),
    );
    ejecutar(state, Accion::ConmutarFin);
    let vuelta = state.ventana_con_foco();
    tracing::info!(vuelta = ?vuelta.as_ref().and_then(crate::handlers::titulo), "segundo Alt+Tab");
    if vuelta != antes {
        tracing::error!("no alterna entre las dos últimas ventanas");
    }

    // Escape lo cierra sin tocar el foco.
    ejecutar(
        state,
        Accion::Conmutar(bookos_shell::conmutador::Modo::Ventanas, 1),
    );
    ejecutar(state, Accion::ConmutarCancelar);
    let tras_escape = state.ventana_con_foco();
    tracing::info!(
        tras_escape = ?tras_escape.as_ref().and_then(crate::handlers::titulo),
        "tras Escape"
    );
    if tras_escape != vuelta || state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        tracing::error!("Escape no dejó las cosas como estaban");
    }
}

/// Los escritorios virtuales y su deslizamiento, por el camino de verdad.
///
/// Es lo único de los escritorios que se puede comprobar sin un TTY —el gesto
/// necesita libinput, pero el atajo y el gesto acaban los dos en
/// `escritorios::cambiar_a`—. Se muestrea la posición de la ventana mientras
/// dura la transición: si el deslizamiento no ocurre, todas las muestras salen
/// iguales y se ve en la traza.
fn comprobar_escritorios(state: &mut BookosComp) {
    use crate::keybinds::{Accion, ejecutar};

    tracing::info!("--- escritorios ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(antes) = state.space.element_location(&window) else {
        tracing::error!("la ventana no está mapeada");
        return;
    };
    let cuantas = state.space.elements().count();
    tracing::info!(
        escritorio = crate::escritorios::activo_aqui(state),
        cuantas,
        ?antes,
        "de partida"
    );

    ejecutar(state, Accion::EscritorioRelativo(1));
    muestrear_deslizamiento(state, window, antes, cuantas, 0);
}

/// Anota dónde está la ventana mientras se desliza, y comprueba el final.
///
/// Cada muestra va en su propio temporizador porque la animación avanza al
/// componer cada frame: en un bucle cerrado no se movería nada y la prueba
/// mediría su propia impaciencia.
fn muestrear_deslizamiento(
    state: &mut BookosComp,
    window: smithay::desktop::Window,
    antes: smithay::utils::Point<i32, smithay::utils::Logical>,
    cuantas: usize,
    paso: u32,
) {
    use crate::keybinds::{Accion, ejecutar};
    const PASOS: u32 = 9;
    const ESPERA: Duration = Duration::from_millis(40);

    let x = state.space.element_location(&window).map(|p| p.x);
    tracing::info!(
        paso,
        ?x,
        deslizando = state.escritorios.deslizando(),
        "saliendo"
    );

    if paso < PASOS {
        let result =
            state
                .loop_handle
                .insert_source(Timer::from_duration(ESPERA), move |_, _, state| {
                    muestrear_deslizamiento(state, window.clone(), antes, cuantas, paso + 1);
                    TimeoutAction::Drop
                });
        if let Err(err) = result {
            tracing::error!("no se pudo programar la muestra: {err}");
        }
        return;
    }

    // Terminada la animación: el escritorio nuevo tiene que estar vacío.
    let vacio = state.space.elements().count();
    tracing::info!(
        escritorio = crate::escritorios::activo_aqui(state),
        quedan = vacio,
        "tras ir al 2"
    );
    if vacio != 0 || state.escritorios.deslizando() {
        tracing::error!(vacio, "el escritorio nuevo debería estar vacío y quieto");
    }

    ejecutar(state, Accion::EscritorioRelativo(-1));
    // Y al volver, la ventana tiene que acabar **en su sitio exacto**, no donde
    // la dejó la animación: es lo que se rompe si el final del deslizamiento no
    // recoloca y se queda con la última interpolación.
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(400)),
        move |_, _, state| {
            let despues = state.space.element_location(&window);
            tracing::info!(
                escritorio = crate::escritorios::activo_aqui(state),
                vuelven = state.space.elements().count(),
                ?despues,
                "de vuelta al 1"
            );
            if despues != Some(antes) {
                tracing::error!(?antes, ?despues, "la ventana no volvió a su sitio");
            }

            // Y el extremo: a la izquierda del primero no hay nada.
            ejecutar(state, Accion::EscritorioRelativo(-1));
            if crate::escritorios::activo_aqui(state) != 0 {
                tracing::error!(
                    activo = crate::escritorios::activo_aqui(state),
                    "el primer escritorio no debería tener nada a su izquierda"
                );
            }

            // Mostrar escritorio: aparta y devuelve. No anima, así que se puede
            // comprobar en el sitio.
            ejecutar(state, Accion::MostrarEscritorio);
            let despejado = state.space.elements().count();
            ejecutar(state, Accion::MostrarEscritorio);
            let recuperadas = state.space.elements().count();
            tracing::info!(despejado, recuperadas, "mostrar escritorio");
            if despejado != 0 || recuperadas != cuantas {
                tracing::error!(despejado, recuperadas, cuantas, "el despejado no cuadra");
            }
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la vuelta: {err}");
    }
}

fn comprobar_ventanas(state: &mut BookosComp) {
    use crate::ventanas::Modo;

    tracing::info!("--- ventanas ---");
    let area = state.work_area();
    tracing::info!(?area, "área útil");

    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(inicial) = state.space.element_location(&window) else {
        tracing::error!("la ventana no está mapeada");
        return;
    };
    let tam = window.geometry().size;
    tracing::info!(?inicial, ?tam, "ventana de prueba");

    // El arrastre empieza donde esté el cursor, así que primero se lleva al
    // centro de la ventana, que es donde apuntaría una mano.
    let centro = (
        inicial.x as f64 + tam.w as f64 / 2.0,
        inicial.y as f64 + tam.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, centro.into(), 0);
    if !state.empezar_arrastre(Modo::Mover) {
        tracing::error!(?centro, "no hay ventana bajo el centro: el hit-test falla");
        return;
    }
    const DX: f64 = 120.0;
    const DY: f64 = -40.0;
    // En varios pasos y no en uno: es lo que hace una mano, y así se ejercita
    // que la geometría salga del punto de partida y no de ir acumulando deltas.
    for i in 1..=6 {
        let f = i as f64 / 6.0;
        crate::input::set_pointer(state, (centro.0 + DX * f, centro.1 + DY * f).into(), i);
    }
    state.arrastre = None;

    let tras_mover = state.space.element_location(&window);
    let esperado = (inicial.x + DX as i32, inicial.y + DY as i32);
    match tras_mover {
        Some(p) if (p.x, p.y) == esperado => tracing::info!(?p, "la ventana se movió donde tocaba"),
        Some(p) => tracing::error!(?p, ?esperado, "la ventana acabó donde no debía"),
        None => tracing::error!("la ventana dejó de estar mapeada al moverla"),
    }

    // Redimensionar tirando de la esquina inferior derecha: crece el tamaño y
    // el origen no se mueve.
    let ahora = tras_mover.unwrap_or(inicial);
    let esquina = (
        ahora.x as f64 + tam.w as f64 - 4.0,
        ahora.y as f64 + tam.h as f64 - 4.0,
    );
    crate::input::set_pointer(state, esquina.into(), 100);
    if state.empezar_arrastre(Modo::Redimensionar {
        izquierda: false,
        arriba: false,
    }) {
        crate::input::set_pointer(state, (esquina.0 + 80.0, esquina.1 + 60.0).into(), 101);
        let modo = state.arrastre.as_ref().map(|a| a.modo);
        // El cuadrante se decide al empezar: tirando de la esquina de abajo a
        // la derecha, ni izquierda ni arriba.
        tracing::info!(?modo, "borde del que se tira");
        state.arrastre = None;
    } else {
        tracing::error!(?esquina, "no se pudo agarrar la esquina para redimensionar");
    }

    // Y ahora tirando del borde de **arriba** hacia arriba, que es el gesto que
    // metía la ventana bajo el panel: el techo tiene que ser el área útil.
    let ahora = state.space.element_location(&window).unwrap_or(inicial);
    let tam = window.geometry().size;
    let borde = (ahora.x as f64 + tam.w as f64 / 2.0, ahora.y as f64 + 2.0);
    crate::input::set_pointer(state, borde.into(), 150);
    if state.empezar_arrastre(Modo::Redimensionar {
        izquierda: false,
        arriba: true,
    }) {
        // Muy por encima del panel, para que el recorte tenga que actuar.
        crate::input::set_pointer(state, (borde.0, -200.0).into(), 151);
        state.soltar_arrastre();
        match state.space.element_location(&window) {
            Some(p) if p.y >= area.loc.y => {
                tracing::info!(
                    y = p.y,
                    techo = area.loc.y,
                    "el borde de arriba respeta el panel"
                )
            }
            otra => tracing::error!(
                ?otra,
                techo = area.loc.y,
                "la ventana se metió bajo el panel"
            ),
        }
    } else {
        tracing::error!(?borde, "no se pudo agarrar el borde de arriba");
    }

    // Arrastrar hasta el borde izquierdo y soltar: tiene que encajar en media
    // pantalla. Es el gesto que no se puede provocar desde un script.
    let ahora = state.space.element_location(&window).unwrap_or(inicial);
    let tam = window.geometry().size;
    let centro = (
        ahora.x as f64 + tam.w as f64 / 2.0,
        ahora.y as f64 + tam.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, centro.into(), 200);
    if state.empezar_arrastre(Modo::Mover) {
        crate::input::set_pointer(state, (1.0, centro.1).into(), 201);
        let previa = state.arrastre.as_ref().and_then(|a| a.zona);
        tracing::info!(?previa, "vista previa con el cursor en el borde izquierdo");
        state.soltar_arrastre();
        let media = crate::ventanas::Zona::Izquierda.rect(area);
        match state.space.element_location(&window) {
            Some(p) if p == media.loc => tracing::info!(?p, "encajó en media pantalla"),
            otra => tracing::error!(?otra, esperado = ?media.loc, "no encajó donde tocaba"),
        }
    }
    // Y volver: arrastrar una encajada la suelta a su tamaño de antes.
    tracing::info!(zona = ?crate::ventanas::zona_de(&window), "estado tras encajar");
    state.desencajar(&window);
    tracing::info!(donde = ?state.space.element_location(&window), "tras desencajar");

    // Y maximizar, que además de cambiar la geometría tiene que dejar apuntado
    // a dónde volver.
    let antes = state.space.element_location(&window);
    state.alternar_maximizada(&window);
    tracing::info!(
        maximizada = crate::ventanas::maximizada(&window),
        donde = ?state.space.element_location(&window),
        "tras maximizar"
    );
    state.alternar_maximizada(&window);
    let vuelta = state.space.element_location(&window);
    if vuelta == antes {
        tracing::info!(?vuelta, "vuelve exactamente a donde estaba");
    } else {
        tracing::error!(?vuelta, ?antes, "no volvió a su sitio al desmaximizar");
    }

    // Un arrastre pedido por el cliente (`move_request`, o sea tirar de su
    // propia barra de título) tiene que devolverle el evento de soltar: él vio
    // la pulsación. Tragárselo lo dejaba creyendo que seguías pulsando, y el
    // siguiente intento de mover la ventana no hacía nada — el "a veces no
    // agarra".
    if let Some(toplevel) = window.toplevel().cloned() {
        state.arrastrar_toplevel(&toplevel, Modo::Mover);
        let del_cliente = state.arrastre.as_ref().map(|a| a.del_cliente);
        let tragado = state.soltar_arrastre();
        match (del_cliente, tragado) {
            (Some(true), false) => {
                tracing::info!("el soltar vuelve al cliente que pidió mover, correcto")
            }
            otro => tracing::error!(?otro, "el cliente se queda con el botón pulsado"),
        }
    }

    // Con cuatro ventanas o más, se reparten en cuartos: es la comprobación de
    // que se pueden tener las cuatro a la vez sin huecos ni solapes.
    let todas: Vec<_> = state.space.elements().cloned().collect();
    if todas.len() >= 4 {
        use crate::ventanas::Zona;
        for (w, zona) in todas
            .iter()
            .zip([Zona::SupIzq, Zona::SupDer, Zona::InfIzq, Zona::InfDer])
        {
            state.encajar(w, zona);
            tracing::info!(
                ?zona,
                donde = ?state.space.element_location(w),
                pedido = ?zona.rect(area).size,
                "cuarto"
            );
        }
        // El tamaño de verdad se comprueba **después**: el cliente tarda uno o
        // dos fotogramas en devolver un buffer del tamaño nuevo, y preguntarlo
        // aquí mide lo que había antes.
        let result = state.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs(2)),
            |_, _, state| {
                for w in state.space.elements() {
                    tracing::info!(
                        tam = ?w.geometry().size,
                        app = ?crate::handlers::app_id(w),
                        "tamaño real tras encajar"
                    );
                }
                TimeoutAction::Drop
            },
        );
        if let Err(err) = result {
            tracing::error!("no se pudo programar la comprobación: {err}");
        }
    } else {
        tracing::info!(n = todas.len(), "menos de cuatro ventanas: no se reparten");
    }

    // El puntero se deja en un sitio fijo y conocido del escritorio, sobre
    // fondo liso: es la única forma de medir el cursor en una captura, porque
    // dónde acaba si no depende de dónde tuviera el ratón el compositor
    // anfitrión.
    const PARKING: (f64, f64) = (400.0, 700.0);
    crate::input::set_pointer(state, PARKING.into(), 300);
    tracing::info!(?PARKING, "puntero aparcado para medirlo");

    tracing::info!("--- fin de ventanas ---");
}

fn comprobar_organizacion(state: &mut BookosComp, intento: u32) {
    use crate::escritorios;
    use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
    let ventanas: Vec<_> = state.space.elements().cloned().collect();
    if ventanas.len() < 2 && intento < 8 {
        if intento == 0 {
            crate::keybinds::lanzar(state, "konsole --separate");
        }
        state
            .loop_handle
            .insert_source(
                Timer::from_duration(Duration::from_secs(1)),
                move |_, _, state| {
                    comprobar_organizacion(state, intento + 1);
                    TimeoutAction::Drop
                },
            )
            .unwrap();
        return;
    }
    assert!(
        ventanas.len() >= 2,
        "organización: hacen falta dos clientes"
    );
    let window = &ventanas[0];
    let otra = &ventanas[1];
    state.alternar_encima(window);
    state.enfocar(otra);
    assert_eq!(
        state.space.elements().next_back(),
        Some(window),
        "enfocar otra ventana no tapa la fijada"
    );
    let origen = state.space.outputs().next().unwrap().clone();
    let posicion = state.space.element_location(window).unwrap();
    state.enfocar(window);
    escritorios::mover_ventana(state, window, 1);
    assert!(state.space.element_location(window).is_none());
    assert_ne!(state.ventana_con_foco().as_ref(), Some(window));
    escritorios::cambiar_a(state, &origen, 1);
    escritorios::terminar_todos(state);
    assert_eq!(state.space.element_location(window), Some(posicion));
    assert!(crate::ventanas::siempre_encima(window));
    // Monitor lógico para probar traslado, escala y coordenadas negativas.
    let monitor = Output::new(
        "selftest-monitor".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "BookOS".into(),
            model: "Test".into(),
        },
    );
    monitor.change_current_state(
        Some(Mode {
            size: (1920, 1080).into(),
            refresh: 60000,
        }),
        None,
        Some(Scale::Fractional(1.5)),
        Some((-1280, 0).into()),
    );
    state.space.map_output(&monitor, (-1280, 0));
    let puntero = state.pointer_location;
    state.mover_a_monitor(window, "selftest-monitor");
    assert_eq!(state.pointer_location, puntero);
    assert_eq!(
        escritorios::salida_de(state, window).as_deref(),
        Some("selftest-monitor")
    );
    state.mover_a_monitor(window, &origen.name());
    state.alternar_maximizada(window);
    state.mover_a_monitor(window, "selftest-monitor");
    assert!(crate::ventanas::maximizada(window));
    state.mover_a_monitor(window, &origen.name());
    state.pantalla_completa(window, true);
    state.mover_a_monitor(window, "selftest-monitor");
    assert!(crate::ventanas::completa(window));
    assert_eq!(
        state.space.element_location(window),
        Some((-1280, 0).into())
    );
    state.mover_a_monitor(window, &origen.name());
    state.pantalla_completa(window, false);
    state.space.unmap_output(&monitor);
    state.alternar_encima(window);
    assert!(!crate::ventanas::siempre_encima(window));
    tracing::info!(
        "BOOKOS_ORGANIZACION_OK: encima, foco, escritorios, monitor, maximizado y fullscreen"
    );
    state.loop_signal.stop();
}

/// Echa el bloqueo, teclea una contraseña **incorrecta** y mira qué contesta
/// PAM.
///
/// `BOOKOS_SELFTEST_BLOQUEO=1`. No comprueba que se pueda entrar —eso pediría
/// la contraseña de verdad—, sino lo otro, que es lo que estaba sin comprobar:
/// que el camino entero desde la tecla hasta PAM **llega a alguna parte**. Un
/// bloqueo que se queda pensando para siempre y uno que rechaza bien se ven
/// igual desde fuera, y hasta ahora ninguno de los dos dejaba rastro.
///
/// Lo que se espera es que el intento termine en «fallo» en unos segundos.
fn comprobar_bloqueo_pam(state: &mut BookosComp) {
    use crate::keybinds::Accion;

    tracing::info!("--- bloqueo: camino hasta PAM ---");
    crate::keybinds::ejecutar(state, Accion::Bloquear);
    if !state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        tracing::error!("SELFTEST_BLOQUEO_FALLO: el bloqueo no se echó");
        state.loop_signal.stop();
        return;
    }
    for c in "contraseña-incorrecta".chars() {
        crate::keybinds::ejecutar(state, Accion::BloqueoEscribir(c));
    }
    let escrito = state.bloqueo.escrito.chars().count();
    if escrito != "contraseña-incorrecta".chars().count() {
        tracing::error!(escrito, "SELFTEST_BLOQUEO_FALLO: el campo perdió teclas");
        state.loop_signal.stop();
        return;
    }
    crate::keybinds::ejecutar(state, Accion::BloqueoComprobar);
    if state.bloqueo.comprobando.is_none() {
        tracing::error!("SELFTEST_BLOQUEO_FALLO: Intro no lanzó la comprobación de PAM");
        state.loop_signal.stop();
        return;
    }
    tracing::info!("comprobación lanzada; esperando el veredicto de PAM");
    // `pam_unix` se toma sus segundos al fallar, a propósito. Se le da margen
    // de sobra y se mira si contestó: quedarse sin contestar es el fallo que
    // este autotest existe para ver.
    let plazo = std::time::Instant::now() + Duration::from_secs(20);
    let r = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(200)),
        move |_, _, state: &mut BookosComp| {
            if state.bloqueo.fallo {
                tracing::info!("SELFTEST_BLOQUEO_OK: PAM rechazó la contraseña incorrecta");
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }
            if !state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
                tracing::error!("SELFTEST_BLOQUEO_FALLO: se desbloqueó con la contraseña mala");
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }
            if std::time::Instant::now() >= plazo {
                tracing::error!("SELFTEST_BLOQUEO_FALLO: PAM no contestó en 20 s");
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }
            TimeoutAction::ToDuration(Duration::from_millis(200))
        },
    );
    if let Err(err) = r {
        tracing::error!("no se pudo esperar a PAM: {err}");
        state.loop_signal.stop();
    }
}

/// Un recorrido escrito a mano: `BOOKOS_SELFTEST_GUION=/ruta/guion.txt`.
///
/// Sirve para mirar aplicaciones de verdad —los menús y submenús de
/// VirtualBox, un diálogo de GTK— sin sesión en un TTY ni herramientas de
/// fuera: el clic entra por [`crate::input::boton`], el mismo camino que uno de
/// libinput. Una orden por línea, `#` comenta:
///
/// ```text
/// espera 3000          # milisegundos
/// mover 400 300        # coordenadas lógicas del escritorio
/// clic 400 300         # botón izquierdo; `clic 400 300 der` para el derecho
///                      # (mover, clic y tecla aceptan al final los ms de espera)
/// tecla meta+w         # combinaciones con `+`; ver `codigo` más abajo
/// tema claro           # o `oscuro`; guarda en panel.conf, usar otro XDG_CONFIG_HOME
/// pulsar 400 300        # baja el botón izquierdo ahí y lo deja pulsado
/// soltar               # lo suelta donde esté el puntero
/// toque 400 300        # un dedo apoyado y levantado, por `input::toque_abajo`
/// arrastre 400 600 400 200  # un dedo de un punto a otro en diez pasos
/// captura              # pantalla entera a $XDG_PICTURES_DIR/Capturas
/// fin                  # cierra el compositor
/// ```
fn guion(state: &mut BookosComp, ruta: &str) {
    let pasos: Vec<String> = match std::fs::read_to_string(ruta) {
        Ok(texto) => texto
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(err) => {
            tracing::error!(ruta, "no se pudo leer el guion: {err}");
            return;
        }
    };
    paso_de_guion(state, std::rc::Rc::new(pasos), 0);
}

fn paso_de_guion(state: &mut BookosComp, pasos: std::rc::Rc<Vec<String>>, i: usize) {
    use smithay::backend::input::ButtonState;
    const BTN_IZQUIERDO: u32 = 0x110;
    const BTN_DERECHO: u32 = 0x111;
    // Entre órdenes, lo justo para que el cliente conteste a la anterior.
    const RESPIRO: u64 = 250;

    let Some(linea) = pasos.get(i) else {
        tracing::info!("guion terminado");
        return;
    };
    tracing::info!(paso = i, linea, "guion");
    let partes: Vec<&str> = linea.split_whitespace().collect();
    let numero = |n: usize| partes.get(n).and_then(|v| v.parse::<f64>().ok());
    // La espera hasta la orden siguiente: la última pieza, si es un número y va
    // detrás de las `fijas`. Para capturar a mitad de una animación de 180 ms
    // el respiro de siempre llega tarde.
    let despues = |fijas: usize| {
        partes
            .get(fijas..)
            .and_then(|resto| resto.last())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(RESPIRO)
    };
    // Los tiempos solo tienen que crecer: los clientes los usan para ordenar.
    let t = 100_000 + i as u32 * 50;
    let espera = match partes.as_slice() {
        ["espera", ms] => ms.parse::<u64>().unwrap_or(RESPIRO),
        ["pulsar", ..] => {
            let (Some(x), Some(y)) = (numero(1), numero(2)) else {
                tracing::error!(linea, "faltan las coordenadas");
                return;
            };
            crate::input::set_pointer(state, (x, y).into(), t);
            crate::input::boton(state, BTN_IZQUIERDO, ButtonState::Pressed, t + 1);
            state.needs_redraw = true;
            despues(3)
        }
        ["toque", ..] | ["arrastre", ..] => {
            let (Some(x), Some(y)) = (numero(1), numero(2)) else {
                tracing::error!(linea, "faltan las coordenadas");
                return;
            };
            let slot = smithay::backend::input::TouchSlot::from(Some(0));
            let frame = |state: &mut BookosComp| {
                if let Some(touch) = state.seat.get_touch() {
                    touch.frame(state);
                }
            };
            crate::input::toque_abajo(state, slot, (x, y).into(), t);
            frame(state);
            let mut fijas = 3;
            // Diez pasos y no un salto: un cliente que desplaza con el dedo
            // necesita ver el recorrido, no solo el destino.
            if let (["arrastre", ..], Some(x2), Some(y2)) =
                (partes.as_slice(), numero(3), numero(4))
            {
                fijas = 5;
                for paso in 1..=10u32 {
                    let f = paso as f64 / 10.0;
                    let punto = (x + (x2 - x) * f, y + (y2 - y) * f).into();
                    crate::input::toque_mueve(state, slot, punto, t + paso);
                    frame(state);
                }
            }
            crate::input::toque_arriba(state, slot, t + 11);
            frame(state);
            state.needs_redraw = true;
            despues(fijas)
        }
        // `rueda dy`: lo que manda una rueda o un touchpad al shell, en
        // píxeles lógicos. Va al mismo `desplazar` que la entrada real.
        ["rueda", ..] => {
            let Some(dy) = numero(1) else {
                tracing::error!(linea, "falta cuánto desplazar");
                return;
            };
            if state.shell.as_mut().is_some_and(|s| s.desplazar(0.0, dy)) {
                state.needs_redraw = true;
            }
            despues(2)
        }
        ["soltar", ..] => {
            crate::input::boton(state, BTN_IZQUIERDO, ButtonState::Released, t);
            state.needs_redraw = true;
            despues(1)
        }
        ["mover", ..] | ["clic", ..] => {
            let (Some(x), Some(y)) = (numero(1), numero(2)) else {
                tracing::error!(linea, "faltan las coordenadas");
                return;
            };
            // Lo que hace `input::handle` con cualquier entrada real: el guion
            // no pasa por ahí, y sin esto no habría forma de probar lo que
            // depende de que el usuario vuelva (la atenuación, el bloqueo).
            state.last_input = Some(std::time::Instant::now());
            crate::atenuacion::actividad(state);
            let tapado = crate::salvapantallas::actividad(state);
            if !tapado {
                crate::input::set_pointer(state, (x, y).into(), t);
            }
            let espera = despues(3);
            if !tapado && partes[0] == "clic" {
                let boton = if partes.get(3) == Some(&"der") {
                    BTN_DERECHO
                } else {
                    BTN_IZQUIERDO
                };
                crate::input::boton(state, boton, ButtonState::Pressed, t + 1);
                crate::input::boton(state, boton, ButtonState::Released, t + 2);
            }
            state.needs_redraw = true;
            espera
        }
        ["tecla", combinacion, ..] => {
            use smithay::backend::input::KeyState;
            // `meta+w`: se pulsan en orden y se sueltan al revés, como con las
            // manos, para que los modificadores estén puestos al llegar la tecla.
            let Some(teclas) = combinacion
                .split('+')
                .map(|n| codigo_tecla(n).map(|c| smithay::input::keyboard::Keycode::new(c + 8)))
                .collect::<Option<Vec<_>>>()
            else {
                tracing::error!(combinacion, "tecla de guion desconocida");
                return;
            };
            for (n, tecla) in teclas.iter().enumerate() {
                crate::input::tecla(state, *tecla, KeyState::Pressed, t + n as u32);
            }
            for (n, tecla) in teclas.iter().rev().enumerate() {
                crate::input::tecla(state, *tecla, KeyState::Released, t + 20 + n as u32);
            }
            state.needs_redraw = true;
            despues(2)
        }
        // Lo que hacen tres dedos hacia arriba, que en el anidado no llegan.
        // Echar y quitar el bloqueo, para mirar sus animaciones. Quitarlo salta
        // PAM, como el autotest del bloqueo: esto solo existe bajo
        // BOOKOS_INPUT_SELFTEST.
        ["bloquear"] => {
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
            RESPIRO
        }
        ["desbloquear"] => {
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::Desbloquear);
            0
        }
        // Pone a pantalla completa, o la quita, la ventana de más arriba. En el
        // anidado no hay ningún reproductor que lo pida solo.
        ["completa"] => {
            if let Some(ventana) = state.space.elements().last().cloned() {
                let poner = !crate::ventanas::completa(&ventana);
                state.pantalla_completa(&ventana, poner);
            }
            RESPIRO
        }
        // El OSD del volumen, sin tocar el volumen real.
        ["osd", ..] => {
            crate::multimedia::avisar(state, ("volumen-medio", Some(50), None));
            despues(1)
        }
        ["exponer"] => {
            crate::input::exponer(state);
            state.needs_redraw = true;
            despues(2)
        }
        // Letra a letra, para escribir en un buscador: `escribir fondo`.
        ["escribir", palabra, ..] => {
            use smithay::backend::input::KeyState;
            for (n, letra) in palabra.chars().enumerate() {
                let Some(c) = codigo_tecla(&letra.to_string()) else {
                    tracing::error!(%letra, "letra de guion desconocida");
                    return;
                };
                let tecla = smithay::input::keyboard::Keycode::new(c + 8);
                let t = t + n as u32 * 4;
                crate::input::tecla(state, tecla, KeyState::Pressed, t);
                crate::input::tecla(state, tecla, KeyState::Released, t + 2);
            }
            state.needs_redraw = true;
            despues(2)
        }
        ["tema", cual, ..] => {
            // Por la misma acción que la tarjeta de Apariencia, que además lo
            // guarda en `panel.conf`: el guion debe correr con un
            // `XDG_CONFIG_HOME` de prueba.
            let modo = match *cual {
                "claro" => bookos_shell::tema::ModoTema::Claro,
                "oscuro" => bookos_shell::tema::ModoTema::Oscuro,
                otro => {
                    tracing::error!(tema = otro, "tema de guion desconocido");
                    return;
                }
            };
            crate::keybinds::hacer(
                state,
                bookos_shell::Accion::Apariencia {
                    modo,
                    acento: bookos_shell::tema::acento_actual(),
                },
            );
            despues(2)
        }
        ["captura"] => {
            let Some(g) = state
                .space
                .outputs()
                .next()
                .and_then(|o| state.space.output_geometry(o))
            else {
                tracing::error!("sin salida que capturar");
                return;
            };
            state.captura_pedida = Some(crate::state::CapturaPedida {
                x: 0,
                y: 0,
                ancho: g.size.w,
                alto: g.size.h,
                guardar: true,
            });
            // Pedida como la pide el portal, que es la vía que no notifica:
            // si no, el aviso de «Captura guardada» saldría en la siguiente.
            let (emisario, _) = crate::portal::promesa();
            state.captura_portal = Some(emisario);
            state.needs_redraw = true;
            // El nombre del fichero lleva la hora al segundo: dos capturas en
            // el mismo segundo se pisarían.
            1100
        }
        ["fin"] => {
            state.loop_signal.stop();
            return;
        }
        _ => {
            tracing::error!(linea, "orden de guion desconocida");
            RESPIRO
        }
    };
    let programado = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(espera)),
        move |_, _, state| {
            paso_de_guion(state, pasos.clone(), i + 1);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = programado {
        tracing::error!("no se pudo programar el siguiente paso del guion: {err}");
    }
}

fn run(state: &mut BookosComp) {
    if let Ok(modo) = std::env::var("BOOKOS_SELFTEST_MULTITAREA") {
        if matches!(modo.as_str(), "x11" | "wayland") {
            if modo == "x11" {
                assert!(
                    state.display_x11.is_some(),
                    "Xwayland ha arrancado antes del cliente de prueba"
                );
            }
            let plataforma = if modo == "x11" { "xcb" } else { "wayland" };
            crate::keybinds::lanzar(
                state,
                &format!(
                    "env QT_QPA_PLATFORM={plataforma} python3 '{}/tests/clientes_multitarea.py'",
                    env!("CARGO_MANIFEST_DIR")
                ),
            );
            state
                .loop_handle
                .insert_source(
                    Timer::from_duration(Duration::from_secs(2)),
                    |_, _, state| {
                        comprobar_multitarea(state, 0);
                        TimeoutAction::Drop
                    },
                )
                .expect("temporizador de arranque de los clientes insertado");
        } else {
            comprobar_multitarea(state, 0);
        }
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ORDEN_DOCK").is_some() {
        comprobar_orden_dock(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_TOQUES_TECLADO").is_some() {
        comprobar_toques_teclado(state, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_SALVAPANTALLAS").is_some() {
        comprobar_salvapantallas(state);
        return;
    }
    if let Ok(ruta) = std::env::var("BOOKOS_SELFTEST_GUION") {
        guion(state, &ruta);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_MEJORAS").is_some() {
        use bookos_shell::{Accion, TeclaPulsada};
        for accion in [
            Accion::Bloquear,
            Accion::CerrarSesion,
            Accion::Lanzar("systemctl poweroff".into()),
            Accion::Lanzar("systemctl reboot".into()),
            Accion::Lanzar("systemctl suspend".into()),
        ] {
            crate::keybinds::hacer(state, accion);
            let shell = state.shell.as_mut().expect("shell");
            assert!(
                !shell.esta_bloqueado(),
                "una petición no debe ejecutarse sin confirmar"
            );
            assert_eq!(shell.emergente_nombre(), Some("apagar"));
            assert!(
                matches!(shell.tecla(TeclaPulsada::Intro), (true, None)),
                "Intro cancela por defecto"
            );
            assert!(shell.emergente_nombre().is_none());
        }
        let shell = state.shell.as_mut().unwrap();
        for tamano in [32, 80, 50] {
            shell.dock_tamano(tamano);
            let (_, _, _, alto) = shell.dock_rect();
            assert!((alto - (tamano + 25) as f64).abs() <= 4.0, "dock: {alto}");
        }
        tracing::info!("SELFTEST_MEJORAS_OK: confirmaciones cancelables y dock en vivo");
        state.loop_signal.stop();
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ORGANIZAR").is_some() {
        comprobar_organizacion(state, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BLOQUEO").is_some() {
        comprobar_bloqueo_pam(state);
        return;
    }
    tracing::info!("--- autotest de entrada ---");

    let escala = state.output_scale();
    let modo = state
        .space
        .outputs()
        .next()
        .and_then(|o| o.current_mode())
        .map(|m| m.size);
    tracing::info!(?modo, escala, "salida");

    comprobar_dock(state);
    if std::env::var_os("BOOKOS_SELFTEST_CARPETA").is_some() {
        comprobar_carpeta(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ANCLAR").is_some() {
        comprobar_anclar(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_MENU").is_some() {
        recorrer_menu(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_CONMUTADOR").is_some() {
        comprobar_conmutador(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ESCRITORIOS").is_some() {
        comprobar_escritorios(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BUSCADOR").is_some() {
        // Se abre y se le escribe una consulta por el mismo camino que el
        // teclado real, y se **queda abierto**: es la única forma de mirar el
        // cristal de debajo, que no existe fuera de una pantalla de verdad.
        let consulta = std::env::var("BOOKOS_SELFTEST_BUSCADOR").unwrap_or_default();
        if let Some(shell) = state.shell.as_mut() {
            shell.alternar_buscador();
            for c in consulta.chars().filter(|c| *c != '1') {
                shell.tecla(bookos_shell::TeclaPulsada::Caracter(c));
            }
        }
        state.needs_redraw = true;
        tracing::info!(consulta, "buscador abierto para mirarlo");
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_DECORACION").is_some() {
        comprobar_decoracion(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_VENTANAS").is_some() {
        comprobar_ventanas(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ENCIERRO").is_some() {
        comprobar_encierro(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ACTIVACION").is_some() {
        comprobar_activacion(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_COMPARTIR").is_some() {
        comprobar_compartir(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_OSD").is_some() {
        // El aviso que sale al tocar el volumen, por el mismo camino que la
        // tecla de función: `multimedia::ejecutar` es quien lo pide.
        use crate::multimedia::Tecla;
        let cual = std::env::var("BOOKOS_SELFTEST_OSD").unwrap_or_default();
        let tecla = match cual.as_str() {
            "brillo" => Tecla::Brillo(1),
            "teclado" => Tecla::BrilloTeclado(1),
            "touchpad" => Tecla::Touchpad,
            _ => Tecla::Volumen(1),
        };
        crate::multimedia::ejecutar(state, tecla);
        tracing::info!(
            ?tecla,
            vivo = state.shell.as_mut().is_some_and(|s| s.osd_vivo().0),
            queda = ?state.shell.as_ref().and_then(|s| s.osd_queda()),
            "aviso mostrado"
        );
        // Y se vuelve a mirar pasado su tiempo: lo que se comprueba de verdad
        // es que **se va solo**, que es donde estaba el fallo —la superficie se
        // quedaba puesta para siempre porque nadie la soltaba.
        let espera = std::time::Duration::from_millis(2_000);
        let _ = state.loop_handle.insert_source(
            Timer::from_duration(espera),
            |_, _, state: &mut BookosComp| {
                let vivo = state.shell.as_mut().is_some_and(|s| s.osd_vivo().0);
                if vivo {
                    tracing::error!("el aviso sigue puesto dos segundos después");
                } else {
                    tracing::info!("el aviso se ha ido solo, correcto");
                }
                TimeoutAction::Drop
            },
        );
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_CAPTURA").is_some() {
        comprobar_captura(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_APARIENCIA").is_some() {
        comprobar_apariencia(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_WIDGETS").is_some() {
        // Pulsa cada widget del panel por el camino de verdad —el hit-test del
        // compositor— y dice qué tarjeta se abrió. Es la comprobación que el
        // test del shell no puede hacer: allí se llama al shell directamente,
        // aquí pasa por las coordenadas de pantalla y la escala.
        for nombre in [
            "bateria",
            "bluetooth",
            "red",
            "volumen",
            "brillo",
            "notificaciones",
            "control",
            "reloj",
        ] {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            // **Sin cerrar la anterior a propósito.** Cambiar de tarjeta tiene
            // que costar un clic, no dos: mientras el clic fuera de la
            // emergente se gastaba en cerrarla, abrir un widget con otro ya
            // abierto no hacía nada la primera vez.
            let Some((x0, x1)) = shell.zona_widget(nombre) else {
                tracing::warn!(nombre, "ese widget no se dibuja en esta máquina");
                continue;
            };
            let centro = (x0 + x1) / 2.0;
            shell.pulsar(centro, 8.0);
            match state.shell.as_ref().and_then(|s| s.emergente_nombre()) {
                Some(abierta) => tracing::info!(nombre, abierta, "el widget abre su tarjeta"),
                None => tracing::error!(nombre, "el widget no abrió nada"),
            }
        }
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_RESIZE").is_some() {
        // Maximiza la primera ventana y va anotando el factor de escala: es la
        // única forma de comprobar que el cambio de tamaño se anima, porque en
        // Wayland puro no hay manera de grabar la pantalla desde un script.
        let Some(window) = state.space.elements().next().cloned() else {
            tracing::error!("no hay ventana que maximizar");
            return;
        };
        let antes = window.geometry().size;
        // A media pantalla y no a máxima: el cliente de prueba ya arranca
        // ocupando el área útil entera, y encajarlo donde ya está no cambia de
        // tamaño y no habría nada que animar.
        state.encajar(&window, crate::ventanas::Zona::Izquierda);
        tracing::info!(?antes, "maximizada");
        muestrear_resize(state, window, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BLOQUEO").is_some() {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
        tracing::info!(
            bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()),
            "tras pedir el bloqueo"
        );
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_LAUNCHPAD").is_some() {
        // Pulsa el primer icono del dock, que es el launchpad, por el mismo
        // camino que un clic: `pulsar` devuelve la acción y el compositor la
        // ejecuta. Así se comprueba de paso que el dock la emite bien.
        let centro = state.shell.as_ref().map(|s| {
            let (dx, dy, _, _) = s.dock_rect();
            (
                dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
                dy + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
            )
        });
        if let Some((x, y)) = centro {
            match state.shell.as_mut().and_then(|s| s.pulsar(x, y)) {
                Some(accion) => {
                    tracing::info!(?accion, "el primer icono del dock pide");
                    crate::keybinds::hacer(state, accion);
                }
                None => tracing::error!("el primer icono del dock no devuelve acción"),
            }
        }
        let abierto = state.shell.as_ref().is_some_and(|s| s.hay_emergente());
        tracing::info!(abierto, "launchpad tras pulsar su icono");
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BARRAS").is_some() {
        comprobar_barras(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_SONIDO").is_some() {
        // Pulsa el icono del volumen y deja la emergente abierta, para poder
        // capturarla: el hit-test se prueba en el shell, esto comprueba que la
        // tarjeta sale **bajo su icono** y no en un sitio fijo.
        let zona = state.shell.as_ref().and_then(|s| s.zona_widget("volumen"));
        match zona {
            Some((x0, x1)) => {
                let x = (x0 + x1) / 2.0;
                tracing::info!(x, "pulsando el icono del volumen");
                if let Some(shell) = state.shell.as_mut() {
                    shell.pulsar(x, 8.0);
                }
                state.needs_redraw = true;
            }
            None => tracing::error!("el volumen no está en el panel"),
        }
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BORDES").is_some() {
        comprobar_bordes(state, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_DOBLECLIC").is_some() {
        comprobar_doble_clic(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_CURSOR").is_some() {
        comprobar_cursor(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_COMPLETA").is_some() {
        comprobar_pantalla_completa(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_XWAYLAND").is_some() {
        comprobar_xwayland(state);
        return;
    }

    let ventanas: Vec<_> = state
        .space
        .elements()
        .map(|w| (w.bbox(), state.space.element_location(w)))
        .collect();
    tracing::info!(n = ventanas.len(), "ventanas en la escena");
    for (bbox, loc) in &ventanas {
        tracing::info!(?bbox, ?loc, "  ventana");
    }
    if ventanas.is_empty() {
        tracing::error!("no hay ninguna ventana mapeada: el clic no tendría destino");
        return;
    }

    // Centro de la primera ventana, en coordenadas lógicas.
    let (bbox, loc) = ventanas[0];
    let loc = loc.unwrap_or_default();
    let punto = (
        loc.x as f64 + bbox.size.w as f64 / 2.0,
        loc.y as f64 + bbox.size.h as f64 / 2.0,
    )
        .into();
    tracing::info!(?punto, "punto de prueba (centro de la primera ventana)");

    match state.surface_under(punto) {
        Some((_surface, pos)) => {
            tracing::info!(?pos, "superficie encontrada bajo el punto")
        }
        None => {
            tracing::error!(
                "NO hay superficie bajo el punto: element_under o surface_under fallan \
                 (mira el espacio de coordenadas)"
            );
            return;
        }
    }

    let focus = state.surface_under(punto);
    state.pointer_location = punto;
    let pointer = state.pointer.clone();
    pointer.motion(
        state,
        focus,
        &MotionEvent {
            location: punto,
            serial: SERIAL_COUNTER.next_serial(),
            time: 0,
        },
    );
    pointer.frame(state);
    tracing::info!(
        tiene_foco = pointer.current_focus().is_some(),
        "tras mover el puntero"
    );

    let serial = SERIAL_COUNTER.next_serial();
    pointer.button(
        state,
        &ButtonEvent {
            button: BTN_LEFT,
            state: smithay::backend::input::ButtonState::Pressed,
            serial,
            time: 0,
        },
    );
    pointer.frame(state);

    // Soltar el botón. Dejarlo pulsado deja al cliente creyendo que arrastras:
    // konsole abría su búsqueda y se quedaba seleccionando texto.
    pointer.button(
        state,
        &ButtonEvent {
            button: BTN_LEFT,
            state: smithay::backend::input::ButtonState::Released,
            serial: SERIAL_COUNTER.next_serial(),
            time: 1,
        },
    );
    pointer.frame(state);

    if let Some(kbd) = state.seat.get_keyboard() {
        let surface = state
            .space
            .elements()
            .next()
            .and_then(|w| w.wl_surface())
            .map(|s| s.into_owned());
        kbd.set_focus(state, surface, serial);
        tracing::info!(
            tiene_foco = kbd.current_focus().is_some(),
            "tras dar el foco de teclado"
        );
    }

    tracing::info!("--- fin del autotest ---");
}

/// Anota el factor de escala del redimensionado unas cuantas veces seguidas.
///
/// Con un solo vistazo no se distingue "anima" de "salta": lo que dice que hay
/// animación es la serie de valores intermedios entre el factor inicial y 1.
fn muestrear_resize(state: &mut BookosComp, window: smithay::desktop::Window, i: u32) {
    const PASOS: u32 = 12;
    let escala = crate::ventanas::escala_resize(&window, window.geometry().size);
    tracing::info!(
        i,
        escala_x = format_args!("{:.3}", escala.x),
        escala_y = format_args!("{:.3}", escala.y),
        tam = ?window.geometry().size,
        "escala del redimensionado"
    );
    if i >= PASOS {
        return;
    }
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(30)),
        move |_, _, state: &mut BookosComp| {
            state.needs_redraw = true;
            muestrear_resize(state, window.clone(), i + 1);
            TimeoutAction::Drop
        },
    );
}

/// El código evdev de una tecla del guion; xkb los numera 8 más arriba. Las
/// letras van por filas de un teclado QWERTY, que es donde están en evdev.
fn comprobar_toques_teclado(state: &mut BookosComp, paso: u8) {
    use smithay::backend::input::TouchSlot;
    use smithay::wayland::text_input::TextInputSeat;

    let slot = TouchSlot::from(Some(0));
    let abierto = |state: &BookosComp| state.shell.as_ref().is_some_and(|s| s.teclado_abierto());
    let ventana = state
        .space
        .elements()
        .next()
        .expect("cliente táctil mapeado");
    let rect = state
        .space
        .element_geometry(ventana)
        .expect("geometría del cliente táctil");
    let punto = (f64::from(rect.loc.x + 80), f64::from(rect.loc.y + 100)).into();
    let time = state.start_time.elapsed().as_millis() as u32;
    match paso {
        0 => {
            assert!(
                state.seat.text_input().campo_activo().is_some(),
                "cliente con text-input habilitado"
            );
            assert!(!abierto(state));
            crate::input::toque_abajo(state, slot, punto, time);
            crate::teclado::revisar(state);
            assert!(!abierto(state), "apoyar el dedo aún no abre el teclado");
        }
        1 => {
            assert!(
                !abierto(state),
                "el procesamiento del cliente tampoco abre al apoyar"
            );
            crate::input::toque_mueve(
                state,
                slot,
                punto + smithay::utils::Point::from((0.0, 80.0)),
                time,
            );
            crate::input::toque_arriba(state, slot, time + 1);
            crate::teclado::revisar(state);
            assert!(!abierto(state), "deslizar no abre el teclado");
        }
        2 => {
            assert!(!abierto(state), "el foco tras desplazar no abre el teclado");
            crate::input::toque_abajo(state, slot, punto, time);
        }
        3 => {
            assert!(
                !abierto(state),
                "el toque breve espera a que se levante el dedo"
            );
            crate::input::toque_arriba(state, slot, time);
        }
        4 => {
            assert!(abierto(state), "un toque completado abre el teclado");
            let salida = state.space.outputs().next().expect("salida de prueba");
            let g = state
                .space
                .output_geometry(salida)
                .expect("geometría de la salida");
            state.captura_pedida = Some(crate::state::CapturaPedida {
                x: 0,
                y: 0,
                ancho: g.size.w,
                alto: g.size.h,
                guardar: true,
            });
            let (emisario, _) = crate::portal::promesa();
            state.captura_portal = Some(emisario);
            state.needs_redraw = true;
        }
        _ => {
            tracing::info!(
                "autotest del teclado correcto: desplazamiento sin abrir y toque al soltar"
            );
            state.loop_signal.stop();
            return;
        }
    }
    if let Some(touch) = state.seat.get_touch() {
        touch.frame(state);
    }
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(if paso >= 3 { 500 } else { 150 })),
            move |_, _, state| {
                comprobar_toques_teclado(state, paso + 1);
                TimeoutAction::Drop
            },
        )
        .expect("temporizador del autotest del teclado insertado");
}

/// Bloqueo, salvapantallas encima y apagado, con plazos de milisegundos. La
/// atenuación no entra: en anidado escribiría el brillo real del portátil.
fn comprobar_escalones(state: &mut BookosComp) {
    state.aplicar_dpms = Some(Box::new(|_| Ok(())));
    state.salvapantallas.inactividad = Duration::from_millis(150);
    state.salvapantallas.apagado = Duration::from_millis(400);
    crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
    state.last_input = Some(std::time::Instant::now());
    crate::salvapantallas::programar(state);
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(280)),
            |_, _, state| {
                assert!(
                    state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()),
                    "sigue bloqueado"
                );
                assert!(
                    state.salvapantallas.activo(),
                    "el salvapantallas sale sobre el bloqueo"
                );
                TimeoutAction::Drop
            },
        )
        .expect("temporizador del autotest insertado");
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(600)),
            |_, _, state| {
                assert!(!state.dpms_encendido, "la inactividad apaga la pantalla");
                assert!(
                    !state.salvapantallas.activo(),
                    "con la pantalla apagada no queda animándose"
                );
                assert!(
                    !state.salvapantallas.pantalla_apagada(),
                    "no es el modo de Meta+Alt+O: la suspensión sigue contando"
                );
                tracing::info!(
                    "autotest del salvapantallas correcto: atajos, sueltas, DPMS, inactividad y escalones"
                );
                state.loop_signal.stop();
                TimeoutAction::Drop
            },
        )
        .expect("temporizador del autotest insertado");
}

fn comprobar_salvapantallas(state: &mut BookosComp) {
    use smithay::backend::input::KeyState;
    use smithay::input::keyboard::Keycode;

    let acorde = |state: &mut BookosComp, evdev| {
        for codigo in [125, 56, evdev] {
            crate::input::tecla(state, Keycode::new(codigo + 8), KeyState::Pressed, 1);
        }
        for codigo in [evdev, 56, 125] {
            crate::input::tecla(state, Keycode::new(codigo + 8), KeyState::Released, 2);
        }
    };
    acorde(state, 31);
    assert!(
        state.salvapantallas.activo(),
        "soltar Meta+Alt+S conserva el salvapantallas"
    );
    assert!(!state.abrir_launchpad, "soltar Meta no abre el launchpad");
    crate::input::tecla(state, Keycode::new(38 + 8), KeyState::Pressed, 3);
    crate::input::tecla(state, Keycode::new(38 + 8), KeyState::Released, 4);
    assert!(
        state.salvapantallas.activo(),
        "otra tecla no retira el salvapantallas"
    );
    assert!(
        !state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()),
        "la L tapada no bloquea"
    );
    acorde(state, 31);
    assert!(
        !state.salvapantallas.activo(),
        "Meta+Alt+S otra vez lo retira"
    );

    // El backend anidado no controla la energía del anfitrión. Este sustituto
    // comprueba el enrutado y los temporizadores sin apagar su pantalla real.
    state.aplicar_dpms = Some(Box::new(|_| Ok(())));
    acorde(state, 24);
    assert!(!state.dpms_encendido, "Meta+Alt+O apaga las salidas");
    assert!(state.salvapantallas.pantalla_apagada());
    assert!(state.tick_suspension.is_none());
    crate::backend::despertar_dpms(state);
    assert!(
        !state.dpms_encendido,
        "la suelta del atajo no enciende la pantalla"
    );
    state.dpms_sordo_hasta = None;
    crate::backend::despertar_dpms(state);
    crate::salvapantallas::actividad(state);
    assert!(state.dpms_encendido);
    assert!(!state.salvapantallas.pantalla_apagada());
    state.aplicar_dpms = None;

    state.salvapantallas.inactividad = Duration::from_millis(100);
    state.last_input = Some(std::time::Instant::now() - Duration::from_secs(1));
    crate::salvapantallas::programar(state);
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(150)),
            |_, _, state| {
                assert!(
                    state.salvapantallas.activo(),
                    "la inactividad activa el salvapantallas"
                );
                assert!(
                    crate::salvapantallas::actividad(state),
                    "la capa se traga el movimiento"
                );
                assert!(
                    !state.salvapantallas.activo(),
                    "el de inactividad se quita al moverse, como en una tele"
                );
                comprobar_escalones(state);
                TimeoutAction::Drop
            },
        )
        .expect("temporizador del autotest insertado");
}

fn codigo_tecla(nombre: &str) -> Option<u32> {
    if matches!(nombre, "." | "punto") {
        return Some(52);
    }
    const FILAS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    if let [letra] = nombre.as_bytes() {
        return FILAS.iter().find_map(|(fila, primero)| {
            fila.bytes()
                .position(|b| b == *letra)
                .map(|i| primero + i as u32)
        });
    }
    Some(match nombre {
        "esc" => 1,
        "retroceso" => 14,
        "tab" => 15,
        "intro" => 28,
        "ctrl" => 29,
        "shift" => 42,
        "alt" => 56,
        "espacio" => 57,
        "arriba" => 103,
        "izquierda" => 105,
        "derecha" => 106,
        "abajo" => 108,
        "supr" => 111,
        "meta" => 125,
        _ => return None,
    })
}

fn comprobar_multitarea(state: &mut BookosComp, paso: u32) {
    use crate::multitarea as m;
    use smithay::backend::input::ButtonState;
    let buscar = |titulo: &str| {
        state
            .space
            .elements()
            .chain(state.minimizadas.iter().map(|(w, _)| w))
            .chain(state.escritorios.apartadas())
            .find(|w| crate::handlers::titulo(w).as_deref() == Some(titulo))
            .cloned()
    };
    let editor = buscar("Editor de prueba").expect("el cliente de prueba ha abierto el editor");
    let referencia =
        buscar("Referencia de prueba").expect("el cliente de prueba ha abierto la referencia");
    let otra = buscar("Otra aplicación de prueba");
    let dialogo = buscar("Párrafo de prueba").expect("el cliente de prueba ha abierto el diálogo");
    match paso {
        0 => {
            let otra = otra
                .as_ref()
                .expect("el cliente de prueba ha abierto otra aplicación");
            assert_eq!(
                m::raiz(state, &dialogo),
                editor,
                "Qt declara el padre del diálogo"
            );
            assert!(
                !m::misma_unidad(state, &editor, &referencia),
                "dos documentos del mismo proceso son independientes"
            );
            state.encajar(&editor, crate::ventanas::Zona::Izquierda);
            state.encajar(&referencia, crate::ventanas::Zona::Derecha);
            assert!(
                m::misma_unidad(state, &editor, &referencia),
                "el encaje complementario forma un grupo"
            );
            state.enfocar(otra);
            state.enfocar(&editor);
            assert_eq!(
                state.ventana_con_foco(),
                Some(dialogo.clone()),
                "volver al editor recupera su diálogo"
            );
            crate::keybinds::ejecutar(
                state,
                crate::keybinds::Accion::Conmutar(bookos_shell::conmutador::Modo::Aplicaciones, 1),
            );
            assert_eq!(
                state.conmutador_destinos.len(),
                2,
                "el grupo y su diálogo ocupan una sola celda"
            );
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarCancelar);
        }
        1 => {
            let izq = state
                .space
                .element_geometry(&editor)
                .expect("editor mapeado");
            let der = state
                .space
                .element_geometry(&referencia)
                .expect("referencia mapeada");
            assert_eq!(izq.loc.x + izq.size.w + 8, der.loc.x);
            let punto = (der.loc.x as f64 - 4.0, der.loc.y as f64 + 80.0).into();
            crate::input::set_pointer(state, punto, 100);
            crate::input::boton(state, BTN_LEFT, ButtonState::Pressed, 101);
            crate::input::set_pointer(state, (punto.x + 100.0, punto.y).into(), 102);
            crate::input::boton(state, BTN_LEFT, ButtonState::Released, 103);
        }
        2 => {
            let izq = state
                .space
                .element_geometry(&editor)
                .expect("editor redimensionado");
            let der = state
                .space
                .element_geometry(&referencia)
                .expect("referencia redimensionada");
            assert_eq!(
                izq.loc.x + izq.size.w + 8,
                der.loc.x,
                "ambos clientes han aceptado el nuevo reparto"
            );
            let pila: Vec<_> = state.space.elements().collect();
            let posicion = |w: &smithay::desktop::Window| {
                pila.iter()
                    .position(|v| *v == w)
                    .expect("ventana mapeada en la pila")
            };
            assert!(
                posicion(&dialogo) > posicion(&editor)
                    && posicion(&dialogo) > posicion(&referencia),
                "redimensionar conserva los diálogos por encima de ambos paneles"
            );
            assert!(
                izq.to_f64().contains(
                    state
                        .space
                        .element_location(&dialogo)
                        .expect("diálogo colocado")
                        .to_f64()
                ),
                "el diálogo permanece en su panel"
            );
            assert!(
                izq.size.w > der.size.w,
                "el separador ha cambiado ambos anchos"
            );
            captura_multitarea(state);
        }
        3 => {
            m::cambiar_disposicion(state, &editor, true);
            assert!(!m::visible(state, &referencia));
            assert!(m::visible(state, &dialogo));
        }
        4 => {
            let rect =
                crate::decoracion::barra_rect(state, &editor).expect("barra de pestañas visible");
            let x = rect.loc.x as f64 + (rect.size.w as f64 - 102.0) * 0.75;
            let punto = (x, rect.loc.y as f64 + 16.0).into();
            crate::input::set_pointer(state, punto, 200);
            crate::input::boton(state, BTN_LEFT, ButtonState::Pressed, 201);
            crate::input::boton(state, BTN_LEFT, ButtonState::Released, 202);
            assert_eq!(state.ventana_con_foco(), Some(referencia.clone()));
            assert!(
                !m::visible(state, &editor) && !m::visible(state, &dialogo),
                "el diálogo se oculta con su pestaña"
            );
            captura_multitarea(state);
        }
        5 => crate::ventanas::minimizar(state, referencia.clone()),
        6 => {
            assert!(state.space.element_location(&editor).is_none());
            assert!(state.space.element_location(&referencia).is_none());
            assert!(state.space.element_location(&dialogo).is_none());
            let app_id = crate::handlers::app_id(&referencia)
                .expect("Qt declara el identificador de aplicación");
            assert!(crate::ventanas::restaurar_minimizada(state, &app_id));
            assert!(state.space.element_location(&editor).is_some());
            assert!(state.space.element_location(&referencia).is_some());
            assert!(state.space.element_location(&dialogo).is_some());
        }
        7 => {
            let otra = otra.as_ref().expect("la tercera aplicación sigue abierta");
            m::unir(state, otra, &referencia, true);
            assert!(
                m::misma_unidad(state, &editor, otra),
                "un grupo admite una tercera pestaña"
            );
            assert_eq!(m::pestanas(state, otra).0.len(), 3);
            if let Some(toplevel) = otra.toplevel() {
                toplevel.send_close();
            } else if let Some(x11) = otra.x11_surface() {
                x11.close()
                    .expect("el cliente X11 de prueba acepta la petición de cierre");
            }
        }
        8 => {
            assert!(otra.is_none(), "el cliente ha cerrado la pestaña activa");
            assert!(
                m::visible(state, &editor) || m::visible(state, &referencia),
                "cerrar una pestaña recupera otra"
            );
            crate::escritorios::mover_ventana(state, &referencia, 1);
            for w in [&editor, &referencia, &dialogo] {
                assert!(
                    state.space.element_location(w).is_none(),
                    "el grupo entero cambia de escritorio"
                );
            }
            crate::escritorios::cambiar_aqui(state, 1);
            crate::escritorios::terminar_todos(state);
            for w in [&editor, &referencia, &dialogo] {
                assert!(state.space.element_location(w).is_some());
            }
            m::separar(state, &referencia);
            assert!(!m::misma_unidad(state, &editor, &referencia));
            assert!(m::misma_unidad(state, &editor, &dialogo));
            m::unir(state, &editor, &referencia, false);
            state.arrastrar_ventana(&referencia, crate::ventanas::Modo::Mover, true);
            assert!(
                !m::misma_unidad(state, &editor, &referencia),
                "arrastrar un panel lo saca del grupo lado a lado"
            );
            state.soltar_arrastre();
            tracing::info!(
                "MULTITAREA_OK: encaje, separador, pestañas, diálogos, Alt+Tab, minimizar y escritorios"
            );
            state.loop_signal.stop();
            return;
        }
        _ => unreachable!("el autotest termina en el paso ocho"),
    }
    state
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(800)),
            move |_, _, state| {
                comprobar_multitarea(state, paso + 1);
                TimeoutAction::Drop
            },
        )
        .expect("temporizador de multitarea insertado");
}

fn captura_multitarea(state: &mut BookosComp) {
    let area = state.pantalla();
    state.captura_pedida = Some(crate::state::CapturaPedida {
        x: area.loc.x,
        y: area.loc.y,
        ancho: area.size.w,
        alto: area.size.h,
        guardar: true,
    });
    let (emisario, _) = crate::portal::promesa();
    state.captura_portal = Some(emisario);
    state.needs_redraw = true;
}
