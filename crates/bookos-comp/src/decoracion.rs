//! La barra de título del escritorio: quién la lleva y qué hace al pulsarla.
//!
//! En Wayland la decoración es del cliente salvo que el compositor la reclame
//! por `xdg-decoration`, así que aquí solo se decoran las ventanas cuyo cliente
//! entiende ese protocolo y no exige dibujarla él. Con GTK —que no lo
//! implementa— se sigue viendo su propia barra, y forzarla desde fuera no es
//! posible: no hay protocolo para eso.
//!
//! ## La barra vive fuera de la ventana
//!
//! El `Space` guarda la posición del **cliente**, y la barra se dibuja en los
//! [`ALTO`] píxeles lógicos que hay justo encima. Se eligió así en vez de
//! agrandar la geometría de la ventana porque todo lo demás del compositor
//! —animaciones, encaje, minimizar hacia el dock, miniaturas— razona con la
//! geometría del cliente, y meterle un marco por dentro obligaba a acordarse de
//! restarlo en cada una de esas cuentas. Lo que sí hay que descontar es el sitio
//! que ocupa, y de eso se encarga [`area_util`].
//!
//! El dibujo está en `bookos-shell`, que es donde están el tema y el
//! rasterizador; aquí queda la decisión de quién la lleva y qué pasa al
//! pulsarla.

use std::cell::Cell;

use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle};

use bookos_shell::decoracion::Boton;

use crate::state::BookosComp;

/// Alto de la barra en píxeles lógicos, el que dibuja el shell.
pub const ALTO: i32 = bookos_shell::decoracion::ALTO as i32;

/// Lo que el compositor guarda por ventana decorada.
///
/// En el `UserDataMap` de la ventana, como el resto del estado de
/// [`crate::ventanas`]: muere con ella sin que nadie tenga que purgarlo.
#[derive(Debug)]
struct Datos {
    /// Clave del buffer de esta barra dentro del shell. Es un contador y no el
    /// puntero de la superficie porque el shell no conoce Wayland.
    id: u64,
    lleva: Cell<bool>,
    grupo: Cell<bool>,
}

/// Da a `window` una barra de título, o se la quita.
pub fn decorar(window: &Window, si: bool) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SIGUIENTE: AtomicU64 = AtomicU64::new(1);

    let datos = window.user_data();
    datos.insert_if_missing(|| Datos {
        id: SIGUIENTE.fetch_add(1, Ordering::Relaxed),
        lleva: Cell::new(false),
        grupo: Cell::new(false),
    });
    if let Some(d) = datos.get::<Datos>() {
        d.lleva.set(si);
    }
}

pub fn agrupar(window: &Window, si: bool) {
    if window.user_data().get::<Datos>().is_none() {
        decorar(window, false);
    }
    if let Some(d) = window.user_data().get::<Datos>() {
        d.grupo.set(si);
    }
}

/// ¿Se le dibuja barra a esta ventana ahora mismo?
///
/// A pantalla completa no: ahí la ventana ocupa todo y el escritorio desaparece
/// entero, barra incluida.
pub fn decorada(window: &Window) -> bool {
    window
        .user_data()
        .get::<Datos>()
        .is_some_and(|d| d.lleva.get() || d.grupo.get())
        && !crate::ventanas::completa(window)
}

/// La clave de su buffer en el shell.
pub fn id(window: &Window) -> Option<u64> {
    window.user_data().get::<Datos>().map(|d| d.id)
}

/// El rectángulo de la barra, en lógicos: los [`ALTO`] píxeles justo encima de
/// la ventana. `None` si esa ventana no lleva.
pub fn barra_rect(state: &BookosComp, window: &Window) -> Option<Rectangle<i32, Logical>> {
    if !decorada(window) {
        return None;
    }
    let loc = state.space.element_location(window)?;
    let ancho = window.geometry().size.w;
    if ancho <= 0 {
        return None;
    }
    Some(Rectangle::new(
        (loc.x, loc.y - ALTO).into(),
        (ancho, ALTO).into(),
    ))
}

/// La ventana cuya barra está bajo el punto, y qué botón, si alguno.
///
/// Se recorre de delante hacia atrás —`elements()` va de atrás a adelante— y se
/// **para en la primera ventana que cubra el punto**, sea con su barra o con su
/// cuerpo. Mirar solo las barras no bastaba: si el cuerpo de la de delante tapa
/// la barra de una de atrás, el clic caía en una barra que no se ve, y pulsar
/// dentro de la ventana con la que estás trabajando cerraba otra. Con el cuerpo
/// cortando el recorrido, lo que está tapado deja de ser alcanzable.
pub fn barra_en(state: &BookosComp, punto: Point<f64, Logical>) -> Option<(Window, Option<Boton>)> {
    let dentro = |r: Rectangle<i32, Logical>| {
        punto.x >= r.loc.x as f64
            && punto.x < (r.loc.x + r.size.w) as f64
            && punto.y >= r.loc.y as f64
            && punto.y < (r.loc.y + r.size.h) as f64
    };
    for window in state
        .space
        .elements()
        .rev()
        .filter(|w| crate::multitarea::visible(state, w))
    {
        if let Some(rect) = barra_rect(state, window)
            && dentro(rect)
        {
            let boton = bookos_shell::decoracion::boton_en(
                rect.size.w as f32,
                (punto.x - rect.loc.x as f64) as f32,
                (punto.y - rect.loc.y as f64) as f32,
            );
            return Some((window.clone(), boton));
        }
        // El cuerpo no devuelve barra, pero sí tapa: quien esté por detrás no
        // puede recibir este clic.
        if state.space.element_geometry(window).is_some_and(&dentro) {
            return None;
        }
    }
    None
}

/// El área donde cabe una ventana **con** su barra: la útil, sin la franja de
/// arriba que la barra necesita.
///
/// Sin esto, una ventana decorada colocada pegada al panel se dibujaba con la
/// barra por detrás de él, y con ella se iba el único sitio desde el que se
/// puede arrastrar.
pub fn area_util(state: &BookosComp, window: &Window) -> Rectangle<i32, Logical> {
    let area = state.work_area();
    if !decorada(window) {
        return area;
    }
    Rectangle::new(
        (area.loc.x, area.loc.y + ALTO).into(),
        (area.size.w, (area.size.h - ALTO).max(1)).into(),
    )
}

/// Lo que hace pulsar un botón. El clic se atiende al **soltar** y sobre el
/// mismo botón donde empezó, como cualquier botón del sistema: bajar el ratón
/// encima de la ✕ y salirse antes de soltar no cierra nada.
pub fn accionar(state: &mut BookosComp, window: &Window, boton: Boton) {
    match boton {
        Boton::Minimizar => crate::ventanas::minimizar(state, window.clone()),
        Boton::Maximizar => state.alternar_maximizada(window),
        Boton::Cerrar => {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            } else if let Some(x11) = window.x11_surface() {
                let _ = x11.close();
            }
        }
    }
    state.needs_redraw = true;
}

/// Lo que el shell necesita saber para dibujar la barra de esta ventana.
pub fn estado_de(state: &BookosComp, window: &Window) -> bookos_shell::decoracion::Estado {
    let activa = state.ventana_con_foco().is_some_and(|f| f == *window);
    let mio = |guardado: &Option<(u64, Boton)>| {
        guardado
            .filter(|(quien, _)| Some(*quien) == id(window))
            .map(|(_, boton)| boton)
    };
    let (pestanas, pestana_activa) = crate::multitarea::pestanas(state, window);
    bookos_shell::decoracion::Estado {
        pestanas,
        pestana_activa,
        titulo: crate::handlers::titulo(window).unwrap_or_default(),
        activa,
        maximizada: crate::ventanas::maximizada(window),
        señalado: mio(&state.decoracion.señalado),
        pulsado: mio(&state.decoracion.pulsado),
    }
}

/// El botón señalado y el que se está pulsando, que son de todo el escritorio y
/// no de una ventana: solo puede haber uno de cada.
#[derive(Debug, Default)]
pub struct Interaccion {
    pub señalado: Option<(u64, Boton)>,
    pub pulsado: Option<(u64, Boton)>,
}

/// Apunta qué botón hay bajo el cursor. `true` si cambió y hay que repintar.
pub fn señalar(state: &mut BookosComp, punto: Point<f64, Logical>) -> bool {
    let nuevo = barra_en(state, punto).and_then(|(w, boton)| Some((id(&w)?, boton?)));
    if state.decoracion.señalado == nuevo {
        return false;
    }
    state.decoracion.señalado = nuevo;
    true
}

/// Marca el botón que se acaba de pulsar.
pub fn pulsar(state: &mut BookosComp, window: &Window, boton: Boton) {
    if let Some(id) = id(window) {
        state.decoracion.pulsado = Some((id, boton));
        state.needs_redraw = true;
    }
}

/// Suelta el botón pulsado y dice cuál era, si el cursor sigue sobre él.
pub fn soltar(state: &mut BookosComp, punto: Point<f64, Logical>) -> Option<(Window, Boton)> {
    let (quien, boton) = state.decoracion.pulsado.take()?;
    state.needs_redraw = true;
    let (window, encima) = barra_en(state, punto)?;
    (id(&window) == Some(quien) && encima == Some(boton)).then_some((window, boton))
}

/// Las claves de las barras que siguen vivas, para que el shell tire los
/// buffers de las ventanas que ya no están.
pub fn vivas(state: &BookosComp) -> Vec<u64> {
    state
        .space
        .elements()
        .chain(state.minimizadas.iter().map(|(w, _)| w))
        .chain(state.escritorios.apartadas())
        .filter_map(id)
        // La barra de una ventana que se está cerrando se sigue pintando
        // mientras dura la animación: sin esto desaparecía de golpe y el
        // cuerpo se desvanecía solo.
        .chain(
            state
                .cierres
                .iter()
                .filter_map(|c| c.barra.as_ref().map(|b| b.id)),
        )
        .collect()
}
