//! Backend de sesión real: DRM/KMS + libinput + libseat.
//!
//! Este es el camino de producción. A diferencia del anidado, aquí **no hay
//! ningún compositor debajo**: hablamos con el kernel directamente, así que es
//! el único sitio donde las mediciones de consumo significan algo y donde se
//! pueden usar las tres optimizaciones grandes del plan:
//!
//! - **Direct scanout.** `DrmOutput` envuelve un `DrmCompositor`, que reparte
//!   los elementos entre los planos del hardware. Una ventana opaca a pantalla
//!   completa va directa al plano primario sin componer nada en la GPU.
//! - **Cursor en plano de hardware.** Los elementos marcados `Kind::Cursor` se
//!   asignan al plano de cursor cuando cabe, y mover el ratón deja de repintar
//!   la escena.
//! - **Ritmo por vblank.** No hay temporizador de sondeo: el siguiente frame se
//!   dibuja cuando el hardware avisa de que enseñó el anterior. En reposo no se
//!   despierta nadie.
//!
//! Todo eso lo decide `DrmCompositor` según lo que quepa en los planos; el
//! Book5 anuncia 12, así que hay sitio de sobra.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::default_primary_scanout_output_compare;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::desktop::utils::{
    OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, update_surface_primary_scanout_output,
};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::drm::control::property::Value as PropValue;
use smithay::reexports::drm::control::{Device as _, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::{AccelProfile, ClickMethod, Device as InputDevice, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::{DeviceFd, Physical, Point};
use smithay::wayland::presentation::Refresh;

use crate::cursor::OverlayElement;
use crate::pantallas::{self, Aplicado, Modo, Peticion, Salida};
use crate::state::BookosComp;

type Alloc = GbmAllocator<DrmDeviceFd>;
/// El exportador convierte los buffers GBM en framebuffers de DRM. No vale el
/// `GbmDevice` pelado: hace falta este envoltorio.
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
type Manager = DrmOutputManager<Alloc, Exporter, OutputPresentationFeedback, DrmDeviceFd>;
type Surface = DrmOutput<Alloc, Exporter, OutputPresentationFeedback, DrmDeviceFd>;

/// Lo que el backend necesita y el estado del compositor no debe conocer.
struct SalidaKms {
    output: Output,
    global: smithay::reexports::wayland_server::backend::GlobalId,
    surface: Surface,
    conector: connector::Handle,
    id: String,
    en_vuelo: bool,
    pendiente: bool,
}

struct Udev {
    manager: Manager,
    renderer: GlesRenderer,
    display_handle: smithay::reexports::wayland_server::DisplayHandle,
    /// Una escena KMS independiente por salida activa. Cada una conserva su
    /// swapchain, damage tracker y reloj de presentación por vblank.
    salidas: Vec<SalidaKms>,
    principal_id: String,
    /// La sesión está en segundo plano (has cambiado de TTY): no se dibuja.
    activa: bool,
    /// La GPU no tiene nodo de render: es simpledrm (arranque en modo básico,
    /// `nomodeset`) y Mesa dibuja con kms_swrast sobre buffers dumb. Ahí la
    /// edad de los buffers de la swapchain no se cumple: fuera del damage el
    /// buffer no conserva lo dibujado dos fotogramas antes, y la pantalla se
    /// quedaba gris salvo el rastro del cursor y lo que se iba repintando.
    repintado_completo: bool,
}

impl Udev {
    /// Hay una salida con damage apuntado que todavía no se ha podido dibujar
    /// porque tenía un frame en el aire.
    ///
    /// Sin mirar esto, ese damage se perdía: `dibujar_todas` limpia
    /// `needs_redraw` aunque la salida se saltara por `en_vuelo`, y el vblank
    /// solo redibujaba si `needs_redraw` seguía puesto. O sea que **todo commit
    /// de un cliente que llegue mientras hay un frame en vuelo —el caso normal
    /// escribiendo en un navegador— se quedaba sin pintar y sin frame callback**
    /// hasta que otro evento cualquiera despertara el bucle. De ahí el segundo
    /// largo de retraso al teclear.
    fn hay_pendiente(&self) -> bool {
        self.salidas.iter().any(|s| s.pendiente && !s.en_vuelo)
    }
}

pub fn run(
    event_loop: &mut EventLoop<'static, BookosComp>,
    state: &mut BookosComp,
    client: Option<String>,
) -> anyhow::Result<()> {
    // libseat nos da el asiento: acceso a la GPU y a los dispositivos de
    // entrada sin ser root, y el cambio de TTY.
    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|err| anyhow::anyhow!("no se pudo abrir la sesión de libseat: {err}"))?;

    // El cambio de TTY se expone al resto del compositor como un cierre: es lo
    // único que `keybinds` necesita de libseat, y así el estado no arrastra el
    // tipo de sesión. `change_vt` es asíncrono — libseat contesta con un
    // PauseSession que ya se atiende más abajo.
    {
        let session = session.clone();
        state.cambiar_vt = Some(Box::new(move |vt| {
            // `session` es Clone y comparte el asiento, pero `change_vt` pide
            // &mut; el clon local lo hace posible sin sincronización.
            let mut session = session.clone();
            if let Err(err) = session.change_vt(vt) {
                tracing::error!(vt, "no se pudo cambiar de TTY: {err}");
            }
        }));
    }

    let gpu = elegir_gpu(&session)?;
    let repintado_completo = !gpu.has_render();
    tracing::info!(?gpu, repintado_completo, "GPU elegida");

    let fd = session
        .open(
            gpu.dev_path()
                .as_deref()
                .unwrap_or(std::path::Path::new("")),
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|err| anyhow::anyhow!("no se pudo abrir la GPU: {err}"))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    // `true` desactiva los conectores que el kernel dejó encendidos: partimos de
    // un estado conocido en vez de heredar lo que hubiera antes.
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd.clone())?;

    // El renderizador va sobre el mismo dispositivo GBM, así que lo que dibuja
    // se puede escanear directamente sin copiar entre GPUs.
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };

    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();

    // El global que hace que los clientes entreguen descriptores de la GPU en
    // vez de píxeles. Ver `backend::anunciar_dmabuf`.
    crate::backend::anunciar_dmabuf(
        state,
        &display,
        renderer.egl_context().dmabuf_texture_formats().clone(),
    );

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let mut manager = Manager::new(
        drm,
        allocator,
        GbmFramebufferExporter::new(gbm.clone(), Some(gpu)),
        Some(gbm),
        // Formatos de color aceptables para el plano primario, en orden de
        // preferencia. Con alfa primero para poder componer sin copia extra.
        [
            smithay::backend::allocator::Fourcc::Argb8888,
            smithay::backend::allocator::Fourcc::Xrgb8888,
        ],
        render_formats,
    );

    let salidas = crear_salidas(&mut manager, &mut renderer, state)?;
    let guardadas = pantallas::cargar();
    let principal = guardadas
        .iter()
        .find(|p| p.activa && p.principal)
        .and_then(|p| salidas.iter().find(|s| s.id == p.id))
        .or_else(|| {
            salidas
                .iter()
                .find(|s| s.output.current_location() == (0, 0).into())
        })
        .unwrap_or(&salidas[0]);
    let principal_id = principal.id.clone();

    let escala = principal.output.current_scale().fractional_scale();
    // La escala puede venir de `pantallas.conf` y no de `panel.conf`, así que
    // hay que reponerla aquí: el resto del compositor la lee de este campo
    // cuando la pantalla cambia de tamaño, y si no, volvería a la de la
    // configuración vieja.
    state.escala_forzada = Some(escala);
    let modo = principal
        .output
        .current_mode()
        .map(|m| m.size)
        .unwrap_or_default();
    // En el kiosko no hay escritorio: sin shell no hay panel, dock, launchpad
    // ni bloqueo, y el resto del compositor ya trata `None` como «no hay».
    let kiosko = state.modo.kiosko();
    if !kiosko {
        state.shell = Some(crate::shell::ShellHost::new(
            modo.w.max(1) as u32,
            modo.h.max(1) as u32,
            escala as f32,
            state.config.take(),
        ));
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.refresh();
    }
    state.last_input = Some(std::time::Instant::now());
    // El indicador de escritorios no puede saber cuántos hay: se lo decimos
    // antes del primer frame, para que salga pintado con el resto del panel y
    // no aparezca un instante después.
    if !kiosko {
        crate::escritorios::avisar_al_panel(state);
    }
    let escala_cursor = salidas
        .iter()
        .map(|s| s.output.current_scale().fractional_scale())
        .max_by(f64::total_cmp)
        .unwrap_or(escala);
    state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(
        escala_cursor,
        state.cursor_nominal,
    ));
    state.cristal = crate::desenfoque::Cristal::new(&mut renderer);
    state.genio = crate::genio::Genio::new(&mut renderer);
    state.contexto_gl = Some(smithay::backend::renderer::Renderer::context_id(&renderer));
    state.velo_captura = crate::captura::Velo::new(&mut renderer);
    state.banda = crate::banda::Banda::new(&mut renderer);
    // Ni fondo ni apariencia publicada en el kiosko: el programa tapa la
    // pantalla entera, y `publicar` como root dejaría una carpeta de «root» en
    // /var/lib/bookos/apariencia que el greeter no tiene por qué ver.
    if !kiosko {
        state.fondo = crate::fondo::Fondo::cargar(&state.fondo_config);
        crate::publicar::activar();
        crate::publicar::publicar(&state.fondo_config);
        crate::backend::actualizar_acento_del_fondo(state);
        crate::backend::programar_fondo_animado(state);
    }
    // El cristal desenfoca el fondo, así que necesita su propia copia con
    // mipmaps. En fondos animados se sustituye al avanzar el fotograma.
    if let (Some(fondo), Some(cristal)) = (state.fondo.as_ref(), state.cristal.as_ref()) {
        let (rgba, tam) = fondo.rgba();
        cristal.borrow_mut().preparar(&mut renderer, rgba, tam);
    }
    for salida in &salidas {
        state
            .space
            .map_output(&salida.output, salida.output.current_location());
    }

    let udev = Rc::new(RefCell::new(Udev {
        manager,
        renderer,
        display_handle: state.display_handle.clone(),
        salidas,
        principal_id,
        activa: true,
        repintado_completo,
    }));

    // `DrmCompositor::clear` quita el framebuffer del conector y equivale a
    // DPMS off. El siguiente `queue_frame`, tras resetear los buffers, vuelve
    // a encenderlo. La operación queda en el hilo del compositor: nunca se
    // toca KMS desde el temporizador ni desde D-Bus.
    {
        let udev = udev.clone();
        state.aplicar_dpms = Some(Box::new(move |encendido| {
            let mut u = udev
                .try_borrow_mut()
                .map_err(|_| "el backend está ocupado dibujando".to_string())?;
            for salida in &mut u.salidas {
                let nombre = salida.output.name();
                if encendido {
                    salida.surface.reset_buffers();
                    salida.en_vuelo = false;
                    salida.pendiente = true;
                } else {
                    salida
                        .surface
                        .with_compositor(|compositor| compositor.clear())
                        .map_err(|err| format!("no se pudo apagar {nombre}: {err}"))?;
                    salida.en_vuelo = false;
                    salida.pendiente = false;
                }
            }
            Ok(())
        }));
    }

    // La curva de la luz nocturna, una por CRTC encendido. Cada CRTC dice
    // cuántas entradas tiene su tabla: no hay un tamaño que valga para todos.
    {
        let udev = udev.clone();
        state.aplicar_gamma = Some(Box::new(move |[r, g, b]| {
            let u = udev
                .try_borrow()
                .map_err(|_| "el backend está ocupado dibujando".to_string())?;
            if !u.activa {
                // En otro TTY la pantalla no es nuestra: se repone al volver.
                return Ok(());
            }
            let device = u.manager.device();
            for salida in &u.salidas {
                let crtc = salida.surface.crtc();
                let n = device
                    .get_crtc(crtc)
                    .map_err(|err| format!("{}: {err}", salida.output.name()))?
                    .gamma_length() as usize;
                if n == 0 {
                    return Err(format!("{} no tiene tabla de gamma", salida.output.name()));
                }
                let rampa = |m| crate::luz_nocturna::rampa(n, m);
                device
                    .set_gamma(crtc, &rampa(r), &rampa(g), &rampa(b))
                    .map_err(|err| format!("{}: {err}", salida.output.name()))?;
            }
            Ok(())
        }));
    }

    // --- Pantallas ---------------------------------------------------------
    // Los dos cierres que van del hilo del compositor a KMS. Nadie más toca el
    // backend: D-Bus solo empuja peticiones por el canal de calloop, y estas se
    // ejecutan aquí, entre fotogramas.
    *state
        .pantallas
        .compartido
        .backend
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = "udev";
    {
        let udev = udev.clone();
        state.censar_pantallas = Some(Box::new(move || match udev.try_borrow() {
            Ok(u) => censar(&u),
            Err(_) => Vec::new(),
        }));
    }
    {
        let udev = udev.clone();
        let lazo = state.loop_handle.clone();
        state.aplicar_pantallas = Some(Box::new(move |peticion| {
            let mut u = udev
                .try_borrow_mut()
                .map_err(|_| "el backend está ocupado dibujando".to_string())?;
            aplicar_en_kms(&mut u, &lazo, peticion)
        }));
    }
    {
        let salidas = censar(&udev.borrow());
        state.pantallas.ultima = pantallas::peticion_de(&salidas);
        state.pantallas.compartido.publicar(salidas);
    }

    // Enchufar o quitar un monitor lo cuenta el kernel por netlink: no hay
    // ningún temporizador mirando si cambió algo. Solo se vuelve a censar y se
    // avisa por `OutputsChanged`; decidir qué hacer con la pantalla nueva es
    // cosa de quien configure.
    match UdevBackend::new(session.seat()) {
        Ok(backend) => {
            let r = event_loop
                .handle()
                .insert_source(backend, |evento, _, state| {
                    if let UdevEvent::Changed { .. } = evento {
                        crate::ajustes::recibir(state, crate::ajustes::Aviso::Redetectar);
                    }
                });
            if let Err(err) = r {
                tracing::warn!("sin aviso de conexión de monitores: {err}");
            }
        }
        Err(err) => tracing::warn!("sin aviso de conexión de monitores: {err}"),
    }

    // --- Entrada -----------------------------------------------------------
    // Los touchpads abiertos se guardan para poder encenderlos y apagarlos con
    // la tecla de función: `DeviceAdded` solo llega con los que aparecen, y el
    // del portátil aparece al arrancar.
    let touchpads: Rc<RefCell<Vec<InputDevice>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let touchpads = touchpads.clone();
        state.aplicar_touchpad = Some(Box::new(move |activo| {
            for device in touchpads.borrow_mut().iter_mut() {
                let _ = device.config_send_events_set_mode(if activo {
                    smithay::reexports::input::SendEventsMode::ENABLED
                } else {
                    smithay::reexports::input::SendEventsMode::DISABLED
                });
            }
        }));
    }

    // Todos los dispositivos, para reaplicar los ajustes de entrada en vivo.
    let dispositivos: Rc<RefCell<Vec<InputDevice>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let dispositivos = dispositivos.clone();
        state.aplicar_entrada = Some(Box::new(move |cfg, touchpad_activo| {
            for device in dispositivos.borrow_mut().iter_mut() {
                configurar_dispositivo(device, cfg, touchpad_activo);
            }
        }));
    }

    // Solo en la sesión real: anidado, la luz del teclado es del anfitrión.
    crate::multimedia::vigilar_luz_teclado(state);
    crate::multimedia::vigilar_tapa_camara(state);

    // Los teclados, por lo mismo: sus luces las tiene que mandar el
    // compositor, y hay que poder hacerlo cuando cambie Bloq Mayús.
    let teclados: Rc<RefCell<Vec<InputDevice>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let teclados = teclados.clone();
        state.aplicar_leds = Some(Box::new(move |leds| {
            for device in teclados.borrow_mut().iter_mut() {
                device.led_update(luces(leds));
            }
        }));
    }

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&session.seat())
        .map_err(|_| anyhow::anyhow!("no se pudo asignar el asiento a libinput"))?;
    event_loop
        .handle()
        .insert_source(
            LibinputInputBackend::new(libinput.clone()),
            move |event, _, state| {
                // Los ajustes se aplican aquí y no en `input::handle` porque ese
                // está escrito contra `InputBackend` genérico y no puede tocar la
                // API de libinput. Llega uno por dispositivo al arrancar y otra
                // tanda tras cada cambio de TTY, cuando libseat los devuelve.
                match &event {
                    smithay::backend::input::InputEvent::DeviceAdded { device } => {
                        let mut device = device.clone();
                        configurar_dispositivo(&mut device, &state.entrada, state.touchpad_activo);
                        dispositivos.borrow_mut().push(device.clone());
                        if device
                            .has_capability(smithay::reexports::input::DeviceCapability::Keyboard)
                        {
                            // Nace con las luces de ahora: tras un cambio de TTY,
                            // o un teclado USB enchufado con Bloq Mayús puesto.
                            device.led_update(luces(state.leds));
                            teclados.borrow_mut().push(device.clone());
                        }
                        if device.config_tap_finger_count() > 0 {
                            touchpads.borrow_mut().push(device);
                        }
                    }
                    smithay::backend::input::InputEvent::DeviceRemoved { device } => {
                        dispositivos.borrow_mut().retain(|d| d != device);
                        teclados.borrow_mut().retain(|d| d != device);
                        touchpads.borrow_mut().retain(|d| d != device);
                    }
                    _ => {}
                }
                crate::input::handle(state, event);
            },
        )
        .map_err(|err| anyhow::anyhow!("insert_source(libinput): {err}"))?;

    // --- Vblank ------------------------------------------------------------
    {
        let udev = udev.clone();
        event_loop
            .handle()
            .insert_source(drm_notifier, move |event, metadata, state| match event {
                DrmEvent::VBlank(crtc) => {
                    let mut u = udev.borrow_mut();
                    // Cerrar el frame anterior antes de plantearse otro: si no,
                    // el swapchain se queda sin buffers libres.
                    if let Some(salida) = u.salidas.iter_mut().find(|s| s.surface.crtc() == crtc) {
                        match salida.surface.frame_submitted() {
                            Ok(Some(mut feedback)) => {
                                if let Some(metadata) = *metadata {
                                    let mhz = salida.output.current_mode()
                                        .map(|m| m.refresh.max(1) as u64).unwrap_or(60_000);
                                    let periodo = Duration::from_nanos(
                                        1_000_000_000_000u64 / mhz);
                                    let refresh = if salida.surface
                                        .with_compositor(|c| c.vrr_enabled()) {
                                        Refresh::variable(periodo)
                                    } else {
                                        Refresh::fixed(periodo)
                                    };
                                    let tiempo = match metadata.time {
                                        smithay::backend::drm::DrmEventTime::Monotonic(t) =>
                                            smithay::utils::Time::<smithay::utils::Monotonic>::from(t),
                                        // Un driver que da el flip en el reloj
                                        // de pared. Se **traslada** al
                                        // monotónico en vez de tirarlo: antes
                                        // aquí se ponía el instante en que el
                                        // bucle atendía el evento, que no es
                                        // cuando la pantalla enseñó el
                                        // fotograma, y esa diferencia es jitter
                                        // que el cliente se come al ajustar su
                                        // ritmo. El desfase entre los dos
                                        // relojes se mide ahora, que es lo más
                                        // cerca del vblank que se puede estar.
                                        smithay::backend::drm::DrmEventTime::Realtime(t) =>
                                            trasladar_a_monotonico(t),
                                    };
                                    feedback.presented(
                                        tiempo, refresh, metadata.sequence as u64,
                                        smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync,
                                    );
                                }
                            }
                            Ok(None) => {}
                            Err(err) => tracing::warn!(?crtc, "frame_submitted: {err}"),
                        }
                        salida.en_vuelo = false;
                        // Contado aquí y no al encolar: esto es lo que el
                        // hardware ha enseñado de verdad.
                        state.metricas.presentado(&salida.output.name());
                    }
                    // Igual que en winit: una animación en marcha pide el
                    // siguiente fotograma aunque nadie haya marcado nada, porque
                    // su paso se calcula al componer.
                    if state.needs_redraw || state.hay_animacion() || u.hay_pendiente() {
                        dibujar_todas(state, &mut u);
                    }
                }
                DrmEvent::Error(err) => tracing::error!("error de DRM: {err}"),
            })
            .map_err(|err| anyhow::anyhow!("insert_source(drm): {err}"))?;
    }

    // --- Cambio de TTY -----------------------------------------------------
    {
        let udev = udev.clone();
        let mut libinput = libinput;
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, state| {
                let mut u = udev.borrow_mut();
                match event {
                    SessionEvent::PauseSession => {
                        // Otro TTY toma el control: se suelta el KMS y se deja
                        // de dibujar hasta volver.
                        tracing::info!("sesión en pausa (cambio de TTY)");
                        libinput.suspend();
                        u.manager.pause();
                        u.activa = false;
                        for salida in &mut u.salidas {
                            salida.en_vuelo = false;
                        }
                    }
                    SessionEvent::ActivateSession => {
                        tracing::info!("sesión reactivada");
                        // La barrera se instala antes de reactivar KMS: el
                        // primer fotograma visible tras volver del TTY ya es
                        // la pantalla de bloqueo, nunca el escritorio previo.
                        if state.shell.as_ref().is_some_and(|s| !s.esta_bloqueado()) {
                            crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
                        }
                        if libinput.resume().is_err() {
                            tracing::error!("libinput no pudo reanudar");
                        }
                        u.activa = true;
                        // Reactivar fuerza un modeset: mientras no éramos
                        // nosotros, otro pudo dejar la pantalla en otro modo.
                        // `false` = no apagar los conectores que ya estaban.
                        if let Err(err) = u.manager.activate(false) {
                            tracing::warn!("activate: {err}");
                        }
                        for salida in &u.salidas {
                            salida.surface.reset_buffers();
                        }
                        state.dpms_encendido = true;
                        state.needs_redraw = true;
                        dibujar_todas(state, &mut u);
                        // El maestro del otro TTY pudo dejar la curva a su
                        // gusto. `aplicar_gamma` toma el backend prestado, así
                        // que va con `u` ya soltado.
                        drop(u);
                        crate::luz_nocturna::reponer(state);
                    }
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(session): {err}"))?;
    }

    crate::backend::schedule_panel_tick(state);
    crate::backend::programar_bloqueo_inactividad(state);
    // El despertar del tema automático, si está puesto. Con un modo fijo no
    // deja ningún temporizador. Ver `crate::apariencia`.
    crate::apariencia::programar_cambio(state);
    let luz = state.luz_nocturna.config;
    crate::luz_nocturna::recargar(state, luz);
    // El tema pudo cambiar con la sesión cerrada —el automático, o editando
    // `panel.conf`—: las apps GTK tienen que arrancar ya con el que toca.
    // El botón de encendido se inhibe para que lo atienda el diálogo del
    // shell; sin shell quedaría muerto, así que en el kiosko lo lleva logind.
    if !kiosko {
        crate::portal::tema_gtk();
        crate::keybinds::inhibir_boton_de_encendido();
    }
    crate::backend::escuchar_reanudacion(state);
    crate::backend::watch_hardware(state);

    // Primer frame: deja el escritorio pintado antes de que arranque nada más,
    // que es lo que evita el parpadeo al iniciar sesión.
    dibujar_todas(state, &mut udev.borrow_mut());
    // Con el escritorio ya en pantalla, no antes: el sonido dice «ya está».
    if !kiosko {
        crate::multimedia::sonar_interfaz(state, "desktop-login");
    }

    tracing::info!(socket = ?state.socket_name, "compositor listo (sesión real)");

    tracing::info!(
        "atajos: Ctrl+Alt+F1..F12 cambia de TTY · Meta+Return abre un terminal · \
         Meta+Q cierra la ventana · Meta+Alt+Retroceso sale"
    );

    match client {
        Some(cmd) if kiosko => crate::keybinds::lanzar_kiosko(state, &cmd)?,
        Some(cmd) => crate::keybinds::lanzar(state, &cmd),
        None => {}
    }

    let udev_loop = udev.clone();
    event_loop.run(None, state, move |state| {
        // Si hay trabajo pendiente y no hay frame en el aire, se dibuja al
        // cerrar la vuelta del bucle. El resto del tiempo el proceso duerme en
        // epoll: no hay sondeo de ningún tipo.
        let pendiente = udev_loop.try_borrow().is_ok_and(|u| u.hay_pendiente());
        if state.needs_redraw || state.hay_animacion() || pendiente {
            match udev_loop.try_borrow_mut() {
                Ok(mut u) => dibujar_todas(state, &mut u),
                // No debería pasar: este callback corre entre vueltas del
                // bucle, con todos los callbacks de las fuentes ya cerrados.
                // Pero si pasara, descartar el frame en silencio dejaría la
                // pantalla congelada para siempre en un escritorio en reposo:
                // `needs_redraw` sigue a `true` y nadie vuelve a mirarlo hasta
                // el siguiente evento, que puede no llegar nunca. El timer
                // fuerza otra vuelta.
                Err(err) => {
                    tracing::warn!("el backend estaba prestado al ir a dibujar: {err}");
                    let r = state
                        .loop_handle
                        .insert_source(Timer::immediate(), |_, _, _| TimeoutAction::Drop);
                    if let Err(err) = r {
                        tracing::error!("no se pudo reintentar el dibujo: {err}");
                    }
                }
            }
        }
        state.post_dispatch();
    })?;
    Ok(())
}

/// La GPU que manda. Con una sola integrada esto es trivial, pero conviene
/// dejarlo explícito para cuando haya híbridas.
fn elegir_gpu(session: &LibSeatSession) -> anyhow::Result<DrmNode> {
    if let Some(path) = primary_gpu(session.seat())? {
        return Ok(DrmNode::from_path(path)?);
    }
    let path = all_gpus(session.seat())?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no hay ninguna GPU disponible en este asiento"))?;
    Ok(DrmNode::from_path(path)?)
}

/// Enciende todas las salidas configuradas, asignando un CRTC distinto a cada
/// conector. Un monitor desconocido se añade a la derecha; uno conocido
/// recupera modo, escala, posición, rotación y VRR por su identificador EDID.
fn crear_salidas(
    manager: &mut Manager,
    renderer: &mut GlesRenderer,
    state: &mut BookosComp,
) -> anyhow::Result<Vec<SalidaKms>> {
    let recursos = manager.device().resource_handles()?;
    let guardadas = pantallas::cargar();
    let activos = recursos
        .connectors()
        .iter()
        .filter_map(|handle| {
            let con = manager.device().get_connector(*handle, true).ok()?;
            if con.state() != connector::State::Connected {
                return None;
            }
            let id = identificador(manager.device(), &con);
            (!guardadas.iter().any(|g| g.id == id && !g.activa)).then_some(*handle)
        })
        .collect::<Vec<_>>();
    let opciones = activos
        .iter()
        .map(|handle| {
            manager
                .device()
                .get_connector(*handle, true)
                .ok()
                .map(|con| crtcs_compatibles(manager, &recursos, &con))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let asignados = asignar_crtcs(&opciones).ok_or_else(|| {
        anyhow::anyhow!("no existe una asignación de CRTC para las pantallas activas")
    })?;
    let mut salidas = Vec::new();
    let mut siguiente_x = 0;

    for handle in recursos.connectors() {
        let Ok(conector) = manager.device().get_connector(*handle, true) else {
            continue;
        };
        if conector.state() != connector::State::Connected {
            continue;
        }
        let id = identificador(manager.device(), &conector);
        let guardada = guardadas.iter().find(|g| g.id == id).cloned();
        if guardada.as_ref().is_some_and(|g| !g.activa) {
            continue;
        }
        let Some(indice) = activos.iter().position(|h| *h == conector.handle()) else {
            continue;
        };
        let crtc = asignados[indice];
        let modo = guardada
            .as_ref()
            .and_then(|g| {
                conector.modes().iter().find(|m| {
                    let (w, h) = m.size();
                    w as u32 == g.ancho
                        && h as u32 == g.alto
                        && m.vrefresh() * 1000 == g.refresco_mhz
                })
            })
            .or_else(|| {
                conector
                    .modes()
                    .iter()
                    .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            })
            .or_else(|| conector.modes().first())
            .copied();
        let Some(modo) = modo else { continue };

        let nombre = format!("{:?}-{}", conector.interface(), conector.interface_id());
        let (mm_w, mm_h) = conector.size().unwrap_or((0, 0));
        let (w, h) = modo.size();
        let (fabricante, modelo_edid, _) = datos_edid(manager.device(), &conector);
        let escala = guardada
            .as_ref()
            .map(|g| g.escala)
            .or_else(|| state.config.as_ref().and_then(|c| c.escala))
            .unwrap_or_else(|| {
                super::escala_sugerida((w as i32, h as i32), (mm_w as i32, mm_h as i32))
            });
        let transformacion = guardada
            .as_ref()
            .and_then(|g| pantallas::transformacion(&g.transformacion))
            .unwrap_or(smithay::utils::Transform::Normal);
        let posicion = guardada
            .as_ref()
            .map_or((siguiente_x, 0).into(), |g| (g.x, g.y).into());
        let output = Output::new(
            nombre.clone(),
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: if fabricante.is_empty() {
                    "BookOS".into()
                } else {
                    fabricante
                },
                model: if modelo_edid.is_empty() {
                    "KMS".into()
                } else {
                    modelo_edid
                },
            },
        );
        let output_mode = OutputMode {
            size: (w as i32, h as i32).into(),
            refresh: (modo.vrefresh() * 1000) as i32,
        };
        output.change_current_state(
            Some(output_mode),
            Some(transformacion),
            Some(Scale::Fractional(escala)),
            Some(posicion),
        );
        output.set_preferred(output_mode);
        let surface = match manager.initialize_output(
            crtc,
            modo,
            &[conector.handle()],
            &output,
            None,
            renderer,
            &DrmOutputRenderElements::<GlesRenderer, OverlayElement>::default(),
        ) {
            Ok(surface) => surface,
            Err(err) => {
                tracing::warn!(pantalla = %nombre, "no se pudo inicializar la salida: {err}");
                continue;
            }
        };
        let global = output.create_global::<BookosComp>(&state.display_handle);
        if guardada.as_ref().is_some_and(|g| g.vrr)
            && let Err(err) = surface.with_compositor(|c| c.use_vrr(true))
        {
            tracing::warn!(pantalla = %nombre, "no se pudo activar VRR: {err}");
        }
        let (lw, _) = pantallas::tamano_logico(
            w as u32,
            h as u32,
            escala,
            pantallas::nombre_transformacion(transformacion),
        );
        siguiente_x = posicion.x + lw;
        tracing::info!(pantalla = %nombre, %id, crtc = ?crtc,
            modo = %format!("{}x{}@{}", w, h, modo.vrefresh()),
            escala, x = posicion.x, y = posicion.y, "salida multipantalla activa");
        if salidas.is_empty() {
            crate::publicar::escala_de_pantalla(escala);
        }
        salidas.push(SalidaKms {
            output,
            global,
            surface,
            conector: conector.handle(),
            id,
            en_vuelo: false,
            pendiente: true,
        });
    }

    if salidas.is_empty() {
        return Err(anyhow::anyhow!(
            "no hay ninguna pantalla conectada utilizable"
        ));
    }
    Ok(salidas)
}

// ── Censo de pantallas ───────────────────────────────────────────────────

/// Fabricante, modelo y número de serie del EDID del monitor.
///
/// Se leen a mano y no con `libdisplay-info` porque hacen falta tres campos de
/// los primeros 128 bytes y la biblioteca traería una dependencia de sistema
/// nueva para eso. El formato de esos bytes lleva congelado desde EDID 1.0.
fn datos_edid(device: &DrmDevice, con: &connector::Info) -> (String, String, String) {
    let Some(bytes) = blob_edid(device, con) else {
        return (String::new(), String::new(), String::new());
    };
    if bytes.len() < 128 {
        return (String::new(), String::new(), String::new());
    }

    // Bytes 8-9: tres letras de cinco bits, en big-endian, con 'A' = 1.
    let id = u16::from_be_bytes([bytes[8], bytes[9]]);
    let letra = |desplazamiento: u16| -> char {
        let v = ((id >> desplazamiento) & 0x1f) as u8;
        if (1..=26).contains(&v) {
            (b'A' + v - 1) as char
        } else {
            '?'
        }
    };
    let fabricante: String = [letra(10), letra(5), letra(0)].iter().collect();

    // Los cuatro descriptores de 18 bytes que empiezan en el 54. El 0xFC lleva
    // el nombre del monitor y el 0xFF su número de serie, ambos en ASCII
    // terminado en 0x0A y rellenado con espacios.
    let mut modelo = String::new();
    let mut serie = String::new();
    for i in 0..4 {
        let d = &bytes[54 + i * 18..54 + i * 18 + 18];
        if d[0] != 0 || d[1] != 0 || d[2] != 0 {
            continue;
        }
        let texto: String = d[5..18]
            .iter()
            .take_while(|&&b| b != 0x0a)
            .map(|&b| b as char)
            .collect();
        match d[3] {
            0xfc => modelo = texto.trim().to_string(),
            0xff => serie = texto.trim().to_string(),
            _ => {}
        }
    }
    if modelo.is_empty() {
        // Sin descriptor de nombre queda el código de producto, que al menos
        // distingue dos paneles del mismo fabricante.
        modelo = format!("{:04X}", u16::from_le_bytes([bytes[10], bytes[11]]));
    }
    if serie.is_empty() {
        let n = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        if n != 0 {
            serie = n.to_string();
        }
    }
    (fabricante, modelo, serie)
}

fn blob_edid(device: &DrmDevice, con: &connector::Info) -> Option<Vec<u8>> {
    let props = device.get_properties(con.handle()).ok()?;
    let (handles, valores) = props.as_props_and_values();
    for (handle, valor) in handles.iter().zip(valores.iter()) {
        let info = device.get_property(*handle).ok()?;
        if info.name().to_str().ok()? != "EDID" {
            continue;
        }
        if let PropValue::Blob(id) = info.value_type().convert_value(*valor) {
            return device.get_property_blob(id).ok();
        }
    }
    None
}

/// ¿El conector admite frecuencia variable? Es una propiedad del conector, así
/// que se puede saber sin encenderlo.
fn vrr_capaz(device: &DrmDevice, con: &connector::Info) -> bool {
    let Ok(props) = device.get_properties(con.handle()) else {
        return false;
    };
    let (handles, valores) = props.as_props_and_values();
    for (handle, valor) in handles.iter().zip(valores.iter()) {
        let Ok(info) = device.get_property(*handle) else {
            continue;
        };
        if info.name().to_str() != Ok("vrr_capable") {
            continue;
        }
        return match info.value_type().convert_value(*valor) {
            PropValue::Boolean(v) => v,
            PropValue::UnsignedRange(v) => v != 0,
            _ => false,
        };
    }
    false
}

/// Identificador estable de una pantalla. Ver la cabecera de
/// [`crate::pantallas`] para el porqué.
fn identificador(device: &DrmDevice, con: &connector::Info) -> String {
    let (fabricante, modelo, serie) = datos_edid(device, con);
    let conector = format!("{:?}-{}", con.interface(), con.interface_id());
    let bruto = if fabricante.is_empty() && modelo.is_empty() {
        conector
    } else if serie.is_empty() {
        // Sin número de serie dos monitores idénticos comparten identificador,
        // así que se desempata con el conector. Es peor que el EDID solo —
        // cambiar de puerto cambia el id— pero es lo único que los distingue.
        format!("{fabricante}-{modelo}-{conector}")
    } else {
        format!("{fabricante}-{modelo}-{serie}")
    };
    bruto
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Los modos de un conector, ya en la forma que viaja por D-Bus.
///
/// El refresco sale de `vrefresh`, que el kernel da redondeado a hercios
/// enteros: 90 y 120 salen exactos, y un modo de 59,94 se anuncia como 60000
/// mHz. Es la misma cuenta que ya hacía el `OutputMode` de esta salida, y
/// mantenerla igual es lo que permite emparejar por número lo que pide Settings
/// con lo que tiene el conector.
fn modos_de(
    con: &connector::Info,
    actual: Option<&smithay::reexports::drm::control::Mode>,
) -> Vec<Modo> {
    con.modes()
        .iter()
        .map(|m| {
            let (w, h) = m.size();
            Modo {
                ancho: w as u32,
                alto: h as u32,
                refresco_mhz: m.vrefresh() * 1000,
                preferido: m.mode_type().contains(ModeTypeFlags::PREFERRED),
                actual: actual
                    .is_some_and(|a| a.size() == m.size() && a.vrefresh() == m.vrefresh()),
            }
        })
        .collect()
}

/// El censo de todo lo que hay enchufado.
///
/// Censo de conectores y de sus escenas KMS activas.
fn censar(u: &Udev) -> Vec<Salida> {
    let device = u.manager.device();
    let Ok(recursos) = device.resource_handles() else {
        return Vec::new();
    };
    recursos
        .connectors()
        .iter()
        .filter_map(|handle| device.get_connector(*handle, true).ok())
        .filter(|con| con.state() == connector::State::Connected)
        .map(|con| {
            let activa = u.salidas.iter().find(|s| s.conector == con.handle());
            let encendida = activa.is_some();
            let (fabricante, modelo, serie) = datos_edid(device, &con);
            let (mm_w, mm_h) = con.size().unwrap_or((0, 0));
            let modo_actual = activa.map(|s| s.surface.with_compositor(|c| c.pending_mode()));
            let modos = modos_de(&con, modo_actual.as_ref());
            let escala = if let Some(salida) = activa {
                salida.output.current_scale().fractional_scale()
            } else {
                // Todavía no está encendida: se enseña lo que se le pondría.
                let preferido = modos.iter().find(|m| m.preferido).or_else(|| modos.first());
                preferido.map_or(1.0, |m| {
                    super::escala_sugerida(
                        (m.ancho as i32, m.alto as i32),
                        (mm_w as i32, mm_h as i32),
                    )
                })
            };
            let (ancho, alto) = modos
                .iter()
                .find(|m| m.actual)
                .or_else(|| modos.iter().find(|m| m.preferido))
                .or_else(|| modos.first())
                .map_or((0, 0), |m| (m.ancho, m.alto));
            let transformacion = activa.map_or("normal", |s| {
                pantallas::nombre_transformacion(s.output.current_transform())
            });
            let posicion = activa
                .map(|s| s.output.current_location())
                .unwrap_or_default();
            let vrr = activa.is_some_and(|s| s.surface.with_compositor(|c| c.vrr_enabled()));
            let (logico_ancho, logico_alto) =
                pantallas::tamano_logico(ancho, alto, escala, transformacion);
            Salida {
                id: identificador(device, &con),
                conector: format!("{:?}-{}", con.interface(), con.interface_id()),
                fabricante,
                modelo,
                serie,
                mm_ancho: mm_w,
                mm_alto: mm_h,
                activa: encendida,
                modos,
                escala,
                escalas: pantallas::ESCALAS.to_vec(),
                x: posicion.x,
                y: posicion.y,
                transformacion: transformacion.to_string(),
                vrr_capaz: vrr_capaz(device, &con),
                vrr: encendida && vrr,
                principal: activa.is_some_and(|s| s.id == u.principal_id),
                logico_ancho,
                logico_alto,
            }
        })
        .collect()
}

/// Aplica una configuración ya validada al hardware.
///
/// Devuelve el censo releído, no lo que se pidió: si el driver acabó en otro
/// sitio, lo que gana es lo que hay.
fn aplicar_en_kms(
    u: &mut Udev,
    lazo: &LoopHandle<'static, BookosComp>,
    peticion: &[Peticion],
) -> Result<Aplicado, String> {
    let recursos = u
        .manager
        .device()
        .resource_handles()
        .map_err(|err| format!("no se pudieron enumerar los recursos DRM: {err}"))?;
    // Preflight completo antes de soltar una salida viva. Los modos y la
    // asignación de CRTC se comprueban primero para que una petición inválida
    // no deje la sesión negra a mitad de la reconstrucción.
    let mut opciones_crtc = Vec::new();
    for pedido in peticion.iter().filter(|p| p.activa) {
        let conector = recursos
            .connectors()
            .iter()
            .filter_map(|h| u.manager.device().get_connector(*h, true).ok())
            .find(|c| {
                c.state() == connector::State::Connected
                    && identificador(u.manager.device(), c) == pedido.id
            })
            .ok_or_else(|| format!("la pantalla «{}» ya no está conectada", pedido.id))?;
        if !conector.modes().iter().any(|m| {
            let (w, h) = m.size();
            w as u32 == pedido.ancho
                && h as u32 == pedido.alto
                && m.vrefresh() * 1000 == pedido.refresco_mhz
        }) {
            return Err(format!(
                "«{}» no tiene el modo {}x{}@{}",
                pedido.id, pedido.ancho, pedido.alto, pedido.refresco_mhz
            ));
        }
        opciones_crtc.push(crtcs_compatibles(&u.manager, &recursos, &conector));
    }
    let asignados = asignar_crtcs(&opciones_crtc).ok_or_else(|| {
        "no existe una asignación de CRTC compatible para esta combinación".to_string()
    })?;

    // Se reconstruyen únicamente las superficies KMS, no el compositor ni la
    // sesión. Soltar `DrmOutput` libera su CRTC y permite encender/apagar o
    // cambiar el cable en caliente sin reiniciar BookOS. Antes se retiran sus
    // globales `wl_output` para no acumular salidas fantasma en los clientes.
    //
    // En dos tiempos, como pide la documentación de `remove_global`: un
    // cliente que hiciera `bind` justo mientras se quita recibiría un error
    // de protocolo y libwayland lo desconectaría. Deshabilitado, ya no se
    // anuncia pero el `bind` en vuelo sigue valiendo; a los cinco segundos
    // —«unos segundos», dice la documentación— se libera.
    let mut viejas: Vec<_> = u
        .salidas
        .drain(..)
        .map(|vieja| {
            u.display_handle
                .disable_global::<BookosComp>(vieja.global.clone());
            vieja.global
        })
        .collect();
    if !viejas.is_empty() {
        let r = lazo.insert_source(
            Timer::from_duration(std::time::Duration::from_secs(5)),
            move |_, _, state| {
                for global in viejas.drain(..) {
                    state.display_handle.remove_global::<BookosComp>(global);
                }
                TimeoutAction::Drop
            },
        );
        if let Err(err) = r {
            tracing::warn!("los wl_output retirados se quedan deshabilitados: {err}");
        }
    }
    for (pedido, crtc) in peticion.iter().filter(|p| p.activa).zip(asignados) {
        let conector = recursos
            .connectors()
            .iter()
            .filter_map(|h| u.manager.device().get_connector(*h, true).ok())
            .find(|c| {
                c.state() == connector::State::Connected
                    && identificador(u.manager.device(), c) == pedido.id
            })
            .ok_or_else(|| format!("la pantalla «{}» ya no está conectada", pedido.id))?;
        let modo = conector
            .modes()
            .iter()
            .find(|m| {
                let (w, h) = m.size();
                w as u32 == pedido.ancho
                    && h as u32 == pedido.alto
                    && m.vrefresh() * 1000 == pedido.refresco_mhz
            })
            .copied()
            .ok_or_else(|| {
                format!(
                    "«{}» no tiene el modo {}x{}@{}",
                    pedido.id, pedido.ancho, pedido.alto, pedido.refresco_mhz
                )
            })?;
        let transformacion = pantallas::transformacion(&pedido.transformacion)
            .ok_or_else(|| format!("rotación «{}» desconocida", pedido.transformacion))?;
        let nombre = format!("{:?}-{}", conector.interface(), conector.interface_id());
        let (mm_w, mm_h) = conector.size().unwrap_or((0, 0));
        let (fabricante, modelo, _) = datos_edid(u.manager.device(), &conector);
        let output = Output::new(
            nombre,
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: if fabricante.is_empty() {
                    "BookOS".into()
                } else {
                    fabricante
                },
                model: if modelo.is_empty() {
                    "KMS".into()
                } else {
                    modelo
                },
            },
        );
        let output_mode = OutputMode {
            size: (pedido.ancho as i32, pedido.alto as i32).into(),
            refresh: pedido.refresco_mhz as i32,
        };
        output.change_current_state(
            Some(output_mode),
            Some(transformacion),
            Some(Scale::Fractional(pedido.escala)),
            Some((pedido.x, pedido.y).into()),
        );
        output.set_preferred(output_mode);
        let surface = u
            .manager
            .initialize_output(
                crtc,
                modo,
                &[conector.handle()],
                &output,
                None,
                &mut u.renderer,
                &DrmOutputRenderElements::<GlesRenderer, OverlayElement>::default(),
            )
            .map_err(|err| format!("KMS rechazó «{}»: {err}", pedido.id))?;
        if pedido.vrr {
            surface
                .with_compositor(|c| c.use_vrr(true))
                .map_err(|err| format!("no se pudo activar VRR en «{}»: {err}", pedido.id))?;
        }
        let global = output.create_global::<BookosComp>(&u.display_handle);
        u.salidas.push(SalidaKms {
            output,
            global,
            surface,
            conector: conector.handle(),
            id: pedido.id.clone(),
            en_vuelo: false,
            pendiente: true,
        });
    }

    let principal = peticion
        .iter()
        .find(|p| p.activa && p.principal)
        .and_then(|p| u.salidas.iter().position(|s| s.id == p.id))
        .unwrap_or(0);
    u.principal_id = u.salidas[principal].id.clone();
    let salidas = censar(u);
    let mapa = u
        .salidas
        .iter()
        .map(|s| (s.output.clone(), s.output.current_location()))
        .collect::<Vec<_>>();
    Ok(Aplicado {
        salidas,
        mapa,
        principal,
    })
}

fn crtcs_compatibles(
    manager: &Manager,
    recursos: &smithay::reexports::drm::control::ResourceHandles,
    con: &connector::Info,
) -> Vec<crtc::Handle> {
    con.encoders()
        .iter()
        .filter_map(|enc| manager.device().get_encoder(*enc).ok())
        .flat_map(|enc| recursos.filter_crtcs(enc.possible_crtcs()))
        .fold(Vec::new(), |mut lista, crtc| {
            if !lista.contains(&crtc) {
                lista.push(crtc);
            }
            lista
        })
}

/// Emparejamiento con retroceso: elegir siempre el primer CRTC compatible
/// falla si A admite 0/1 y B solo 0. Hay muy pocos CRTCs (normalmente 2–4), así
/// que probar las combinaciones es más seguro y despreciable en coste.
fn asignar_crtcs(opciones: &[Vec<crtc::Handle>]) -> Option<Vec<crtc::Handle>> {
    fn buscar(
        i: usize,
        opciones: &[Vec<crtc::Handle>],
        usados: &mut Vec<crtc::Handle>,
        resultado: &mut Vec<crtc::Handle>,
    ) -> bool {
        if i == opciones.len() {
            return true;
        }
        for crtc in opciones[i].iter().copied() {
            if usados.contains(&crtc) {
                continue;
            }
            usados.push(crtc);
            resultado.push(crtc);
            if buscar(i + 1, opciones, usados, resultado) {
                return true;
            }
            resultado.pop();
            usados.pop();
        }
        false
    }
    let mut usados = Vec::new();
    let mut resultado = Vec::with_capacity(opciones.len());
    buscar(0, opciones, &mut usados, &mut resultado).then_some(resultado)
}

/// Dibuja cada salida que esté libre. Los vblank son independientes: una
/// pantalla a 144 Hz no espera a otra de 60 Hz y cada swapchain conserva su
/// propio daño.
fn dibujar_todas(state: &mut BookosComp, u: &mut Udev) {
    // Con DPMS apagado no se encola nada: tras `clear()` el siguiente
    // `queue_frame` vuelve a encender el panel. Al dormir desde el diálogo, la
    // entrada animada del bloqueo lo encendía justo después de apagarlo, y se
    // veía el candado hasta que el equipo se suspendía. `needs_redraw` se
    // conserva para que lo primero al encender sea el estado de entonces.
    if !u.activa || !state.dpms_encendido {
        return;
    }
    // Antes de mirar nada: lo que se mueve solo avanza **una vez** por vuelta,
    // no una por salida. Ver `backend::avanzar_animaciones`.
    crate::backend::avanzar_animaciones(state);
    let animando = state.hay_animacion();
    let Udev {
        renderer,
        salidas,
        repintado_completo,
        ..
    } = u;
    // Antes de componer nada: si el tema cambió, el fondo y el cristal tienen
    // que ser ya los nuevos en este mismo fotograma.
    crate::backend::recargar_fondo(state, renderer);
    crate::multimedia::vigilar_centro(state);
    if state.needs_redraw || animando {
        for salida in salidas.iter_mut() {
            salida.pendiente = true;
        }
    }
    for salida in salidas {
        if *repintado_completo {
            // Edad 0 obliga a dibujar la salida entera sin reasignar buffers.
            salida
                .surface
                .with_compositor(|compositor| compositor.reset_buffer_ages());
        }
        dibujar_salida(state, renderer, salida, animando);
    }
    state.needs_redraw = false;
}

fn dibujar_salida(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    salida: &mut SalidaKms,
    animando: bool,
) {
    if salida.en_vuelo {
        // Hay trabajo y el hardware todavía no ha enseñado el fotograma
        // anterior: se apunta y se dibuja en el vblank. Contarlo es lo que
        // distingue «el compositor va justo» de «el compositor va sobrado».
        if salida.pendiente || animando {
            state.metricas.aplazado(&salida.output.name());
        }
        return;
    }
    if !salida.pendiente && !animando {
        return;
    }
    // El nombre se pide aquí y no arriba porque `name()` clona la cadena, y con
    // dos monitores esta función se llama también para el que no tiene nada que
    // dibujar.
    let nombre = salida.output.name();

    let escala = salida.output.current_scale().fractional_scale();
    let modo_actual = salida.output.current_mode();
    if let Some(modo) = modo_actual {
        state.metricas.modo(
            &nombre,
            modo.size.w,
            modo.size.h,
            escala,
            // `refresh` viene en mHz.
            modo.refresh as f32 / 1000.0,
            salida.surface.with_compositor(|c| c.vrr_enabled()),
        );
    }
    // Un refresco, para el umbral de los frame callbacks (ver `enviar_frames`).
    // Sin modo —no debería pasar con la salida encendida— se usan 60 Hz, que
    // solo decide cada cuánto arranca una superficie todavía sin salida
    // asignada, no el ritmo de dibujo.
    let periodo = modo_actual
        .filter(|m| m.refresh > 0)
        .map(|m| Duration::from_nanos(1_000_000_000_000u64 / m.refresh as u64))
        .unwrap_or(Duration::from_millis(16));

    let cronometro = std::time::Instant::now();
    let elementos = crate::backend::escena(state, renderer, &salida.output);
    let origen: Point<i32, Physical> = salida
        .output
        .current_location()
        .to_f64()
        .to_physical_precise_round(escala);
    // `escena` trabaja en el escritorio lógico global. KMS espera coordenadas
    // físicas locales al CRTC; trasladar al final conserva ventanas que cruzan
    // dos salidas incluso cuando cada una usa una escala distinta.
    let elementos = elementos
        .into_iter()
        .map(|elemento| {
            RelocateRenderElement::from_element(
                elemento,
                (-origen.x, -origen.y),
                Relocate::Relative,
            )
        })
        .collect::<Vec<_>>();

    let t_escena = cronometro.elapsed();
    let cronometro = std::time::Instant::now();

    // `FrameFlags::DEFAULT` deja que el compositor use los planos: es lo que
    // habilita el direct scanout y el cursor por hardware.
    //
    // **Sin `SKIP_CURSOR_ONLY_UPDATES`**, aunque suene a que ahorraría trabajo
    // en el caso más común —mover el ratón—. Lo que hace ese flag es marcar el
    // plano del cursor como `skip` y devolver el fotograma como vacío; aquí eso
    // significa salir sin `queue_frame`, o sea que el movimiento **nunca llega
    // a KMS** y el puntero se queda clavado hasta que otra cosa dañe la
    // pantalla. En Smithay existe para VRR, donde un movimiento de ratón no
    // debe forzar un flip antes de tiempo, no como optimización general.
    let resultado = salida.surface.render_frame(
        renderer,
        &elementos,
        [0.05, 0.05, 0.06, 1.0],
        FrameFlags::DEFAULT,
    );

    let resultado = match resultado {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("fallo al dibujar: {err}");
            // El pendiente se suelta aunque el fotograma se pierda: si se
            // dejara puesto, `hay_pendiente` haría que el bucle volviera a
            // intentarlo sin descanso y un fallo persistente del driver
            // pasaría de una pantalla congelada a un núcleo al 100 %.
            salida.pendiente = false;
            return;
        }
    };

    if resultado.is_empty {
        // Nada cambió: no se toca la pantalla. Este es el caso normal en un
        // escritorio en reposo.
        state.metricas.saltado(&nombre);
        salida.pendiente = false;
        // Los frame callbacks salen igual. Un cliente puede pedir el callback
        // sin dañar nada —Firefox lo hace para engancharse al vsync— y si aquí
        // se vuelve sin contestarle se queda esperando un frame que nadie le va
        // a dar: deja de dibujar del todo hasta que otro evento mueva la
        // pantalla.
        enviar_frames(state, &salida.output, periodo);
        return;
    }

    let mut feedback = OutputPresentationFeedback::new(&salida.output);
    for window in state.space.elements() {
        window.with_surfaces(|surface, data| {
            update_surface_primary_scanout_output(
                surface,
                &salida.output,
                data,
                &resultado.states,
                default_primary_scanout_output_compare,
            );
        });
        window.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, &resultado.states)
            },
        );
    }

    match salida.surface.queue_frame(feedback) {
        Ok(()) => {
            salida.en_vuelo = true;
            salida.pendiente = false;
        }
        Err(err) => tracing::error!("queue_frame: {err}"),
    }
    // El coste se apunta con el encolado dentro: `queue_frame` es donde se
    // paga el atómico de KMS, y dejarlo fuera escondería justo la parte que se
    // come el plazo cuando el driver va apretado.
    state
        .metricas
        .dibujado(&nombre, t_escena, cronometro.elapsed());
    // Los frame callbacks salen después de encolar: es la señal de "puedes
    // dibujar el siguiente".
    enviar_frames(state, &salida.output, periodo);
    // Las capturas van al final, con la escena ya presentada, y componen su
    // propio buffer. Ver `backend::servir_capturas`.
    crate::backend::servir_capturas(state, renderer, &salida.output);
    crate::backend::servir_captura_propia(state, renderer, &salida.output);
}

/// Pasa un instante del reloj de pared al monotónico, midiendo ahora el desfase
/// entre los dos.
///
/// Hace falta para los drivers que anuncian los page-flip en `CLOCK_REALTIME`.
/// `wp_presentation` se anunció con el reloj monotónico (`PresentationState::
/// new(&dh, 1)`), así que entregarle otra cosa sería mentirle al cliente.
fn trasladar_a_monotonico(
    t: std::time::SystemTime,
) -> smithay::utils::Time<smithay::utils::Monotonic> {
    use smithay::reexports::rustix::time::{ClockId, clock_gettime};
    let ts = clock_gettime(ClockId::Monotonic);
    let mono = Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32);
    // Cuánto hace del flip. `unwrap_or_default` cubre la marca posterior a
    // "ahora" —relojes reajustados entre medias—: ahí lo más cercano a la
    // verdad es "acaba de pasar", no un instante en el futuro.
    let antiguedad = t.elapsed().unwrap_or_default();
    smithay::utils::Time::<smithay::utils::Monotonic>::from(mono.saturating_sub(antiguedad))
}

/// Contesta a los frame callbacks de todo lo que se enseña en esta salida.
///
/// El `throttle` **no es `None`**, y ese detalle decide si una ventana nueva
/// llega a pintar. Con `None`, Smithay solo contesta a las superficies cuya
/// salida de scanout principal es esta —`frame_overdue` se queda en `false`
/// para siempre, ver `SurfaceFrameThrottlingState::update`—, y esa salida solo
/// se asigna en el camino de fotograma **no vacío**, recorriendo
/// `resultado.states`. O sea que una superficie que todavía no ha aparecido en
/// ningún `states` —la pestaña que acabas de abrir, el primer fotograma de una
/// ventana— no recibe callback **nunca** y se queda esperando a que otra cosa
/// mueva la pantalla. Es el «abro una pestaña y se queda pillada».
///
/// Con un periodo de refresco como umbral, esas superficies reciben un
/// callback por vuelta de vblank y arrancan, mientras que las que están de
/// verdad ocultas tras otra ventana siguen limitadas a uno por refresco en vez
/// de correr en vacío, que es lo que pasaría con `Some(Duration::ZERO)`.
fn enviar_frames(state: &mut BookosComp, output: &Output, periodo: Duration) {
    let tiempo = state.start_time.elapsed();
    for window in state.space.elements() {
        window.send_frame(
            output,
            tiempo,
            Some(periodo),
            surface_primary_scanout_output,
        );
    }
    crate::backend::enviar_frames_capas(output, tiempo, periodo);
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.vista_escritorios_abierta())
    {
        let salida_vista = crate::escritorios::salida_para_vista(state);
        let activo = state.escritorios.activo_en(&salida_vista);
        for (i, ventanas) in crate::escritorios::ventanas_para_vista(state, &salida_vista)
            .iter()
            .enumerate()
        {
            if i == activo {
                continue;
            }
            for (window, _) in ventanas {
                window.send_frame(
                    output,
                    tiempo,
                    Some(periodo),
                    surface_primary_scanout_output,
                );
            }
        }
    }
}

/// Aplica los ajustes de entrada a un dispositivo recién aparecido.
///
/// Sin esto los dispositivos se quedan con los valores de fábrica de libinput,
/// que en un portátil significa touchpad lento, sin toque para hacer clic y con
/// el scroll invertido respecto a lo que hace todo lo demás. No es un detalle
/// de comodidad: es la diferencia entre poder usar el escritorio o no.
///
/// El `Device` llega clonado, y da igual: el clon es un `libinput_device_ref`
/// del mismo dispositivo, no una copia de sus ajustes.
fn configurar_dispositivo(
    device: &mut InputDevice,
    cfg: &bookos_shell::Entrada,
    touchpad_activo: bool,
) {
    // Un `config_*_set_*` que falla casi siempre es "este dispositivo no tiene
    // eso" —un teclado no tiene aceleración— y no un error que merezca traza.
    // Lo que sí merece traza es el resultado, para poder ver de un vistazo qué
    // le tocó a cada dispositivo cuando algo se sienta raro.
    let touchpad = device.config_tap_finger_count() > 0;
    let mut aplicado = Vec::new();

    if device.config_accel_is_available() {
        let velocidad = if touchpad {
            cfg.velocidad_touchpad
        } else {
            cfg.velocidad_raton
        };
        // Adaptativo y no plano: es el que acelera según lo rápido que muevas,
        // o sea el que deja llegar de esquina a esquina de una pasada sin
        // perder precisión al ir despacio. Es también lo que hacen KWin y
        // GNOME por defecto.
        let _ = device.config_accel_set_profile(AccelProfile::Adaptive);
        if device.config_accel_set_speed(velocidad).is_ok() {
            aplicado.push(format!("velocidad={velocidad}"));
        }
    }

    if touchpad {
        // Apagado con la tecla de función, el dispositivo se desactiva entero:
        // así no llegan ni toques ni desplazamientos, que es lo que se espera
        // al apagarlo para escribir sin que la palma mueva el cursor.
        let _ = device.config_send_events_set_mode(if touchpad_activo {
            smithay::reexports::input::SendEventsMode::ENABLED
        } else {
            smithay::reexports::input::SendEventsMode::DISABLED
        });
        if device.config_tap_set_enabled(cfg.toque_para_clic).is_ok() {
            aplicado.push(format!("toque={}", cfg.toque_para_clic));
        }
        // Clickfinger en vez de zonas: en un clickpad sin botones físicos,
        // repartir la mitad inferior en zonas se come sitio útil y hace que un
        // clic normal abajo a la derecha salga como botón derecho sin querer.
        let _ = device.config_click_set_method(ClickMethod::Clickfinger);
        // El puntero se congela mientras se escribe. Sin esto, la palma roza
        // el touchpad a mitad de una frase y el cursor se va a otra línea.
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(true);
        }
    }

    if device.has_capability(smithay::reexports::input::DeviceCapability::Touch) {
        let modo = if cfg.pantalla_tactil {
            smithay::reexports::input::SendEventsMode::ENABLED
        } else {
            smithay::reexports::input::SendEventsMode::DISABLED
        };
        if device.config_send_events_set_mode(modo).is_ok() {
            aplicado.push(format!("tactil={}", cfg.pantalla_tactil));
        }
    }

    if device.config_scroll_has_natural_scroll()
        && device
            .config_scroll_set_natural_scroll_enabled(cfg.scroll_natural)
            .is_ok()
    {
        aplicado.push(format!("scroll_natural={}", cfg.scroll_natural));
    }

    if !aplicado.is_empty() {
        tracing::info!(
            dispositivo = device.name(),
            touchpad,
            ajustes = aplicado.join(" "),
            "entrada configurada"
        );
    }
}

/// Las luces de Smithay en el formato de libinput. `None` es «este teclado no
/// sabe de esa luz»: se deja apagada.
fn luces(leds: smithay::input::keyboard::LedState) -> smithay::reexports::input::Led {
    use smithay::reexports::input::Led;
    let mut luces = Led::empty();
    luces.set(Led::CAPSLOCK, leds.caps == Some(true));
    luces.set(Led::NUMLOCK, leds.num == Some(true));
    luces.set(Led::SCROLLLOCK, leds.scroll == Some(true));
    luces
}
