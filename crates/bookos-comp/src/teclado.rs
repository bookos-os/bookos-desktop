//! El teclado en pantalla, del lado del compositor.
//!
//! El dibujo y las capas son de `bookos_shell::teclado`; aquí se decide cuándo
//! sale, a quién llega lo que se toca encima y cómo un carácter se convierte en
//! teclas.
//!
//! **El texto se escribe pulsando teclas de verdad**, no con `commit_string` de
//! text-input-v3: así llega igual a GTK, a Qt, a una WebView y a XWayland,
//! hablen o no ese protocolo. Cada carácter se busca en el keymap puesto, así
//! que vale con la distribución que tenga el usuario. Text-input solo se usa
//! para saber que hay un campo enfocado y de qué tipo es.

use std::time::{Duration, Instant};

use smithay::backend::input::{KeyState, TouchSlot};
use smithay::desktop::Window;
use smithay::input::keyboard::{FilterResult, Keycode, Keysym, keysyms, xkb};
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::ContentPurpose;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::text_input::TextInputSeat;

use bookos_shell::teclado::{Accion, Capa, Salida};

use crate::state::BookosComp;

/// Quién tiene apretada una tecla del teclado en pantalla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dedo {
    Toque(TouchSlot),
    Raton,
}

/// En coordenadas lógicas: el margen no cambia con la densidad del panel.
const MARGEN_TOQUE: f64 = 12.0;
const DURACION_TOQUE: u32 = 500;
/// El cliente puede habilitar text-input después de recibir `up`. El permiso
/// caduca para que un foco programático posterior no se tome por ese toque.
const ESPERA_CAMPO: Duration = Duration::from_secs(1);

struct Toque {
    slot: TouchSlot,
    origen: Point<f64, Logical>,
    desde: u32,
}

#[derive(Default)]
struct Toques {
    activos: Vec<TouchSlot>,
    candidato: Option<Toque>,
}

impl Toques {
    fn iniciar(&mut self, slot: TouchSlot, origen: Point<f64, Logical>, time: u32) {
        self.activos.push(slot);
        self.candidato = (self.activos.len() == 1).then_some(Toque {
            slot,
            origen,
            desde: time,
        });
    }

    fn mover(&mut self, slot: TouchSlot, punto: Point<f64, Logical>) {
        if self.candidato.as_ref().is_some_and(|t| {
            let delta = punto - t.origen;
            t.slot == slot && delta.x * delta.x + delta.y * delta.y > MARGEN_TOQUE * MARGEN_TOQUE
        }) {
            self.candidato = None;
        }
    }

    fn terminar(&mut self, slot: TouchSlot, time: u32) -> bool {
        self.activos.retain(|s| *s != slot);
        self.candidato
            .take()
            .is_some_and(|t| t.slot == slot && time.wrapping_sub(t.desde) <= DURACION_TOQUE)
    }
}

/// Lo que el compositor recuerda del teclado entre eventos.
#[derive(Default)]
pub struct Estado {
    /// La última entrada fue un toque. El teclado solo sale solo en ese caso,
    /// como Gboard en un convertible: con el teclado físico en uso, que
    /// aparezca al hacer clic en un campo es un estorbo.
    pub tactil: bool,
    /// La superficie con un campo habilitado, la última vez que se miró.
    campo: Option<WlSurface>,
    /// Lo último que se tocó de un cliente, hasta que se use para sacar el
    /// teclado. No basta con mirar si el campo cambia: uno que ya tenía el foco
    /// al abrirse la ventana se habilita antes del primer toque, y tocarlo
    /// después no manda nada nuevo.
    tocada: Option<(WlSurface, Instant)>,
    toques: Toques,
    superficie_toque: Option<WlSurface>,
    pulsado: Option<Dedo>,
    /// La ventana que se subió para que el teclado no tapara el cursor de
    /// texto, y dónde estaba.
    subida: Option<(Window, Point<i32, Logical>)>,
}

/// Un dedo o un clic en `punto`. Devuelve si era del teclado.
pub fn pulsar(state: &mut BookosComp, punto: Point<f64, Logical>, dedo: Dedo) -> bool {
    let Some(shell) = state.shell.as_mut() else {
        return false;
    };
    let Some((x, y)) = shell.teclado_en(punto.x, punto.y) else {
        return false;
    };
    // Un segundo dedo encima mientras el primero sigue apretado se ignora:
    // dos teclas a la vez no significan nada aquí.
    if state.teclado.pulsado.is_some() {
        return true;
    }
    state.teclado.pulsado = Some(dedo);
    let accion = shell.teclado_pulsar(x, y);
    state.needs_redraw = true;
    if let Some(accion) = accion {
        hacer(state, accion);
    }
    true
}

/// El dedo o el botón se levanta. Devuelve si era del teclado.
pub fn soltar(state: &mut BookosComp, dedo: Dedo) -> bool {
    if state.teclado.pulsado != Some(dedo) {
        return false;
    }
    state.teclado.pulsado = None;
    let accion = state.shell.as_mut().and_then(|s| s.teclado_soltar());
    state.needs_redraw = true;
    if let Some(accion) = accion {
        hacer(state, accion);
    }
    true
}

pub fn iniciar_toque(
    state: &mut BookosComp,
    slot: TouchSlot,
    punto: Point<f64, Logical>,
    time: u32,
) {
    state.teclado.tactil = true;
    state.teclado.tocada = None;
    state.teclado.superficie_toque = None;
    state.teclado.toques.iniciar(slot, punto, time);
}

/// El destino se conserva sin abrir nada hasta distinguir toque de arrastre.
pub fn tocado(state: &mut BookosComp, surface: &WlSurface) {
    state.teclado.superficie_toque = Some(surface.clone());
}

pub fn mover_toque(state: &mut BookosComp, slot: TouchSlot, punto: Point<f64, Logical>) {
    state.teclado.toques.mover(slot, punto);
}

pub fn terminar_toque(state: &mut BookosComp, slot: TouchSlot, time: u32) {
    let superficie = state.teclado.superficie_toque.take();
    if state.teclado.toques.terminar(slot, time) {
        state.teclado.tocada = superficie.map(|s| (s, Instant::now()));
    }
}

pub fn cancelar_toques(state: &mut BookosComp) {
    state.teclado.toques = Toques::default();
    state.teclado.superficie_toque = None;
    state.teclado.tocada = None;
}

/// Se dejó de escribir con el dedo —se tocó el teclado físico o se apagó la
/// pantalla táctil—: el teclado en pantalla sobra.
pub fn retirar(state: &mut BookosComp) {
    cancelar_toques(state);
    state.teclado.tactil = false;
    if state.shell.as_ref().is_some_and(|s| s.teclado_abierto()) {
        ocultar(state);
    }
}

/// Mira si se enfocó o se dejó un campo de texto, y saca o guarda el teclado.
///
/// Se llama tras cada tanda de peticiones de clientes: el aviso de un campo
/// nuevo es un `enable` de text-input, que llega por ahí.
pub fn revisar(state: &mut BookosComp) {
    if state
        .teclado
        .tocada
        .as_ref()
        .is_some_and(|(_, t)| t.elapsed() > ESPERA_CAMPO)
    {
        state.teclado.tocada = None;
    }
    let Some(campo) = state.seat.text_input().campo_activo() else {
        if state.teclado.campo.take().is_some() {
            ocultar(state);
        }
        return;
    };
    state.teclado.campo = Some(campo.surface.clone());
    // El toque puede caer en una subsuperficie o en un popup del mismo
    // cliente: lo que cuenta es que sea la aplicación del campo.
    let tocada = state
        .teclado
        .tocada
        .as_ref()
        .is_some_and(|(t, _)| t.id().same_client_as(&campo.surface.id()));
    if !tocada || !state.teclado.tactil {
        return;
    }
    state.teclado.tocada = None;
    let capa = match campo.content_type.map(|(_, proposito)| proposito) {
        Some(
            ContentPurpose::Digits
            | ContentPurpose::Number
            | ContentPurpose::Phone
            | ContentPurpose::Pin,
        ) => Capa::Simbolos,
        _ => Capa::Letras,
    };
    if let Some(shell) = state.shell.as_mut()
        && !shell.teclado_abierto()
    {
        shell.mostrar_teclado(capa);
        state.needs_redraw = true;
    }
    if let Some(rect) = campo.cursor_rectangle {
        subir_ventana(state, &campo.surface, rect.loc.y + rect.size.h);
    }
}

fn ocultar(state: &mut BookosComp) {
    if let Some(shell) = state.shell.as_mut() {
        shell.ocultar_teclado();
    }
    if let Some((window, donde)) = state.teclado.subida.take() {
        state.space.map_element(window, donde, false);
    }
    state.needs_redraw = true;
}

/// Sube la ventana del campo lo justo para que su cursor de texto quede por
/// encima del teclado, como `adjustPan` en Android. No se redimensiona: un
/// cliente que cambia de tamaño reflota el contenido y el campo puede acabar
/// en otro sitio, mientras que moverla deja todo donde el usuario lo veía.
fn subir_ventana(state: &mut BookosComp, surface: &WlSurface, cursor_abajo: i32) {
    let Some(window) = state.window_for_surface(surface) else {
        return;
    };
    let Some(donde) = state.space.element_location(&window) else {
        return;
    };
    let alto = state.shell.as_ref().map_or(0.0, |s| s.area_principal().1);
    // Algo de aire sobre el teclado: con el cursor pegado al borde, la línea
    // que se escribe queda a ras y no se lee la de debajo.
    let limite = alto - f64::from(bookos_shell::teclado::ALTO) - 16.0;
    let origen = donde - window.geometry().loc;
    let sobra = f64::from(origen.y + cursor_abajo) - limite;
    if sobra <= 0.0 {
        return;
    }
    let nuevo = Point::from((donde.x, donde.y - sobra.ceil() as i32));
    state.teclado.subida.get_or_insert((window.clone(), donde));
    state.space.map_element(window, nuevo, false);
}

fn hacer(state: &mut BookosComp, accion: Accion) {
    match accion {
        Accion::Escribe(salida) => escribir(state, salida, Fase::Entera),
        Accion::Pulsa(salida) => escribir(state, salida, Fase::Baja),
        Accion::Suelta(salida) => escribir(state, salida, Fase::Sube),
        Accion::Ocultar => ocultar(state),
    }
}

/// Qué parte de una pulsación se envía. Bajar y subir por separado solo lo usa
/// el retroceso, que se mantiene, y no lleva modificadores.
#[derive(Clone, Copy)]
enum Fase {
    Entera,
    Baja,
    Sube,
}

/// Pulsa y suelta lo que haga falta para que salga `salida`.
fn escribir(state: &mut BookosComp, salida: Salida, fase: Fase) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let pasos = keyboard.with_xkb_state(state, |ctx| {
        let xkb = ctx.xkb().lock().expect("el mutex de xkb no se envenena");
        // SAFETY: la referencia al keymap no sale del cierre.
        let keymap = unsafe { xkb.keymap() };
        let layout = xkb.active_layout().0;
        pasos(keymap, layout, salida)
    });
    let Some(pasos) = pasos else {
        tracing::warn!(
            ?salida,
            "el teclado en pantalla no encuentra esa tecla en el keymap"
        );
        return;
    };
    let time = state.start_time.elapsed().as_millis() as u32;
    match fase {
        Fase::Entera => {
            for (keycode, modificadores) in &pasos {
                for m in modificadores {
                    enviar(state, &keyboard, *m, KeyState::Pressed, time);
                }
                enviar(state, &keyboard, *keycode, KeyState::Pressed, time);
                enviar(state, &keyboard, *keycode, KeyState::Released, time);
                for m in modificadores.iter().rev() {
                    enviar(state, &keyboard, *m, KeyState::Released, time);
                }
            }
        }
        Fase::Baja | Fase::Sube => {
            let estado = match fase {
                Fase::Baja => KeyState::Pressed,
                _ => KeyState::Released,
            };
            if let Some((keycode, _)) = pasos.last() {
                enviar(state, &keyboard, *keycode, estado, time);
            }
        }
    }
}

fn enviar(
    state: &mut BookosComp,
    keyboard: &smithay::input::keyboard::KeyboardHandle<BookosComp>,
    keycode: Keycode,
    fase: KeyState,
    time: u32,
) {
    // Directo al cliente, sin pasar por los atajos del compositor: una «q»
    // del teclado en pantalla con Meta apretado en el físico no debe cerrar
    // la ventana.
    keyboard.input::<(), _>(
        state,
        keycode,
        fase,
        SERIAL_COUNTER.next_serial(),
        time,
        |_, _, _| FilterResult::Forward,
    );
}

/// Las teclas que producen `salida`: una por paso, con los modificadores que
/// hay que mantener durante ella. Una letra con tilde que no esté en el
/// keymap son dos pasos: la muerta y la vocal, como en un teclado físico.
fn pasos(
    keymap: &xkb::Keymap,
    layout: u32,
    salida: Salida,
) -> Option<Vec<(Keycode, Vec<Keycode>)>> {
    let una = |sym: Keysym| tecla_para(keymap, layout, sym).map(|t| vec![t]);
    match salida {
        Salida::Intro => una(Keysym::from(keysyms::KEY_Return)),
        Salida::Retroceso => una(Keysym::from(keysyms::KEY_BackSpace)),
        Salida::Caracter(c) => una(xkb::utf32_to_keysym(c as u32)).or_else(|| {
            let (muerta, base) = descomponer(c)?;
            let muerta = tecla_para(
                keymap,
                layout,
                xkb::keysym_from_name(muerta, xkb::KEYSYM_NO_FLAGS),
            )?;
            let base = tecla_para(keymap, layout, xkb::utf32_to_keysym(base as u32))?;
            Some(vec![muerta, base])
        }),
    }
}

/// La tecla y los modificadores que dan `sym` en la distribución puesta.
///
/// xkbcommon 0.8 no expone qué modificadores llevan a cada nivel
/// (`xkb_keymap_key_get_mods_for_level` es de la 1.0 de la biblioteca de C y
/// no está en el binding), así que se usa la convención de los tipos
/// `TWO_LEVEL`, `ALPHABETIC` y `FOUR_LEVEL` de xkeyboard-config: nivel 2 con
/// Mayúsculas, 3 con AltGr y 4 con los dos. Es lo que tienen todas las teclas
/// de escribir de las distribuciones latinas.
fn tecla_para(keymap: &xkb::Keymap, layout: u32, sym: Keysym) -> Option<(Keycode, Vec<Keycode>)> {
    let buscar = |sym: Keysym, solo_base: bool| {
        let mut mejor: Option<(u32, Keycode)> = None;
        keymap.key_for_each(|keymap, keycode| {
            let niveles = keymap.num_levels_for_key(keycode, layout);
            let niveles = if solo_base {
                niveles.min(1)
            } else {
                niveles.min(4)
            };
            for nivel in 0..niveles {
                if keymap.key_get_syms_by_level(keycode, layout, nivel) == [sym]
                    && mejor.is_none_or(|(n, _)| nivel < n)
                {
                    mejor = Some((nivel, keycode));
                }
            }
        });
        mejor
    };
    let (nivel, keycode) = buscar(sym, false)?;
    let modificador = |nombre| buscar(Keysym::from(nombre), true).map(|(_, k)| k);
    let mods = match nivel {
        0 => vec![],
        1 => vec![modificador(keysyms::KEY_Shift_L)?],
        2 => vec![modificador(keysyms::KEY_ISO_Level3_Shift)?],
        _ => vec![
            modificador(keysyms::KEY_Shift_L)?,
            modificador(keysyms::KEY_ISO_Level3_Shift)?,
        ],
    };
    Some((keycode, mods))
}

/// La tecla muerta y la letra base de una vocal acentuada.
fn descomponer(c: char) -> Option<(&'static str, char)> {
    const TABLA: [(&str, &str, &str); 4] = [
        ("dead_acute", "áéíóúÁÉÍÓÚ", "aeiouAEIOU"),
        ("dead_grave", "àèìòùÀÈÌÒÙ", "aeiouAEIOU"),
        ("dead_diaeresis", "äëïöüÄËÏÖÜ", "aeiouAEIOU"),
        ("dead_circumflex", "âêîôûÂÊÎÔÛ", "aeiouAEIOU"),
    ];
    TABLA.iter().find_map(|(muerta, con, sin)| {
        let i = con.chars().position(|x| x == c)?;
        Some((*muerta, sin.chars().nth(i)?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(n: u32) -> TouchSlot {
        TouchSlot::from(Some(n))
    }

    #[test]
    fn un_toque_breve_admite_el_pequeno_movimiento_del_dedo() {
        let mut toques = Toques::default();
        toques.iniciar(slot(0), (100.0, 200.0).into(), 100);
        toques.mover(slot(0), (106.0, 205.0).into());
        assert!(toques.terminar(slot(0), 200));
    }

    #[test]
    fn desplazar_y_volver_al_origen_no_se_convierte_en_un_toque() {
        let mut toques = Toques::default();
        toques.iniciar(slot(0), (100.0, 200.0).into(), 100);
        toques.mover(slot(0), (100.0, 160.0).into());
        toques.mover(slot(0), (100.0, 200.0).into());
        assert!(!toques.terminar(slot(0), 200));
    }

    #[test]
    fn un_desplazamiento_horizontal_tampoco_abre_el_teclado() {
        let mut toques = Toques::default();
        toques.iniciar(slot(0), (100.0, 200.0).into(), 100);
        toques.mover(slot(0), (140.0, 200.0).into());
        assert!(!toques.terminar(slot(0), 200));
    }

    #[test]
    fn varios_dedos_no_abren_el_teclado_al_levantarse_en_cualquier_orden() {
        for orden in [[0, 1], [1, 0]] {
            let mut toques = Toques::default();
            toques.iniciar(slot(0), (100.0, 200.0).into(), 100);
            toques.iniciar(slot(1), (150.0, 200.0).into(), 110);
            for dedo in orden {
                assert!(!toques.terminar(slot(dedo), 200));
            }
            toques.iniciar(slot(0), (100.0, 200.0).into(), 300);
            assert!(
                toques.terminar(slot(0), 400),
                "un toque nuevo vuelve a valer"
            );
        }
    }

    #[test]
    fn mantener_el_dedo_no_se_confunde_con_tocar() {
        let mut toques = Toques::default();
        toques.iniciar(slot(0), (100.0, 200.0).into(), 100);
        assert!(!toques.terminar(slot(0), 800));
    }

    #[test]
    fn el_reloj_de_entrada_puede_dar_la_vuelta_durante_el_toque() {
        let mut toques = Toques::default();
        toques.iniciar(slot(0), (100.0, 200.0).into(), u32::MAX - 100);
        assert!(toques.terminar(slot(0), 100));
    }

    fn keymap(layout: &str) -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&ctx, "", "", layout, "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("xkeyboard-config tiene la distribución")
    }

    #[test]
    fn en_espanol_las_tildes_son_muerta_y_vocal() {
        let km = keymap("es");
        let a = pasos(&km, 0, Salida::Caracter('á')).expect("á se puede escribir");
        assert_eq!(a.len(), 2);
        let n = pasos(&km, 0, Salida::Caracter('ñ')).expect("ñ tiene tecla propia");
        assert_eq!(n.len(), 1);
        assert!(n[0].1.is_empty());
    }

    #[test]
    fn mayusculas_y_altgr_llevan_su_modificador() {
        let km = keymap("es");
        let (_, mods) = tecla_para(&km, 0, xkb::utf32_to_keysym('A' as u32)).expect("A");
        assert_eq!(mods.len(), 1);
        // La arroba es AltGr+2 en la distribución española.
        let (_, mods) = tecla_para(&km, 0, xkb::utf32_to_keysym('@' as u32)).expect("@");
        assert_eq!(mods.len(), 1);
        let (_, mods) = tecla_para(&km, 0, xkb::utf32_to_keysym('a' as u32)).expect("a");
        assert!(mods.is_empty());
    }

    #[test]
    fn todas_las_teclas_de_las_capas_se_escriben_en_espanol() {
        let km = keymap("es");
        let faltan: String = "qwertyuiopasdfghjklñzxcvbnm1234567890@#€_&-+()/*\"':;!?,. áéíóúçÁ¿¡"
            .chars()
            .filter(|c| pasos(&km, 0, Salida::Caracter(*c)).is_none())
            .collect();
        assert_eq!(faltan, "");
    }
}
