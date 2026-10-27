//! Implementación de los protocolos Wayland que el compositor expone.
//!
//! Cada `commit` de un cliente marca `needs_redraw`. Ese es el único camino por
//! el que se llega a dibujar: si nadie hace commit, no hay frame.

use smithay::desktop::Window;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::Client;
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState,
};
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
    ServerDndGrabHandler,
};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleHandler};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface as InputMethodPopupSurface};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::primary_selection::{
    set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::desktop::{layer_map_for_output, LayerSurface as DesktopLayerSurface, WindowSurfaceType};
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_fractional_scale,
    delegate_output, delegate_primary_selection, delegate_seat, delegate_shm, delegate_viewporter,
    delegate_xdg_shell, delegate_presentation, delegate_idle_inhibit,
    delegate_idle_notify,
    delegate_pointer_constraints, delegate_relative_pointer, delegate_text_input_manager,
    delegate_input_method_manager,
    delegate_layer_shell,
    delegate_foreign_toplevel_list,
};

use crate::state::{BookosComp, ClientState};

impl CompositorHandler for BookosComp {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    /// XWayland no se conecta como los demás: Smithay le pone su propio
    /// `ClientData` al lanzarlo, así que buscar aquí el nuestro devuelve `None`
    /// y antes eso era un `unwrap` — el compositor se caía en el primer commit
    /// de la primera ventana X11, antes incluso de que se viera.
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(datos) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &datos.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("todo cliente entra por nuestro socket o es XWayland")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        use smithay::backend::renderer::utils::on_commit_buffer_handler;
        on_commit_buffer_handler::<Self>(surface);

        // Sube al toplevel: el estado de la ventana vive en la superficie raíz.
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        // Un menú de cliente. No es una ventana del `Space` —cuelga de una— y
        // su primer commit, vacío, es el que espera el `configure` sin el que
        // no puede dibujarse.
        self.popups.commit(surface);
        if let Some(smithay::desktop::PopupKind::Xdg(popup)) = self.popups.find_popup(&root) {
            if !popup.is_initial_configure_sent()
                && let Err(err) = popup.send_configure()
            {
                tracing::warn!("no se pudo configurar un menú del cliente: {err:?}");
            }
            self.needs_redraw = true;
            return;
        }

        // Las layer-surfaces no son ventanas del `Space`, pero sus commits sí
        // cambian una parte visible de la escena. Reorganizar aquí respeta el
        // tamaño, anclas y zona exclusiva que el cliente acaba de confirmar.
        if let Some(layer) = self.space.layer_for_surface(&root, WindowSurfaceType::ALL) {
            for output in self.space.outputs().cloned().collect::<Vec<_>>() {
                let mut map = layer_map_for_output(&output);
                if map
                    .layer_for_surface(&root, WindowSurfaceType::ALL)
                    .is_some()
                {
                    map.arrange();
                    layer.layer_surface().send_pending_configure();
                    break;
                }
            }
            self.needs_redraw = true;
            return;
        }

        // Una sola búsqueda para todo lo que viene. Antes se recorría el
        // espacio dos veces —aquí y otra vez dentro de `send_initial_configure`
        // — en **cada** commit de **cada** superficie, subsuperficies y cursor
        // incluidos.
        let Some(window) = self.window_for_surface(&root) else {
            // El commit no es de una ventana. Solo obliga a repintar si es la
            // superficie que el cliente ha puesto de cursor: lo demás no se ve,
            // y marcarlo arrastraba un frame entero por cada commit invisible.
            if self.cursor_surface_is(surface) {
                self.needs_redraw = true;
            }
            return;
        };

        if !is_sync_subsurface(surface) {
            window.on_commit();
        }

        // La primera configuración de un xdg_surface tiene que salir antes de
        // que el cliente pueda adjuntar buffer; si no, se queda esperando.
        self.send_initial_configure(&window, &root);
        self.colocar_si_es_nueva(&window);
        if crate::ventanas::tamano_nuevo(&window) {
            self.recolocar_si_creció(&window);
            self.revisar_barras();
        }
        self.needs_redraw = true;
    }
}

impl BookosComp {
    /// Reenvía a cada superficie la escala de la salida que ocupa. Si una
    /// ventana cruza dos monitores se elige la mayor, para que ninguno tenga
    /// que ampliar un buffer de menor resolución; el compositor recorta y
    /// reduce la misma superficie independientemente en cada escena.
    ///
    /// Hace falta al cambiar de escala en caliente: `new_fractional_scale` solo
    /// cubre a los clientes que se conectan después.
    pub fn broadcast_preferred_scale(&mut self, scale: f64) {
        use smithay::wayland::compositor::with_states;
        let surfaces: Vec<_> = self
            .space
            .elements()
            .filter_map(|w| w.wl_surface().map(|s| s.into_owned()))
            .collect();
        for surface in surfaces {
            let scale = self
                .window_for_surface(&surface)
                .and_then(|window| {
                    self.space
                        .outputs_for_element(&window)
                        .into_iter()
                        .map(|o| o.current_scale().fractional_scale())
                        .max_by(f64::total_cmp)
                })
                .unwrap_or(scale);
            with_states(&surface, |states| {
                with_fractional_scale(states, |fractional| {
                    fractional.set_preferred_scale(scale);
                });
            });
        }
    }

    /// El tema que toca ahora mismo con el modo elegido y el reloj.
    pub fn tema_que_toca(&self) -> bookos_shell::tema::Tema {
        self.modo_tema.resolver(
            self.horas_tema.0,
            self.horas_tema.1,
            bookos_shell::hora_local_ahora(),
        )
    }

    /// Cuánto falta para el próximo cambio automático, si va a haberlo.
    pub fn hasta_el_cambio_de_tema(&self) -> Option<std::time::Duration> {
        self.modo_tema
            .minutos_al_cambio(
                self.horas_tema.0,
                self.horas_tema.1,
                bookos_shell::hora_local_ahora(),
            )
            .map(|min| std::time::Duration::from_secs(min as u64 * 60))
    }

    /// La salida donde está el puntero, o la principal si está en un hueco.
    ///
    /// Es la salida sobre la que actúan las acciones que tienen que elegir una:
    /// cambiar de escritorio, maximizar, encajar. La principal se reconoce por
    /// tener el origen en (0,0) —`pantallas::normalizar` la lleva ahí—.
    pub fn salida_del_puntero(&self) -> Option<&smithay::output::Output> {
        self.space
            .outputs()
            .find(|o| {
                self.space
                    .output_geometry(o)
                    .is_some_and(|r| r.to_f64().contains(self.pointer_location))
            })
            .or_else(|| {
                self.space
                    .outputs()
                    .find(|o| o.current_location() == (0, 0).into())
            })
            .or_else(|| self.space.outputs().next())
    }

    /// Escala de la salida bajo el puntero (o de la principal). Es la mejor
    /// aproximación antes de que una superficie nueva esté mapeada.
    pub fn output_scale(&self) -> f64 {
        self.space
            .outputs()
            .find(|o| {
                self.space
                    .output_geometry(o)
                    .is_some_and(|r| r.to_f64().contains(self.pointer_location))
            })
            .or_else(|| {
                self.space
                    .outputs()
                    .find(|o| o.current_location() == (0, 0).into())
            })
            .or_else(|| self.space.outputs().next())
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0)
    }

    /// La ventana cuya superficie raíz es `surface`.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.wl_surface().is_some_and(|s| s.as_ref() == surface))
            .cloned()
    }

    /// Le pasa al dock qué aplicaciones tienen ventana.
    ///
    /// Se llama al mapear, al cerrar y cuando un cliente fija su `app_id`, no
    /// por frame ni por commit: son los tres únicos momentos en que la lista
    /// puede cambiar, y recorrer las ventanas leyendo su estado no es algo que
    /// deba pasar sesenta veces por segundo.
    pub fn actualizar_dock(&mut self) {
        use smithay::utils::IsAlive;
        // Una minimizada puede morir mientras está guardada —el cliente se
        // cierra solo, o lo mata alguien—: aquí es donde se entierra, que es el
        // mismo momento en que el dock se entera de que ya no está.
        self.minimizadas.retain(|(w, _)| w.alive());
        self.minimizando.retain(|(w, _)| w.alive());
        // Las minimizadas cuentan como abiertas: no están en el `Space`, pero
        // el punto del dock dice «esta aplicación está en marcha», no «esta
        // aplicación se ve ahora mismo». Sin ellas, el icono se apagaba al
        // minimizar y volvía a encenderse al restaurar.
        //
        // Las de **otros escritorios** no cuentan, y es a propósito: el dock
        // enseña lo que hay en el escritorio en el que estás. Viven en
        // `escritorios::guardadas` y no se consultan aquí. Las apartadas por
        // «mostrar el escritorio» en este sí: siguen siendo de aquí.
        let ids: Vec<String> = self
            .space
            .elements()
            .chain(self.minimizadas.iter().map(|(w, _)| w))
            .chain(self.escritorios.escondidas_activas())
            .filter(|w| w.alive())
            .filter_map(|w| crate::ventanas::app_id_de_grupo(self, w))
            .collect();
        if self.shell.as_mut().is_some_and(|s| s.ventanas(&ids)) {
            self.needs_redraw = true;
        }
        // Aquí es también donde se tiran las barras de las ventanas que ya no
        // están: son los mismos tres momentos —mapear, cerrar, cambiar de
        // app_id— y así no hay una segunda lista que purgar.
        let barras = crate::decoracion::vivas(self);
        if let Some(shell) = self.shell.as_mut() {
            shell.barras_retener(&barras);
        }
    }

    /// ¿Es `surface` la que el cliente ha puesto como imagen de cursor?
    ///
    /// Sirve para no repintar por commits que no se ven: el cursor es la única
    /// superficie que se dibuja sin estar en el `Space`.
    fn cursor_surface_is(&self, surface: &WlSurface) -> bool {
        use smithay::input::pointer::CursorImageStatus;
        matches!(&self.cursor_status, CursorImageStatus::Surface(s) if s == surface)
    }

    fn send_initial_configure(&mut self, window: &Window, surface: &WlSurface) {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let pending = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .map(|d| d.lock().unwrap().initial_configure_sent)
                .unwrap_or(true)
        });
        if !pending {
            // La ventana del kiosko nace a pantalla completa: va ya en esta
            // primera configuración, que es la única que el cliente espera antes
            // de pintar, y así su primer buffer ya tiene el tamaño de la
            // pantalla. Los diálogos, con padre, siguen siendo ventanas.
            if self.modo.kiosko() && toplevel.parent().is_none() {
                let pantalla = self.pantalla();
                toplevel.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some(pantalla.size);
                });
            }
            toplevel.send_configure();
        }
    }
}

/// El `app_id` que el cliente ha declarado, si ya lo ha hecho.
pub fn app_id(window: &Window) -> Option<String> {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

    // Una ventana X11 no tiene `xdg_toplevel` y su `wl_surface` no lleva
    // ningún `app_id`: lo que la identifica es su `WM_CLASS`.
    if let Some(clase) = crate::xwayland::clase(window) {
        return Some(clase);
    }

    let surface = window.wl_surface()?;
    with_states(&surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?
            .app_id
            .clone()
    })
}

/// El título de la ventana, para distinguir dos del mismo programa.
///
/// Mismo camino que [`app_id`]: X11 lo lleva en su propia propiedad y Wayland en
/// el estado del `xdg_toplevel`. Un título vacío es `None` y no una cadena
/// vacía — quien lo enseña tiene que poder caer al nombre de la aplicación.
pub fn titulo(window: &Window) -> Option<String> {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

    if let Some(x11) = window.x11_surface() {
        return Some(x11.title()).filter(|t| !t.trim().is_empty());
    }

    let surface = window.wl_surface()?;
    with_states(&surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?
            .title
            .clone()
    })
    .filter(|t| !t.trim().is_empty())
}

impl XdgShellHandler for BookosComp {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let area = self.work_area();
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
            // `size = None` es "elige tú": el cliente sale con su tamaño
            // natural, que es lo que hace flotante al escritorio. Antes se le
            // imponía el área útil y `Maximized`, y con eso **todas** las
            // ventanas caían en el mismo sitio tapándose por completo: abrir un
            // segundo terminal parecía no hacer nada.
            state.size = None;
            // El límite sí se le dice, para que un cliente que se maximiza solo
            // no pida más pantalla de la que hay bajo el panel.
            state.bounds = Some(area.size);
        });
        let window = Window::new_wayland_window(surface);
        // Se mapea ya para que exista en el espacio, pero la posición de verdad
        // se decide en el primer commit con tamaño (`colocar_si_es_nueva`);
        // hasta entonces `ventanas::animacion` la deja invisible.
        self.space.map_element(window, area.loc, false);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// El `app_id` llega en un commit posterior a `new_toplevel`, así que el
    /// dock no puede saber de quién es la ventana hasta aquí.
    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.actualizar_dock();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // Lo primero, mientras la superficie sigue viva con sus texturas: en
        // el mismo lote el cliente destruye la `wl_surface` y ya no quedaría
        // nada que animar.
        if let (Some(window), Some(contexto)) =
            (self.window_de_toplevel(&surface), self.contexto_gl.as_ref())
            && let Some(loc) = self.space.element_location(&window)
        {
            let posicion = crate::ventanas::posicion(&window, loc);
            let barra = crate::decoracion::decorada(&window)
                .then(|| crate::decoracion::id(&window))
                .flatten()
                .map(|id| crate::cierre::Barra {
                    id,
                    ancho: window.geometry().size.w,
                    estado: crate::decoracion::estado_de(self, &window),
                });
            match crate::cierre::Cierre::capturar(contexto, &window, posicion, barra) {
                Some(cierre) => {
                    tracing::debug!("ventana cerrada: animando su salida");
                    self.cierres.push(cierre);
                }
                None => tracing::debug!("ventana cerrada sin nada que animar"),
            }
        }
        // La lista y sus geometrías dejarían de corresponderse al desaparecer
        // una celda. Cancelar es seguro y evita enfocar o renderizar un destino
        // muerto mientras se mantiene el modificador.
        if self.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
            crate::keybinds::ejecutar(self, crate::keybinds::Accion::ConmutarCancelar);
        }
        // Al cerrarse una ventana puede quedar libre el sitio de una barra.
        self.revisar_barras();
        // Y si estaba apartada en otro escritorio, sacarla de allí: nadie la ha
        // desmapeado del `Space` porque no estaba en él.
        crate::escritorios::purgar(self);
        // El foco de teclado apuntaba a la ventana que acaba de morir: si no se
        // reasigna, las que siguen abiertas dejan de recibir teclas y el
        // escritorio parece congelado sin estarlo.
        if let Some(window) = self.window_de_toplevel(&surface) {
            crate::multitarea::retirar(self, &window);
        }
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        // La ventana muerta sigue en el espacio hasta el siguiente `refresh()`,
        // así que `actualizar_dock` la filtra por `alive()`.
        self.actualizar_dock();
    }

    /// El cliente pide mover su propia ventana: es lo que hace GTK cuando
    /// arrastras su barra de título, que la dibuja él y no nosotros.
    ///
    /// El serial se ignora a propósito. Lo correcto sería comprobar que
    /// corresponde a un clic reciente para que un cliente no pueda secuestrar el
    /// puntero sin que lo hayan tocado; aquí el arrastre se cancela al soltar
    /// cualquier botón, así que lo peor que consigue es que la ventana siga al
    /// cursor hasta el siguiente clic.
    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        self.arrastrar_toplevel(&surface, crate::ventanas::Modo::Mover);
    }

    /// Y lo mismo para el borde: GTK y Qt dibujan sus propios agarraderos.
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        use xdg_toplevel::ResizeEdge as E;
        // El cliente dice de qué borde tira; se cree, porque él sabe dónde ha
        // puesto su agarradero mejor que nosotros mirando cuadrantes.
        let modo = crate::ventanas::Modo::Redimensionar {
            izquierda: matches!(edges, E::Left | E::TopLeft | E::BottomLeft),
            arriba: matches!(edges, E::Top | E::TopLeft | E::TopRight),
        };
        self.arrastrar_toplevel(&surface, modo);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface)
            && !crate::ventanas::maximizada(&window)
        {
            self.alternar_maximizada(&window);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface)
            && crate::ventanas::maximizada(&window)
        {
            self.alternar_maximizada(&window);
        }
    }

    /// Las apps sin barra nuestra (Tauri con `decorations: false`) minimizan
    /// con `set_minimized` desde su propio botón. El trait lo ignora por
    /// defecto, así que ese botón no hacía nada.
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            crate::ventanas::minimizar(self, window);
        }
    }

    /// Pantalla completa: el cliente la pide con F11, con su propio botón o al
    /// darle a "reproducir a pantalla completa". La salida `output` se ignora
    /// porque de momento solo hay una pantalla.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            self.pantalla_completa(&window, true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            self.pantalla_completa(&window, false);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.recolocar_popup(&surface);
        if let Err(err) = self
            .popups
            .track_popup(smithay::desktop::PopupKind::Xdg(surface))
        {
            tracing::warn!("un menú de un cliente que ya se fue: {err:?}");
        }
        self.needs_redraw = true;
    }

    /// El menú se queda con el ratón y el teclado hasta que se cierra: un clic
    /// fuera lo descarta en vez de llegar a la ventana de debajo, y las flechas
    /// lo recorren. Sin esto un menú abierto no se cerraba nunca.
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        use smithay::desktop::{
            PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy,
            find_popup_root_surface,
        };
        use smithay::input::pointer::Focus;
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        let kind = PopupKind::Xdg(surface);
        let Ok(raiz) = find_popup_root_surface(&kind) else {
            return;
        };
        let mut grab = match self.popups.grab_popup(raiz, kind, &seat, serial) {
            Ok(grab) => grab,
            Err(err) => {
                tracing::debug!(?serial, "grab de menú rechazado: {err:?}");
                return;
            }
        };
        // Un grab solo vale si lo pide el mismo clic que tiene el ratón, o el
        // del menú padre en un submenú. Si no, es un menú abierto a destiempo
        // y se cierra entero, que es lo que dice el protocolo.
        let anterior = grab.previous_serial().unwrap_or(serial);
        let teclado = seat.get_keyboard();
        let puntero = seat.get_pointer();
        // Un grab solo vale si lo pide el mismo clic que tiene el ratón, o el
        // del menú padre en un submenú. Si no, es un menú abierto a destiempo
        // y se cierra entero, que es lo que dice el protocolo.
        let a_destiempo = |agarrado: bool, tiene: &dyn Fn(Serial) -> bool| {
            agarrado && !(tiene(serial) || tiene(anterior))
        };
        if teclado
            .as_ref()
            .is_some_and(|t| a_destiempo(t.is_grabbed(), &|s| t.has_grab(s)))
            || puntero
                .as_ref()
                .is_some_and(|p| a_destiempo(p.is_grabbed(), &|s| p.has_grab(s)))
        {
            tracing::debug!(?serial, "menú abierto a destiempo");
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }
        // El foco del teclado **no** pasa al menú al abrirse: se le da con la
        // primera tecla (`input::tecla`), que es lo que hace KWin. Dárselo aquí
        // rompía cambiar de menú por la barra —de «Máquina» a «Ayuda» sin
        // soltar—: Qt recibía el `wl_keyboard.enter` del menú nuevo mientras
        // aún devolvía el foco del viejo a su ventana, y cerraba el nuevo.
        // Visto con `WAYLAND_DEBUG=client`: el `destroy` llegaba 0,3 ms después
        // del `enter`, sin nada del compositor entre medias; y sin ese `enter`
        // el menú se queda.
        //
        // El teclado se suelta **antes** de sustituir el grab de ratón: al
        // quitar el viejo, Smithay deshace también el grab de teclado de su
        // mismo serial y manda el foco a la ventana, que Qt también toma por
        // una activación.
        if let Some(teclado) = teclado.as_ref() {
            teclado.unset_grab(self);
        }
        if let Some(puntero) = puntero {
            puntero.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
        if let Some(teclado) = teclado {
            teclado.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        self.menu_sin_foco = Some(grab);
    }

    /// Si el menú que se va tenía el teclado, el foco vuelve a su ventana. Sin
    /// esto el teclado se quedaba sin superficie, Qt lo tomaba por que la
    /// aplicación había perdido el foco y cerraba también el menú que abría a
    /// continuación: con la flecha izquierda, de «Ayuda» a «Máquina», el
    /// segundo menú se destruía 3 ms después de crearse.
    fn popup_destroyed(&mut self, surface: PopupSurface) {
        use smithay::desktop::{PopupKind, find_popup_root_surface};
        let Some(teclado) = self.seat.get_keyboard() else {
            return;
        };
        if teclado.current_focus().as_ref() != Some(surface.wl_surface()) {
            return;
        }
        let Ok(raiz) = find_popup_root_surface(&PopupKind::Xdg(surface)) else {
            return;
        };
        teclado.set_focus(
            self,
            Some(raiz),
            smithay::utils::SERIAL_COUNTER.next_serial(),
        );
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|estado| {
            estado.geometry = positioner.get_geometry();
            estado.positioner = positioner;
        });
        self.recolocar_popup(&surface);
        surface.send_repositioned(token);
        self.needs_redraw = true;
    }
}

impl BookosComp {
    /// Mete el menú dentro de las pantallas donde está su ventana. El cliente
    /// dice dónde quiere abrirlo respecto a su padre y cómo puede moverse —dar
    /// la vuelta, deslizarse— si no cabe; el compositor es el único que sabe
    /// dónde acaba la pantalla. Sin esto, un menú abierto cerca del borde de
    /// abajo salía cortado.
    fn recolocar_popup(&self, popup: &PopupSurface) {
        use smithay::desktop::{PopupKind, find_popup_root_surface, get_popup_toplevel_coords};
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(raiz) = find_popup_root_surface(&kind) else {
            return;
        };
        let Some(window) = self.window_for_surface(&raiz) else {
            return;
        };
        let Some(ventana) = self.space.element_geometry(&window) else {
            return;
        };
        let Some(pantallas) = self
            .space
            .outputs_for_element(&window)
            .iter()
            .filter_map(|o| self.space.output_geometry(o))
            .reduce(|a, b| a.merge(b))
        else {
            return;
        };
        // El positioner razona respecto al padre, así que el rectángulo
        // permitido se lleva a esas coordenadas.
        let mut permitido = pantallas;
        permitido.loc -= get_popup_toplevel_coords(&kind);
        permitido.loc -= ventana.loc;
        popup.with_pending_state(|estado| {
            estado.geometry = estado.positioner.get_unconstrained_geometry(permitido);
        });
    }
}

impl SeatHandler for BookosComp {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn led_state_changed(
        &mut self,
        _seat: &Seat<Self>,
        led_state: smithay::input::keyboard::LedState,
    ) {
        self.leds = led_state;
        if let Some(aplicar) = self.aplicar_leds.as_ref() {
            aplicar(led_state);
        }
    }

    /// El portapapeles va **con el foco**: el cliente enfocado es el único al
    /// que se le ofrece la selección, y el único al que se le puede aceptar que
    /// la cambie.
    ///
    /// Esto estaba vacío, y por eso copiar y pegar entre dos aplicaciones no
    /// terminaba de funcionar: sin este aviso, el `wl_data_device` del cliente
    /// nuevo nunca recibía la oferta de lo que había en el portapapeles.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        use smithay::reexports::wayland_server::Resource as _;
        let cliente = focused.and_then(|s| self.display_handle.get_client(s.id()).ok());
        set_data_device_focus(&self.display_handle, seat, cliente.clone());
        set_primary_focus(&self.display_handle, seat, cliente);
        let ventana = focused.and_then(|s| self.window_for_surface(s));
        // El foco de entrada de X11 es aparte del de `wl_keyboard`, y Smithay
        // solo lo pone desde el `KeyboardTarget` de `X11Surface`. Como aquí el
        // foco es la `WlSurface`, no se ponía nunca: el servidor X se quedaba
        // en `PointerRoot` (medido con `xdpyinfo`) y las teclas iban a la
        // ventana X11 que tuviera el puntero encima, no a la enfocada. El
        // `enter` repetido que le llega a XWayland va sin teclas y no hace nada.
        if let Some(x11) = ventana.as_ref().and_then(|w| w.x11_surface()) {
            smithay::input::keyboard::KeyboardTarget::enter(
                x11,
                seat,
                self,
                Vec::new(),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
        }
        let app = ventana.and_then(|w| app_id(&w));
        if let Some(shell) = self.shell.as_mut() {
            shell.poner_app_enfocada(app);
        }
        self.needs_redraw = true;
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // El cliente propone; el compositor dispone (y dibuja). Con
        // `wp_cursor_shape_v1` lo que llega es `Named` —"texto", "mano"— y se
        // resuelve contra nuestro tema; sin él, el cliente manda su propia
        // superficie ya pintada y no queda más que componerla tal cual.
        tracing::debug!(?image, "el cliente pide un cursor");
        self.cursor_status = image;
        self.needs_redraw = true;
    }
}

impl OutputHandler for BookosComp {}

impl FractionalScaleHandler for BookosComp {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // En cuanto un cliente pregunta, se le dice a qué escala va a dibujarse.
        // Si no, asume 1 y se ve borroso en un panel de 2880×1800.
        let scale = self.output_scale();
        smithay::wayland::compositor::with_states(&surface, |states| {
            with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(scale);
            });
        });
    }
}

impl BufferHandler for BookosComp {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for BookosComp {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl smithay::wayland::dmabuf::DmabufHandler for BookosComp {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }

    /// Un cliente nos ofrece un buffer de la GPU. Aquí solo se contesta si lo
    /// vamos a saber leer; el uso de verdad ocurre al componer, y de eso ya se
    /// encarga `GlesRenderer` por su cuenta con `ImportDmaWl`.
    ///
    /// La comprobación es hacer la `EGLImage` y tirarla. Se hace con la
    /// `EGLDisplay` y no con el renderizador porque el renderizador vive dentro
    /// del backend y aquí no se ve —y porque la pregunta es de EGL, no de GL:
    /// si EGL sabe construir la imagen, el import del fotograma también podrá.
    /// Sin `EGLDisplay` (no debería pasar: el global lo crea el backend, que es
    /// quien la deja puesta) se rechaza en vez de aceptar a ciegas, que daría
    /// una ventana en negro en vez de un fallo que se ve.
    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notificar: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        let Some(display) = self.egl_display.as_ref() else {
            tracing::error!("import de dmabuf sin EGLDisplay: se rechaza");
            notificar.failed();
            return;
        };
        match display.create_image_from_dmabuf(&dmabuf) {
            Ok(imagen) => {
                // La imagen era la pregunta, no la respuesta: quien la vuelve a
                // crear para dibujar es el renderizador. Dejarla viva sería una
                // fuga de un descriptor por buffer y por cliente.
                unsafe {
                    smithay::backend::egl::ffi::egl::DestroyImageKHR(
                        **display.get_display_handle(),
                        imagen,
                    );
                }
                let _ = notificar.successful::<Self>();
            }
            Err(err) => {
                tracing::debug!("dmabuf rechazado: {err}");
                notificar.failed();
            }
        }
    }
}

/// Una selección cuyo dueño, a ojos de los clientes Wayland, es el compositor.
#[derive(Clone)]
pub enum Seleccion {
    /// Bytes propios: hoy, la captura de pantalla.
    Bytes(std::sync::Arc<Vec<u8>>),
    /// La puso un programa X11. XWayland entero es un solo cliente y no tiene
    /// `wl_data_device` propio: el puente es el gestor de ventanas, y cuando
    /// alguien pega hay que pedírsela a él.
    X11,
}

/// Escribe `datos` por el descriptor de quien pega.
///
/// **En un hilo aparte, y no aquí.** Un pipe tiene 64 KB de buffer y una
/// captura son varios megas: escribirla desde el bucle dejaría el compositor
/// bloqueado hasta que el otro extremo terminara de leer, o sea el escritorio
/// congelado mientras pegas. El hilo se muere solo al acabar.
pub fn escribir_seleccion(
    datos: std::sync::Arc<Vec<u8>>,
    mime_type: String,
    fd: std::os::fd::OwnedFd,
) {
    let hilo = std::thread::Builder::new()
        .name("portapapeles".into())
        .spawn(move || {
            use smithay::reexports::rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
            use std::io::Write;
            // El pipe que abre el gestor X11 viene con `O_NONBLOCK` y el hilo
            // está para bloquearse: sin quitarlo, `write_all` se topa con
            // `EAGAIN` al llenar el buffer y a X11 llegaban 65 536 bytes de una
            // captura de 434 478.
            if let Ok(flags) = fcntl_getfl(&fd) {
                let _ = fcntl_setfl(&fd, flags - OFlags::NONBLOCK);
            }
            let mut destino = std::fs::File::from(fd);
            if let Err(err) = destino.write_all(&datos).and_then(|()| destino.flush()) {
                // Que el otro lado cierre antes de leerlo todo es normal
                // —hay clientes que solo miran la cabecera—, así que esto
                // se cuenta y no se grita.
                tracing::debug!(%mime_type, "el pegado se cortó: {err}");
            }
        });
    if let Err(err) = hilo {
        tracing::error!("no se pudo lanzar el hilo del portapapeles: {err}");
    }
}

impl SelectionHandler for BookosComp {
    /// Es un `Arc` dentro porque el mismo contenido lo puede pedir más de un
    /// cliente y no tiene sentido copiarlo por cada pegado.
    type SelectionUserData = Seleccion;

    /// Un cliente Wayland cambia la selección: X11 tiene que enterarse, o lo
    /// copiado en Firefox no llega nunca a VirtualBox ni a nada que vaya por
    /// XWayland. Con `None` el gestor suelta la selección X11 si era suya.
    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(ty, source.map(|s| s.mime_types()))
        {
            tracing::warn!(?ty, "no se pudo pasar la selección a X11: {err}");
        }
    }

    /// Un cliente Wayland pega algo cuyo dueño somos nosotros.
    fn send_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        datos: &Self::SelectionUserData,
    ) {
        match datos {
            Seleccion::Bytes(datos) => escribir_seleccion(datos.clone(), mime_type, fd),
            Seleccion::X11 => {
                let Some(xwm) = self.xwm.as_mut() else {
                    // XWayland murió con la selección puesta: el `fd` se cierra
                    // al soltarlo y quien pega recibe vacío.
                    return;
                };
                if let Err(err) = xwm.send_selection(ty, mime_type, fd, self.loop_handle.clone()) {
                    tracing::warn!(?ty, "no se pudo leer la selección de X11: {err}");
                }
            }
        }
    }
}

impl DataDeviceHandler for BookosComp {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for BookosComp {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl DataControlHandler for BookosComp {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

impl ClientDndGrabHandler for BookosComp {}
impl ServerDndGrabHandler for BookosComp {}

/// La decoración: por defecto la dibuja el escritorio.
///
/// Un cliente que pida `ClientSide` se sale con la suya —Wayland es así, y
/// pelearse con GTK acaba en dos barras de título—, pero el que no diga nada o
/// deje elegir se lleva la nuestra.
impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for BookosComp {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.modo_decoracion(&toplevel, DecorationMode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        self.modo_decoracion(&toplevel, mode);
    }

    /// «Me da igual»: entonces la ponemos nosotros.
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.modo_decoracion(&toplevel, DecorationMode::ServerSide);
    }
}

impl BookosComp {
    /// Le dice al cliente quién dibuja su barra y lo apunta para el render.
    ///
    /// El `configure` sale aquí mismo salvo que el cliente aún no haya recibido
    /// el primero: en ese caso lo manda `send_initial_configure` con el modo ya
    /// puesto, que es lo que espera el protocolo.
    fn modo_decoracion(&mut self, toplevel: &ToplevelSurface, modo: DecorationMode) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(modo));
        if let Some(window) = self.window_de_toplevel(toplevel) {
            crate::decoracion::decorar(&window, modo == DecorationMode::ServerSide);
            // La ventana ya colocada se baja para dejar sitio a la barra: sin
            // esto, una que se decore después de mapearse se queda con la barra
            // metida bajo el panel.
            self.recolocar_por_barra(&window);
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        self.needs_redraw = true;
    }
}

smithay::delegate_xdg_decoration!(BookosComp);
delegate_compositor!(BookosComp);
delegate_xdg_shell!(BookosComp);
delegate_shm!(BookosComp);
smithay::delegate_dmabuf!(BookosComp);
delegate_seat!(BookosComp);
delegate_data_device!(BookosComp);
delegate_primary_selection!(BookosComp);
delegate_data_control!(BookosComp);
delegate_output!(BookosComp);
delegate_fractional_scale!(BookosComp);
delegate_viewporter!(BookosComp);
delegate_presentation!(BookosComp);

impl IdleInhibitHandler for BookosComp {
    fn inhibit(&mut self, surface: WlSurface) {
        use smithay::reexports::wayland_server::Resource as _;
        self.idle_inhibidores.retain(|actual| actual.is_alive());
        if !self
            .idle_inhibidores
            .iter()
            .any(|actual| actual == &surface)
        {
            self.idle_inhibidores.push(surface);
            self.idle_notifier_state.set_is_inhibited(true);
            crate::backend::programar_bloqueo_inactividad(self);
            crate::backend::programar_suspension_inactividad(self);
            tracing::debug!("una aplicación ha inhibido el idle de pantalla");
        }
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        use smithay::reexports::wayland_server::Resource as _;
        self.idle_inhibidores
            .retain(|actual| actual.is_alive() && actual != &surface);
        self.idle_notifier_state.set_is_inhibited(
            !self.idle_inhibidores.is_empty() || bookos_shell::mantener_despierto(),
        );
        if self.idle_inhibidores.is_empty() {
            self.last_input = Some(std::time::Instant::now());
        }
        crate::backend::programar_bloqueo_inactividad(self);
        crate::backend::programar_suspension_inactividad(self);
        tracing::debug!("una aplicación ha liberado el idle de pantalla");
    }
}

delegate_idle_inhibit!(BookosComp);

impl IdleNotifierHandler for BookosComp {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

delegate_idle_notify!(BookosComp);
delegate_relative_pointer!(BookosComp);
smithay::delegate_pointer_gestures!(BookosComp);

impl WlrLayerShellHandler for BookosComp {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = output
            .as_ref()
            .and_then(smithay::output::Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            tracing::warn!("layer-shell recibido antes de tener una salida");
            return;
        };
        let layer = DesktopLayerSurface::new(surface, namespace);
        if let Err(err) = layer_map_for_output(&output).map_layer(&layer) {
            tracing::warn!("no se pudo mapear layer-shell: {err}");
            return;
        }
        self.needs_redraw = true;
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layer = map
                .layers()
                .find(|layer| layer.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
                self.needs_redraw = true;
                break;
            }
        }
    }
}

delegate_layer_shell!(BookosComp);

impl ForeignToplevelListHandler for BookosComp {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}

delegate_foreign_toplevel_list!(BookosComp);

impl PointerConstraintsHandler for BookosComp {
    fn new_constraint(
        &mut self,
        surface: &WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
    ) {
        let enfocado = self
            .surface_under(self.pointer_location)
            .is_some_and(|(actual, _)| actual == *surface);
        if enfocado {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &smithay::input::pointer::PointerHandle<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        // El hint solo se usa para recolocar el cursor al activar un juego.
        // La posición real se conserva en `pointer_location`; el compositor
        // no la cambia hasta que llega movimiento del dispositivo.
    }
}

delegate_pointer_constraints!(BookosComp);
delegate_text_input_manager!(BookosComp);

impl InputMethodHandler for BookosComp {
    fn new_popup(&mut self, _surface: InputMethodPopupSurface) {}
    fn dismiss_popup(&mut self, _surface: InputMethodPopupSurface) {}
    fn popup_repositioned(&mut self, _surface: InputMethodPopupSurface) {}
    fn parent_geometry(
        &self,
        _parent: &WlSurface,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        smithay::utils::Rectangle::default()
    }
}

delegate_input_method_manager!(BookosComp);

/// `wp_cursor_shape_v1` comparte el manejo del cursor con el de las tabletas,
/// así que exige este trait aunque aquí no haya ninguna. Todos sus métodos
/// tienen implementación por defecto: declararlo basta, y el día que haya una
/// Wacom el sitio donde escribir su cursor ya está.
impl smithay::wayland::tablet_manager::TabletSeatHandler for BookosComp {}
smithay::delegate_cursor_shape!(BookosComp);
smithay::delegate_xdg_activation!(BookosComp);

/// Quién puede llevarse el foco y cuándo.
///
/// El protocolo existe para separar dos cosas que sin él son la misma: «una
/// ventana nueva aparece» y «el usuario ha pedido esta ventana». Sin él solo
/// hay dos políticas y las dos molestan —enfocar siempre te quita el teclado de
/// donde estabas escribiendo, no enfocar nunca deja lo que lanzas del dock sin
/// responder—. El vale dice cuál de las dos es.
impl smithay::wayland::xdg_activation::XdgActivationHandler for BookosComp {
    fn activation_state(&mut self) -> &mut smithay::wayland::xdg_activation::XdgActivationState {
        &mut self.activacion_state
    }

    fn request_activation(
        &mut self,
        token: smithay::wayland::xdg_activation::XdgActivationToken,
        datos: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let caducado = datos.timestamp.elapsed() > crate::keybinds::VALE_VALIDO;
        // Se retira siempre, valga o no: es de un solo uso.
        self.activacion_state.remove_token(&token);
        if caducado {
            tracing::debug!(?datos.app_id, "vale de activación caducado; sin foco");
            return;
        }
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&surface))
            .cloned()
        else {
            // Todavía no está en el `Space`: la ventana pide el foco antes de
            // mapearse. No es un error y no hace falta guardarlo —el camino de
            // `lanzar` marca la suya al mapear, ver `ventanas::mapear`—.
            return;
        };
        tracing::debug!(?datos.app_id, "activación concedida");
        self.enfocar(&window);
    }
}
