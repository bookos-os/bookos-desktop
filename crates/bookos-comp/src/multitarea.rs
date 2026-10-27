//! Grupos de trabajo y familias de diálogos. Las pestañas permanecen mapeadas
//! para conservar sus escritorios y su ciclo de vida, pero solo la activa se
//! dibuja y recibe entrada.

use smithay::desktop::Window;
use smithay::utils::{IsAlive, Logical, Point, Rectangle};
use smithay::wayland::seat::WaylandFocus;

use crate::state::BookosComp;
use crate::ventanas::{self, Duena};

const SEPARADOR: i32 = 8;

#[derive(Debug, Clone, Copy)]
pub enum Disposicion {
    Lados(f64),
    Pestanas(usize),
}

#[derive(Debug)]
pub struct Grupo {
    ventanas: Vec<(Window, Rectangle<i32, Logical>)>,
    disposicion: Disposicion,
    area: Rectangle<i32, Logical>,
    area_salida: Rectangle<i32, Logical>,
    separador_id: smithay::backend::renderer::element::Id,
}

#[derive(Debug, Default)]
pub struct Multitarea {
    grupos: Vec<Grupo>,
    separador: Option<(Window, f64, bool)>,
    pub clic_consumido: bool,
}

pub fn raiz(state: &BookosComp, window: &Window) -> Window {
    match ventanas::duena(state, window) {
        Some(Duena::Padre(padre)) => padre,
        Some(Duena::Proceso(_)) | None => window.clone(),
    }
}

fn indice(state: &BookosComp, window: &Window) -> Option<usize> {
    if state.multitarea.grupos.is_empty() {
        return None;
    }
    let raiz = raiz(state, window);
    state
        .multitarea
        .grupos
        .iter()
        .position(|g| g.ventanas.iter().any(|(w, _)| *w == raiz))
}

pub fn agrupada(state: &BookosComp, window: &Window) -> bool {
    indice(state, window).is_some()
}

pub fn misma_unidad(state: &BookosComp, a: &Window, b: &Window) -> bool {
    raiz(state, a) == raiz(state, b)
        || indice(state, a).is_some_and(|i| Some(i) == indice(state, b))
}

pub fn visible(state: &BookosComp, window: &Window) -> bool {
    let Some(i) = indice(state, window) else {
        return true;
    };
    let grupo = &state.multitarea.grupos[i];
    match grupo.disposicion {
        Disposicion::Lados(_) => true,
        Disposicion::Pestanas(activa) => grupo
            .ventanas
            .get(activa)
            .is_some_and(|(w, _)| *w == raiz(state, window)),
    }
}

pub fn familia(state: &BookosComp, window: &Window) -> Vec<Window> {
    state
        .space
        .elements()
        .filter(|w| w.alive() && misma_unidad(state, window, w))
        .cloned()
        .collect()
}

pub fn candidata(state: &BookosComp, window: &Window) -> Option<Window> {
    state
        .space
        .elements()
        .filter(|w| w.alive() && ventanas::colocada(w) && !ventanas::completa(w))
        .filter(|w| raiz(state, w) == **w && !misma_unidad(state, window, w))
        .filter(|w| {
            crate::escritorios::salida_de(state, w) == crate::escritorios::salida_de(state, window)
        })
        .max_by_key(|w| ventanas::enfocada_en(w))
        .cloned()
}

pub fn unir(state: &mut BookosComp, window: &Window, otra: &Window, pestanas: bool) {
    if misma_unidad(state, window, otra) {
        return;
    }
    let window = raiz(state, window);
    let otra = raiz(state, otra);
    if window == otra || ventanas::completa(&window) || ventanas::completa(&otra) {
        return;
    }
    // Un grupo lateral tiene dos paneles; al añadir más aplicaciones se usan
    // pestañas para que ningún panel acabe reducido a una columna ilegible.
    let mut miembros = Vec::new();
    for w in [&otra, &window] {
        if let Some(i) = indice(state, w) {
            let grupo = state.multitarea.grupos.remove(i);
            miembros.extend(grupo.ventanas);
        } else if let Some(loc) = state.space.element_location(w) {
            miembros.push((w.clone(), Rectangle::new(loc, w.geometry().size)));
        }
    }
    if miembros.len() < 2 {
        return;
    }
    for (w, _) in &miembros {
        crate::decoracion::agrupar(w, true);
        ventanas::soltar_encaje(w);
    }
    let disposicion = if pestanas || miembros.len() > 2 {
        Disposicion::Pestanas(miembros.len() - 1)
    } else {
        Disposicion::Lados(0.5)
    };
    let area = state.work_area();
    state.multitarea.grupos.push(Grupo {
        ventanas: miembros,
        disposicion,
        area,
        area_salida: area,
        separador_id: smithay::backend::renderer::element::Id::new(),
    });
    recolocar(state, state.multitarea.grupos.len() - 1);
    state.enfocar(&window);
}

pub fn cambiar_disposicion(state: &mut BookosComp, window: &Window, pestanas: bool) {
    let Some(i) = indice(state, window) else {
        return;
    };
    let grupo = &mut state.multitarea.grupos[i];
    if !pestanas && grupo.ventanas.len() != 2 {
        return;
    }
    grupo.disposicion = if pestanas {
        Disposicion::Pestanas(0)
    } else {
        Disposicion::Lados(0.5)
    };
    recolocar(state, i);
    state.enfocar(window);
}

pub fn separar(state: &mut BookosComp, window: &Window) {
    let Some(i) = indice(state, window) else {
        return;
    };
    let grupo = state.multitarea.grupos.remove(i);
    for (w, previa) in grupo.ventanas {
        crate::decoracion::agrupar(&w, false);
        aplicar(state, &w, previa);
    }
    state.needs_redraw = true;
}

/// Saca del grupo la ventana que se empieza a arrastrar si está lado a lado.
/// Es lo que hace soltar una ventana encajada: sin esto, arrastrar una de las
/// dos movía el grupo entero y la única salida era un menú escondido.
/// Las pestañas sí se mueven juntas: ahí el grupo es una sola ventana.
pub fn sacar(state: &mut BookosComp, window: &Window) -> bool {
    let Some(i) = indice(state, window) else {
        return false;
    };
    if raiz(state, window) != *window
        || !matches!(
            state.multitarea.grupos[i].disposicion,
            Disposicion::Lados(_)
        )
    {
        return false;
    }
    separar(state, window);
    true
}

pub fn preparar_foco(state: &mut BookosComp, window: &Window) -> Window {
    if let Some(i) = indice(state, window) {
        let raiz = raiz(state, window);
        let grupo = &mut state.multitarea.grupos[i];
        if let Disposicion::Pestanas(ref mut activa) = grupo.disposicion
            && let Some(n) = grupo.ventanas.iter().position(|(w, _)| *w == raiz)
        {
            *activa = n;
        }
    }
    let familia = familia(state, window);
    // Primero las principales, después sus diálogos. Volver desde Alt+Tab o
    // el dock no debe enterrar un diálogo debajo de la ventana que lo abrió.
    let orden: Vec<_> = familia
        .iter()
        .filter(|w| raiz(state, w) == **w)
        .chain(familia.iter().filter(|w| raiz(state, w) != **w))
        .cloned()
        .collect();
    for w in &orden {
        state.space.raise_element(w, false);
        if let (Some(x11), Some(xwm)) = (w.x11_surface(), state.xwm.as_mut()) {
            let _ = xwm.raise_window(x11);
        }
    }
    let raiz = raiz(state, window);
    if raiz != *window {
        return window.clone();
    }
    familia
        .into_iter()
        .filter(|w| *w != raiz && self::raiz(state, w) == raiz && visible(state, w))
        .max_by_key(ventanas::enfocada_en)
        .unwrap_or_else(|| window.clone())
}

pub fn pestanas(state: &BookosComp, window: &Window) -> (Vec<String>, usize) {
    if raiz(state, window) != *window {
        return (Vec::new(), 0);
    }
    let Some(i) = indice(state, window) else {
        return (Vec::new(), 0);
    };
    let grupo = &state.multitarea.grupos[i];
    let Disposicion::Pestanas(activa) = grupo.disposicion else {
        return (Vec::new(), 0);
    };
    (
        grupo
            .ventanas
            .iter()
            .map(|(w, _)| crate::handlers::titulo(w).unwrap_or_else(|| "Aplicación".into()))
            .collect(),
        activa,
    )
}

pub fn pulsar_pestana(state: &mut BookosComp, window: &Window, x: f64) -> bool {
    if raiz(state, window) != *window {
        return false;
    }
    let Some(i) = indice(state, window) else {
        return false;
    };
    let grupo = &state.multitarea.grupos[i];
    if !matches!(grupo.disposicion, Disposicion::Pestanas(_)) {
        return false;
    }
    let Some(n) = bookos_shell::decoracion::pestana_en(
        window.geometry().size.w as f32,
        grupo.ventanas.len(),
        x as f32,
    ) else {
        return false;
    };
    let window = grupo.ventanas[n].0.clone();
    state.enfocar(&window);
    state.multitarea.clic_consumido = true;
    true
}

fn min_ancho(window: &Window) -> i32 {
    if let Some(x11) = window.x11_surface() {
        return x11.min_size().map_or(120, |s| s.w.max(120));
    }
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::SurfaceCachedState;
    window.wl_surface().map_or(120, |s| {
        with_states(&s, |states| {
            states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .min_size
                .w
                .max(120)
        })
    })
}

fn reparto(
    area: Rectangle<i32, Logical>,
    proporcion: f64,
    minimos: [i32; 2],
) -> [Rectangle<i32, Logical>; 2] {
    let ancho = (area.size.w - SEPARADOR).max(2);
    let (minimo, maximo) = if minimos[0] + minimos[1] <= ancho {
        (minimos[0].max(1), (ancho - minimos[1]).min(ancho - 1))
    } else {
        (ancho / 2, ancho / 2)
    };
    let izquierda = ((ancho as f64 * proporcion).round() as i32).clamp(minimo, maximo);
    [
        Rectangle::new(area.loc, (izquierda, area.size.h).into()),
        Rectangle::new(
            (area.loc.x + izquierda + SEPARADOR, area.loc.y).into(),
            (ancho - izquierda, area.size.h).into(),
        ),
    ]
}

fn recolocar(state: &mut BookosComp, i: usize) {
    let pila: Vec<_> = state.space.elements().cloned().collect();
    let grupo = &state.multitarea.grupos[i];
    let destinos: Vec<_> = match grupo.disposicion {
        Disposicion::Pestanas(_) => vec![grupo.area; grupo.ventanas.len()],
        Disposicion::Lados(p) => reparto(
            grupo.area,
            p,
            [
                min_ancho(&grupo.ventanas[0].0),
                min_ancho(&grupo.ventanas[1].0),
            ],
        )
        .to_vec(),
    };
    let ventanas: Vec<_> = grupo.ventanas.iter().map(|(w, _)| w.clone()).collect();
    for (w, mut rect) in ventanas.into_iter().zip(destinos) {
        rect.loc.y += crate::decoracion::ALTO;
        rect.size.h = (rect.size.h - crate::decoracion::ALTO).max(1);
        aplicar(state, &w, rect);
        colocar_dialogos(state, &w, rect);
    }
    // map_element cambia la pila aunque no active la ventana. Conservarla
    // evita que el segundo panel se coloque por encima del diálogo del primero.
    for w in pila {
        state.space.raise_element(&w, false);
    }
    state.revisar_barras();
    state.needs_redraw = true;
}

fn aplicar(state: &mut BookosComp, window: &Window, rect: Rectangle<i32, Logical>) {
    let delta = state.space.element_location(window).map(|p| rect.loc - p);
    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|s| {
            s.size = Some(rect.size);
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
            s.states.unset(State::Maximized);
        });
        toplevel.send_pending_configure();
    }
    if let Some(x11) = window.x11_surface() {
        let _ = x11.set_maximized(false);
        let _ = x11.configure(rect);
    }
    state.space.map_element(window.clone(), rect.loc, false);
    if let Some(delta) = delta {
        acompanar(state, window, delta);
    }
}

pub fn acompanar(state: &mut BookosComp, window: &Window, delta: Point<i32, Logical>) {
    let hijos: Vec<_> = state
        .space
        .elements()
        .filter(|w| *w != window && raiz(state, w) == *window)
        .filter_map(|w| Some((w.clone(), state.space.element_location(w)? + delta)))
        .collect();
    for (hijo, loc) in hijos {
        if let Some(x11) = hijo.x11_surface() {
            let _ = x11.configure(Rectangle::new(loc, hijo.geometry().size));
        }
        state.space.map_element(hijo, loc, false);
    }
}

pub fn separador_en(state: &BookosComp, punto: Point<f64, Logical>) -> Option<Window> {
    if state.multitarea.grupos.is_empty() {
        return None;
    }
    // Una ventana flotante o un diálogo por encima del grupo tapa también su
    // separador: no se puede redimensionar lo que no se está viendo.
    for w in state.space.elements().rev().filter(|w| visible(state, w)) {
        if state
            .space
            .element_geometry(w)
            .into_iter()
            .chain(crate::decoracion::barra_rect(state, w))
            .any(|r| r.to_f64().contains(punto))
        {
            return None;
        }
        let Some(i) = indice(state, w) else { continue };
        let g = &state.multitarea.grupos[i];
        if let Disposicion::Lados(p) = g.disposicion {
            let [izq, _] = reparto(
                g.area,
                p,
                [min_ancho(&g.ventanas[0].0), min_ancho(&g.ventanas[1].0)],
            );
            let rect = Rectangle::new(
                (izq.loc.x + izq.size.w, izq.loc.y).into(),
                (SEPARADOR, izq.size.h).into(),
            );
            if rect.to_f64().contains(punto) {
                return Some(g.ventanas[0].0.clone());
            }
        }
    }
    None
}

pub fn empezar_separador(state: &mut BookosComp) -> bool {
    let Some(w) = separador_en(state, state.pointer_location) else {
        return false;
    };
    empezar_redimension(state, &w, false)
}

pub fn seguir_separador(state: &mut BookosComp) -> bool {
    let Some((w, desfase, _)) = state.multitarea.separador.clone() else {
        return false;
    };
    let Some(i) = indice(state, &w) else {
        return false;
    };
    let grupo = &mut state.multitarea.grupos[i];
    grupo.disposicion = Disposicion::Lados(
        (state.pointer_location.x - desfase - grupo.area.loc.x as f64)
            / (grupo.area.size.w - SEPARADOR).max(1) as f64,
    );
    recolocar(state, i);
    true
}

pub fn soltar(state: &mut BookosComp) -> bool {
    state
        .multitarea
        .separador
        .take()
        .is_some_and(|(_, _, del_cliente)| !del_cliente)
        | std::mem::take(&mut state.multitarea.clic_consumido)
}

pub fn arrastrar(state: &mut BookosComp, window: &Window, delta: Point<i32, Logical>) -> bool {
    if raiz(state, window) != *window {
        return false;
    }
    let Some(i) = indice(state, window) else {
        return false;
    };
    state.multitarea.grupos[i].area.loc += delta;
    recolocar(state, i);
    true
}

pub fn purgar(state: &mut BookosComp) {
    let mut cambio = false;
    for g in &mut state.multitarea.grupos {
        let activa = match g.disposicion {
            Disposicion::Pestanas(n) => g.ventanas.get(n).map(|(w, _)| w.clone()),
            _ => None,
        };
        let antes = g.ventanas.len();
        g.ventanas.retain(|(w, _)| w.alive());
        if antes != g.ventanas.len() {
            cambio = true;
            if let Disposicion::Pestanas(ref mut n) = g.disposicion {
                *n = activa
                    .and_then(|w| g.ventanas.iter().position(|(v, _)| *v == w))
                    .unwrap_or(0);
            }
        }
    }
    state.multitarea.grupos.retain(|g| {
        if g.ventanas.len() >= 2 {
            return true;
        }
        for (w, _) in &g.ventanas {
            crate::decoracion::agrupar(w, false);
        }
        false
    });
    state.needs_redraw |= cambio;
}

pub fn ajustar_monitor(state: &mut BookosComp, window: &Window, nombre: &str) {
    let Some(i) = indice(state, window) else {
        return;
    };
    let Some(rect) = state
        .space
        .outputs()
        .find(|o| o.name() == nombre)
        .and_then(|o| state.space.output_geometry(o))
    else {
        return;
    };
    let puntero = state.pointer_location;
    state.pointer_location = rect.loc.to_f64() + Point::from((1.0, 1.0));
    let area = state.work_area();
    state.multitarea.grupos[i].area = area;
    state.multitarea.grupos[i].area_salida = area;
    state.pointer_location = puntero;
    recolocar(state, i);
}

pub fn empezar_redimension(state: &mut BookosComp, window: &Window, del_cliente: bool) -> bool {
    if raiz(state, window) != *window {
        return false;
    }
    let Some(i) = indice(state, window) else {
        return false;
    };
    let g = &state.multitarea.grupos[i];
    let Disposicion::Lados(p) = g.disposicion else {
        return false;
    };
    let [izq, _] = reparto(
        g.area,
        p,
        [min_ancho(&g.ventanas[0].0), min_ancho(&g.ventanas[1].0)],
    );
    let desfase = state.pointer_location.x - (izq.loc.x + izq.size.w) as f64;
    state.multitarea.separador = Some((window.clone(), desfase, del_cliente));
    true
}

pub fn encajada(state: &mut BookosComp, window: &Window, zona: ventanas::Zona) {
    use ventanas::Zona;
    let opuesta = match zona {
        Zona::Izquierda => Zona::Derecha,
        Zona::Derecha => Zona::Izquierda,
        _ => return,
    };
    let otra = state
        .space
        .elements()
        .rev()
        .find(|w| {
            **w != *window
                && ventanas::zona_de(w) == Some(opuesta)
                && crate::escritorios::salida_de(state, w)
                    == crate::escritorios::salida_de(state, window)
        })
        .cloned();
    if let Some(otra) = otra {
        if zona == Zona::Derecha {
            unir(state, window, &otra, false);
        } else {
            unir(state, &otra, window, false);
        }
        state.enfocar(window);
    }
}

pub fn redimensionar(
    state: &mut BookosComp,
    window: &Window,
    mut rect: Rectangle<i32, Logical>,
) -> bool {
    if raiz(state, window) != *window {
        return false;
    }
    let Some(i) = indice(state, window) else {
        return false;
    };
    rect.loc.y -= crate::decoracion::ALTO;
    rect.size.h += crate::decoracion::ALTO;
    state.multitarea.grupos[i].area = rect;
    recolocar(state, i);
    true
}

pub fn retirar(state: &mut BookosComp, window: &Window) {
    for g in &mut state.multitarea.grupos {
        if let Disposicion::Pestanas(ref mut activa) = g.disposicion
            && let Some(n) = g.ventanas.iter().position(|(w, _)| w == window)
        {
            if n < *activa {
                *activa -= 1;
            } else if n == *activa {
                *activa = 0;
            }
        }
        g.ventanas.retain(|(w, _)| w != window);
    }
    crate::decoracion::agrupar(window, false);
    purgar(state);
}

fn colocar_dialogos(state: &mut BookosComp, window: &Window, rect: Rectangle<i32, Logical>) {
    let hijos: Vec<_> = state
        .space
        .elements()
        .filter(|w| *w != window && raiz(state, w) == *window)
        .cloned()
        .collect();
    for hijo in hijos {
        let mut area = rect;
        if crate::decoracion::decorada(&hijo) {
            area.loc.y += crate::decoracion::ALTO;
            area.size.h = (area.size.h - crate::decoracion::ALTO).max(1);
        }
        let loc = ventanas::encima_de(hijo.geometry().size, rect, area);
        if let Some(x11) = hijo.x11_surface() {
            let _ = x11.configure(Rectangle::new(loc, hijo.geometry().size));
        }
        state.space.map_element(hijo, loc, false);
    }
}

pub fn ajustar_areas(state: &mut BookosComp) {
    for i in 0..state.multitarea.grupos.len() {
        let w = &state.multitarea.grupos[i].ventanas[0].0;
        let Some(nombre) = crate::escritorios::salida_de(state, w) else {
            continue;
        };
        let Some(rect) = state
            .space
            .outputs()
            .find(|o| o.name() == nombre)
            .and_then(|o| state.space.output_geometry(o))
        else {
            continue;
        };
        let puntero = state.pointer_location;
        state.pointer_location = rect.loc.to_f64() + Point::from((1.0, 1.0));
        let area = state.work_area();
        state.pointer_location = puntero;
        let g = &mut state.multitarea.grupos[i];
        if g.area_salida == area {
            continue;
        }
        if g.area.size.w == g.area_salida.size.w {
            g.area.size.w = area.size.w;
        }
        if g.area.size.h == g.area_salida.size.h {
            g.area.size.h = area.size.h;
        }
        g.area.size.w = g.area.size.w.min(area.size.w);
        g.area.size.h = g.area.size.h.min(area.size.h);
        g.area.loc.x = g
            .area
            .loc
            .x
            .clamp(area.loc.x, area.loc.x + area.size.w - g.area.size.w);
        g.area.loc.y = g
            .area
            .loc
            .y
            .clamp(area.loc.y, area.loc.y + area.size.h - g.area.size.h);
        g.area_salida = area;
        recolocar(state, i);
    }
}

pub fn separador_de(
    state: &BookosComp,
    window: &Window,
) -> Option<(
    smithay::backend::renderer::element::Id,
    Rectangle<i32, Logical>,
)> {
    let i = indice(state, window)?;
    let g = &state.multitarea.grupos[i];
    if g.ventanas[0].0 != *window {
        return None;
    }
    let Disposicion::Lados(p) = g.disposicion else {
        return None;
    };
    let [izq, _] = reparto(
        g.area,
        p,
        [min_ancho(&g.ventanas[0].0), min_ancho(&g.ventanas[1].0)],
    );
    Some((
        g.separador_id.clone(),
        Rectangle::new(
            (izq.loc.x + izq.size.w, izq.loc.y).into(),
            (SEPARADOR, izq.size.h).into(),
        ),
    ))
}

pub fn admite_dos_paneles(state: &BookosComp, window: &Window) -> bool {
    indice(state, window).is_some_and(|i| state.multitarea.grupos[i].ventanas.len() == 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambos_paneles_comparten_el_separador_sin_perder_pixeles() {
        let area = Rectangle::new((-1600, 32).into(), (1599, 900).into());
        for p in [0.0, 0.25, 0.5, 0.8, 1.0] {
            let [izq, der] = reparto(area, p, [300, 400]);
            assert_eq!(izq.size.w + SEPARADOR + der.size.w, area.size.w);
            assert_eq!(izq.loc.x + izq.size.w + SEPARADOR, der.loc.x);
            assert_eq!(der.loc.x + der.size.w, area.loc.x + area.size.w);
            assert!(izq.size.w >= 300 && der.size.w >= 400);
        }
    }

    #[test]
    fn una_pantalla_estrecha_conserva_dos_paneles_alcanzables() {
        let area = Rectangle::from_size((200, 600).into());
        let [izq, der] = reparto(area, -10.0, [400, 400]);
        assert!(izq.size.w > 0 && der.size.w > 0);
        assert_eq!(izq.size.w + SEPARADOR + der.size.w, area.size.w);
    }
}
