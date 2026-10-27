//! Backends de salida y entrada.
//!
//! - `winit`: anidado dentro de otra sesión, para desarrollo.
//! - `udev`:  DRM/KMS atómico + libinput + libseat. El de producción.
//!
//! Aquí vive lo que comparten: cómo se arma la escena y cada cuánto despierta
//! el panel. Que la escena se construya en un solo sitio importa — es lo que
//! garantiza que lo que ves anidado es exactamente lo que verás en la sesión
//! real.

#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;

use smithay::backend::renderer::element::AsRenderElements;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{
    ConstrainAlign, ConstrainScaleBehavior, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::layer_map_for_output;
use smithay::desktop::space::{ConstrainBehavior, ConstrainReference, constrain_space_element};
use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Logical, Point, Rectangle, Scale};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::cursor::OverlayElement;
use crate::state::BookosComp;

/// Arma la lista de elementos de un frame, de arriba abajo: cursor, shell y
/// ventanas.
/// Ya no es genérica sobre el renderer: los dos backends componen con GLES y
/// el cristal necesita copiar el framebuffer, que es GL crudo. La genericidad
/// solo servía para un segundo renderer que no existe.
/// Avanza todo lo que se mueve solo: las animaciones del shell, el
/// deslizamiento entre escritorios y las ventanas que van o vuelven del dock.
///
/// Vive aparte de [`escena`] porque **`escena` se llama una vez por salida**, y
/// esto no es idempotente: `escritorios::animar` mueve ventanas dentro del
/// `Space` y varias de las `animar_*` del shell rasterizan un buffer con
/// tiny-skia. Con dos monitores eso avanzaba y repintaba dos veces por
/// fotograma, y además la segunda salida leía posiciones ya adelantadas: las
/// dos pantallas deslizaban desfasadas entre sí. Se llama una sola vez por
/// vuelta, antes de componer ninguna salida.
pub fn avanzar_animaciones(state: &mut BookosComp) {
    // Con el fondo tapado se para aquí y no solo en el temporizador: otra cosa
    // que se mueva —un vídeo a pantalla completa pide redibujo a 60 Hz— pasaría
    // por esta función, y el fondo avanzaría con ella.
    let fondo_visible = fondo_visible(state);
    if fondo_visible
        && state
            .fondo
            .as_mut()
            .is_some_and(crate::fondo::Fondo::avanzar)
    {
        state.fondo_cristal_pendiente = true;
        state.needs_redraw = true;
    }
    // La reanudación. Lo que destapa el fondo —salir de pantalla completa,
    // encender la pantalla— siempre pide un redibujo, y el redibujo pasa por
    // aquí: no hace falta un temporizador vigilando mientras está en pausa.
    if fondo_visible && state.tick_fondo.is_none() {
        programar_fondo_animado(state);
    }
    if !state.brillo_poll_activo && tarjeta_brillo_abierta(state) {
        state.brillo_poll_activo = true;
        let result = state.loop_handle.insert_source(
            Timer::from_duration(std::time::Duration::from_millis(500)),
            |_, _, state| {
                if !tarjeta_brillo_abierta(state) {
                    state.brillo_poll_activo = false;
                    return TimeoutAction::Drop;
                }
                if state.shell.as_mut().is_some_and(|s| s.refresh_emergente()) {
                    state.needs_redraw = true;
                }
                TimeoutAction::ToDuration(std::time::Duration::from_millis(500))
            },
        );
        if result.is_err() {
            state.brillo_poll_activo = false;
        }
    }
    let app_en_foco = state
        .seat
        .get_keyboard()
        .and_then(|k| k.current_focus())
        .and_then(|surface| state.window_for_surface(&surface))
        .and_then(|window| crate::handlers::app_id(&window));
    // Retirar el aviso caducado antes de componer: su superficie vive en el
    // compositor y nadie más la suelta. Sin esto el OSD se quedaba en pantalla
    // para siempre —el alfa llegaba a cero, pero el buffer seguía puesto y
    // volvía a verse en cuanto algo cambiaba la escena.
    if let Some(shell) = state.shell.as_mut() {
        let visible = shell.actividad_app_id().is_some_and(|actividad| {
            shell.actividad_clase() != Some(bookos_shell::actividad::Clase::Player)
                || !app_en_foco
                    .as_deref()
                    .is_some_and(|foco| ventana_de_actividad(actividad, foco))
        });
        if shell.actividad_visible(visible) {
            state.needs_redraw = true;
        }
        if shell.osd_vivo().1 {
            state.needs_redraw = true;
        }
        if shell.teclado_tick() {
            state.needs_redraw = true;
        }
        // Y lo mismo con el aviso de una notificación: su superficie vive aquí
        // y nadie más la suelta cuando se le acaba el tiempo.
        if shell.toasts_vivos().1 {
            state.needs_redraw = true;
        }
        shell.animar_toast();
        shell.animar_emergente();
        shell.animar_dock();
        shell.animar_panel();
        shell.animar_conmutador();
        shell.animar_osd();
        shell.animar_actividad();
        if shell.animar_bloqueo() {
            state.needs_redraw = true;
        }
    }
    // El deslizamiento entre escritorios mueve ventanas de verdad dentro del
    // `Space`, así que tiene que correr **antes** de recoger los elementos de
    // la escena: hacerlo después dejaría cada frame un paso por detrás.
    crate::escritorios::animar(state);
    // Y las que se están yendo al dock: cuando llegan, salen del `Space`.
    let minimizando = crate::ventanas::animar_minimizados(state);
    // El commit del shader tiene que cambiar tanto de ida como de vuelta. Antes
    // solo se incrementaba por `minimizando`; una restaurada ya no estaba en
    // esa lista y el damage tracker congelaba su primer fotograma junto al
    // icono, para saltar después directamente a la ventana abierta.
    let restaurando = state
        .space
        .elements()
        .any(|window| crate::ventanas::encogido(window).is_some());
    // La captura congelada del genio solo hace falta mientras la ventana está
    // recorriendo el embudo. Al terminar la apertura o la restauración,
    // `encogido` pasa a `None` y había que soltar esos ~20 MB de textura GPU
    // (además de evitar que un minimizado posterior reutilizara la foto vieja).
    if !state.capturas.is_empty() {
        use smithay::utils::IsAlive;
        state
            .capturas
            .retain(|(w, _)| w.alive() && crate::ventanas::encogido(w).is_some());
    }
    if minimizando || restaurando {
        state.genio_commit.increment();
    }
}

pub fn escena(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> Vec<OverlayElement> {
    if state.salvapantallas.activo() {
        return state.salvapantallas.elementos(renderer, output);
    }
    // Si se acaba de pasar de claro a oscuro, antes de pintar el tema nuevo se
    // vuelve un instante al viejo para fotografiar la pantalla tal como está.
    // Ver `crate::fundido`.
    if let Some((tema, acento)) = state.shell.as_mut().and_then(|s| s.tema_anterior.take()) {
        let nuevo = (
            bookos_shell::tema::actual(),
            bookos_shell::tema::acento_actual(),
        );
        if let Some(shell) = state.shell.as_mut() {
            shell.aplicar_apariencia(tema, acento);
        }
        let viejos = componer(state, renderer, output);
        state.fundido = crate::fundido::Fundido::fotografiar(renderer, &viejos, output);
        if let Some(shell) = state.shell.as_mut() {
            shell.aplicar_apariencia(nuevo.0, nuevo.1);
            // Reponer el nuevo lo vuelve a apuntar como un cambio: ya está
            // fotografiado.
            shell.tema_anterior = None;
        }
    }
    let mut elementos = componer(state, renderer, output);
    if state.fundido.as_ref().is_some_and(|f| f.terminado()) {
        state.fundido = None;
    }
    if let (Some(fundido), Some(contexto)) = (state.fundido.as_ref(), state.contexto_gl.as_ref()) {
        elementos.insert(0, OverlayElement::Textura(fundido.elemento(contexto)));
    }
    elementos
}

/// La escena de un fotograma, de delante hacia atrás.
fn componer(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> Vec<OverlayElement> {
    if std::mem::take(&mut state.fondo_cristal_pendiente)
        && let (Some(fondo), Some(cristal)) = (state.fondo.as_ref(), state.cristal.as_ref())
    {
        let (rgba, tam) = fondo.rgba();
        cristal.borrow_mut().preparar(renderer, rgba, tam);
        state.cristal_commit.increment();
    }
    let scale = output.current_scale().fractional_scale();
    let cursor_en = (
        state.pointer_location.x * scale,
        state.pointer_location.y * scale,
    )
        .into();

    // El cursor va delante: los elementos se apilan de delante hacia atrás, así
    // que el primero de la lista queda arriba del todo.
    let _t = std::time::Instant::now();
    let mut elementos = crate::cursor::elements(
        renderer,
        &state.cursor_status,
        state.cursor_theme.as_ref(),
        cursor_en,
        scale,
    );

    let t_cursor = _t.elapsed();
    let _t = std::time::Instant::now();
    // Con una ventana a pantalla completa no se dibujan el panel, el dock ni la
    // isla: un vídeo con el panel encima no está a pantalla completa, está
    // maximizado. Lo que se abre a propósito —captura, launchpad, emergentes,
    // OSD— sí sale encima, y el cursor también, que es lo que hace todo el
    // mundo.
    //
    // Salvo con el bloqueo echado: el bloqueo es parte del shell, y las
    // ventanas no se pintan mientras está puesto. Con un vídeo a pantalla
    // completa no quedaba nada que dibujar y el bloqueo salía en negro.
    let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    let completa = !bloqueado && state.hay_pantalla_completa_en(output);
    let vista_escritorios = state
        .shell
        .as_ref()
        .is_some_and(|s| s.vista_escritorios_abierta());

    // Capas externas por encima de las ventanas. El bloqueo siempre manda:
    // ninguna superficie de cliente puede cubrirlo.
    if !bloqueado {
        elementos.extend(elementos_capas(
            renderer,
            output,
            &[Layer::Overlay, Layer::Top],
        ));
    }

    // Meta+Tab reutiliza directamente las superficies de los clientes. No se
    // captura ningún bitmap ni se abre un temporizador: Smithay las escala con
    // la GPU y su damage normal mantiene vivas las miniaturas.
    //
    // Van **delante** de la tarjeta del conmutador, no detrás: con las celdas
    // detrás no podían tener fondo propio —cualquier relleno tapaba la ventana
    // viva— y una celda sin fondo es un marco vacío flotando.
    let mostrando_ventanas =
        state.conmutador_modo == Some(bookos_shell::conmutador::Modo::Ventanas);
    // Lo andado de la entrada de tres dedos, de 0 a 1. Las ventanas salen de
    // donde estaban y la tarjeta aparece con ellas: sin viaje, la exposición
    // brotaba de golpe y no se veía qué miniatura era qué ventana.
    let entrada = match (state.conmutador_salida, state.conmutador_entrada) {
        (Some(desde), _) => {
            1.0 - bookos_shell::tema::C_SUAVE.eval(bookos_shell::tema::avance(
                desde.elapsed(),
                bookos_shell::tema::D_VENTANA,
            )) as f64
        }
        (None, Some(desde))
            if state.conmutador_pegado && !bookos_shell::tema::efectos_reducidos() =>
        {
            bookos_shell::tema::C_SUAVE.eval(bookos_shell::tema::avance(
                desde.elapsed(),
                bookos_shell::tema::D_VENTANA,
            )) as f64
        }
        (None, _) => 1.0,
    };
    if mostrando_ventanas {
        let huecos = state
            .shell
            .as_ref()
            .map(|s| s.conmutador_miniaturas())
            .unwrap_or_default();
        let origen_salida = state
            .space
            .output_geometry(output)
            .map(|g| g.loc)
            .unwrap_or_default();
        for (window, hueco) in state.conmutador_destinos.iter().zip(huecos) {
            let hueco = match state.space.element_geometry(window) {
                Some(sitio) if entrada < 1.0 => {
                    let mezclar = |a: i32, b: i32| a + ((b - a) as f64 * entrada).round() as i32;
                    let desde = sitio.loc - origen_salida;
                    Rectangle::new(
                        (mezclar(desde.x, hueco.loc.x), mezclar(desde.y, hueco.loc.y)).into(),
                        (
                            mezclar(sitio.size.w, hueco.size.w),
                            mezclar(sitio.size.h, hueco.size.h),
                        )
                            .into(),
                    )
                }
                _ => hueco,
            };
            elementos.extend(constrain_space_element::<GlesRenderer, _, OverlayElement>(
                renderer,
                window,
                hueco.loc,
                1.0,
                Scale::from(scale),
                hueco,
                ConstrainBehavior {
                    reference: ConstrainReference::Geometry,
                    behavior: ConstrainScaleBehavior::Fit,
                    align: ConstrainAlign::CENTER,
                },
            ));
        }
    }

    // El recuadro de la elegida y su rótulo: detrás de las miniaturas —la
    // ventana viva tapa el relleno y deja ver el marco— y delante de la tarjeta.
    if let Some(shell) = state.shell.as_ref() {
        elementos.extend(
            shell
                .conmutador_extras(renderer, entrada as f32)
                .into_iter()
                .map(OverlayElement::Memory),
        );
    }

    // El selector es modal y se mantiene incluso sobre pantalla completa. Va
    // separado del panel/dock, que sí se ocultan en ese caso.
    if let Some(elemento) = state
        .shell
        .as_ref()
        .and_then(|shell| shell.conmutador_element(renderer, entrada as f32))
    {
        elementos.push(OverlayElement::Memory(elemento));
    }

    // La capa de captura, y detrás su velo: delante del resto del shell, que
    // queda atenuado por debajo igual que el escritorio.
    //
    // Y delante de la vista de escritorios: sus miniaturas y su rejilla se
    // componen aquí debajo, por delante de todo lo del shell, y con la captura
    // en su sitio de antes la barra de modos y el «guardar / copiar» quedaban
    // tapados por las ventanas.
    if let Some(shell) = state.shell.as_ref() {
        elementos.extend(
            shell
                .captura_elements(renderer)
                .into_iter()
                .map(OverlayElement::Memory),
        );
        match (state.velo_captura.as_mut(), shell.datos_velo_captura()) {
            (Some(velo), Some((pantalla, escala, marcado))) => {
                elementos.push(OverlayElement::Velo(
                    velo.elemento(pantalla, escala, marcado),
                ));
            }
            (None, Some(_)) => {
                elementos.extend(shell.velo_captura().into_iter().map(OverlayElement::Color));
            }
            (_, None) => {}
        }
    }

    // La isla, por lo mismo: desplegada cuelga sobre las miniaturas.
    if vista_escritorios && let Some(shell) = state.shell.as_ref() {
        elementos.extend(
            shell
                .elements(renderer, false, crate::shell::CapaShell::SoloIsla, scale)
                .into_iter()
                .map(OverlayElement::Memory),
        );
    }

    // La vista de escritorios compone las mismas superficies Wayland, no
    // capturas. También las apartadas conservan su buffer vivo, de modo que una
    // terminal que siga imprimiendo se actualiza dentro de su miniatura sin un
    // temporizador ni una copia por frame.
    //
    // Delante de la franja de la vista por lo mismo que el conmutador: es lo
    // que permite que cada previa enseñe el fondo de pantalla en vez de ser un
    // marco transparente.
    if vista_escritorios {
        let huecos = state
            .shell
            .as_ref()
            .map(|s| s.escritorios_miniaturas())
            .unwrap_or_default();
        // La vista general enseña los escritorios de **una** salida: la que
        // tiene el puntero, que es sobre la que van a actuar el clic y el
        // gesto. Mezclar las ventanas de los dos monitores en las mismas
        // miniaturas no describiría ningún estado que exista.
        let colocadas = crate::escritorios::ventanas_en_vista(state);
        let arrastrada = state
            .arrastre_vista
            .as_ref()
            .filter(|a| a.moviendo)
            .map(|a| (a.window.clone(), a.rect(state.pointer_location)));
        // La que se lleva el puntero va delante de todo lo de la vista, y sale
        // de su miniatura mientras tanto: verla en los dos sitios diría que se
        // está copiando, no moviendo.
        if let Some((window, destino)) = arrastrada.as_ref() {
            elementos.extend(constrain_space_element::<GlesRenderer, _, OverlayElement>(
                renderer,
                window,
                destino.loc,
                1.0,
                Scale::from(scale),
                *destino,
                ConstrainBehavior {
                    reference: ConstrainReference::Geometry,
                    behavior: ConstrainScaleBehavior::Fit,
                    align: ConstrainAlign::CENTER,
                },
            ));
        }
        for (i, hueco) in huecos.into_iter().enumerate() {
            // El orden original se conserva: la última ventana del Space es
            // la que está arriba, y los elementos se entregan de delante atrás.
            for (_, window, destino) in colocadas.iter().rev().filter(|(e, _, _)| *e == i) {
                if arrastrada.as_ref().is_some_and(|(w, _)| w == window) {
                    continue;
                }
                elementos.extend(constrain_space_element::<GlesRenderer, _, OverlayElement>(
                    renderer,
                    window,
                    destino.loc,
                    1.0,
                    Scale::from(scale),
                    *destino,
                    ConstrainBehavior {
                        reference: ConstrainReference::Geometry,
                        behavior: ConstrainScaleBehavior::Fit,
                        align: ConstrainAlign::CENTER,
                    },
                ));
            }
            // Y detrás de sus ventanas, el fondo de pantalla: es lo que hace
            // que un escritorio vacío se vea como un escritorio y no como un
            // agujero negro en la franja.
            if let Some(fondo) = state.fondo.as_ref()
                && let Some(elemento) = fondo.miniatura(
                    renderer,
                    i,
                    (hueco.loc.x as f64 * scale, hueco.loc.y as f64 * scale),
                    (hueco.size.w, hueco.size.h),
                )
            {
                elementos.push(OverlayElement::Memory(elemento));
            }
        }
    }

    // Las del escritorio actual, repartidas bajo la franja. Van fuera del
    // `if` de la vista porque también se pintan mientras se cierra, volviendo
    // a su sitio.
    let expuestas = crate::escritorios::expuestas_ahora(state, vista_escritorios);
    let arrastrada_vista = state
        .arrastre_vista
        .as_ref()
        .filter(|a| a.moviendo)
        .map(|a| a.window.clone());
    // El ✕ de la ventana bajo el puntero, delante de todo lo de la rejilla:
    // los elementos van de delante a atrás, así que entra el primero.
    if vista_escritorios
        && arrastrada_vista.is_none()
        && let Some((_, boton)) =
            crate::escritorios::con_cierre_visible(&expuestas, state.pointer_location)
    {
        let lado = (crate::escritorios::LADO_CERRAR as f64 * scale).round() as i32;
        let tema = bookos_shell::tema::actual();
        let señalado = boton.to_f64().contains(state.pointer_location);
        if state
            .boton_cerrar_vista
            .as_ref()
            .is_none_or(|(l, t, s, _)| (*l, *t, *s) != (lado, tema, señalado))
        {
            state.boton_cerrar_vista = crate::escritorios::imagen_cerrar(lado, señalado)
                .map(|b| (lado, tema, señalado, b));
        }
        if let Some((lado, _, _, buffer)) = state.boton_cerrar_vista.as_ref()
            && let Ok(elemento) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                boton.loc.to_f64().to_physical_precise_round(scale),
                buffer,
                None,
                Some(Rectangle::from_size((*lado as f64, *lado as f64).into())),
                Some(boton.size),
                Kind::Unspecified,
            )
        {
            elementos.push(OverlayElement::Memory(elemento));
        }
    }
    for (window, destino) in expuestas.iter().rev() {
        if arrastrada_vista.as_ref() == Some(window) {
            continue;
        }
        // Con su barra de título, a la misma escala: sin ella, una ventana
        // decorada por BookOS se veía sin nombre y no se sabía cuál era.
        if crate::decoracion::decorada(window)
            && let Some(id) = crate::decoracion::id(window)
        {
            let geo = window.geometry().size;
            let factor = destino.size.w as f64 / geo.w.max(1) as f64;
            let alto = (crate::decoracion::ALTO as f64 * factor).round() as i32;
            let esquina = Point::<i32, Logical>::from((destino.loc.x, destino.loc.y - alto))
                .to_f64()
                .to_physical_precise_round(scale);
            let estado = crate::decoracion::estado_de(state, window);
            if let Some(shell) = state.shell.as_mut()
                && let Some(barra) = shell.barra_ventana(renderer, id, geo.w, estado, esquina, 1.0)
            {
                elementos.push(OverlayElement::MemoriaEscalada(
                    RescaleRenderElement::from_element(
                        barra,
                        esquina.to_i32_round(),
                        Scale::from(factor),
                    ),
                ));
            }
        }
        elementos.extend(constrain_space_element::<GlesRenderer, _, OverlayElement>(
            renderer,
            window,
            destino.loc,
            1.0,
            Scale::from(scale),
            *destino,
            ConstrainBehavior {
                reference: ConstrainReference::Geometry,
                behavior: ConstrainScaleBehavior::Fit,
                align: ConstrainAlign::CENTER,
            },
        ));
    }

    elementos.extend(
        state
            .shell
            .as_ref()
            .map(|shell| {
                let capa = if vista_escritorios {
                    crate::shell::CapaShell::SinIsla
                } else {
                    crate::shell::CapaShell::Todo
                };
                shell.elements(renderer, completa && !vista_escritorios, capa, scale)
            })
            .unwrap_or_default()
            .into_iter()
            .map(OverlayElement::Memory),
    );

    // El bloqueo interactivo vive en la salida principal. Las demás deben
    // quedar cubiertas también: nunca se deja una ventana visible porque el
    // monitor se conectó después de bloquear. Al desbloquear desaparece en el
    // mismo frame que la superficie principal.
    if bloqueado && output.current_location() != (0, 0).into() {
        static BLOQUEO_SECUNDARIO: std::sync::OnceLock<Id> = std::sync::OnceLock::new();
        let escala = output.current_scale().fractional_scale();
        let origen = output
            .current_location()
            .to_f64()
            .to_physical_precise_round(escala);
        let tamano = output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1, 1).into());
        elementos.push(OverlayElement::Color(SolidColorRenderElement::new(
            BLOQUEO_SECUNDARIO.get_or_init(Id::new).clone(),
            Rectangle::new(origen, tamano),
            0,
            [0.015, 0.02, 0.03, 1.0],
            Kind::Unspecified,
        )));
    }

    // El velo del conmutador va detrás del panel y del dock —que siguen
    // legibles— y delante de las ventanas del escritorio.
    if mostrando_ventanas {
        static VELO_CONMUTADOR: std::sync::OnceLock<Id> = std::sync::OnceLock::new();
        let (w, h) = output
            .current_mode()
            .map(|m| (m.size.w, m.size.h))
            .unwrap_or((1, 1));
        elementos.push(OverlayElement::Color(SolidColorRenderElement::new(
            VELO_CONMUTADOR.get_or_init(Id::new).clone(),
            Rectangle::new((0, 0).into(), (w, h).into()),
            0,
            [0.0, 0.0, 0.0, 0.56],
            Kind::Unspecified,
        )));
    }
    let t_shell = _t.elapsed();
    let _t = std::time::Instant::now();

    // La vista previa del encaje va delante de las ventanas —tiene que verse
    // sobre la que estás arrastrando— pero detrás del panel y del dock, que
    // mandan siempre.
    let area = state.work_area();
    elementos.extend(
        state
            .arrastre
            .as_ref()
            .map(|a| a.previa(area, scale))
            .unwrap_or_default()
            .into_iter()
            .map(OverlayElement::Color),
    );

    // El velo de la emergente va detrás del shell y delante de las ventanas: es
    // lo que hace que el escritorio se vea apagado por debajo sin que el shell
    // tenga que rasterizar la pantalla entera en CPU.
    elementos.extend(
        state
            .shell
            .as_ref()
            .and_then(|shell| shell.velo())
            .map(OverlayElement::Color),
    );

    // El cristal va detrás del panel, del dock y del velo, y delante de las
    // ventanas: se dibuja cuando estas ya están en el framebuffer, que es de
    // donde copia lo que desenfoca.
    //
    // Detrás del velo y no delante porque el launchpad desenfoca la pantalla
    // entera: con el cristal por delante, el velo quedaba tapado y el
    // escritorio se veía nítido y sin apagar debajo de los iconos.
    //
    // El commit sube en cada frame a propósito. El desenfoque depende de lo que
    // haya debajo, así que cualquier cambio en la escena lo invalida; llevar la
    // cuenta de qué zonas de debajo cambiaron sería lo suyo, pero hoy el
    // compositor solo dibuja cuando algo cambió, y en reposo no dibuja nada.
    let (ancho_pantalla, alto_pantalla) = output
        .current_mode()
        .map(|m| (m.size.w, m.size.h))
        .unwrap_or((1, 1));
    // Con los efectos reducidos no hay cristal: el desenfoque es lo más caro
    // que dibuja este compositor —copia el framebuffer y lo vuelve a muestrear
    // por cada zona— y el panel y el dock se ven igual de bien con su color
    // plano, que es lo que hacían antes de que existiera.
    if !bookos_shell::tema::efectos_reducidos()
        && let (Some(cristal), Some(shell)) = (state.cristal.clone(), state.shell.as_ref())
    {
        let zonas = shell.zonas_cristal(completa && !vista_escritorios);
        // El commit **no** sube por frame: el cristal desenfoca el fondo,
        // que no cambia. Solo se invalida cuando la barra se mueve, y de eso
        // se encarga el propio rectángulo del elemento.
        // Sin fondo subido no hay nada que desenfocar: el panel y el dock
        // se dibujan con su color, que es lo que hacían antes del cristal.
        if let Some((textura, fondo_tam)) = cristal.borrow().textura() {
            let pantalla = (ancho_pantalla, alto_pantalla);
            for (i, (rect, radio, fuerza)) in zonas.into_iter().enumerate() {
                elementos.push(OverlayElement::Cristal(crate::desenfoque::Desenfoque::new(
                    id_cristal(i),
                    state.cristal_commit,
                    rect,
                    radio,
                    fondo_tam,
                    pantalla,
                    textura.clone(),
                    cristal.clone(),
                    fuerza,
                )));
            }
        }
    }

    // Con el bloqueo echado no se dibuja ninguna ventana. No es cosmética: si
    // se dibujan, el escritorio se ve por debajo de la pantalla de bloqueo y
    // esta deja de proteger nada. Se comprobó en pantalla — konsole se veía
    // entera detrás del reloj.

    // Las ventanas que se están cerrando, encima de las demás: es la última
    // imagen de algo que ya no está, y lo que tapaba se ve a través mientras se
    // desvanece.
    state.cierres.retain(|c| !c.terminado());
    if let Some(contexto) = state.contexto_gl.clone().filter(|_| !bloqueado) {
        for cierre in &state.cierres {
            if let (Some(barra), Some(ahora), Some(shell)) = (
                cierre.barra.as_ref(),
                cierre.barra_ahora(scale),
                state.shell.as_mut(),
            ) && let Some(elemento) = shell.barra_ventana(
                renderer,
                barra.id,
                barra.ancho,
                barra.estado.clone(),
                ahora.origen,
                ahora.alfa,
            ) {
                elementos.push(OverlayElement::MemoriaEscalada(
                    RescaleRenderElement::from_element(
                        elemento,
                        ahora.centro,
                        Scale::from(ahora.zoom),
                    ),
                ));
            }
            elementos.extend(
                cierre
                    .elementos(&contexto, scale)
                    .into_iter()
                    .map(OverlayElement::Textura),
            );
        }
    }

    // Y por último las ventanas, que quedan debajo de todo lo anterior.
    //
    // Se generan a mano en vez de con `space_render_elements` para no arrastrar
    // el tipo envuelto `SpaceRenderElements` al enum: aquí solo queremos
    // superficies. `elements()` va de abajo arriba, así que se recorre al revés.
    let ventanas: Vec<_> = if bloqueado {
        Vec::new()
    } else {
        state
            .space
            .elements()
            .rev()
            .filter(|w| crate::multitarea::visible(state, w))
            .filter_map(|w| state.space.element_location(w).map(|loc| (w.clone(), loc)))
            .collect()
    };
    // Al desbloquear, las ventanas entran a la vez que el bloqueo se va: antes
    // aparecían enteras en el primer fotograma y solo el reloj y el campo se
    // desvanecían encima, así que el escritorio saltaba a la vista de golpe.
    // Crecen un 4 % desde su centro, lo justo para que parezca que el
    // escritorio se acerca en vez de encenderse.
    let desbloqueo = state.shell.as_ref().and_then(|s| s.progreso_desbloqueo());
    for (window, loc) in ventanas {
        // Las que están en la rejilla de la vista general, o volviendo de
        // ella, ya se han pintado allí. Pintarlas también aquí las duplicaba;
        // con la vista abierta no se notaba solo porque comparten id con su
        // copia y el damage tracker no volvía a pintar su sitio, que es
        // casualidad y no garantía.
        if expuestas.iter().any(|(w, _)| *w == window) {
            continue;
        }
        // Una ventana que se va al dock —o que vuelve— manda sobre las demás
        // animaciones: su recorrido, su tamaño y su desvanecido salen de un
        // solo sitio, y mezclarlo con el zoom de entrada daría dos escalas
        // multiplicándose.
        let encogido = crate::ventanas::encogido(&window);
        // Con shader, el minimizar es el «magic lamp»: la ventana se congela en
        // una textura la primera vez y a partir de ahí la dibuja el genio, no
        // sus superficies. Sin shader se sigue por el camino de abajo, que la
        // encoge sin deformarla.
        if let (Some(e), Some(genio), true) = (encogido, state.genio.as_ref(), state.genio_activo) {
            if !state.capturas.iter().any(|(w, _)| *w == window)
                && let Some(captura) = crate::genio::capturar(renderer, &window, scale)
            {
                state.capturas.push((window.clone(), captura));
            }
            if let Some((_, captura)) = state.capturas.iter().find(|(w, _)| *w == window) {
                elementos.push(OverlayElement::Genio(crate::genio::Elemento::new(
                    captura,
                    genio,
                    state.genio_commit,
                    e.origen,
                    e.destino,
                    crate::ventanas::progreso_encogido(e),
                    scale,
                )));
                continue;
            }
        }
        let (alfa, escala_ventana, zoom_entrada, redimensionando) = match encogido {
            Some(e) => {
                let (_, escala, alfa) = crate::ventanas::encogido_ahora(e);
                (alfa, Scale::from(escala), escala, false)
            }
            None => {
                let (alfa, zoom) = crate::ventanas::animacion(&window);
                let (alfa, zoom) = match desbloqueo {
                    Some(p) => (alfa * p, zoom * (0.96 + 0.04 * f64::from(p))),
                    None => (alfa, zoom),
                };
                // Al maximizar o encajar, el buffer nuevo entra escalado desde
                // el tamaño viejo: es la parte de la animación que faltaba,
                // porque `posicion` solo movía la esquina y el tamaño cambiaba
                // de golpe.
                let resize = crate::ventanas::escala_resize(&window, window.geometry().size);
                let redimensionando = resize.x != 1.0 || resize.y != 1.0;
                (
                    alfa,
                    Scale::from((zoom * resize.x, zoom * resize.y)),
                    zoom,
                    redimensionando,
                )
            }
        };
        // Una ventana que aún no tiene sitio no se dibuja. Enseñarla mientras
        // tanto significa un fotograma con el cliente pegado a la esquina
        // superior izquierda antes de saltar al centro, y ese salto se ve.
        if alfa <= 0.0 {
            continue;
        }
        // La posición de dibujo descuenta el marco del cliente: `geometry().loc`
        // es el hueco entre el borde del buffer y la ventana visible (sombras).
        let animada = match encogido {
            Some(e) => crate::ventanas::encogido_ahora(e).0,
            None => crate::ventanas::posicion(&window, loc),
        };
        let origen = (animada - window.geometry().loc.to_f64()).to_physical_precise_round(scale);
        let superficies = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            origen,
            Scale::from(scale),
            alfa,
        );

        // El centro de la ventana, que es desde donde crece el zoom de entrada.
        // Se calcula antes que nada porque la barra de título comparte esa
        // animación: escalarla desde su propio centro la separaría de la
        // ventana a la que va pegada.
        let geo = window.geometry().size;
        let centro = Point::<f64, Logical>::from((
            animada.x + geo.w as f64 / 2.0,
            animada.y + geo.h as f64 / 2.0,
        ))
        .to_physical_precise_round(scale);
        // Un resize se ancla en la esquina interpolada: así cada borde ocupa
        // exactamente el rectángulo intermedio. La entrada de una ventana sí
        // nace desde el centro, que es un gesto distinto.
        let ancla_ventana = if redimensionando {
            Point::<f64, Logical>::from((animada.x, animada.y)).to_physical_precise_round(scale)
        } else {
            centro
        };

        if encogido.is_none()
            && let Some((id, rect)) = crate::multitarea::separador_de(state, &window)
        {
            let color = bookos_shell::tema::card();
            elementos.push(OverlayElement::Color(SolidColorRenderElement::new(
                id,
                Rectangle::new(
                    rect.loc.to_f64().to_physical_precise_round(scale),
                    rect.size.to_f64().to_physical_precise_round(scale),
                ),
                0,
                [color.r, color.g, color.b, 1.0],
                Kind::Unspecified,
            )));
        }

        // La barra va **delante de su ventana y detrás de las de encima**, así
        // que se emite aquí dentro y no con el resto del shell: en
        // `custom_elements` la barra de una ventana tapada se dibujaría sobre la
        // que tiene delante.
        if crate::decoracion::decorada(&window) {
            let barra = crate::decoracion::estado_de(state, &window);
            let id = crate::decoracion::id(&window);
            let origen_barra = Point::<f64, Logical>::from((
                animada.x,
                animada.y - crate::decoracion::ALTO as f64,
            ))
            .to_physical_precise_round(scale);
            if let (Some(id), Some(shell)) = (id, state.shell.as_mut())
                && let Some(elemento) =
                    shell.barra_ventana(renderer, id, geo.w, barra, origen_barra, alfa)
            {
                // La barra acompaña el ancho, pero mantiene sus 32 px de
                // alto durante el resize; escalarla en Y haría que los
                // botones engordasen y se separasen del cliente.
                let escala_barra = if redimensionando {
                    Scale::from((escala_ventana.x, zoom_entrada))
                } else {
                    escala_ventana
                };
                let ancla_barra = if redimensionando {
                    origen_barra.to_i32_round()
                } else {
                    centro
                };
                elementos.push(if escala_barra.x == 1.0 && escala_barra.y == 1.0 {
                    OverlayElement::Memory(elemento)
                } else {
                    OverlayElement::MemoriaEscalada(RescaleRenderElement::from_element(
                        elemento,
                        ancla_barra,
                        escala_barra,
                    ))
                });
            }
        }

        if escala_ventana.x == 1.0 && escala_ventana.y == 1.0 {
            elementos.extend(superficies.into_iter().map(OverlayElement::Surface));
            continue;
        }
        if std::env::var_os("BOOKOS_PERFIL").is_some() {
            // La animación de una ventana no se puede capturar con las
            // herramientas del sistema —en Wayland puro no hay `import -window
            // root`—, así que la forma de comprobar que de verdad se dibujan
            // fotogramas intermedios es verlos pasar aquí.
            tracing::info!(
                alfa = format_args!("{alfa:.2}"),
                escala_x = format_args!("{:.3}", escala_ventana.x),
                escala_y = format_args!("{:.3}", escala_ventana.y),
                "fotograma de entrada"
            );
        }
        // La entrada crece desde el centro; el resize, desde su esquina visible
        // interpolada. `ancla_ventana` elige uno u otro sin cambiar el tipo de
        // elemento que compone la GPU.
        elementos.extend(superficies.into_iter().map(|elemento| {
            OverlayElement::Escalada(RescaleRenderElement::from_element(
                elemento,
                ancla_ventana,
                escala_ventana,
            ))
        }));
    }

    // Las apartadas por «mostrar escritorio», asomando por el borde y detrás
    // de todo lo que se abra mientras tanto. Su superficie sigue viva aunque
    // no estén en el `Space`, igual que en la vista de escritorios.
    if !bloqueado && !completa && !vista_escritorios {
        for (window, loc) in crate::escritorios::asomadas(state, &output.name()) {
            let origen =
                (loc.to_f64() - window.geometry().loc.to_f64()).to_physical_precise_round(scale);
            if crate::decoracion::decorada(&window) {
                let barra = crate::decoracion::estado_de(state, &window);
                let origen_barra = Point::<f64, Logical>::from((
                    loc.x as f64,
                    (loc.y - crate::decoracion::ALTO) as f64,
                ))
                .to_physical_precise_round(scale);
                if let (Some(id), Some(shell)) =
                    (crate::decoracion::id(&window), state.shell.as_mut())
                    && let Some(elemento) = shell.barra_ventana(
                        renderer,
                        id,
                        window.geometry().size.w,
                        barra,
                        origen_barra,
                        1.0,
                    )
                {
                    elementos.push(OverlayElement::Memory(elemento));
                }
            }
            elementos.extend(
                window
                    .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                        renderer,
                        origen,
                        Scale::from(scale),
                        1.0,
                    )
                    .into_iter()
                    .map(OverlayElement::Surface),
            );
        }
    }

    if !bloqueado {
        elementos.extend(elementos_capas(renderer, output, &[Layer::Bottom]));
    }

    // Los iconos del escritorio, entre el fondo y las ventanas: son parte del
    // escritorio, no de lo que va por encima. Con el bloqueo echado no se
    // dibujan por lo mismo que las ventanas — enseñarían qué hay en la carpeta
    // de quien no ha desbloqueado todavía.
    //
    // La banda elástica va delante de ellos: al barrer se ve el filo cruzando
    // por encima de los nombres, no por debajo.
    if !bloqueado && !completa {
        if let Some(shell) = state.shell.as_ref() {
            match (state.banda.as_mut(), shell.banda()) {
                (Some(banda), Some((rect, escala))) => {
                    elementos.push(OverlayElement::Velo(banda.elemento(rect, escala)));
                }
                (None, Some(_)) => elementos.extend(
                    shell
                        .banda_elementos()
                        .into_iter()
                        .map(OverlayElement::Color),
                ),
                (_, None) => {}
            }
        }
        let iconos = state
            .shell
            .as_ref()
            .map(|shell| shell.elementos_escritorio(renderer))
            .unwrap_or_default();
        elementos.extend(iconos.into_iter().map(OverlayElement::Memory));
    }

    if !bloqueado {
        elementos.extend(elementos_capas(renderer, output, &[Layer::Background]));
    }

    // El fondo va el último de la lista, o sea el más atrás de todo: detrás de
    // las ventanas, del shell y del cristal.
    // El avance del fundido se pide **antes** del `if`: es también quien suelta
    // el fondo saliente al terminar, y dentro del `if` no correría en el caso en
    // que no hay fondo nuevo.
    let fundido = avance_fundido(state);
    if let Some(fondo) = state.fondo.as_ref() {
        let geo = state
            .space
            .output_geometry(output)
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1, 1).into()));
        let (lw, lh) = (geo.size.w, geo.size.h);
        // Mientras se cambia de escritorio el fondo no se queda quieto: se
        // agranda un poco y se corre en sentido contrario a la vista. Fuera de
        // la transición esto devuelve la posición y el tamaño de siempre.
        // Al cambiar de escritorio el fondo se desliza con las ventanas, a la
        // misma velocidad: la pantalla entera es una tira que se corre de lado.
        // Son dos porque el que se va deja el borde al descubierto y detrás
        // está el del escritorio al que se llega, pegado a él.
        // La tira es la de **esta** salida: con un cambio en marcha en un solo
        // monitor, el fondo del otro no se mueve.
        let (saliente, entrante) = match state.escritorios.tira_fondo(&output.name()) {
            Some((s, e)) => (s, Some(e)),
            None => (0, None),
        };
        if let Some(x) = entrante
            && let Some(elemento) =
                fondo.elemento_gemelo_en(renderer, (geo.loc.x + x, geo.loc.y), (lw, lh))
        {
            elementos.push(OverlayElement::Memory(elemento));
        }
        if let Some(elemento) = fondo.elemento_con_alfa(
            renderer,
            (geo.loc.x + saliente, geo.loc.y),
            (lw, lh),
            // Mientras dura el cambio de imagen, el fondo nuevo entra
            // apareciendo por encima del que se va. Fuera de la transición
            // esto es 1,0 y no cuesta nada.
            fundido.unwrap_or(1.0),
        ) {
            elementos.push(OverlayElement::Memory(elemento));
        }
    }

    // Y debajo de todo, el fondo que se está yendo. Va el último de la lista
    // —los elementos se apilan de delante hacia atrás— para que el nuevo se
    // mezcle **sobre** él y no al revés.
    if fundido.is_some()
        && let Some((saliente, _)) = state.fondo_saliente.as_ref()
    {
        let geo = state
            .space
            .output_geometry(output)
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1, 1).into()));
        // Su buffer propio, no el gemelo: son dos `Fondo` distintos, así
        // que los identificadores ya son distintos y el seguimiento de daño
        // los ve como dos rectángulos. Pedir el gemelo aquí sería una
        // segunda copia de veinte megas para nada.
        if let Some(elemento) =
            saliente.elemento_en(renderer, (geo.loc.x, geo.loc.y), (geo.size.w, geo.size.h))
        {
            elementos.push(OverlayElement::Memory(elemento));
        }
    }

    if std::env::var_os("BOOKOS_PERFIL").is_some() {
        let t_ventanas = _t.elapsed();
        if t_cursor.as_millis() > 1 || t_shell.as_millis() > 1 || t_ventanas.as_millis() > 1 {
            tracing::info!(
                cursor_ms = format_args!("{:.2}", t_cursor.as_secs_f64() * 1000.0),
                shell_ms = format_args!("{:.2}", t_shell.as_secs_f64() * 1000.0),
                ventanas_ms = format_args!("{:.2}", t_ventanas.as_secs_f64() * 1000.0),
                "escena lenta"
            );
        }
    }
    elementos
}

/// Convierte las layer-surfaces de una salida en elementos GLES. La posición
/// se expresa en el escritorio global; KMS traslada luego toda la escena al
/// origen local del CRTC correspondiente.
fn elementos_capas(
    renderer: &mut GlesRenderer,
    output: &Output,
    capas: &[Layer],
) -> Vec<OverlayElement> {
    let scale = output.current_scale().fractional_scale();
    let origen_salida = output.current_location();
    let map = layer_map_for_output(output);
    capas
        .iter()
        .flat_map(|capa| map.layers_on(*capa).rev())
        .filter_map(|surface| map.layer_geometry(surface).map(|geo| (surface, geo.loc)))
        .flat_map(|(surface, loc)| {
            surface.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                (loc + origen_salida)
                    .to_f64()
                    .to_physical_precise_round(scale),
                Scale::from(scale),
                1.0,
            )
        })
        .map(OverlayElement::Surface)
        .collect()
}

/// Entrega el reloj de frame también a paneles y fondos layer-shell. Sin este
/// callback la primera imagen aparece, pero el cliente queda esperando para
/// siempre antes de producir la siguiente.
pub fn enviar_frames_capas(
    output: &Output,
    tiempo: std::time::Duration,
    periodo: std::time::Duration,
) {
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.send_frame(
            output,
            tiempo,
            Some(periodo),
            smithay::desktop::utils::surface_primary_scanout_output,
        );
    }
}

/// El identificador estable de cada zona de cristal.
///
/// Tiene que ser el mismo de un frame al siguiente o el damage tracker cree que
/// cada frame trae elementos nuevos y repinta la pantalla entera.
fn id_cristal(i: usize) -> smithay::backend::renderer::element::Id {
    use smithay::backend::renderer::element::Id;
    use std::sync::OnceLock;
    static IDS: OnceLock<[Id; 2]> = OnceLock::new();
    IDS.get_or_init(|| std::array::from_fn(|_| Id::new()))[i.min(1)].clone()
}

/// A cuántos puntos por pulgada se ve un escritorio del tamaño correcto.
///
/// No son los 96 de la convención de X11, y por eso hay una constante y no una
/// división directa: 96 sale de los monitores CRT de los noventa y en un panel
/// moderno da escalas disparatadas. Esta pantalla —2880×1800 en 300×190 mm, o
/// sea 242 DPI— pide un 175 %, que es lo que se acaba eligiendo a mano en
/// cualquier escritorio; 242/1,75 = 138, de ahí el número.
///
/// Comprobado contra los casos que importan, y sale lo que la gente elige:
/// 1080p en 15,6" → 1,0; 1440p en 27" → 1,0; 4K en 27" → 1,25; 4K en 15,6" → 2,0.
const DPI_OBJETIVO: f64 = 138.0;

/// La escala que le toca a una pantalla por su tamaño físico.
///
/// Devuelve 1,0 cuando el monitor no dice cuánto mide —muchos proyectores y
/// algunas KVM mandan 0×0— porque inventar una escala a partir de un dato que
/// no existe es peor que quedarse en la que siempre funciona.
/// Sirve las copias de pantalla pendientes de esta salida y, si hay alguna
/// sesión compartiendo esta pantalla, le manda su fotograma.
///
/// Los dos consumidores salen del **mismo** composite: `zwlr_screencopy` y el
/// portal de PipeWire piden lo mismo —la salida entera, ya compuesta— y hacerlo
/// dos veces era pagar dos veces por el mismo píxel.
///
/// ## Se compone aparte, no se lee del fotograma que se acaba de enseñar
///
/// Lo primero que probé fue leer directamente del framebuffer recién dibujado,
/// que es lo barato: la imagen ya está en la GPU. **No vale.** `copy_framebuffer`
/// deja el contexto donde quiere —Smithay lo avisa: «may change or invalidate
/// the current bind»—, y medido, cada captura tumbaba la superficie EGL del
/// backend anidado: `BAD_SURFACE` en `eglSwapBuffers` y el contexto perdido,
/// una vez por captura y de forma reproducible. El compositor se recuperaba,
/// pero perdiendo un fotograma cada vez.
///
/// Así que la captura compone la escena **otra vez**, en su propio buffer, y se
/// hace después de presentar. Cuesta un segundo composite por captura: para una
/// captura de pantalla no se nota, y para compartir pantalla es el doble de
/// trabajo de composición, que se podrá quitar cuando la copia salga por dmabuf
/// en vez de por memoria compartida. A cambio, el camino es **el mismo en los
/// dos backends** y no toca nada de lo que hay dibujando.
pub fn servir_capturas(state: &mut BookosComp, renderer: &mut GlesRenderer, output: &Output) {
    use smithay::backend::renderer::damage::OutputDamageTracker;
    use smithay::backend::renderer::{ExportMem, Offscreen};

    // Solo las de esta salida; las de otra esperan a que le toque dibujar.
    let mias: Vec<usize> = state
        .capturas_pantalla
        .iter()
        .enumerate()
        .filter(|(_, p)| p.output == *output)
        .map(|(i, _)| i)
        .collect();
    let ahora = std::time::Instant::now();
    let emisiones = state.emisiones.tocan(output, ahora);
    if mias.is_empty() && emisiones.is_empty() {
        return;
    }

    let tamano = crate::captura::tamano_de(output);
    let mut destino =
        match Offscreen::<smithay::backend::renderer::gles::GlesRenderbuffer>::create_buffer(
            renderer,
            smithay::backend::allocator::Fourcc::Abgr8888,
            tamano,
        ) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!("no se pudo crear el buffer de la captura: {err}");
                fallar(state, &mias);
                return;
            }
        };
    // El buffer donde se compone va en `Abgr8888` —es el formato natural del
    // renderizador—; la conversión al que espera el cliente la hace
    // `copy_framebuffer` al leerlo.

    let elementos = escena(state, renderer, output);
    let mut seguimiento = OutputDamageTracker::from_output(output);
    let mut framebuffer = match smithay::backend::renderer::Bind::bind(renderer, &mut destino) {
        Ok(fb) => fb,
        Err(err) => {
            tracing::warn!("no se pudo apuntar a la captura: {err}");
            fallar(state, &mias);
            return;
        }
    };
    // `age = 0` es un redibujo completo, que es justo lo que hace falta: el
    // buffer es nuevo y no tiene nada de antes que reaprovechar.
    if let Err(err) = seguimiento.render_output(
        renderer,
        &mut framebuffer,
        0,
        &elementos,
        [0.05, 0.05, 0.06, 1.0],
    ) {
        tracing::warn!("no se pudo componer la captura: {err:?}");
        drop(framebuffer);
        fallar(state, &mias);
        return;
    }

    // De atrás hacia delante para que quitarlas no mueva los índices que
    // quedan por mirar.
    // Píxeles e inversión vertical, o `None` si la copia falló.
    type Resultado = (crate::captura::Pendiente, Option<(Vec<u8>, bool)>);
    let mut resultados: Vec<Resultado> = Vec::new();
    for i in mias.into_iter().rev() {
        let pendiente = state.capturas_pantalla.remove(i);
        let mapeo = match renderer.copy_framebuffer(
            &framebuffer,
            crate::captura::region_gl(
                pendiente.region,
                tamano.h,
                crate::captura::hay_que_voltear(&pendiente.output),
            ),
            // **`Xrgb8888`, que es lo que se le anunció al cliente.** Los
            // nombres de fourcc describen el entero de 32 bits, así que en
            // little-endian `Xrgb8888` son los bytes B,G,R,X y `Abgr8888` son
            // R,G,B,A. Pedir aquí el segundo mientras el protocolo anuncia el
            // primero saca la captura con el rojo y el azul cambiados: medido,
            // el fondo `Light/blue.png` es (61,172,200) y salía (200,172,61).
            smithay::backend::allocator::Fourcc::Xrgb8888,
        ) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!("no se pudo leer la captura: {err}");
                resultados.push((pendiente, None));
                continue;
            }
        };
        // Del `Transform` de la salida y no de `TextureMapping::flipped`, que es
        // una constante: ver `crate::captura::hay_que_voltear`.
        let invertida = crate::captura::hay_que_voltear(&pendiente.output);
        match renderer.map_texture(&mapeo) {
            // Se copia a un `Vec` en vez de usar el préstamo: el mapeo toma
            // prestado el renderizador y `volcar` no lo necesita, pero
            // mantenerlo vivo obligaría a soltar el framebuffer antes de tiempo.
            Ok(pixeles) => resultados.push((pendiente, Some((pixeles.to_vec(), invertida)))),
            Err(err) => {
                tracing::warn!("no se pudo mapear la captura: {err}");
                resultados.push((pendiente, None));
            }
        }
    }
    // La emisión lee la salida entera de una vez: el consumidor negoció ese
    // tamaño y no hay recorte que valga.
    if !emisiones.is_empty() {
        match renderer.copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(tamano),
            smithay::backend::allocator::Fourcc::Xrgb8888,
        ) {
            Ok(mapeo) => {
                let invertida = crate::captura::hay_que_voltear(output);
                match renderer.map_texture(&mapeo) {
                    // Aquí sí se pasa el préstamo en vez de copiar a un `Vec`
                    // como hace el bucle de arriba: `emitir` copia a su propio
                    // buffer reciclado y no vuelve a tocar el renderizador.
                    Ok(pixeles) => {
                        // De atrás hacia delante: una emisión puede caerse si
                        // la salida cambió de tamaño, y eso movería los índices
                        // que quedan por mirar.
                        for i in emisiones.into_iter().rev() {
                            if let Some(ruta) =
                                state.emisiones.emitir(i, pixeles, tamano, invertida, ahora)
                            {
                                crate::portal::sesion_cerrada(state.bus_portal.as_ref(), &ruta);
                            }
                        }
                    }
                    Err(err) => tracing::warn!("no se pudo mapear el fotograma a compartir: {err}"),
                }
            }
            Err(err) => tracing::warn!("no se pudo leer el fotograma a compartir: {err}"),
        }
    }
    drop(framebuffer);

    for (pendiente, datos) in resultados {
        match datos {
            Some((pixeles, invertida)) => {
                if crate::captura::volcar(&pendiente, &pixeles, invertida) {
                    crate::captura::contestar(&pendiente, pendiente.region.size);
                }
            }
            None => pendiente.frame.failed(),
        }
    }
}

/// Hace la captura que pidió la capa del escritorio: compone, recorta, guarda
/// el PNG y avisa.
///
/// Va por el mismo camino que las de `zwlr_screencopy` —componer aparte en su
/// propio buffer— y por el mismo motivo, que está explicado en
/// [`servir_capturas`]: leer del framebuffer que se está enseñando tumba la
/// superficie EGL.
///
/// Solo la salida principal: la capa se dibuja ahí y sus coordenadas son las de
/// esa pantalla. Capturar una secundaria pide elegirla primero, y eso es otra
/// tanda.
pub fn servir_captura_propia(state: &mut BookosComp, renderer: &mut GlesRenderer, output: &Output) {
    use smithay::backend::renderer::damage::OutputDamageTracker;
    use smithay::backend::renderer::{ExportMem, Offscreen};

    let Some(pedida) = state.captura_pedida else {
        return;
    };
    // La principal tiene el origen en (0,0); ver `pantallas::normalizar`.
    if output.current_location() != (0, 0).into() {
        return;
    };
    state.captura_pedida = None;

    let escala = output.current_scale().fractional_scale();
    let tamano = crate::captura::tamano_de(output);
    // De lógicos a físicos, que es en lo que está el buffer. Se redondea hacia
    // fuera para no perder media fila de píxeles en el borde del recorte.
    let fisico = |v: i32| (v as f64 * escala).round() as i32;
    let region = smithay::utils::Rectangle::new(
        (fisico(pedida.x), fisico(pedida.y)).into(),
        (fisico(pedida.ancho).max(1), fisico(pedida.alto).max(1)).into(),
    );
    let Some(region) = region.intersection(smithay::utils::Rectangle::from_size(tamano)) else {
        tracing::warn!(?region, "la captura pedida cae fuera de la pantalla");
        return;
    };

    let mut destino =
        match Offscreen::<smithay::backend::renderer::gles::GlesRenderbuffer>::create_buffer(
            renderer,
            smithay::backend::allocator::Fourcc::Abgr8888,
            tamano,
        ) {
            Ok(b) => b,
            Err(err) => {
                tracing::error!("no se pudo crear el buffer de la captura: {err}");
                return;
            }
        };
    let elementos = escena(state, renderer, output);
    let mut seguimiento = OutputDamageTracker::from_output(output);
    let mut framebuffer = match smithay::backend::renderer::Bind::bind(renderer, &mut destino) {
        Ok(fb) => fb,
        Err(err) => {
            tracing::error!("no se pudo apuntar a la captura: {err}");
            return;
        }
    };
    if let Err(err) = seguimiento.render_output(
        renderer,
        &mut framebuffer,
        0,
        &elementos,
        [0.05, 0.05, 0.06, 1.0],
    ) {
        tracing::error!("no se pudo componer la captura: {err:?}");
        drop(framebuffer);
        return;
    }
    let leido = renderer
        .copy_framebuffer(
            &framebuffer,
            crate::captura::region_gl(region, tamano.h, crate::captura::hay_que_voltear(output)),
            // `Xrgb8888` y no `Abgr8888`, aunque el PNG quiera R,G,B,A y este
            // sea B,G,R,X: **medido**, leer en `Abgr8888` devuelve valores más
            // oscuros que los del fondo que se está enseñando —(1,31,62) donde
            // la imagen tiene (10,79,141)—, mientras que en `Xrgb8888` la
            // captura cuadra exacta con el original. La conversión a R,G,B,A la
            // hace el bucle de abajo, que ya recorre las filas para darles la
            // vuelta y no cuesta nada más.
            smithay::backend::allocator::Fourcc::Xrgb8888,
        )
        .and_then(|mapeo| renderer.map_texture(&mapeo).map(<[u8]>::to_vec));
    let invertida = crate::captura::hay_que_voltear(output);
    drop(framebuffer);

    let pixeles = match leido {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("no se pudo leer la captura: {err}");
            return;
        }
    };
    let (w, h) = (region.size.w as u32, region.size.h as u32);
    // De B,G,R,X a R,G,B,A, y de abajo arriba a arriba abajo si hace falta.
    //
    // La vuelta depende del `Transform` de la salida, no del readback: ver
    // `crate::captura::hay_que_voltear`. Se hace aquí y no al guardar porque el
    // PNG no tiene forma de decir «esto va invertido».
    let fila = w as usize * 4;
    let mut rgba = vec![0u8; fila * h as usize];
    for y in 0..h as usize {
        let origen = if invertida { h as usize - 1 - y } else { y };
        for x in 0..w as usize {
            let (o, d) = (origen * fila + x * 4, y * fila + x * 4);
            rgba[d] = pixeles[o + 2];
            rgba[d + 1] = pixeles[o + 1];
            rgba[d + 2] = pixeles[o];
            // La `X` no lleva información: la captura es opaca.
            rgba[d + 3] = 255;
        }
    }
    if pedida.guardar {
        match bookos_shell::captura::guardar_png(&rgba, w, h) {
            Ok(ruta) => {
                tracing::info!(?ruta, w, h, "captura guardada");
                // Si la pidió el portal, la ruta es la respuesta y no hay nada
                // que notificar: quien la quería es la aplicación, que ya la
                // está esperando, y el usuario no ha pulsado Impr.
                match state.captura_portal.take() {
                    Some(quien) => quien.entregar(ruta),
                    None => {
                        let nombre = ruta
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        notificar_captura(state, "Captura guardada", &nombre);
                    }
                }
            }
            Err(err) => {
                tracing::error!("no se pudo guardar la captura: {err}");
                match state.captura_portal.take() {
                    // Soltarlo sin entregar nada ya es el «no se pudo».
                    Some(_) => {}
                    None => {
                        notificar_captura(state, "No se pudo guardar la captura", &err.to_string())
                    }
                }
            }
        }
    } else {
        match bookos_shell::captura::png_en_memoria(&rgba, w, h) {
            Ok(png) => {
                let bytes = png.len();
                al_portapapeles(state, png);
                tracing::info!(w, h, bytes, "captura al portapapeles");
                notificar_captura(state, "Captura copiada", "Ya se puede pegar");
            }
            Err(err) => {
                tracing::error!("no se pudo codificar la captura: {err}");
                notificar_captura(state, "No se pudo copiar la captura", &err.to_string());
            }
        }
    }
    state.needs_redraw = true;
}

/// Pone la captura en el portapapeles.
///
/// El compositor pasa a ser el **dueño** de la selección: cuando alguien pegue,
/// Smithay llamará a `SelectionHandler::send_selection` y allí se escriben
/// estos bytes. No hay ningún proceso de por medio, así que la captura sigue
/// pegándose aunque no haya gestor de portapapeles instalado; a cambio, se
/// pierde al cerrar la sesión, que es lo que hace cualquier compositor.
fn al_portapapeles(state: &mut BookosComp, png: Vec<u8>) {
    use smithay::wayland::selection::data_device::set_data_device_selection;
    // Los dos nombres del mismo tipo. `image/png` es el que mira todo el mundo;
    // `PNG` a secas lo piden algunas aplicaciones de X11 a través de XWayland, y
    // anunciarlo no cuesta nada.
    let tipos = vec!["image/png".to_string(), "PNG".to_string()];
    // Smithay no avisa a `SelectionHandler::new_selection` cuando la pone el
    // propio compositor, así que a X11 se le dice aquí.
    if let Some(xwm) = state.xwm.as_mut()
        && let Err(err) = xwm.new_selection(
            smithay::wayland::selection::SelectionTarget::Clipboard,
            Some(tipos.clone()),
        )
    {
        tracing::warn!("no se pudo ofrecer la captura a X11: {err}");
    }
    set_data_device_selection(
        &state.display_handle,
        &state.seat,
        tipos,
        crate::handlers::Seleccion::Bytes(std::sync::Arc::new(png)),
    );
}

fn notificar_captura(state: &mut BookosComp, resumen: &str, cuerpo: &str) {
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    // Por el mismo camino que las notificaciones de las aplicaciones: el aviso
    // sale arriba a la derecha y se va solo, como cualquier otro.
    let notificacion = bookos_shell::notificaciones::Notificacion::nueva(
        0,
        "BookOS".into(),
        resumen.into(),
        cuerpo.into(),
        // El icono de la aplicación de imágenes si el tema lo trae; si no,
        // `nueva` cae al de notificaciones.
        "image-x-generic",
        false,
    );
    shell.notificar(notificacion, 4000);
    state.needs_redraw = true;
}

/// Contesta `failed` a un puñado de capturas y las quita de la lista.
fn fallar(state: &mut BookosComp, indices: &[usize]) {
    for i in indices.iter().rev() {
        let pendiente = state.capturas_pantalla.remove(*i);
        pendiente.frame.failed();
    }
}

/// Vuelve a cargar el fondo si alguien lo ha pedido, y con él la copia
/// mipmapeada que usa el cristal.
///
/// **Las dos cosas, y por eso están juntas.** El desenfoque del panel y del
/// dock no muestrea la pantalla: muestrea una textura aparte del fondo, subida
/// una sola vez al arrancar. Recargar solo `state.fondo` dejaba el escritorio
/// con la imagen nueva y las barras esmerilando la vieja, que es más raro que
/// no haber cambiado nada.
///
/// Vive aquí y no en `keybinds` porque decodificar y subir necesita el
/// `GlesRenderer`, que es del backend. Quien quiere el cambio pone
/// `state.recargar_fondo` y esto lo consume, igual que `needs_redraw`.
/// ¿Es `foco` la ventana de la app que publica la actividad?
///
/// La actividad llega con el identificador de Tauri (`com.bookos.player`) y la
/// ventana con el nombre del binario (`bookos-player`: es lo que manda GTK en
/// `set_app_id`, visto con `WAYLAND_DEBUG`). Con solo `mismo_programa` nunca
/// casaban y la isla seguía encima del propio reproductor.
fn ventana_de_actividad(actividad: &str, foco: &str) -> bool {
    let binario = actividad.split('.').skip(1).collect::<Vec<_>>().join("-");
    bookos_shell::mismo_programa(actividad, foco) || foco.eq_ignore_ascii_case(&binario)
}

pub fn recargar_fondo(state: &mut BookosComp, renderer: &mut GlesRenderer) {
    if !std::mem::take(&mut state.recargar_fondo) {
        return;
    }
    let Some(fondo) = crate::fondo::Fondo::cargar(&state.fondo_config) else {
        // Sin imagen se deja la que había: quedarse sin fondo por no encontrar
        // la nueva es peor que seguir con la anterior.
        tracing::warn!("no se encontró ningún fondo: se deja el que estaba");
        return;
    };
    if let Some(cristal) = state.cristal.as_ref() {
        let (rgba, tam) = fondo.rgba();
        cristal.borrow_mut().preparar(renderer, rgba, tam);
    }
    // El que había se queda para fundirse con el nuevo. **El cristal no se
    // funde**: el desenfoque muestrea una sola textura y mezclar dos pediría
    // otro shader, así que las barras cambian de golpe mientras el escritorio
    // se disuelve. Se nota poco —el desenfoque ya es una mancha de color— y la
    // alternativa era no fundir nada.
    //
    // Si ya había uno yéndose, se suelta y manda el nuevo: dos fundidos
    // encadenados sobre tres imágenes no describen nada que nadie haya pedido,
    // y guardar la cadena serían sesenta megas de texturas vivas.
    if let Some(anterior) = state.fondo.take() {
        state.fondo_saliente = Some((anterior, std::time::Instant::now()));
    }
    state.fondo = Some(fondo);
    actualizar_acento_del_fondo(state);
    programar_fondo_animado(state);
    state.needs_redraw = true;
}

pub fn actualizar_acento_del_fondo(state: &mut BookosComp) {
    let Some(fondo) = state.fondo.as_ref() else {
        return;
    };
    let cambio = bookos_shell::tema::actualizar_acento_del_fondo(fondo.rgba().0);
    if cambio {
        let acento = bookos_shell::tema::acento_actual();
        if let Some(shell) = state.shell.as_mut() {
            shell.aplicar_apariencia(bookos_shell::tema::actual(), acento);
        }
        if acento == bookos_shell::tema::Acento::Automatico {
            crate::portal::apariencia_cambiada(state.bus_portal.as_ref());
        }
        state.needs_redraw = true;
    }
}

/// ¿Se ve el fondo, o algo que sale de él, en alguna salida?
///
/// Si no, el fondo animado se para: un vídeo tapado seguía decodificando y
/// subiendo dos texturas de 8 MB a 30 fps —`from_buffer` sube aunque el
/// seguimiento de daño luego no dibuje el elemento—. Al pararse, el canal del
/// lector se llena y ffmpeg se queda bloqueado en la tubería sin gastar CPU;
/// al volver sigue por donde iba.
///
/// Solo cuenta como tapado lo que lo tapa seguro: la pantalla apagada y una
/// ventana a pantalla completa. Con una maximizada el fondo sigue asomando por
/// el cristal del panel y del dock, que lo desenfocan a él y no a la ventana.
/// El bloqueo, la vista de escritorios y el deslizamiento entre escritorios lo
/// enseñan aunque haya una ventana a pantalla completa.
fn fondo_visible(state: &BookosComp) -> bool {
    if !state.dpms_encendido || state.salvapantallas.activo() {
        return false;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.esta_bloqueado() || s.vista_escritorios_abierta())
    {
        return true;
    }
    state.space.outputs().any(|output| {
        !state.hay_pantalla_completa_en(output)
            || state.escritorios.tira_fondo(&output.name()).is_some()
    })
}

/// Deja un único temporizador en el instante del siguiente fotograma.
///
/// Tras marcar el redibujo espera 16 ms: para entonces `avanzar_animaciones`
/// ya ha decodificado el fotograma y ha dejado programado su retraso real. Así
/// no hay un sondeo a 60 Hz mientras un WebP enseña un fotograma durante 5 s.
pub fn programar_fondo_animado(state: &mut BookosComp) {
    if let Some(token) = state.tick_fondo.take() {
        state.loop_handle.remove(token);
    }
    let Some(espera) = state.fondo.as_ref().and_then(|f| f.hasta_siguiente()) else {
        return;
    };
    let resultado = state
        .loop_handle
        .insert_source(Timer::from_duration(espera), |_, _, state| {
            let espera = state
                .fondo
                .as_ref()
                .and_then(|f| f.hasta_siguiente())
                .filter(|_| fondo_visible(state));
            let Some(espera) = espera else {
                state.tick_fondo = None;
                return TimeoutAction::Drop;
            };
            if espera.is_zero() {
                state.needs_redraw = true;
                TimeoutAction::ToDuration(std::time::Duration::from_millis(16))
            } else {
                TimeoutAction::ToDuration(espera)
            }
        });
    match resultado {
        Ok(token) => state.tick_fondo = Some(token),
        Err(err) => tracing::warn!(%err, "no se pudo programar el fondo animado"),
    }
}

/// Cuánto dura el fundido entre dos fondos.
///
/// La «transición de página completa» del sistema de diseño: cambiar de fondo
/// **es** eso, la pantalla entera pasando a otra cosa, y usar la misma tabla que
/// el launchpad y el cambio de escritorio es lo que hace que el escritorio se
/// mueva siempre igual. Nada de un valor propio aquí.
const FUNDIDO: std::time::Duration = bookos_shell::tema::D_PAGINA;

/// Cuánto lleva el fundido del fondo, de 0 a 1 y con su curva. `None` cuando no
/// hay ninguno en marcha.
///
/// Al llegar a uno suelta el fondo saliente, que son veinte megas: la función
/// que consulta el avance es también la que cierra la transición porque es la
/// única que se llama en **todos** los caminos de dibujo, y dejar la limpieza
/// en otro sitio significaría que un fondo se queda vivo si ese sitio no corre.
fn avance_fundido(state: &mut BookosComp) -> Option<f32> {
    let (_, desde) = state.fondo_saliente.as_ref()?;
    let t = bookos_shell::tema::fraccion(desde.elapsed(), FUNDIDO);
    if t >= 1.0 {
        state.fondo_saliente = None;
        return None;
    }
    Some(bookos_shell::tema::C_ENTRADA.eval(t))
}

/// Anuncia `zwp_linux_dmabuf_v1` con los formatos que este contexto EGL sabe
/// leer como textura.
///
/// Es el global que decide si el escritorio mueve píxeles o descriptores. Sin
/// él, Mesa no encuentra manera de acelerar EGL sobre Wayland y **todo**
/// cliente cae a software: el navegador rasteriza en llvmpipe y entrega el
/// resultado por `wl_shm`, lo que obliga a una copia CPU→GPU por ventana y por
/// fotograma. Y como un buffer de memoria compartida no se puede exportar a un
/// plano DRM, el direct scanout tampoco podía ocurrir para ningún cliente.
///
/// Medido con Firefox 153 a pantalla completa sobre una página con cuarenta
/// círculos animados, anidado y en `--release`, con y sin este global sobre el
/// **mismo** binario (`BOOKOS_SIN_DMABUF`):
///
/// | | fps | escena | peor | saltados/s |
/// |---|---|---|---|---|
/// | sin dmabuf | 14,7 | 0,50 ms | 6,5 ms | 75 |
/// | con dmabuf | 29,4 | 0,15 ms | 1,1 ms | 0 |
///
/// O sea: el doble de fotogramas —el tope anidado son 30—, un tercio del
/// tiempo de escena y seis veces menos en el peor fotograma. Los 75 saltados
/// por segundo de la primera fila son despertares que no produjeron daño: el
/// navegador hacía commit y el fotograma salía vacío porque iba por detrás.
///
/// **Versión 4 con feedback, no versión 3.** Medido con el mismo Firefox: con
/// el global de la versión 3 se enlaza al protocolo y aun así sigue pidiendo
/// `wl_shm.create_pool` de 21 MB para su ventana. Necesita que el feedback le
/// diga sobre qué nodo DRM asignar, y eso solo está en la 4; con la 4 pasa a
/// entregar la ventana como `create_immed` de 2350×1712 en AR24. `create_global`
/// a secas se queda en la 3, así que no vale aquí.
///
/// El nodo sale de `EGL_EXT_device_drm_render_node`, o sea del propio contexto
/// que va a hacer la importación, y no de la GPU que se abrió para KMS: son la
/// misma en un portátil, pero preguntárselo a EGL vale igual en la sesión real
/// y anidado, y evita tener dos caminos que se pueden desviar.
///
/// Se anuncian los formatos de **textura** y no los de render: la pregunta es
/// qué sabemos leer, no qué sabemos dibujar, y la de textura es la lista ancha.
/// Los estrechos —los que además valen para un plano— ya se los queda el
/// `DrmOutputManager` al elegir el formato del primario.
///
/// Con `BOOKOS_SIN_DMABUF` no se anuncia nada: sirve para medir con y sin, y
/// para salir del paso si un driver importa mal. Los clientes vuelven a
/// `wl_shm`, que es exactamente lo que había antes de esto.
pub fn anunciar_dmabuf(
    state: &mut BookosComp,
    display: &smithay::backend::egl::EGLDisplay,
    formatos: smithay::backend::allocator::format::FormatSet,
) {
    use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

    if std::env::var_os("BOOKOS_SIN_DMABUF").is_some() {
        tracing::info!("dmabuf desactivado por BOOKOS_SIN_DMABUF");
        return;
    }
    let nodo = match smithay::backend::egl::EGLDevice::device_for_display(display)
        .and_then(|d| d.try_get_render_node())
    {
        Ok(Some(nodo)) => nodo,
        // Sin nodo no hay feedback, y sin feedback el navegador se queda en
        // shm igual: más vale decirlo que anunciar una versión 3 que no sirve.
        otro => {
            tracing::error!(?otro, "sin nodo de render: no se anuncia dmabuf");
            return;
        }
    };
    match DmabufFeedbackBuilder::new(nodo.dev_id(), formatos).build() {
        Ok(feedback) => {
            let dh = state.display_handle.clone();
            state.dmabuf_global = Some(
                state
                    .dmabuf_state
                    .create_global_with_default_feedback::<BookosComp>(&dh, &feedback),
            );
            state.egl_display = Some(display.clone());
            tracing::info!(nodo = ?nodo.dev_path(), "zwp_linux_dmabuf_v1 anunciado (versión 4)");
        }
        // Sin el global la sesión sigue, con los clientes por shm.
        Err(err) => tracing::error!("no se pudo construir el feedback de dmabuf: {err}"),
    }
}

pub fn escala_sugerida(px: (i32, i32), mm: (i32, i32)) -> f64 {
    if mm.0 <= 0 || mm.1 <= 0 || px.0 <= 0 || px.1 <= 0 {
        return 1.0;
    }
    const MM_POR_PULGADA: f64 = 25.4;
    // Se promedian los dos ejes: un EDID con los milímetros redondeados hacia
    // distinto lado da DPI distintos en horizontal y en vertical, y elegir uno
    // de los dos sería arbitrario.
    let dpi_x = px.0 as f64 * MM_POR_PULGADA / mm.0 as f64;
    let dpi_y = px.1 as f64 * MM_POR_PULGADA / mm.1 as f64;
    let escala = (dpi_x + dpi_y) / 2.0 / DPI_OBJETIVO;

    // A cuartos, que es el paso que usan todos los escritorios: una escala de
    // 1,73 no se distingue de 1,75 y en cambio hace que ningún tamaño en
    // píxeles caiga redondo.
    let escala = (escala * 4.0).round() / 4.0;
    // Por debajo de 1 no se baja: encoger el escritorio no arregla ninguna
    // pantalla, y sí rompe el panel, que tiene alturas pensadas en lógicos.
    escala.clamp(1.0, 3.0)
}

/// Programa el siguiente repintado del panel.
///
/// El momento lo eligen los widgets, no esta función: el reloj pide el cambio
/// de minuto, no un tick de 1 Hz, porque un temporizador de un segundo para
/// algo que solo enseña minutos son 59 despertares de más por minuto. Cada uno
/// saca a la CPU de su C-state, y esa es la clase de goteo que arruina la
/// autonomía en reposo.
///
/// Si ningún widget pide alarma no se programa **nada**: un panel de widgets
/// que solo reaccionan a eventos deja el proceso durmiendo en `epoll`.
pub fn schedule_panel_tick(state: &mut BookosComp) {
    let Some(delay) = state.shell.as_ref().and_then(|s| s.next_tick()) else {
        return;
    };
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(delay), |_, _, state| {
            refresh_panel(state);
            schedule_panel_tick(state);
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar el tick del panel: {err}");
    }
}

/// Programa el bloqueo automático como una alarma única.
///
/// La alarma se calcula desde `last_input`, pero no se reinicia con cada
/// movimiento del ratón: hacerlo convertiría la entrada a 1 kHz en una
/// sucesión de altas y bajas de fuentes de calloop. Cuando vence, se comprueba
/// de nuevo la edad de la última entrada y se ajusta la alarma si el usuario
/// acaba de interactuar.
pub fn programar_bloqueo_inactividad(state: &mut BookosComp) {
    use smithay::utils::IsAlive;
    if let Some(token) = state.tick_bloqueo.take() {
        state.loop_handle.remove(token);
    }
    state.idle_inhibidores.retain(|s| s.alive());
    crate::salvapantallas::programar(state);
    crate::atenuacion::programar(state);
    if state.bloqueo_inactividad.is_zero()
        || !state.idle_inhibidores.is_empty()
        || bookos_shell::mantener_despierto()
        || state
            .shell
            .as_ref()
            .is_none_or(|shell| shell.esta_bloqueado())
    {
        return;
    }
    let desde_entrada = state
        .last_input
        .map(|instante| instante.elapsed())
        .unwrap_or_default();
    let espera = state
        .bloqueo_inactividad
        .saturating_sub(desde_entrada)
        .max(std::time::Duration::from_millis(50));
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(espera), |_, _, state| {
            use smithay::utils::IsAlive;
            state.tick_bloqueo = None;
            state.idle_inhibidores.retain(|s| s.alive());
            if state
                .shell
                .as_ref()
                .is_some_and(|shell| shell.esta_bloqueado())
            {
                return TimeoutAction::Drop;
            }
            if !state.idle_inhibidores.is_empty() || bookos_shell::mantener_despierto() {
                return TimeoutAction::Drop;
            }
            let inactivo = state
                .last_input
                .map(|instante| instante.elapsed() >= state.bloqueo_inactividad)
                .unwrap_or(false);
            if inactivo {
                crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
                // El bloqueo a mano apaga la pantalla a los 10 s; el
                // automático no, porque detrás vienen el salvapantallas y
                // `apagado_inactividad`, que es quien la apaga.
                if let Some(token) = state.tick_dpms.take() {
                    state.loop_handle.remove(token);
                }
                tracing::info!(
                    segundos = state.bloqueo_inactividad.as_secs(),
                    "bloqueo automático por inactividad"
                );
                TimeoutAction::Drop
            } else {
                programar_bloqueo_inactividad(state);
                TimeoutAction::Drop
            }
        });
    match result {
        Ok(token) => state.tick_bloqueo = Some(token),
        Err(err) => tracing::error!("no se pudo programar el bloqueo automático: {err}"),
    }
}

/// Cancela la alarma de suspensión sin alterar la política configurada.
pub fn cancelar_suspension_inactividad(state: &mut BookosComp) {
    if let Some(token) = state.tick_suspension.take() {
        state.loop_handle.remove(token);
    }
}

/// Programa una única suspensión tras permanecer bloqueado.
///
/// Cero la desactiva. Los inhibidores Wayland cancelan la alarma y, al
/// liberarse el último, el plazo empieza de nuevo. La orden final pasa por
/// logind, que conserva la última palabra sobre sus propios inhibidores.
pub fn programar_suspension_inactividad(state: &mut BookosComp) {
    use smithay::utils::IsAlive;
    cancelar_suspension_inactividad(state);
    state.idle_inhibidores.retain(|s| s.alive());
    if state.suspension_inactividad.is_zero()
        || state.salvapantallas.pantalla_apagada()
        || !state.idle_inhibidores.is_empty()
        || bookos_shell::mantener_despierto()
        || state
            .shell
            .as_ref()
            .is_none_or(|shell| !shell.esta_bloqueado())
    {
        return;
    }
    let espera = state.suspension_inactividad;
    let resultado = state
        .loop_handle
        .insert_source(Timer::from_duration(espera), |_, _, state| {
            use smithay::utils::IsAlive;
            state.tick_suspension = None;
            state.idle_inhibidores.retain(|s| s.alive());
            let bloqueado = state
                .shell
                .as_ref()
                .is_some_and(|shell| shell.esta_bloqueado());
            if bloqueado
                && !state.salvapantallas.pantalla_apagada()
                && state.idle_inhibidores.is_empty()
                && !bookos_shell::mantener_despierto()
            {
                tracing::info!(
                    segundos = state.suspension_inactividad.as_secs(),
                    "suspensión automática tras bloqueo"
                );
                crate::keybinds::ejecutar(
                    state,
                    crate::keybinds::Accion::Energia(bookos_shell::bloqueo::Peticion::Suspender),
                );
            }
            TimeoutAction::Drop
        });
    match resultado {
        Ok(token) => state.tick_suspension = Some(token),
        Err(err) => tracing::error!("no se pudo programar la suspensión automática: {err}"),
    }
}

/// Apaga KMS diez segundos después de mostrar el bloqueo.
pub fn programar_dpms_bloqueo(state: &mut BookosComp) {
    if let Some(token) = state.tick_dpms.take() {
        state.loop_handle.remove(token);
    }
    if state.aplicar_dpms.is_none()
        || state
            .shell
            .as_ref()
            .is_none_or(|shell| !shell.esta_bloqueado())
    {
        return;
    }
    let resultado = state.loop_handle.insert_source(
        Timer::from_duration(std::time::Duration::from_secs(10)),
        |_, _, state| {
            state.tick_dpms = None;
            if state
                .shell
                .as_ref()
                .is_some_and(|shell| shell.esta_bloqueado())
            {
                cambiar_dpms(state, false);
            }
            TimeoutAction::Drop
        },
    );
    match resultado {
        Ok(token) => state.tick_dpms = Some(token),
        Err(err) => tracing::error!("no se pudo programar DPMS: {err}"),
    }
}

/// Apaga las salidas sin esperar los 10 s del bloqueo. La pantalla se queda
/// sin ningún fotograma pendiente, y lo siguiente que pinte al encenderse es
/// lo que haya entonces: el bloqueo, si se echó antes.
pub fn apagar_dpms(state: &mut BookosComp) {
    if let Some(token) = state.tick_dpms.take() {
        state.loop_handle.remove(token);
    }
    cambiar_dpms(state, false);
    // 2 s: lo que tarda de sobra logind en suspender. Si la suspensión no
    // llega —un inhibidor—, pasado el margen basta con tocar algo.
    state.dpms_sordo_hasta = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
}

/// Enciende las salidas al volver de la suspensión.
///
/// Al dormir desde el menú se apaga la pantalla antes de que logind suspenda,
/// y la primera entrada la vuelve a encender. Pero abrir la tapa despierta el
/// equipo sin que llegue necesariamente una tecla o un movimiento, y se
/// quedaría en negro. `PrepareForSleep(false)` es lo que manda logind justo al
/// reanudar.
pub fn escuchar_reanudacion(state: &mut BookosComp) {
    let (aviso, fuente) = smithay::reexports::calloop::channel::channel();
    let insertado = state.loop_handle.insert_source(fuente, |evento, _, state| {
        if let smithay::reexports::calloop::channel::Event::Msg(()) = evento {
            tracing::info!("reanudado: se enciende la pantalla");
            state.dpms_sordo_hasta = None;
            despertar_dpms(state);
            crate::keybinds::reanudar_huella(state);
        }
    });
    if let Err(err) = insertado {
        tracing::error!("no se pudo escuchar la reanudación: {err}");
        return;
    }
    let lanzado = std::thread::Builder::new()
        .name("bookos-reanudar".into())
        .spawn(move || {
            let señales = zbus::blocking::Connection::system()
                .and_then(|conexion| {
                    zbus::blocking::Proxy::new(
                        &conexion,
                        "org.freedesktop.login1",
                        "/org/freedesktop/login1",
                        "org.freedesktop.login1.Manager",
                    )
                })
                .and_then(|proxy| proxy.receive_signal("PrepareForSleep"));
            let señales = match señales {
                Ok(señales) => señales,
                Err(err) => {
                    tracing::warn!(
                        "sin logind, la pantalla no se encenderá sola al despertar: {err}"
                    );
                    return;
                }
            };
            for mensaje in señales {
                if mensaje.body().deserialize::<bool>() == Ok(false) && aviso.send(()).is_err() {
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("no se pudo escuchar la reanudación: {err}");
    }
}

/// Reactiva las salidas ante la primera entrada tras DPMS.
pub fn despertar_dpms(state: &mut BookosComp) {
    if state
        .dpms_sordo_hasta
        .is_some_and(|hasta| std::time::Instant::now() < hasta)
    {
        return;
    }
    if !state.dpms_encendido {
        cambiar_dpms(state, true);
        crate::brillo_auto::evaluar(state);
        state.needs_redraw = true;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|shell| shell.esta_bloqueado())
    {
        programar_dpms_bloqueo(state);
    }
}

fn cambiar_dpms(state: &mut BookosComp, encendido: bool) {
    if state.dpms_encendido == encendido {
        return;
    }
    let Some(mut aplicar) = state.aplicar_dpms.take() else {
        return;
    };
    match aplicar(encendido) {
        Ok(()) => {
            state.dpms_encendido = encendido;
            tracing::info!(encendido, "estado DPMS");
        }
        Err(err) => tracing::warn!(encendido, "no se pudo cambiar DPMS: {err}"),
    }
    state.aplicar_dpms = Some(aplicar);
}

/// Relee los estados y marca repintado **solo** si algo cambió de verdad.
fn tarjeta_brillo_abierta(state: &BookosComp) -> bool {
    state
        .shell
        .as_ref()
        .is_some_and(|s| matches!(s.emergente_nombre(), Some("brillo" | "centro")))
}

fn refresh_panel(state: &mut BookosComp) {
    if state.shell.as_mut().is_some_and(|s| s.refresh()) {
        state.needs_redraw = true;
    }
    // Aquí y no en un temporizador propio: este refresco ya ocurre al cambiar
    // el hardware —enchufar el cargador es un evento de udev— y una vez por
    // minuto con el tick del reloj. La batería no baja del 21 al 20 % más
    // deprisa que eso.
    if state.shell.as_ref().is_some_and(|s| s.revisar_efectos()) {
        tracing::info!(
            reducidos = bookos_shell::tema::efectos_reducidos(),
            "efectos visuales"
        );
        state.needs_redraw = true;
    }
}

/// Despierta el panel cuando el kernel avisa de un cambio de hardware.
///
/// El tick del reloj es de un minuto, y para la hora está bien. Para el resto
/// de estados no: enchufar el cargador, perder el wifi o subir el brillo tienen
/// que verse en el momento, y la alternativa —sondear sysfs varias veces por
/// segundo— es justo el goteo de despertares que este compositor evita.
///
/// El kernel ya manda esos avisos por netlink; aquí solo se escuchan. Entre
/// evento y evento el descriptor duerme en `epoll` y no cuesta nada.
#[cfg(feature = "udev")]
pub fn watch_hardware(state: &mut BookosComp) {
    use smithay::reexports::calloop::generic::Generic;
    use smithay::reexports::calloop::{Interest, Mode, PostAction};
    use smithay::reexports::udev::MonitorBuilder;

    // Los subsistemas los declaran los widgets: si el panel no lleva el de
    // brillo, el kernel no nos despierta al mover la tecla. Antes la lista
    // estaba a fuego y cualquier evento releía las cuatro fuentes de sysfs.
    //
    // El shell incluye también `leds`: la tarjeta de brillo muestra el teclado.
    let mut subsistemas = state
        .shell
        .as_ref()
        .map(|s| s.subsistemas())
        .unwrap_or_default();
    // El aviso del cargador no depende de que el panel lleve la batería.
    if !subsistemas.contains(&"power_supply") {
        subsistemas.push("power_supply");
    }
    if subsistemas.is_empty() {
        tracing::debug!("ningún widget pide avisos de hardware");
        return;
    }
    let monitor = MonitorBuilder::new()
        .and_then(|m| {
            subsistemas
                .iter()
                .try_fold(m, |m, sub| m.match_subsystem(sub))
        })
        .and_then(|m| m.listen());
    let monitor = match monitor {
        Ok(monitor) => monitor,
        Err(err) => {
            // Sin esto el panel sigue funcionando, solo que al ritmo del reloj.
            tracing::warn!("sin avisos de hardware, el panel irá al minuto: {err}");
            return;
        }
    };

    let source = Generic::new(monitor, Interest::READ, Mode::Level);
    let result = state
        .loop_handle
        .insert_source(source, |_, monitor, state| {
            // Hay que vaciar la cola sí o sí: la fuente es de nivel, así que un
            // evento sin leer volvería a despertarnos para siempre. Da igual
            // *qué* cambió — releer los cuatro ficheros de sysfs cuesta menos
            // que averiguarlo.
            let cambios = monitor.iter().count();
            if cambios > 0 {
                tracing::debug!(cambios, "aviso de hardware, releyendo el panel");
                refresh_panel(state);
                crate::multimedia::cargador(state);
                crate::atenuacion::bateria(state);
            }
            Ok(PostAction::Continue)
        });
    if let Err(err) = result {
        tracing::error!("no se pudo escuchar los cambios de hardware: {err}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn la_isla_reconoce_la_ventana_de_su_app() {
        assert!(super::ventana_de_actividad(
            "com.bookos.player",
            "bookos-player"
        ));
        assert!(super::ventana_de_actividad(
            "com.bookos.player",
            "com.bookos.player"
        ));
        assert!(!super::ventana_de_actividad("com.bookos.player", "konsole"));
        assert!(!super::ventana_de_actividad(
            "com.bookos.player",
            "bookos-clock"
        ));
    }

    use super::escala_sugerida;

    #[test]
    fn esta_pantalla_pide_el_175_por_ciento() {
        // El Book5 Pro: 2880×1800 en 300×190 mm, leído con `--drm-info`. 175 %
        // es lo que se elige a mano en KDE en esta misma máquina, y es el caso
        // contra el que está calibrada la constante.
        assert_eq!(escala_sugerida((2880, 1800), (300, 190)), 1.75);
    }

    #[test]
    fn las_pantallas_normales_se_quedan_al_cien() {
        // Un portátil de 15,6" a 1080p y un monitor de 27" a 1440p: los dos
        // rondan los 110-140 DPI y no necesitan escalado. Si alguno de estos dos
        // sale a 1,25, el escritorio se ve enorme en media España.
        assert_eq!(escala_sugerida((1920, 1080), (345, 194)), 1.0);
        assert_eq!(escala_sugerida((2560, 1440), (597, 336)), 1.0);
    }

    #[test]
    fn el_4k_sube_segun_el_tamano_del_panel() {
        // El mismo número de píxeles pide cosas distintas según cuánto midan:
        // en 27" basta un cuarto más, en un portátil de 15,6" hace falta el
        // doble. Es justo lo que se pierde al cablear una escala fija.
        assert_eq!(escala_sugerida((3840, 2160), (597, 336)), 1.25);
        assert_eq!(escala_sugerida((3840, 2160), (345, 194)), 2.0);
    }

    #[test]
    fn sin_tamano_fisico_no_se_inventa_nada() {
        // Proyectores y algunas KVM anuncian 0×0. Deducir una escala de un dato
        // que no existe es peor que quedarse en la que siempre funciona.
        assert_eq!(escala_sugerida((1920, 1080), (0, 0)), 1.0);
        assert_eq!(escala_sugerida((0, 0), (300, 190)), 1.0);
    }

    #[test]
    fn siempre_sale_un_cuarto_exacto() {
        // De esto depende que los tamaños en lógicos caigan redondos. Se barren
        // pantallas plausibles en vez de un par de casos elegidos.
        for ancho in [1366, 1920, 2256, 2880, 3840] {
            for mm in [200, 250, 300, 345, 600] {
                let e = escala_sugerida((ancho, ancho * 10 / 16), (mm, mm * 10 / 16));
                assert_eq!(e * 4.0, (e * 4.0).round(), "{ancho}px en {mm}mm da {e}");
                assert!((1.0..=3.0).contains(&e));
            }
        }
    }
}

/// Sin el backend real no hay libudev enlazado. El panel se queda con el tick
/// del reloj, que para desarrollar anidado sobra.
#[cfg(not(feature = "udev"))]
pub fn watch_hardware(_state: &mut BookosComp) {}
