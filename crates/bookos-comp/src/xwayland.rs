//! XWayland: las aplicaciones que solo hablan X11.
//!
//! Sin esto no arranca nada que no sea Wayland nativo, que en un escritorio
//! real es bastante: Steam, Wine, buena parte de lo que trae Electron viejo y
//! cualquier programa de los 2000 que uno siga usando. El compositor se
//! convierte en el gestor de ventanas X11 de esa gente, hablando el protocolo
//! con `x11rb` a través de Smithay.
//!
//! **Todo XWayland es un solo cliente de Wayland.** Sus ventanas X11 no son
//! `xdg_toplevel`; llegan por aquí como [`X11Surface`] y se envuelven en el
//! mismo `desktop::Window` que las demás, de modo que el resto del compositor
//! —animaciones, encaje, dock, foco— no necesita saber de dónde vienen. Los
//! únicos sitios que distinguen son los que mandan un tamaño al cliente:
//! X11 no tiene `configure` diferido, así que se le dice y punto.
//!
//! **Se arranca al vuelo y su fallo no es fatal.** Si `Xwayland` no está
//! instalado se avisa y la sesión sigue: perder los programas X11 es malo,
//! pero mucho menos que no tener escritorio.
//!
//! **Escala.** X11 no sabe de escalas. Antes las ventanas X11 iban en lógicos
//! y el compositor ampliaba su imagen al factor del panel: tamaño correcto,
//! pero borrosas (VirtualBox, Electron sin Wayland). Ahora XWayland ve la
//! pantalla en píxeles físicos (`set_client_scale`, lo mismo que hace KWin con
//! «dejar que las aplicaciones escalen solas») y a las aplicaciones se les
//! dice el factor con `Xft.dpi`. Qt 6, Electron/Chromium y los textos de GTK
//! lo leen y se dibujan grandes y nítidas; una aplicación que lo ignore se ve
//! pequeña, que es el precio que pagan todos los escritorios que hacen esto.

use std::io::Write;
use std::os::fd::OwnedFd;

use smithay::utils::{Logical, Rectangle};
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::xwm::{Reorder, ResizeEdge, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent};

use crate::handlers::Seleccion;
use crate::state::BookosComp;

/// Lanza XWayland y deja programado el arranque de su gestor de ventanas.
///
/// El `XWaylandEvent::Ready` no llega hasta que el servidor está escuchando, y
/// hasta entonces no hay con quién hablar X11: por eso el gestor se crea en el
/// callback y no aquí.
pub fn arrancar(state: &mut BookosComp) {
    let (xwayland, cliente) = match XWayland::spawn(
        &state.display_handle,
        // Sin número de pantalla fijo: que Smithay busque el primero libre. Si
        // se pidiera el :0 y hubiera otra sesión X viva, esto fallaría entero.
        None,
        std::iter::empty::<(String, String)>(),
        true,
        std::process::Stdio::null(),
        std::process::Stdio::null(),
        |_| {},
    ) {
        Ok(par) => par,
        Err(err) => {
            tracing::warn!("no se pudo arrancar XWayland, la sesión sigue sin X11: {err}");
            return;
        }
    };

    state.cliente_x11 = Some(cliente.clone());
    let resultado =
        state
            .loop_handle
            .insert_source(xwayland, move |evento, _, state| match evento {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let wm =
                        X11Wm::start_wm(state.loop_handle.clone(), x11_socket, cliente.clone());
                    match wm {
                        Ok(wm) => {
                            // DISPLAY solo se publica cuando el gestor está en pie.
                            // Publicarlo antes deja una ventana de tiempo en la que
                            // un cliente X11 se conecta, no encuentra gestor y se
                            // dibuja sin decoración ni foco.
                            state.xwm = Some(wm);
                            state.display_x11 = Some(display_number);
                            actualizar_escala(state);
                            tracing::info!(
                                display = format!(":{display_number}"),
                                "XWayland listo"
                            );
                        }
                        Err(err) => tracing::error!("no se pudo arrancar el gestor X11: {err}"),
                    }
                }
                XWaylandEvent::Error => {
                    tracing::error!("XWayland murió al arrancar; la sesión sigue sin X11");
                }
            });
    if let Err(err) = resultado {
        tracing::error!("no se pudo escuchar a XWayland: {err}");
    }
}

/// Pone a XWayland en la escala del monitor más denso y se lo dice a las
/// aplicaciones con `Xft.dpi`. Se llama al arrancar y cada vez que cambian las
/// pantallas.
///
/// Con varios monitores manda el más denso: en el otro las ventanas X11 se
/// verán grandes, pero nítidas; al revés se verían borrosas en el panel, que
/// es donde se trabaja. Las ventanas X11 ya abiertas no se redibujan con el
/// DPI nuevo hasta que se reinician: `Xft.dpi` se lee al arrancar.
pub fn actualizar_escala(state: &mut BookosComp) {
    let Some(cliente) = state.cliente_x11.as_ref() else {
        return;
    };
    let escala = state
        .space
        .outputs()
        .map(|o| o.current_scale().fractional_scale())
        .max_by(f64::total_cmp)
        .unwrap_or(1.0);
    let datos = cliente
        .get_data::<smithay::xwayland::XWaylandClientData>()
        .expect("el cliente de XWayland lleva sus datos desde el spawn");
    if datos.compositor_state.client_scale() != escala {
        datos.compositor_state.set_client_scale(escala);
        // Smithay solo reenvía el tamaño de la salida en la escala nueva dentro de
        // `change_current_state`, y XWayland ya la tenía enlazada a escala 1. Sin
        // esto su raíz se quedaba en lógicos (medido con `xdpyinfo`: 1440x900 en
        // el panel de 2880x1800) mientras las ventanas iban en físicos: todo lo
        // que caía fuera de ese cuarto de pantalla no recibía el puntero.
        for salida in state.space.outputs() {
            salida.change_current_state(None, None, None, None);
        }
        tracing::info!(escala, "escala de XWayland");
    }
    let Some(n) = state.display_x11 else {
        return;
    };
    // 96 es el DPI de referencia de X11: 192 equivale a escala 2.
    let dpi = (96.0 * escala).round() as u32;
    let mut recursos = format!("Xft.dpi: {dpi}\n");
    if let Some(tema) = state.cursor_theme.as_ref() {
        let tamano = tema.tamano_logico();
        recursos.push_str(&format!(
            "Xcursor.theme: {}\nXcursor.size: {tamano}\n",
            tema.nombre(),
        ));
        // GTK/Electron consultan XSettings; libXcursor también puede leer los
        // recursos de la raíz. Ambos deben coincidir con los clientes lanzados
        // desde el dock. El escalado del cursor X11 lo hace el compositor.
        if let Some(wm) = state.xwm.as_mut()
            && let Err(err) = wm.set_xsettings(
                [
                    (
                        "Gtk/CursorThemeName".to_string(),
                        tema.nombre().to_string().into(),
                    ),
                    ("Gtk/CursorThemeSize".to_string(), (tamano as i32).into()),
                ]
                .into_iter(),
            )
        {
            tracing::warn!("no se pudieron publicar los ajustes del cursor X11: {err}");
        }
    }
    match std::process::Command::new("xrdb")
        .arg("-merge")
        .env("DISPLAY", format!(":{n}"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut hijo) => {
            let mut entrada = hijo
                .stdin
                .take()
                .expect("xrdb se lanzó con stdin canalizado");
            if let Err(err) = entrada.write_all(recursos.as_bytes()) {
                tracing::warn!("no se pudieron escribir los recursos de X11: {err}");
            }
            state.hijos.push(hijo);
        }
        Err(err) => tracing::warn!("no se pudieron publicar los recursos de X11: {err}"),
    }
}

/// El `app_id` equivalente de una ventana X11.
///
/// En X11 el par que identifica al programa es `WM_CLASS`, y de sus dos campos
/// el que se parece a un `app_id` de Wayland es la clase: `firefox` da
/// `instance="Navigator"` pero `class="firefox"`, que es lo que el dock busca.
pub fn clase(window: &smithay::desktop::Window) -> Option<String> {
    let x11 = window.x11_surface()?;
    let clase = x11.class();
    (!clase.is_empty()).then_some(clase)
}

impl BookosComp {
    /// La ventana que envuelve a una superficie X11.
    fn window_de_x11(&self, surface: &X11Surface) -> Option<smithay::desktop::Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }
}

impl smithay::xwayland::XwmHandler for BookosComp {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("el gestor X11 existe mientras el bucle le entrega eventos")
    }

    /// Existe pero aún no se ve. No se mapea nada todavía: muchas ventanas X11
    /// se crean para no enseñarse nunca —diálogos que el programa se guarda,
    /// ventanas de utilidad de las toolkits— y mapearlas aquí llenaría el
    /// escritorio de cosas invisibles ocupando sitio en el espacio.
    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// La ventana pide enseñarse. Hay que concedérselo explícitamente: en X11
    /// el gestor de ventanas puede decir que no, y si nadie dice que sí la
    /// ventana no aparece nunca.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_mapped(true);
        let area = self.work_area();
        // El límite se le manda ya: un cliente X11 que se maximice solo no debe
        // meterse bajo el panel.
        let geo = window.geometry();
        let _ = window.configure(Rectangle::new(geo.loc, geo.size));
        // En X11 no hay `xdg-decoration`: la barra la ponemos nosotros salvo que
        // la ventana declare (`_MOTIF_WM_HINTS`) que dibuja la suya, o que sea
        // un menú, un tooltip o algo que no es una ventana de verdad. Sin esto
        // VirtualBox, Steam o cualquier programa Qt/GTK sobre X11 salían sin
        // botones de ventana. Va antes de `map_element` para que
        // `colocar_si_es_nueva` cuente con el sitio de la barra.
        use smithay::xwayland::xwm::WmWindowType as T;
        let con_barra = !window.is_decorated()
            && matches!(
                window.window_type(),
                None | Some(T::Normal | T::Dialog | T::Utility)
            );
        let elemento = smithay::desktop::Window::new_x11_window(window);
        crate::decoracion::decorar(&elemento, con_barra);
        self.space.map_element(elemento, area.loc, false);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// Ya está enseñada de verdad. Aquí es donde se coloca, y no en
    /// `map_window_request`: hasta este momento la geometría que anuncia puede
    /// ser todavía la de fábrica (1x1 en varias toolkits).
    fn map_window_notify(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.colocar_si_es_nueva(&elemento);
        }
        self.needs_redraw = true;
    }

    /// Las override-redirect son menús, tooltips y demás: el gestor no manda
    /// sobre ellas y se limita a ponerlas donde el cliente diga.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let geo = window.geometry();
        let elemento = smithay::desktop::Window::new_x11_window(window);
        self.space.map_element(elemento, geo.loc, true);
        self.needs_redraw = true;
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if self.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
            crate::keybinds::ejecutar(self, crate::keybinds::Accion::ConmutarCancelar);
        }
        if let Some(elemento) = self.window_de_x11(&window) {
            crate::multitarea::retirar(self, &elemento);
            self.space.unmap_elem(&elemento);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {
        if self.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
            crate::keybinds::ejecutar(self, crate::keybinds::Accion::ConmutarCancelar);
        }
        crate::escritorios::purgar(self);
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// El cliente pide un tamaño o una posición. Se le concede tal cual salvo
    /// que esté encajado: ahí manda el compositor, o una ventana a media
    /// pantalla se saldría de su zona en cuanto el programa decidiera crecer.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let elemento = self.window_de_x11(&window);
        if let Some(elemento) = &elemento
            && (crate::ventanas::encajada(elemento)
                || (crate::multitarea::agrupada(self, elemento)
                    && crate::multitarea::raiz(self, elemento) == *elemento))
        {
            let geo = window.geometry();
            let _ = window.configure(geo);
            return;
        }
        let anterior = window.geometry();
        let destino = Rectangle::new(
            (x.unwrap_or(anterior.loc.x), y.unwrap_or(anterior.loc.y)).into(),
            (
                w.map(|v| v as i32).unwrap_or(anterior.size.w).max(1),
                h.map(|v| v as i32).unwrap_or(anterior.size.h).max(1),
            )
                .into(),
        );
        let _ = window.configure(destino);
    }

    /// El cliente ya se movió o cambió de tamaño. Para las override-redirect
    /// esto es lo único que hay: se mueven cuando quieren y el espacio tiene
    /// que seguirlas o el menú se queda pintado donde ya no está.
    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let Some(elemento) = self.window_de_x11(&window) else {
            return;
        };
        let nuevo_tamano = crate::ventanas::tamano_nuevo(&elemento);
        if self.space.element_location(&elemento) != Some(geometry.loc) {
            // Confirmar la posición no cambia el orden de las ventanas: un
            // ConfigureNotify atrasado no puede tapar un diálogo ya enfocado.
            let pila: Vec<_> = self.space.elements().cloned().collect();
            self.space.map_element(elemento, geometry.loc, false);
            for w in pila {
                self.space.raise_element(&w, false);
            }
        }
        // Un cliente X11 confirma aquí su tamaño nuevo: ver `tamano_nuevo`.
        if nuevo_tamano {
            self.revisar_barras();
        }
        self.needs_redraw = true;
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window)
            && !crate::ventanas::maximizada(&elemento)
        {
            self.alternar_maximizada(&elemento);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window)
            && crate::ventanas::maximizada(&elemento)
        {
            self.alternar_maximizada(&elemento);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.pantalla_completa(&elemento, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.pantalla_completa(&elemento, false);
        }
    }

    /// Mover y redimensionar pedidos por el cliente, que en X11 llegan con el
    /// botón ya agarrado. Se reutiliza el mismo arrastre que el resto: la
    /// ventana ya está en el espacio y el cursor ya está donde tiene que estar.
    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edge: ResizeEdge) {
        let Some(elemento) = self.window_de_x11(&window) else {
            return;
        };
        use ResizeEdge as E;
        let modo = crate::ventanas::Modo::Redimensionar {
            izquierda: matches!(edge, E::Left | E::TopLeft | E::BottomLeft),
            arriba: matches!(edge, E::Top | E::TopLeft | E::TopRight),
        };
        self.arrastrar_ventana(&elemento, modo, true);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.arrastrar_ventana(&elemento, crate::ventanas::Modo::Mover, true);
        }
    }

    /// Un programa X11 pide leer una selección que tiene un cliente Wayland.
    ///
    /// Siempre que sí, sin mirar el foco. VirtualBox pregunta los tipos
    /// (`TARGETS`) en cuanto cambia el dueño, para anunciárselos al invitado,
    /// y en ese momento quien tiene el foco es la ventana donde se copió, no
    /// la máquina virtual: atado al foco, el invitado no se entera nunca. Entre
    /// programas X11 tampoco hay más barrera que esta.
    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        true
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        use smithay::wayland::selection::data_device::{
            current_data_device_selection_userdata, request_data_device_client_selection,
        };
        use smithay::wayland::selection::primary_selection::{
            current_primary_selection_userdata, request_primary_client_selection,
        };
        // Si el dueño es el compositor —la captura—, Smithay no la sirve:
        // `request_*_client_selection` es solo para leer la de un cliente.
        let propia = match selection {
            SelectionTarget::Clipboard => {
                current_data_device_selection_userdata(&self.seat).map(|d| d.clone())
            }
            SelectionTarget::Primary => {
                current_primary_selection_userdata(&self.seat).map(|d| d.clone())
            }
        };
        // Los dos `request_*` tienen tipos de error distintos: de ahí el texto.
        let resultado = match propia {
            Some(Seleccion::Bytes(datos)) => {
                crate::handlers::escribir_seleccion(datos, mime_type, fd);
                return;
            }
            // Que pida la suya propia no debería pasar: el gestor no reenvía
            // peticiones cuando el dueño X11 es otro programa X11.
            Some(Seleccion::X11) => return,
            None => match selection {
                SelectionTarget::Clipboard => {
                    request_data_device_client_selection(&self.seat, mime_type, fd)
                        .map_err(|e| e.to_string())
                }
                SelectionTarget::Primary => {
                    request_primary_client_selection(&self.seat, mime_type, fd)
                        .map_err(|e| e.to_string())
                }
            },
        };
        if let Err(err) = resultado {
            tracing::warn!(
                ?selection,
                "X11 no pudo leer la selección de Wayland: {err}"
            );
        }
    }

    /// Un programa X11 copia: los clientes Wayland lo ven como selección del
    /// compositor, y al pegar se le pide a XWayland (`Seleccion::X11`).
    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        use smithay::wayland::selection::data_device::set_data_device_selection;
        use smithay::wayland::selection::primary_selection::set_primary_selection;
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                Seleccion::X11,
            ),
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, Seleccion::X11)
            }
        }
    }

    /// El programa X11 soltó su selección. Solo se borra la de Wayland si
    /// seguía siendo la suya: si entretanto copió un cliente Wayland, esa manda.
    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        use smithay::wayland::selection::data_device::{
            clear_data_device_selection, current_data_device_selection_userdata,
        };
        use smithay::wayland::selection::primary_selection::{
            clear_primary_selection, current_primary_selection_userdata,
        };
        match selection {
            SelectionTarget::Clipboard => {
                if matches!(
                    current_data_device_selection_userdata(&self.seat).as_deref(),
                    Some(Seleccion::X11)
                ) {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if matches!(
                    current_primary_selection_userdata(&self.seat).as_deref(),
                    Some(Seleccion::X11)
                ) {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    /// XWayland se ha caído. Se suelta el gestor para que nadie siga mandándole
    /// órdenes —`enfocar` le pide subir ventanas en cada clic— y la sesión
    /// continúa sin X11, que es lo mismo que si nunca hubiera arrancado.
    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("XWayland se desconectó; la sesión sigue sin X11");
        self.xwm = None;
        self.display_x11 = None;
    }
}

impl smithay::wayland::xwayland_shell::XWaylandShellHandler for BookosComp {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// `colocar_si_es_nueva` enfoca la ventana en el `MapNotify`, pero ahí una
    /// ventana X11 todavía no tiene `wl_surface` y el foco de teclado se queda
    /// en nada: no se podía escribir en ella hasta hacerle clic. Se repite
    /// aquí, que es cuando ya hay a quién dárselo.
    fn surface_associated(
        &mut self,
        _xwm: XwmId,
        _wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        surface: X11Surface,
    ) {
        if surface.is_activated()
            && let Some(elemento) = self.window_de_x11(&surface)
        {
            self.enfocar(&elemento);
        }
    }
}

smithay::delegate_xwayland_shell!(BookosComp);
