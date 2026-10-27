//! Text-input entrega la secuencia completa, incluidos ZWJ y selectores de
//! variante. El pegado cubre los clientes que no habilitan ese protocolo.

use std::sync::Arc;

use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keycode, ModifiersState, keysyms};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::set_data_device_selection;
use smithay::wayland::text_input::TextInputSeat;

use crate::state::BookosComp;

#[derive(Default)]
pub struct Estado {
    destino: Option<WlSurface>,
    recientes: Vec<String>,
}

pub fn alternar(state: &mut BookosComp) {
    state.emojis.destino = state.seat.get_keyboard().and_then(|k| k.current_focus());
    let campo = state.seat.text_input().campo_activo();
    let punto = campo
        .as_ref()
        .and_then(|campo| {
            let rect = campo.cursor_rectangle?;
            let ventana = state.window_for_surface(&campo.surface)?;
            let origen = state.space.element_location(&ventana)? - ventana.geometry().loc;
            Some((
                origen.x as f32 + rect.loc.x as f32,
                origen.y as f32 + (rect.loc.y + rect.size.h) as f32 + 8.0,
            ))
        })
        .unwrap_or((
            state.pointer_location.x as f32 + 8.0,
            state.pointer_location.y as f32 + 8.0,
        ));
    if let Some(shell) = state.shell.as_mut() {
        shell.alternar_emojis(punto.0, punto.1, &state.emojis.recientes);
    }
    crate::keybinds::liberar_puntero_para_capa(state);
    state.needs_redraw = true;
}

pub fn insertar(state: &mut BookosComp, texto: &str) {
    let Some(teclado) = state.seat.get_keyboard() else {
        return;
    };
    let Some(destino) = state.emojis.destino.take() else {
        copiar(state, texto);
        recordar(&mut state.emojis.recientes, texto);
        if let Some(shell) = state.shell.as_mut() {
            shell.notificar(
                bookos_shell::notificaciones::Notificacion::nueva(
                    0,
                    "BookOS".into(),
                    "Emoji copiado".into(),
                    "Puedes pegarlo en cualquier aplicación".into(),
                    "input-keyboard",
                    false,
                ),
                2500,
            );
        }
        state.needs_redraw = true;
        return;
    };
    // Una ventana puede desaparecer o cambiar el foco mientras se busca.
    // Nunca se entrega la elección a otra aplicación que haya pasado delante.
    if teclado.current_focus().as_ref() != Some(&destino) {
        return;
    }
    let entrada = state.seat.text_input();
    let mut entregado = false;
    entrada.with_active_text_input(|campo, superficie| {
        if superficie == &destino {
            campo.commit_string(Some(texto.to_string()));
            entregado = true;
        }
    });
    if entregado {
        entrada.done(false);
        tracing::debug!("emoji enviado por text-input");
    } else {
        let tecla_v = teclado.with_xkb_state(state, |ctx| {
            let xkb = ctx.xkb().lock().expect("el mutex de xkb no se envenena");
            // SAFETY: la referencia al mapa solo se utiliza dentro del cierre.
            let mapa = unsafe { xkb.keymap() };
            let mut encontrada = None;
            mapa.key_for_each(|mapa, codigo| {
                if mapa
                    .key_get_syms_by_level(codigo, xkb.active_layout().0, 0)
                    .iter()
                    .any(|s| s.raw() == keysyms::KEY_v)
                {
                    encontrada = Some(codigo);
                }
            });
            encontrada
        });
        let Some(tecla_v) = tecla_v else {
            tracing::warn!("el mapa de teclado no tiene una V para pegar el emoji");
            return;
        };
        let terminal = state
            .window_for_surface(&destino)
            .and_then(|v| crate::handlers::app_id(&v))
            .is_some_and(|id| es_terminal(&id));
        copiar(state, texto);
        pegar(state, &teclado, &destino, tecla_v, terminal);
        tracing::debug!("emoji enviado mediante pegado");
    }
    recordar(&mut state.emojis.recientes, texto);
    state.needs_redraw = true;
}

fn copiar(state: &mut BookosComp, texto: &str) {
    let tipos = vec![
        "text/plain;charset=utf-8".into(),
        "text/plain".into(),
        "UTF8_STRING".into(),
    ];
    if let Some(xwm) = state.xwm.as_mut()
        && let Err(err) = xwm.new_selection(SelectionTarget::Clipboard, Some(tipos.clone()))
    {
        tracing::warn!("no se pudo ofrecer el emoji a X11: {err}");
    }
    set_data_device_selection(
        &state.display_handle,
        &state.seat,
        tipos,
        crate::handlers::Seleccion::Bytes(Arc::new(texto.as_bytes().to_vec())),
    );
}

fn pegar(
    state: &mut BookosComp,
    teclado: &smithay::input::keyboard::KeyboardHandle<BookosComp>,
    destino: &WlSurface,
    tecla: Keycode,
    terminal: bool,
) {
    // Se manda el modificador de pegado explícitamente: Intro o Meta pueden
    // seguir físicamente pulsados al elegir, pero no forman parte del texto.
    let anteriores = teclado.modifier_state();
    teclado.set_modifier_state(ModifiersState {
        ctrl: true,
        shift: terminal,
        caps_lock: anteriores.caps_lock,
        num_lock: anteriores.num_lock,
        ..Default::default()
    });
    // input_forward envía la tecla antes que los modificadores. Medido con
    // Qt/XWayland, eso insertaba «v»; Ctrl debe llegar antes de la pulsación.
    let seat = state.seat.clone();
    smithay::input::keyboard::KeyboardTarget::modifiers(
        destino,
        &seat,
        state,
        teclado.modifier_state(),
        SERIAL_COUNTER.next_serial(),
    );
    let time = state.start_time.elapsed().as_millis() as u32;
    teclado.input_forward(
        state,
        tecla,
        KeyState::Pressed,
        SERIAL_COUNTER.next_serial(),
        time,
        false,
    );
    teclado.input_forward(
        state,
        tecla,
        KeyState::Released,
        SERIAL_COUNTER.next_serial(),
        time,
        false,
    );
    teclado.set_modifier_state(anteriores);
    smithay::input::keyboard::KeyboardTarget::modifiers(
        destino,
        &seat,
        state,
        anteriores,
        SERIAL_COUNTER.next_serial(),
    );
}

fn es_terminal(id: &str) -> bool {
    let id = id.to_lowercase();
    id.contains("terminal")
        || id.contains("konsole")
        || id.contains("alacritty")
        || id.contains("wezterm")
        || matches!(
            id.as_str(),
            "foot" | "footclient" | "kitty" | "xterm" | "urxvt" | "st" | "tilix" | "terminator"
        )
}

fn recordar(recientes: &mut Vec<String>, texto: &str) {
    recientes.retain(|s| s != texto);
    recientes.insert(0, texto.to_string());
    recientes.truncate(32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recientes_no_duplica_y_conserva_la_secuencia_completa() {
        let mut recientes = vec!["😀".into(), "🧑‍💻".into()];
        recordar(&mut recientes, "🧑‍💻");
        assert_eq!(recientes, ["🧑‍💻", "😀"]);
        for i in 0..40 {
            recordar(&mut recientes, &i.to_string());
        }
        assert_eq!(recientes.len(), 32);
        assert_eq!(recientes[0], "39");
    }

    #[test]
    fn el_pegado_del_terminal_lleva_mayusculas() {
        for id in [
            "org.gnome.Terminal",
            "org.kde.konsole",
            "Alacritty",
            "foot",
            "kitty",
        ] {
            assert!(es_terminal(id), "{id}");
        }
        assert!(!es_terminal("org.bookos.notepad"));
    }
}
