//! Backend anidado: el compositor corre como una ventana dentro de la sesión
//! actual. Es la forma de iterar sin salir a un TTY.
//!
//! **No es el camino de producción y no mide nada útil de consumo**: aquí hay
//! un compositor por debajo (KWin) haciendo el trabajo de verdad, y el pump de
//! winit obliga a un temporizador. El ahorro real se juega en el backend udev,
//! donde el ritmo lo marcan los eventos de vblank de DRM.

use std::time::Duration;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::desktop::space::render_output;
use smithay::desktop::utils::surface_primary_scanout_output;
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Transform;

use crate::pantallas::{self, Aplicado, Modo, Salida};
use crate::state::BookosComp;

/// Cadencia del pump de winit **mientras pasa algo**: 8 ms ≈ 120 Hz, que es lo
/// que da el panel del Book5. Marca cada cuánto se *consultan* eventos; que se
/// dibuje o no lo sigue decidiendo `needs_redraw`.
const PUMP_ACTIVE: Duration = Duration::from_millis(8);
/// Lo más rápido que se dibuja: 8 ms ≈ 120 Hz, que es lo que da el panel del
/// Book5. **No es lo mismo que la cadencia del pump.**
///
/// Preguntar por eventos es barato y conviene hacerlo a menudo para que el
/// cursor no se arrastre; dibujar es caro. Al juntarlos, bombear a 1 kHz
/// significaba componer mil fotogramas por segundo —cada uno con su subida de
/// texturas y su pasada por el compositor anfitrión— y el escritorio se movía a
/// tirones justo cuando más se le pedía. Son dos ritmos y van por separado.
const MIN_FRAME: Duration = Duration::from_millis(8);
/// Cadencia mientras la cola del anfitrión **todavía trae eventos**.
///
/// Un ratón manda a 1 kHz y el pump se llamaba cada 8 ms, así que cada vuelta
/// se tragaba una ráfaga entera y el cursor —que aquí lo dibujamos nosotros,
/// con el del anfitrión oculto— avanzaba a saltos de 8 ms. No es cero para no
/// dejar el bucle girando sin ceder la CPU.
const PUMP_BUSY: Duration = Duration::from_millis(1);
/// Cadencia en reposo. Sondear a 120 Hz un escritorio quieto es justo el goteo
/// de despertares que este proyecto intenta evitar, así que en cuanto deja de
/// haber actividad el sondeo se relaja.
const PUMP_IDLE: Duration = Duration::from_millis(33);
/// Cuánto se sigue considerando "activo" tras el último evento de entrada. Sin
/// esta cola, mover el ratón despacio alternaría entre 8 y 33 ms y se notaría.
const ACTIVE_TAIL: Duration = Duration::from_millis(400);

pub fn run(
    event_loop: &mut EventLoop<'static, BookosComp>,
    state: &mut BookosComp,
    client: Option<String>,
    fullscreen: bool,
) -> anyhow::Result<()> {
    // Esto conecta con el compositor *anfitrión*, leyendo el WAYLAND_DISPLAY
    // del entorno. Tiene que ocurrir antes de anunciar el nuestro a nadie.
    let (mut backend, mut winit_loop) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow::anyhow!("no se pudo iniciar el backend winit: {err}"))?;

    // Smithay deja su propio nombre como título de la ventana anidada. Eso
    // hacía que en el selector de ventanas pareciese una herramienta interna
    // y, con más de una previsualización abierta, no hubiese forma humana de
    // reconocer cuál era el escritorio. En una sesión real (backend udev) no
    // existe esta ventana, así que el cambio solo afecta al modo de desarrollo.
    backend.window().set_title("BookOS Desktop");

    if fullscreen {
        // Previsualizar el DE entero: la ventana anidada ocupa la pantalla y lo
        // que se ve es exactamente lo que verá la sesión real, salvo que debajo
        // sigue habiendo un compositor anfitrión.
        use smithay::reexports::winit::window::Fullscreen;
        backend
            .window()
            .set_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let size = backend.window_size();
    // El refresco del monitor donde está la ventana. Estaba cableado a 60 Hz, y
    // eso hacía que un panel de 120 se anunciara a la mitad: los clientes que
    // ajustan su animación al refresco de `wl_output` —Firefox, GTK4— pintaban
    // a 60 dentro de una ventana que el anfitrión presentaba a 120. Si winit no
    // sabe en qué monitor está, se queda en 60 000 mHz, que es lo que había.
    let refresco = backend
        .window()
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .unwrap_or(60_000) as i32;
    tracing::info!(hz = refresco as f64 / 1000.0, "refresco del anfitrión");
    // La escala la marca el anfitrión (aquí KWin), salvo que la configuración
    // diga otra cosa. Esa excepción existe para poder **ver** el escritorio a
    // otra escala sin cambiar de monitor ni arrancar en un TTY: es la única
    // forma de comprobar aquí lo que le va a pasar a la sesión real, donde la
    // escala se deduce del tamaño físico del panel.
    let scale = state
        .config
        .as_ref()
        .and_then(|c| c.escala)
        .unwrap_or_else(|| backend.scale_factor());
    let mode = Mode {
        size,
        refresh: refresco,
    };
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "BookOS".into(),
            model: "Anidado".into(),
        },
    );
    output.create_global::<BookosComp>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        Some(Scale::Fractional(scale)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    // El shell se crea aquí, en cuanto se sabe el ancho de la pantalla, y ya
    // queda pintado para el primer frame: no hay un segundo proceso al que
    // esperar ni un hueco en el que se vea el escritorio a medio montar.
    // En el kiosko no hay escritorio. Ver el mismo punto en `udev`.
    if !state.modo.kiosko() {
        state.shell = Some(crate::shell::ShellHost::new(
            size.w.max(1) as u32,
            size.h.max(1) as u32,
            scale as f32,
            state.config.take(),
        ));
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.refresh();
    }
    state.last_input = Some(std::time::Instant::now());
    crate::backend::schedule_panel_tick(state);
    crate::backend::programar_bloqueo_inactividad(state);
    // El despertar del tema automático, si está puesto. Con un modo fijo no
    // deja ningún temporizador. Ver `crate::apariencia`.
    crate::apariencia::programar_cambio(state);
    // También anidado: los estados salen de sysfs, no del compositor de debajo,
    // así que aquí se ven igual de vivos que en la sesión real.
    crate::backend::watch_hardware(state);
    state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(
        scale,
        state.cursor_nominal,
    ));
    // dmabuf también anidado: lo que se prueba aquí tiene que ser el mismo
    // camino de buffers que en la sesión real, o lo medido no dice nada del
    // escritorio de verdad. Versión 3 —`create_global` a secas— y no 4: el
    // feedback de la 4 nombra un nodo DRM, y anidado el KMS no es nuestro.
    // Anidado también: lo que se prueba aquí tiene que ser el mismo camino de
    // buffers que en la sesión real, o lo medido no dice nada del escritorio de
    // verdad. Ver `backend::anunciar_dmabuf`.
    {
        let formatos = backend
            .renderer()
            .egl_context()
            .dmabuf_texture_formats()
            .clone();
        let display = backend.renderer().egl_context().display().clone();
        crate::backend::anunciar_dmabuf(state, &display, formatos);
    }
    state.cristal = crate::desenfoque::Cristal::new(backend.renderer());
    state.genio = crate::genio::Genio::new(backend.renderer());
    state.contexto_gl = Some(smithay::backend::renderer::Renderer::context_id(
        backend.renderer(),
    ));
    state.velo_captura = crate::captura::Velo::new(backend.renderer());
    state.banda = crate::banda::Banda::new(backend.renderer());
    state.fondo = crate::fondo::Fondo::cargar(&state.fondo_config);
    crate::backend::actualizar_acento_del_fondo(state);
    crate::backend::programar_fondo_animado(state);
    // El cristal desenfoca el fondo, así que necesita su propia copia con
    // mipmaps. En fondos animados se sustituye al avanzar el fotograma.
    if let (Some(fondo), Some(cristal)) = (state.fondo.as_ref(), state.cristal.as_ref()) {
        let (rgba, tam) = fondo.rgba();
        cristal.borrow_mut().preparar(backend.renderer(), rgba, tam);
    }
    // El anfitrión no debe pintar *también* su cursor encima del nuestro: se
    // verían dos punteros desalineados.
    backend.window().set_cursor_visible(false);
    crate::selftest::schedule(state);
    if std::env::var_os("BOOKOS_BLOQUEAR").is_some() {
        let _ = state.loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(
                std::time::Duration::from_secs(4),
            ),
            |_, _, state| {
                crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        );
    }
    instalar_pantallas(state, &output);

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    // EGL_BUFFER_AGE_EXT no es válido hasta que la superficie ha pasado por un
    // swap: preguntarlo antes devuelve EGL_BAD_SURFACE. Da igual, porque el
    // primer frame tiene que ser un redibujo completo de todas formas, que es
    // justo lo que significa age = 0.
    let mut rendered_once = false;
    // Si el fotograma anterior compuso además en un offscreen. Ver dónde se usa.
    let mut uso_offscreen = false;
    let mut ultimo_frame = std::time::Instant::now() - MIN_FRAME;
    let mut vueltas = 0u32;
    let mut con_pendiente = 0u32;
    let mut reloj_vueltas = std::time::Instant::now();

    state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            let hubo_eventos = pump(&mut winit_loop, state, refresco);
            let toca_dibujar = ultimo_frame.elapsed() >= MIN_FRAME;
            vueltas += 1;
            if state.needs_redraw {
                con_pendiente += 1;
            }
            if std::env::var_os("BOOKOS_PERFIL").is_some()
                && reloj_vueltas.elapsed() >= Duration::from_secs(1)
            {
                tracing::info!(vueltas, con_pendiente, "vueltas del pump por segundo");
                vueltas = 0;
                con_pendiente = 0;
                reloj_vueltas = std::time::Instant::now();
            }
            // `hay_animacion()` va en la condición, no solo `needs_redraw`, y no
            // es un detalle: lo que hace avanzar las animaciones vive dentro de
            // `draw`, y `draw` limpia `needs_redraw` cuando el damage sale
            // vacío. En cuanto la ventana que se desliza salía de la pantalla el
            // fotograma no cambiaba nada, se limpiaba la marca, y a la vuelta
            // siguiente ya no se entraba aquí: **el cambio de escritorio se
            // quedaba congelado a medias**, con la ventana fuera de sitio y
            // `deslizando` puesto para siempre. Medido con
            // `BOOKOS_SELFTEST_ESCRITORIOS`: la ventana se paraba en x=-1507 de
            // los -1646 que tenía que recorrer. `udev` ya preguntaba esto en
            // `dibujar_todas`; aquí faltaba.
            if (state.needs_redraw || state.hay_animacion()) && toca_dibujar {
                ultimo_frame = std::time::Instant::now();
                let cronometro = std::time::Instant::now();
                // La edad del buffer **no se puede preguntar** si el último
                // que compuso algo fue el offscreen de una captura:
                // `EGL_BUFFER_AGE_EXT` solo vale sobre la superficie activa en
                // este hilo, y `Bind<EGLSurface>` de Smithay no la activa —lo
                // hace `render_output` al dibujar—. Preguntarla igualmente no
                // devolvía nada útil y además llenaba el registro:
                // medido, 194 `BAD_SURFACE` en 14 s compartiendo pantalla.
                // Con 0 se redibuja entero, que es exactamente lo que ya
                // ocurría cuando la consulta fallaba, pero sin el ruido.
                let age = if rendered_once && !uso_offscreen {
                    backend.buffer_age().unwrap_or(0)
                } else {
                    0
                };
                uso_offscreen = state.hay_offscreen_pendiente();
                match draw(&mut backend, state, &output, &mut damage_tracker, age) {
                    Ok(submitted) => rendered_once |= submitted,
                    Err(err) => tracing::error!("fallo al dibujar: {err}"),
                }
                // Anidado no hay vblank: el fotograma se da por presentado al
                // volver de `draw`, que es lo más cerca de la verdad que se
                // puede estar sin hablar con el hardware. Las cifras de aquí
                // valen para comparar cambios entre sí, no como medida absoluta
                // — para eso está el backend de KMS.
                let _ = cronometro;
            }
            // Sin post_dispatch aquí: el callback de `event_loop.run` ya lo hace
            // al cerrar cada vuelta, y repetirlo son refresh + flush por
            // segundo tirados.
            //
            // El backend anidado no tiene forma de esperar en `epoll` a los
            // eventos de winit —no expone su descriptor—, así que hay que
            // sondear. Lo que sí se puede es sondear rápido solo cuando hay
            // algo en marcha.
            // Con trabajo pendiente que no se ha podido dibujar todavía, hay
            // que volver justo cuando toque el siguiente fotograma; ni antes,
            // que sería girar en vacío, ni después, que se vería a saltos.
            if state.needs_redraw && !toca_dibujar {
                return TimeoutAction::ToDuration(MIN_FRAME.saturating_sub(ultimo_frame.elapsed()));
            }
            // `hay_animacion` va aquí porque `needs_redraw` ya se ha limpiado
            // al dibujar: sin preguntarlo, el ritmo se relajaba a 33 ms en
            // mitad de cualquier animación y se veían cuatro fotogramas de los
            // veintidós que caben en 180 ms. Medido abriendo el launchpad.
            let activo = state.needs_redraw
                || state.hay_animacion()
                || state.last_input.is_some_and(|t| t.elapsed() < ACTIVE_TAIL);
            // Si esta vuelta trajo eventos, el anfitrión tiene más en camino:
            // esperar 8 ms a preguntar otra vez es exactamente lo que se veía
            // como cursor pastoso sobre el dock. Mientras el ratón se mueve se
            // pregunta a 1 kHz; en cuanto la cola se vacía, se vuelve a dormir.
            TimeoutAction::ToDuration(if hubo_eventos {
                PUMP_BUSY
            } else if activo {
                PUMP_ACTIVE
            } else {
                PUMP_IDLE
            })
        })
        .map_err(|err| anyhow::anyhow!("insert_source(timer): {err}"))?;

    tracing::info!(socket = ?state.socket_name, "compositor listo (backend anidado)");

    match client {
        Some(cmd) if state.modo.kiosko() => crate::keybinds::lanzar_kiosko(state, &cmd)?,
        Some(cmd) => crate::keybinds::lanzar(state, &cmd),
        None => {}
    }

    event_loop.run(None, state, |state| {
        state.post_dispatch();
    })?;
    Ok(())
}

/// Vacía la cola de eventos del anfitrión. Devuelve `true` si había alguno.
fn pump(winit_loop: &mut winit::WinitEventLoop, state: &mut BookosComp, refresco: i32) -> bool {
    let mut hubo = false;
    winit_loop.dispatch_new_events(|event| {
        hubo = true;
        match event {
            WinitEvent::Resized { size, scale_factor } => {
                // La escala de la configuración vuelve a mandar sobre la del
                // anfitrión: sin esto, el primer cambio de tamaño —que con
                // --fullscreen llega siempre— deshacía lo que se pidió.
                let scale_factor = state.escala_forzada.unwrap_or(scale_factor);
                tracing::info!(
                    w = size.w,
                    h = size.h,
                    escala = scale_factor,
                    "salida reconfigurada"
                );
                // Con --fullscreen el tamaño llega después de que el anfitrión
                // conceda la pantalla completa, así que la salida y el shell se
                // reconfiguran aquí y no en el arranque.
                let mode = Mode {
                    size,
                    refresh: refresco,
                };
                if let Some(output) = state.space.outputs().next().cloned() {
                    output.change_current_state(
                        Some(mode),
                        None,
                        Some(Scale::Fractional(scale_factor)),
                        None,
                    );
                    output.set_preferred(mode);
                }
                if let Some(shell) = state.shell.as_mut() {
                    shell.resize(
                        size.w.max(1) as u32,
                        size.h.max(1) as u32,
                        scale_factor as f32,
                    );
                    shell.refresh();
                }
                // Los clientes ya conectados tienen que enterarse de la escala nueva.
                state.broadcast_preferred_scale(scale_factor);
                state.recolocar_encajadas();
                state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(
                    scale_factor,
                    state.cursor_nominal,
                ));
                state.needs_redraw = true;
            }
            WinitEvent::Redraw => {
                state.needs_redraw = true;
            }
            WinitEvent::CloseRequested => {
                state.loop_signal.stop();
            }
            WinitEvent::Input(event) => crate::input::handle(state, event),
            _ => {}
        }
    });
    hubo
}

/// Sirve las capturas pendientes y deja el contexto donde estaba.
///
/// Las capturas componen su propio buffer —leer del framebuffer del anfitrión
/// tumbaba su superficie EGL, ver `backend::servir_capturas`—, así que hay que
/// volver a apuntar a la ventana al terminar: la vuelta siguiente pregunta la
/// edad del buffer **antes** de apuntar, y sin esto `eglQuerySurface` fallaba y
/// cada captura costaba además un redibujo completo del fotograma siguiente.
fn servir_capturas(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    state: &mut BookosComp,
    output: &Output,
) {
    if !state.hay_offscreen_pendiente() {
        return;
    }
    crate::backend::servir_capturas(state, backend.renderer(), output);
    crate::backend::servir_captura_propia(state, backend.renderer(), output);
    if let Err(err) = backend.bind() {
        tracing::warn!("no se pudo volver a apuntar a la ventana: {err}");
    }
}

/// Devuelve `true` si llegó a enviarse un frame a la pantalla.
fn draw(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    state: &mut BookosComp,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    age: usize,
) -> anyhow::Result<bool> {
    // Lo que se mueve solo avanza antes de componer, y una sola vez: anidado
    // hay una única salida, pero el orden tiene que ser el mismo que en KMS
    // para que lo que se prueba aquí valga. Ver `backend::avanzar_animaciones`.
    crate::backend::avanzar_animaciones(state);

    let (renderer, mut framebuffer) = backend
        .bind()
        .map_err(|err| anyhow::anyhow!("bind del framebuffer: {err}"))?;
    // Si el tema cambió, el fondo y el cristal tienen que ser ya los nuevos en
    // este mismo fotograma.
    crate::backend::recargar_fondo(state, renderer);
    crate::multimedia::vigilar_centro(state);

    // El panel va por delante de las ventanas: `custom_elements` se apila
    // encima de los espacios.
    let _t = std::time::Instant::now();
    let overlay = crate::backend::escena(state, renderer, output);
    let t_escena = _t.elapsed();
    let _t = std::time::Instant::now();

    let result = render_output::<_, crate::cursor::OverlayElement, _, _>(
        output,
        renderer,
        &mut framebuffer,
        1.0,
        age,
        [] as [&smithay::desktop::Space<smithay::desktop::Window>; 0],
        &overlay,
        damage_tracker,
        [0.05, 0.05, 0.06, 1.0],
    )
    .map_err(|err| anyhow::anyhow!("render_output: {err:?}"))?;
    let t_render = _t.elapsed();
    let _t = std::time::Instant::now();

    drop(framebuffer);

    let nombre = output.name();
    // Un refresco, para el umbral de los frame callbacks. Ver la explicación en
    // `udev::enviar_frames`: aquí se usa la misma política a propósito, porque
    // si anidado los clientes se comportan distinto que en la sesión real, lo
    // que se mide anidado no dice nada del escritorio de verdad. Antes esto era
    // `Some(Duration::ZERO)`, que da callback a **todo** en cada fotograma,
    // ocluidos incluidos: los clientes tapados corrían en vacío.
    let periodo = output
        .current_mode()
        .filter(|m| m.refresh > 0)
        .map(|m| Duration::from_nanos(1_000_000_000_000u64 / m.refresh as u64))
        .unwrap_or(Duration::from_millis(16));
    if let Some(modo) = output.current_mode() {
        state.metricas.modo(
            &nombre,
            modo.size.w,
            modo.size.h,
            output.current_scale().fractional_scale(),
            modo.refresh as f32 / 1000.0,
            false,
        );
    }
    state.metricas.dibujado(&nombre, t_escena, t_render);

    // Qué superficie se enseña en qué salida. Anidado solo hay una, pero sin
    // esto `surface_primary_scanout_output` devuelve `None` para todo y los
    // frame callbacks pasan a depender solo del umbral: hasta la única ventana
    // visible recibiría uno por refresco en vez de uno por fotograma. Es lo que
    // hacía que anidado y KMS no se comportaran igual.
    for window in state.space.elements() {
        window.with_surfaces(|surface, data| {
            smithay::desktop::utils::update_surface_primary_scanout_output(
                surface,
                output,
                data,
                &result.states,
                smithay::backend::renderer::element::default_primary_scanout_output_compare,
            );
        });
    }

    // `damage == None` significa que el damage tracker no encontró nada que
    // cambiara: se salta el submit entero y no se toca la pantalla.
    match result.damage {
        Some(damage) => {
            backend
                .submit(Some(damage))
                .map_err(|err| anyhow::anyhow!("submit: {err}"))?;
        }
        None => {
            state.needs_redraw = false;
            state.metricas.saltado(&nombre);
            // Aun sin damage hay que contestar a los frame callbacks: un
            // cliente que pidió uno para engancharse al vsync se queda parado
            // para siempre si aquí se sale sin decirle nada.
            let time = state.start_time.elapsed();
            for window in state.space.elements() {
                window.send_frame(output, time, Some(periodo), surface_primary_scanout_output);
            }
            crate::backend::enviar_frames_capas(output, time, periodo);
            // Y las capturas también aquí: componen su propio buffer y no
            // dependen de que este fotograma haya cambiado nada. Sin esto, una
            // captura pedida con el escritorio quieto no se servía nunca —el
            // primer fotograma sin daño salía por este `return` y ya no se
            // volvía a entrar.
            servir_capturas(backend, state, output);
            return Ok(false);
        }
    }

    if std::env::var_os("BOOKOS_PERFIL").is_some() {
        tracing::info!(
            escena_ms = format_args!("{:.2}", t_escena.as_secs_f64() * 1000.0),
            render_ms = format_args!("{:.2}", t_render.as_secs_f64() * 1000.0),
            submit_ms = format_args!("{:.2}", _t.elapsed().as_secs_f64() * 1000.0),
            elementos = overlay.len(),
            "frame"
        );
    }

    // Los frame callbacks salen después del submit: es la señal de "puedes
    // dibujar el siguiente". Enviarlos antes hace que los clientes corran en
    // vacío y es una fuga de consumo clásica.
    let time = state.start_time.elapsed();
    for window in state.space.elements() {
        window.send_frame(output, time, Some(periodo), surface_primary_scanout_output);
    }
    crate::backend::enviar_frames_capas(output, time, periodo);

    state.metricas.presentado(&nombre);

    servir_capturas(backend, state, output);

    state.needs_redraw = false;
    Ok(true)
}

// ── Pantallas, anidado ───────────────────────────────────────────────────

/// El censo de la única "pantalla" que hay aquí: la ventana del anfitrión.
///
/// No es un adorno para que Settings no se caiga: la escala y la rotación se
/// aplican de verdad y se ven, y es como se prueba la escala fraccional sin
/// salir a un TTY. Lo que no se puede es cambiar el modo —el tamaño lo decide
/// el compositor de debajo— ni pedir frecuencia variable.
fn censo_anidado(output: &Output) -> Vec<Salida> {
    let modo = output.current_mode().unwrap_or(Mode {
        size: (0, 0).into(),
        refresh: 60_000,
    });
    let escala = output.current_scale().fractional_scale();
    let transformacion = pantallas::nombre_transformacion(output.current_transform());
    let (logico_ancho, logico_alto) = pantallas::tamano_logico(
        modo.size.w.max(0) as u32,
        modo.size.h.max(0) as u32,
        escala,
        transformacion,
    );
    vec![Salida {
        id: "winit".into(),
        conector: "winit".into(),
        fabricante: "BookOS".into(),
        modelo: "Anidado".into(),
        serie: String::new(),
        mm_ancho: 0,
        mm_alto: 0,
        activa: true,
        modos: vec![Modo {
            ancho: modo.size.w.max(0) as u32,
            alto: modo.size.h.max(0) as u32,
            refresco_mhz: modo.refresh.max(0) as u32,
            preferido: true,
            actual: true,
        }],
        escala,
        escalas: pantallas::ESCALAS.to_vec(),
        x: output.current_location().x,
        y: output.current_location().y,
        transformacion: transformacion.to_string(),
        vrr_capaz: false,
        vrr: false,
        principal: true,
        logico_ancho,
        logico_alto,
    }]
}

fn instalar_pantallas(state: &mut BookosComp, output: &Output) {
    *state
        .pantallas
        .compartido
        .backend
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = "winit";
    {
        let output = output.clone();
        state.censar_pantallas = Some(Box::new(move || censo_anidado(&output)));
    }
    {
        let output = output.clone();
        state.aplicar_pantallas = Some(Box::new(move |peticion| {
            let mia = peticion
                .iter()
                .find(|p| p.id == "winit")
                .ok_or_else(|| "la configuración no dice nada de la ventana anidada".to_string())?;
            if !mia.activa {
                return Err("no se puede apagar la ventana del compositor anidado".into());
            }
            let modo = output
                .current_mode()
                .ok_or_else(|| "la ventana anidada todavía no tiene tamaño".to_string())?;
            if mia.ancho as i32 != modo.size.w || mia.alto as i32 != modo.size.h {
                return Err(
                    "anidado, el tamaño lo decide el compositor de debajo: redimensiona la ventana"
                        .into(),
                );
            }
            // La salida anidada nace en `Flipped180` porque el framebuffer de
            // GL tiene el eje Y al revés que el de la ventana; componer una
            // rotación encima de ese volteo no es girar la pantalla, es girar
            // también el volteo, y sale una imagen en espejo. Rotar es cosa de
            // la sesión real, y `GetCapabilities` ya lo dice (`rotation` es
            // false con este backend).
            let actual = pantallas::nombre_transformacion(output.current_transform());
            if mia.transformacion != actual {
                return Err(
                    "anidado no se puede rotar la pantalla: pruébalo en la sesión real".into(),
                );
            }
            output.change_current_state(
                None,
                None,
                Some(Scale::Fractional(mia.escala)),
                Some((mia.x, mia.y).into()),
            );
            Ok(Aplicado {
                salidas: censo_anidado(&output),
                mapa: vec![(output.clone(), (mia.x, mia.y).into())],
                principal: 0,
            })
        }));
    }
    let salidas = censo_anidado(output);
    state.pantallas.ultima = pantallas::peticion_de(&salidas);
    state.pantallas.compartido.publicar(salidas);
}
