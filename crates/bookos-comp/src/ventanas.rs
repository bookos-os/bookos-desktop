//! Colocación, foco y animación de las ventanas.
//!
//! Hasta aquí el compositor mapeaba cada ventana en `(0, alto del panel)` con el
//! tamaño del área útil y el estado `Maximized`. Eso hacía que **todas** las
//! ventanas cayeran exactamente en el mismo sitio, unas encima de otras: abrir
//! dos terminales daba la sensación de que la segunda había sustituido a la
//! primera. Aquí las ventanas pasan a ser flotantes, con el tamaño que pide el
//! cliente, centradas en el área útil y en cascada si ya hay alguna donde iban.
//!
//! ## Por qué el estado vive en la ventana y no en un mapa aparte
//!
//! `Window` lleva un `UserDataMap` propio, así que el estado muere con ella sin
//! que nadie tenga que acordarse de limpiarlo. Un `HashMap` paralelo en el
//! compositor tendría que purgarse en `toplevel_destroyed`, y ese es justo el
//! sitio donde se olvida y queda una fuga que solo se nota tras horas de sesión.
//!
//! ## Minimizar sí se anima, cerrar no
//!
//! Cuando llega `toplevel_destroyed` la `wl_surface` ya no existe y con ella se
//! ha ido el buffer: no queda nada que dibujar, así que una animación de cierre
//! exige capturar la ventana a una textura *antes* de que el cliente la suelte.
//! Eso es trabajo del renderer (un paso a framebuffer offscreen por ventana) y
//! no se hace aquí. Las ventanas entran animadas y desaparecen de golpe, y es
//! preferible eso a fingir una animación con un rectángulo vacío.
//!
//! Minimizar es otra cosa: la ventana **sigue viva** con su buffer, solo deja
//! de estar en el `Space`. Por eso sí se puede animar de verdad —se encoge
//! hacia su icono del dock— y por eso el desmapeo espera a que la animación
//! termine.

use std::cell::Cell;
use std::time::{Duration, Instant};

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Scale, Size};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;

use bookos_shell::tema;

use crate::state::BookosComp;

// Las dos parejas duración/curva salen de la tabla de movimiento del sistema de
// diseño (§2.7), y son dos y no una porque el sistema las empareja así:
//
// - El **tamaño** de una ventana que aparece es el "pop de modal": 250 ms con
//   muelle. Se pasa de su tamaño un 3 % y vuelve, que es lo que separa
//   "aparece" de "se despliega". Antes esto era `1-(1-t)⁵` a 260 ms, que se
//   parece a la curva de entrada pero no sabe rebotar.
// - La **opacidad** es el fadeIn de tarjeta: 220 ms sin rebote. Va aparte
//   porque un muelle en el alfa lo llevaría por encima de 1 y habría que
//   recortarlo, o sea gastar la parte interesante de la curva en nada. Que
//   cierre antes que el tamaño además ayuda: la ventana se lee completa
//   mientras todavía está asentándose.
const ENTRADA_ALFA: Duration = tema::D_TARJETA;
const ENTRADA_ZOOM: Duration = tema::D_MODAL;

/// Lo que tarda una ventana en llegar a su sitio cuando el compositor la mueve
/// o la cambia de tamaño: maximizar, restaurar, recolocar al cambiar la
/// pantalla. Es la "transición de página completa" del sistema, que es
/// exactamente lo que pasa en pantalla.
///
/// No cubre el arrastre con el ratón: ahí la ventana tiene que ir pegada al
/// cursor, y suavizarla se siente como arrastrar algo con retardo.
const MOVIMIENTO: Duration = tema::D_VENTANA;

/// De qué tamaño arranca una ventana al aparecer.
///
/// 0,92 y no algo más pequeño porque a partir de ahí el texto del cliente se ve
/// escalado y borroso durante la animación: el buffer es del tamaño final y lo
/// estamos encogiendo, no re-rasterizando.
const ZOOM_INICIAL: f64 = 0.92;

/// Desplazamiento de la cascada cuando una ventana nueva caería justo encima de
/// otra ya colocada.
const CASCADA: i32 = 32;

/// Estado que el compositor guarda por ventana.
///
/// Todo son `Cell` porque se consulta desde `&Window` —el `UserDataMap` solo
/// presta referencias compartidas— y el compositor es de un solo hilo, así que
/// no hace falta nada más caro.
#[derive(Debug, Default)]
pub struct Estado {
    encima: Cell<bool>,
    /// Cuándo apareció. `None` mientras no se ha colocado todavía: una ventana
    /// sin tamaño aún no puede centrarse ni empezar a animarse.
    nacida: Cell<Option<Instant>>,
    /// De dónde viene un movimiento en curso, y cuándo empezó.
    desde: Cell<Option<(Rectangle<i32, Logical>, Instant)>>,
    /// Dónde estaba antes de encajarse, para poder volver.
    restaurar: Cell<Option<Rectangle<i32, Logical>>>,
    /// En qué zona está encajada, si lo está.
    zona: Cell<Option<Zona>>,
    /// Cuándo se enfocó por última vez, para el orden del conmutador.
    ///
    /// No vale el orden del `Space`: `escritorios::animar` desmapea y vuelve a
    /// mapear las ventanas al cambiar de escritorio, y eso las manda al final
    /// como si acabaran de usarse. Con un sello propio el orden sobrevive a
    /// cualquier recolocación.
    enfocada: Cell<Option<Instant>>,
    /// El encogido hacia el dock —o el crecimiento desde él— en marcha.
    ///
    /// Guarda a qué rectángulo va (el icono del dock), desde qué geometría
    /// salió y cuándo empezó. Es lo que dibuja el minimizar: la ventana sigue
    /// viva y con su buffer mientras dura, y solo al terminar se saca del
    /// `Space`.
    encogido: Cell<Option<Encogido>>,
    /// Está a pantalla completa: tapa también el panel y el dock.
    ///
    /// Va aparte de `zona` y no como una `Zona` más porque no es una forma de
    /// repartir el área útil, sino de dejar de respetarla: mientras esté
    /// puesto, el shell no se dibuja.
    completa: Cell<bool>,
    /// El tamaño con que se miraron las barras por última vez. Ver
    /// [`tamano_nuevo`].
    tamano_visto: Cell<Size<i32, Logical>>,
}

/// ¿Ha cambiado de tamaño desde la última vez que se preguntó?
///
/// Maximizar le **pide** el tamaño al cliente y `revisar_barras` se llamaba en
/// ese momento, con la ventana todavía en el tamaño de antes: sin llegar al
/// dock, que se quedaba a la vista encima de ella. El tamaño de verdad llega en
/// un commit posterior, y ahí es donde hay que volver a mirar. Solo cuando
/// cambia: un vídeo hace sesenta commits por segundo sin moverse.
pub fn tamano_nuevo(window: &Window) -> bool {
    let tamano = window.geometry().size;
    estado(window).tamano_visto.replace(tamano) != tamano
}

/// Un minimizar o un restaurar a medio camino.
#[derive(Debug, Clone, Copy)]
pub struct Encogido {
    /// La geometría de la que sale la ventana.
    pub origen: Rectangle<i32, Logical>,
    /// El icono del dock al que va —o del que viene—.
    pub destino: Rectangle<i32, Logical>,
    pub desde: Instant,
    /// `true` minimiza y `false` restaura, que es la misma animación al revés.
    pub hacia_el_dock: bool,
}

/// Lo que tarda una ventana en irse al dock o en volver.
///
/// La "transición de página completa" del sistema, la misma que el cambio de
/// escritorio: es un recorrido largo de un extremo a otro de la pantalla, y con
/// los 250 ms del pop de modal la ventana llegaba antes de que el ojo la
/// siguiera.
const ENCOGIDO: Duration = tema::D_PAGINA;

/// Empieza el encogido hacia el dock, o el crecimiento de vuelta.
pub fn encoger(
    window: &Window,
    origen: Rectangle<i32, Logical>,
    destino: Rectangle<i32, Logical>,
    hacia_el_dock: bool,
) {
    estado(window).encogido.set(Some(Encogido {
        origen,
        destino,
        desde: Instant::now(),
        hacia_el_dock,
    }));
}

/// El encogido en marcha, o `None` si ya terminó.
pub fn encogido(window: &Window) -> Option<Encogido> {
    let e = estado(window).encogido.get()?;
    // Con los efectos reducidos el minimizar no tiene recorrido: la ventana se
    // va al dock en el mismo fotograma. Devolver `None` es además lo que hace
    // que `animar_minimizados` la desmapee ya, sin dejarla colgando.
    if tema::efectos_reducidos() || e.desde.elapsed() >= ENCOGIDO {
        estado(window).encogido.set(None);
        return None;
    }
    Some(e)
}

/// Cuánto lleva recorrido el encogido, de 0 (en su sitio) a 1 (en el dock).
///
/// Va con la curva de entrada y no lineal, que es la misma que usa el resto del
/// recorrido: el shader del genio recibe esto y no el tiempo crudo.
pub fn progreso_encogido(e: Encogido) -> f32 {
    avance_encogido(tema::avance(e.desde.elapsed(), ENCOGIDO), e.hacia_el_dock)
}

fn avance_encogido(t: f32, hacia_el_dock: bool) -> f32 {
    tema::C_ENTRADA.eval(if hacia_el_dock { t } else { 1.0 - t })
}

/// Dónde y a qué tamaño se dibuja una ventana que va —o viene— del dock.
///
/// Devuelve la esquina en lógicos, el factor de escala y el alfa. La escala sale
/// del **ancho**: encoger a la vez por los dos ejes con proporciones distintas
/// deforma el buffer, y lo que se busca es que la ventana se vaya achicando
/// hacia su icono, no que se aplaste.
pub fn encogido_ahora(e: Encogido) -> (Point<f64, Logical>, f64, f32) {
    // De ida el tiempo corre hacia delante y de vuelta hacia atrás: es la misma
    // curva recorrida al revés, y por eso no hay dos animaciones que mantener.
    let s = avance_encogido(tema::avance(e.desde.elapsed(), ENCOGIDO), e.hacia_el_dock) as f64;
    let escala = 1.0 + (e.destino.size.w as f64 / e.origen.size.w.max(1) as f64 - 1.0) * s;
    let centro_x = e.origen.loc.x as f64 + e.origen.size.w as f64 / 2.0;
    let centro_y = e.origen.loc.y as f64 + e.origen.size.h as f64 / 2.0;
    let destino_x = e.destino.loc.x as f64 + e.destino.size.w as f64 / 2.0;
    let destino_y = e.destino.loc.y as f64 + e.destino.size.h as f64 / 2.0;
    let x = centro_x + (destino_x - centro_x) * s - e.origen.size.w as f64 * escala / 2.0;
    let y = centro_y + (destino_y - centro_y) * s - e.origen.size.h as f64 * escala / 2.0;
    // El alfa se va al final, no linealmente: si se apaga desde el principio, la
    // ventana desaparece a mitad de camino y el recorrido no se ve.
    let alfa = (1.0 - s * s) as f32;
    ((x, y).into(), escala, alfa.clamp(0.0, 1.0))
}

/// Minimiza la ventana: la manda encogiéndose a su icono del dock.
///
/// El desmapeo **no** ocurre aquí sino cuando la animación termina, en
/// [`animar_minimizados`]: si se sacara ya del `Space` no habría nada que
/// dibujar y el encogido no se vería.
pub fn minimizar(state: &mut BookosComp, window: Window) {
    let familia = crate::multitarea::familia(state, &window);
    for w in familia {
        minimizar_una(state, w);
    }
    let siguiente = state
        .space
        .elements()
        .rev()
        .find(|w| {
            !state.minimizando.iter().any(|(m, _)| m == *w) && crate::multitarea::visible(state, w)
        })
        .cloned();
    if let Some(w) = siguiente {
        state.enfocar(&w);
    } else if let Some(kbd) = state.seat.get_keyboard() {
        kbd.set_focus(state, None, SERIAL_COUNTER.next_serial());
    }
}

fn minimizar_una(state: &mut BookosComp, window: Window) {
    // El botón puede recibir otro clic mientras la lámpara sigue en marcha.
    // No debemos arrancar una segunda animación ni guardar la misma ventana
    // dos veces: al terminar, eso dejaría una entrada fantasma en el dock.
    if state.minimizando.iter().any(|(w, _)| *w == window)
        || state.minimizadas.iter().any(|(w, _)| *w == window)
    {
        return;
    }
    let Some(origen) = state.space.element_geometry(&window) else {
        return;
    };
    let app_id = app_id_de_grupo(state, &window).unwrap_or_default();
    let destino = destino_dock(state, &app_id, origen);
    encoger(&window, origen, destino, true);
    state.minimizando.push((window, origen.loc));
    state.actualizar_dock();
    state.needs_redraw = true;
}

/// ¿Tiene esta aplicación alguna ventana minimizada?
pub fn hay_minimizada(state: &BookosComp, app_id: &str) -> bool {
    state.minimizadas.iter().any(|(w, _)| {
        crate::handlers::app_id(w).is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
    })
}

/// ¿Esta aplicación ya está recorriendo la lámpara hacia el dock?
///
/// Durante esos 280 ms la ventana aún pertenece al `Space`, así que buscarla
/// como una ventana abierta también la encuentra. Ignorar un segundo clic evita
/// insertar dos veces la misma ventana en `minimizando`.
pub fn esta_minimizando(state: &BookosComp, app_id: &str) -> bool {
    state.minimizando.iter().any(|(w, _)| {
        crate::handlers::app_id(w).is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
    })
}

/// Devuelve al escritorio la última ventana minimizada de una aplicación.
///
/// La última y no la primera porque el icono del dock se pulsa para recuperar
/// «lo que acabo de guardar», que es lo que espera cualquiera con tres ventanas
/// del mismo programa escondidas.
pub fn restaurar_minimizada(state: &mut BookosComp, app_id: &str) -> bool {
    let Some(i) = state.minimizadas.iter().rposition(|(w, _)| {
        crate::handlers::app_id(w).is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
    }) else {
        return false;
    };
    let window = state.minimizadas[i].0.clone();
    let familia: Vec<_> = state
        .minimizadas
        .iter()
        .filter(|(w, _)| crate::multitarea::misma_unidad(state, &window, w))
        .map(|(w, _)| w.clone())
        .collect();
    for w in familia {
        let Some(i) = state.minimizadas.iter().position(|(v, _)| *v == w) else {
            continue;
        };
        let (w, posicion) = state.minimizadas.remove(i);
        state.capturas.retain(|(v, _)| *v != w);
        state.space.map_element(w.clone(), posicion, true);
        let origen = Rectangle::new(posicion, w.geometry().size);
        let destino = destino_dock(state, app_id, origen);
        encoger(&w, origen, destino, false);
    }
    state.enfocar(&window);
    // Vuelve al `Space` sin pasar por mover ni mapear, que es donde se miran
    // las barras: una maximizada traída del dock dejaba el panel encima.
    state.revisar_barras();
    state.needs_redraw = true;
    true
}

/// A qué rectángulo se encoge una ventana de esta aplicación.
///
/// Su icono del dock si lo tiene, y si no el borde de abajo justo debajo de
/// ella: una ventana sin icono no puede irse a ninguna parte concreta, y bajarla
/// en vertical al menos dice hacia dónde se fue.
fn destino_dock(
    state: &BookosComp,
    app_id: &str,
    origen: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let icono = state.shell.as_ref().and_then(|s| s.icono_dock(app_id));
    match icono {
        Some((x, y, w, h)) => Rectangle::new(
            (x.round() as i32, y.round() as i32).into(),
            (w.round().max(1.0) as i32, h.round().max(1.0) as i32).into(),
        ),
        None => {
            let (_, alto) = state.pantalla_logica();
            Rectangle::new(
                (origen.loc.x + origen.size.w / 2 - 24, alto as i32).into(),
                (48, 48).into(),
            )
        }
    }
}

/// Termina los minimizados que han llegado al dock: los saca del `Space`.
///
/// Devuelve `true` mientras quede alguno moviéndose, que es lo que mantiene el
/// bucle dibujando hasta el final de la animación.
pub fn animar_minimizados(state: &mut BookosComp) -> bool {
    if state.minimizando.is_empty() {
        return false;
    }
    let mut sigue = false;
    let pendientes = std::mem::take(&mut state.minimizando);
    for (window, posicion) in pendientes {
        if encogido(&window).is_some() {
            state.minimizando.push((window, posicion));
            sigue = true;
            continue;
        }
        state.space.unmap_elem(&window);
        // La textura congelada ya no hace falta: son varios megas de GPU por
        // ventana y quedarse con ellos «por si vuelve» es una fuga con nombre
        // bonito. Al restaurar se captura otra vez, que cuesta un frame.
        state.capturas.retain(|(w, _)| *w != window);
        state.minimizadas.push((window, posicion));
    }
    // La ventana sigue abierta aunque ya no esté mapeada: el dock debe
    // enterarse de inmediato para conservar su indicador y permitir restaurarla.
    state.actualizar_dock();
    // Ya no está donde estaba: si era la que tapaba una barra, que vuelva.
    state.revisar_barras();
    state.needs_redraw = true;
    sigue
}

/// Cuándo se enfocó por última vez. `None` = nunca ha tenido el foco.
pub fn enfocada_en(window: &Window) -> Option<Instant> {
    estado(window).enfocada.get()
}

/// El estado de una ventana, creándolo la primera vez.
pub fn estado(window: &Window) -> &Estado {
    window.user_data().get_or_insert(Estado::default)
}

/// ¿Ya se colocó esta ventana? Mientras no, no se dibuja: no tiene sitio.
pub fn colocada(window: &Window) -> bool {
    estado(window).nacida.get().is_some()
}

/// Marca la ventana como colocada y arranca su animación de entrada.
pub fn nace(window: &Window) {
    estado(window).nacida.set(Some(Instant::now()));
}

/// Apunta que la ventana viene de `origen` y tiene que llegar animada a su
/// posición actual.
pub fn mover_desde(window: &Window, origen: Rectangle<i32, Logical>) {
    estado(window).desde.set(Some((origen, Instant::now())));
}

/// Progreso único para posición y tamaño durante un cambio de geometría.
///
/// Mantenerlo en una función evita que maximizar use una curva para la esquina
/// y otra para los bordes. Es una cubic-bezier sin rebote: el contenido del
/// cliente ya está rasterizado y cualquier sobrepaso se leería como borrosidad.
fn avance_movimiento(pasado: Duration) -> f64 {
    tema::C_VENTANA.eval(tema::avance(pasado, MOVIMIENTO)) as f64
}

/// Interpola las cuatro aristas de un rectángulo con una sola fracción.
fn interpolar_rect(
    origen: Rectangle<i32, Logical>,
    destino: Rectangle<i32, Logical>,
    s: f64,
) -> Rectangle<i32, Logical> {
    let lerp = |a: i32, b: i32| (a as f64 + (b - a) as f64 * s).round() as i32;
    Rectangle::new(
        (
            lerp(origen.loc.x, destino.loc.x),
            lerp(origen.loc.y, destino.loc.y),
        )
            .into(),
        (
            lerp(origen.size.w, destino.size.w).max(1),
            lerp(origen.size.h, destino.size.h).max(1),
        )
            .into(),
    )
}

/// Geometría que se está viendo ahora, aunque el `Space` ya guarde el destino.
/// Sirve para redirigir maximizar/restaurar sin saltar al extremo anterior.
fn geometria_visual(window: &Window, destino: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let Some((origen, t0)) = estado(window).desde.get() else {
        return destino;
    };
    if t0.elapsed() >= MOVIMIENTO {
        return destino;
    }
    interpolar_rect(origen, destino, avance_movimiento(t0.elapsed()))
}

/// Guarda —o consume— la geometría a la que hay que volver al desmaximizar.
pub fn guardar_restaurar(window: &Window, geo: Rectangle<i32, Logical>) {
    estado(window).restaurar.set(Some(geo));
}

pub fn tomar_restaurar(window: &Window) -> Option<Rectangle<i32, Logical>> {
    estado(window).restaurar.take()
}

/// ¿Sigue moviéndose algo de esta ventana?
///
/// Mira si la animación **está en marcha**, no si existe: preguntar por
/// `is_some()` deja al compositor dibujando para siempre, que es exactamente el
/// ciclo que ya se coló una vez en las emergentes del shell.
pub fn animando(window: &Window) -> bool {
    if tema::efectos_reducidos() {
        return false;
    }
    let estado = estado(window);
    estado
        .encogido
        .get()
        .is_some_and(|e| e.desde.elapsed() < ENCOGIDO)
        || estado
            .nacida
            .get()
            .is_some_and(|t| t.elapsed() < ENTRADA_ZOOM)
        || estado
            .desde
            .get()
            .is_some_and(|(_, t)| t.elapsed() < MOVIMIENTO)
}

/// Alfa y factor de tamaño con los que toca componer la ventana ahora mismo.
///
/// No se repinta nada: el cliente ya dibujó su buffer y lo único que cambia es
/// cómo lo compone la GPU.
pub fn animacion(window: &Window) -> (f32, f64) {
    let Some(nacida) = estado(window).nacida.get() else {
        // Sin colocar todavía: invisible. Devolver 1.0 la enseñaría un
        // fotograma en la esquina antes de saltar a su sitio.
        return (0.0, 1.0);
    };
    let t = nacida.elapsed();
    let alfa = tema::C_ENTRADA.eval(tema::avance(t, ENTRADA_ALFA));
    let zoom = tema::C_MUELLE.eval(tema::avance(t, ENTRADA_ZOOM));
    (alfa, ZOOM_INICIAL + (1.0 - ZOOM_INICIAL) * zoom as f64)
}

/// Dónde hay que dibujar la ventana, contando el movimiento en curso.
///
/// `destino` es dónde dice el `Space` que está; durante una animación se dibuja
/// en el camino entre el origen guardado y ese destino.
pub fn posicion(window: &Window, destino: Point<i32, Logical>) -> Point<f64, Logical> {
    let estado = estado(window);
    let Some((origen, t0)) = estado.desde.get() else {
        return destino.to_f64();
    };
    if t0.elapsed() >= MOVIMIENTO {
        // El recorrido acabado se borra en vez de dejarlo puesto: si no, cada
        // fotograma del resto de la vida de la ventana vuelve a interpolar algo
        // que ya terminó, y una segunda animación tendría que competir con la
        // vieja en vez de sustituirla.
        estado.desde.set(None);
        return destino.to_f64();
    }
    let s = avance_movimiento(t0.elapsed());
    (
        origen.loc.x as f64 + (destino.x - origen.loc.x) as f64 * s,
        origen.loc.y as f64 + (destino.y - origen.loc.y) as f64 * s,
    )
        .into()
}

/// Cuánto hay que escalar el buffer para que el cambio de tamaño se vea como
/// un crecimiento y no como un salto.
///
/// El cliente responde al `configure` cuando quiere: hasta entonces dibuja al
/// tamaño viejo y, en el frame en que responde, aparece de golpe con el nuevo.
/// Escalando el buffer desde la proporción vieja hasta 1 durante el mismo
/// recorrido que hace la posición, el salto se reparte.
///
/// Los dos ejes se interpolan por separado. Antes se usaba la media geométrica,
/// que convertía maximizar en un zoom uniforme: el ancho podía haber llegado
/// mientras el alto aún no, y la barra se despegaba visualmente del contenido.
pub fn escala_resize(window: &Window, actual: Size<i32, Logical>) -> Scale<f64> {
    let Some((origen, t0)) = estado(window).desde.get() else {
        return Scale::from(1.0);
    };
    let t = t0.elapsed();
    if t >= MOVIMIENTO || actual.w <= 0 || actual.h <= 0 {
        return Scale::from(1.0);
    }
    if origen.size.w <= 0 || origen.size.h <= 0 || origen.size == actual {
        return Scale::from(1.0);
    }
    let intermedio = interpolar_rect(
        origen,
        Rectangle::new(origen.loc, actual),
        avance_movimiento(t),
    );
    Scale::from((
        intermedio.size.w as f64 / actual.w as f64,
        intermedio.size.h as f64 / actual.h as f64,
    ))
}

/// Tamaño mínimo al que se deja encoger una ventana arrastrando.
///
/// Sin tope, un tirón rápido la deja en 0×0 y ya no hay dónde agarrarla para
/// devolverle el tamaño.
const MINIMO: i32 = 120;

/// Una ventana que el usuario está moviendo o redimensionando con el ratón.
///
/// No se usa `PointerGrab` de Smithay a propósito. El grab es la vía completa
/// —maneja el foco, el toque, los ejes— pero son dos implementaciones de trait
/// de un par de cientos de líneas para algo que aquí se resuelve descartando el
/// evento antes de reenviarlo. Si algún día hace falta arrastrar con el táctil
/// o mantener el grab a través de un cambio de foco, ese es el momento de
/// cambiarlo, no antes.
#[derive(Debug, Clone)]
pub struct Arrastre {
    pub window: Window,
    pub modo: Modo,
    /// Dónde estaba el puntero al empezar, y qué geometría tenía la ventana
    /// entonces. Se guarda el punto de partida en vez de ir acumulando deltas
    /// porque acumular arrastra el error de redondeo: la ventana se va
    /// separando del cursor cuanto más rato la muevas.
    pub inicio: Point<f64, Logical>,
    pub geo: Rectangle<i32, Logical>,
    /// En qué zona encajaría si se soltase ahora. Se recalcula en cada
    /// movimiento y es lo que dibuja la vista previa.
    pub zona: Option<Zona>,
    /// De qué rectángulo viene la vista previa y cuándo empezó a moverse, para
    /// que al pasar de media pantalla a un cuarto se deslice en vez de saltar.
    pub previa_desde: Option<(Rectangle<i32, Logical>, Instant)>,
    /// Cuándo apareció la vista previa, para el fundido de entrada.
    pub previa_nacida: Option<Instant>,
    /// Si el cliente llegó a ver la pulsación que empezó esto.
    ///
    /// Con Meta+ratón el compositor se queda el clic entero y el cliente no se
    /// entera de nada. Pero cuando el arrastre lo pide él —`move_request` al
    /// tirar de su propia barra de título— la pulsación ya se la reenviamos, y
    /// entonces **hay que reenviarle también el soltar**: sin él se queda
    /// creyendo que sigues pulsando y no vuelve a pedir otro movimiento. Ese
    /// era el "a veces no agarra".
    pub del_cliente: bool,
}

/// Cuánto de una ventana tiene que quedar dentro del área útil.
///
/// Se puede sacar una ventana por los lados y por abajo —es cómodo para
/// apartarla— pero nunca del todo: sin este mínimo, un tirón la deja fuera de
/// la pantalla y ya no hay forma de traerla de vuelta con el ratón.
const VISIBLE: i32 = 96;

/// Recorta una posición para que la ventana siga siendo alcanzable.
///
/// El techo es duro: **nunca** por encima del área útil. Una ventana que sube
/// por detrás del panel esconde su barra de título, y con ella el único sitio
/// del que se la puede agarrar — se queda ahí para siempre. Por los lados y por
/// abajo sí se la deja salir, que es cómodo para apartarla, dejando `VISIBLE`
/// dentro para poder traerla de vuelta.
fn confinar(
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let asomo = VISIBLE.min(size.w);
    (
        loc.x.clamp(
            area.loc.x - (size.w - asomo).max(0),
            area.loc.x + area.size.w - asomo,
        ),
        loc.y
            .clamp(area.loc.y, area.loc.y + area.size.h - VISIBLE.min(size.h)),
    )
        .into()
}

/// Qué se está haciendo con la ventana arrastrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Mover,
    /// Redimensionar tirando de un lado. Los dos `bool` son "el borde que se
    /// mueve es el izquierdo / el de arriba": tirar de la izquierda cambia la
    /// posición además del tamaño, y tirar de la derecha no.
    Redimensionar {
        izquierda: bool,
        arriba: bool,
    },
}

impl Modo {
    /// De qué borde se está tirando, según en qué cuadrante de la ventana
    /// empezó el arrastre. Es la regla de KWin y de i3: no hace falta acertarle
    /// a un borde de tres píxeles, basta con estar en la mitad correcta.
    pub fn por_cuadrante(punto: Point<f64, Logical>, geo: Rectangle<i32, Logical>) -> Modo {
        Modo::Redimensionar {
            izquierda: punto.x < geo.loc.x as f64 + geo.size.w as f64 / 2.0,
            arriba: punto.y < geo.loc.y as f64 + geo.size.h as f64 / 2.0,
        }
    }
}

impl Arrastre {
    /// La geometría que le toca a la ventana con el puntero en `puntero`.
    pub fn geometria(
        &self,
        puntero: Point<f64, Logical>,
        area: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        geometria_de(self.modo, self.inicio, self.geo, puntero, area)
    }
}

/// El cálculo del arrastre, sin la `Window` de por medio.
///
/// Va suelto para poder probarlo: fabricar una `Window` en un test exige un
/// cliente Wayland vivo, y esta es justamente la parte que ya se equivocó una
/// vez —el borde de arriba se metía bajo el panel.
fn geometria_de(
    modo: Modo,
    inicio: Point<f64, Logical>,
    geo: Rectangle<i32, Logical>,
    puntero: Point<f64, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let dx = (puntero.x - inicio.x).round() as i32;
    let dy = (puntero.y - inicio.y).round() as i32;
    match modo {
        Modo::Mover => {
            let destino = (geo.loc.x + dx, geo.loc.y + dy).into();
            Rectangle::new(confinar(destino, geo.size, area), geo.size)
        }
        Modo::Redimensionar { izquierda, arriba } => {
            // Al tirar del borde izquierdo la anchura crece hacia atrás, así
            // que el desplazamiento entra con signo cambiado y además hay
            // que mover el origen.
            let w = if izquierda {
                (geo.size.w - dx).max(MINIMO)
            } else {
                (geo.size.w + dx).max(MINIMO)
            };
            let h = if arriba {
                (geo.size.h - dy).max(MINIMO)
            } else {
                (geo.size.h + dy).max(MINIMO)
            };
            // El origen se recalcula desde el borde **opuesto**, que es el
            // que no se mueve. Sumar `dx` sin más deja la ventana temblando
            // cuando el tamaño topa con el mínimo: el borde fijo se movía.
            let x = if izquierda {
                geo.loc.x + geo.size.w - w
            } else {
                geo.loc.x
            };
            let y = if arriba {
                geo.loc.y + geo.size.h - h
            } else {
                geo.loc.y
            };
            // Tirando del borde de arriba, el techo es el mismo que al
            // mover: el área útil. Sin esto la ventana seguía subiendo y se
            // metía **debajo del panel**, que se dibuja siempre encima —
            // arrastrando de un borde se conseguía justo lo que `confinar`
            // impide al mover. Se recorta el alto en vez de la posición
            // para que el borde de abajo, que es el que no se toca, no se
            // mueva.
            let (y, h) = if y < area.loc.y {
                (area.loc.y, (h - (area.loc.y - y)).max(MINIMO))
            } else {
                (y, h)
            };
            Rectangle::new((x, y).into(), (w, h).into())
        }
    }
}

/// Ancho de la franja de borde que dispara el encaje.
///
/// 12 px lógicos, el mismo que usa KWin. Más estrecho obliga a apuntar; más
/// ancho hace que una ventana que solo pasaba cerca del borde salte sola.
const BORDE: i32 = 12;

/// Qué parte del borde toca el cursor decide en cuánto de la pantalla encaja la
/// ventana al soltarla.
///
/// Es el encaje de KWin y el Snap de Windows: los laterales dan media pantalla,
/// las esquinas un cuarto —de ahí salen las cuatro ventanas a la vez— y el
/// borde de arriba maximiza. El borde de abajo **no** hace nada a propósito:
/// ahí está el dock, y encajar al acercarse a él haría imposible arrastrar una
/// ventana hasta cerca del dock sin que saltara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zona {
    Maxima,
    Izquierda,
    Derecha,
    SupIzq,
    SupDer,
    InfIzq,
    InfDer,
}

impl Zona {
    /// El rectángulo que ocupará la ventana dentro del área útil.
    ///
    /// Las mitades se reparten con `w - w/2` en la de la derecha para que dos
    /// ventanas encajadas no dejen una raya de fondo entre ellas cuando el
    /// ancho es impar.
    pub fn rect(self, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let (x, y, w, h) = (area.loc.x, area.loc.y, area.size.w, area.size.h);
        let (mitad_w, resto_w) = (w / 2, w - w / 2);
        let (mitad_h, resto_h) = (h / 2, h - h / 2);
        let (loc, size) = match self {
            Zona::Maxima => ((x, y), (w, h)),
            Zona::Izquierda => ((x, y), (mitad_w, h)),
            Zona::Derecha => ((x + mitad_w, y), (resto_w, h)),
            Zona::SupIzq => ((x, y), (mitad_w, mitad_h)),
            Zona::SupDer => ((x + mitad_w, y), (resto_w, mitad_h)),
            Zona::InfIzq => ((x, y + mitad_h), (mitad_w, resto_h)),
            Zona::InfDer => ((x + mitad_w, y + mitad_h), (resto_w, resto_h)),
        };
        Rectangle::new(loc.into(), size.into())
    }
}

/// Grosor del marco de la vista previa, en píxeles lógicos.
const MARCO: i32 = 2;

impl Arrastre {
    /// El rectángulo que enseña la vista previa ahora mismo, deslizándose si
    /// viene de otra zona.
    fn previa_rect(&self, area: Rectangle<i32, Logical>) -> Option<Rectangle<i32, Logical>> {
        let destino = self.zona?.rect(area);
        let Some((origen, t0)) = self.previa_desde else {
            return Some(destino);
        };
        let s = tema::C_ENTRADA.eval(tema::avance(t0.elapsed(), tema::D_POPOVER));
        let mezcla = |a: i32, b: i32| a + ((b - a) as f32 * s).round() as i32;
        Some(Rectangle::new(
            (
                mezcla(origen.loc.x, destino.loc.x),
                mezcla(origen.loc.y, destino.loc.y),
            )
                .into(),
            (
                mezcla(origen.size.w, destino.size.w),
                mezcla(origen.size.h, destino.size.h),
            )
                .into(),
        ))
    }

    /// ¿Sigue moviéndose la vista previa?
    pub fn animando(&self) -> bool {
        if tema::efectos_reducidos() {
            return false;
        }
        self.previa_nacida
            .is_some_and(|t| t.elapsed() < tema::D_TARJETA)
            || self
                .previa_desde
                .is_some_and(|(_, t)| t.elapsed() < tema::D_POPOVER)
    }

    /// Los rectángulos de color de la vista previa: el relleno y los cuatro
    /// lados del marco.
    ///
    /// Se dibuja con rectángulos sólidos y no con un buffer del shell porque
    /// esto vive **entre** las ventanas y el shell, cambia en cada fotograma
    /// del arrastre y no lleva ni texto ni esquinas redondeadas: rasterizarlo
    /// en CPU sería pagar un repintado por movimiento del ratón para dibujar
    /// cinco rectángulos que la GPU compone gratis.
    pub fn previa(
        &self,
        area: Rectangle<i32, Logical>,
        escala: f64,
    ) -> Vec<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
        use smithay::backend::renderer::element::{Id, Kind};

        let Some(rect) = self.previa_rect(area) else {
            return Vec::new();
        };
        // Un identificador estable por rectángulo: el damage tracker compara
        // elementos por id, y uno nuevo en cada fotograma le diría que ha
        // cambiado la pantalla entera.
        static IDS: std::sync::OnceLock<[Id; 5]> = std::sync::OnceLock::new();
        let ids = IDS.get_or_init(|| std::array::from_fn(|_| Id::new()));

        let alfa = self
            .previa_nacida
            .map(|t| tema::C_ENTRADA.eval(tema::avance(t.elapsed(), tema::D_TARJETA)))
            .unwrap_or(1.0);

        let fisico = |r: Rectangle<i32, Logical>| {
            Rectangle::<i32, smithay::utils::Physical>::new(
                (
                    (r.loc.x as f64 * escala).round() as i32,
                    (r.loc.y as f64 * escala).round() as i32,
                )
                    .into(),
                (
                    (r.size.w as f64 * escala).round() as i32,
                    (r.size.h as f64 * escala).round() as i32,
                )
                    .into(),
            )
        };
        // Premultiplicado, que es lo que espera el renderer.
        let acento = tema::acento();
        let color = |a: f32| {
            let a = a * alfa;
            [acento.r * a, acento.g * a, acento.b * a, a]
        };

        let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        let lados = [
            Rectangle::new((x, y).into(), (w, MARCO).into()),
            Rectangle::new((x, y + h - MARCO).into(), (w, MARCO).into()),
            Rectangle::new((x, y).into(), (MARCO, h).into()),
            Rectangle::new((x + w - MARCO, y).into(), (MARCO, h).into()),
        ];

        std::iter::once((rect, color(0.22)))
            .chain(lados.into_iter().map(|l| (l, color(0.9))))
            .zip(ids)
            .map(|((r, color), id)| {
                SolidColorRenderElement::new(id.clone(), fisico(r), 0, color, Kind::Unspecified)
            })
            .collect()
    }
}

/// Hacia dónde empuja una de las cuatro flechas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direccion {
    Izquierda,
    Derecha,
    Arriba,
    Abajo,
}

/// En qué zona queda la ventana al empujarla con una flecha.
///
/// `None` significa "suelta, a su tamaño flotante". La tabla es la de KDE y la
/// de Windows: la primera flecha manda a media pantalla y la perpendicular
/// parte esa mitad en un cuarto, así que Meta+Izquierda y luego Meta+Arriba
/// dejan la ventana arriba a la izquierda. Volver sobre los pasos deshace.
pub fn empujar(actual: Option<Zona>, hacia: Direccion) -> Option<Zona> {
    use Direccion as D;
    use Zona::*;
    match (hacia, actual) {
        // Arriba: maximiza si estaba suelta, sube la mitad a un cuarto y
        // devuelve un cuarto de abajo a su mitad.
        (D::Arriba, None) => Some(Maxima),
        (D::Arriba, Some(Izquierda)) => Some(SupIzq),
        (D::Arriba, Some(Derecha)) => Some(SupDer),
        (D::Arriba, Some(InfIzq)) => Some(Izquierda),
        (D::Arriba, Some(InfDer)) => Some(Derecha),
        (D::Arriba, otra) => otra,

        // Abajo deshace lo que hizo arriba, y suelta lo que ya estaba abajo.
        (D::Abajo, Some(Maxima)) => None,
        (D::Abajo, Some(Izquierda)) => Some(InfIzq),
        (D::Abajo, Some(Derecha)) => Some(InfDer),
        (D::Abajo, Some(SupIzq)) => Some(Izquierda),
        (D::Abajo, Some(SupDer)) => Some(Derecha),
        (D::Abajo, _) => None,

        // A los lados: la mitad contraria pasa por el centro. Que Meta+Derecha
        // desde la mitad izquierda suelte la ventana en vez de cruzarla de un
        // salto es lo que permite deshacer sin pensar.
        (D::Izquierda, None | Some(Maxima)) => Some(Izquierda),
        (D::Izquierda, Some(Derecha)) => None,
        (D::Izquierda, Some(SupDer)) => Some(SupIzq),
        (D::Izquierda, Some(InfDer)) => Some(InfIzq),
        (D::Izquierda, otra) => otra,

        (D::Derecha, None | Some(Maxima)) => Some(Derecha),
        (D::Derecha, Some(Izquierda)) => None,
        (D::Derecha, Some(SupIzq)) => Some(SupDer),
        (D::Derecha, Some(InfIzq)) => Some(InfDer),
        (D::Derecha, otra) => otra,
    }
}

/// En qué zona de encaje cae el cursor, si cae en alguna.
///
/// Se mira el **cursor** y no el borde de la ventana, igual que KWin: si se
/// mirara la ventana, arrastrar una que ya ocupa media pantalla dispararía el
/// encaje sin haberla movido.
pub fn zona_en(punto: Point<f64, Logical>, area: Rectangle<i32, Logical>) -> Option<Zona> {
    let (x, y) = (punto.x as i32, punto.y as i32);
    let izquierda = x <= area.loc.x + BORDE;
    let derecha = x >= area.loc.x + area.size.w - BORDE;
    let arriba = y <= area.loc.y + BORDE;

    // Las esquinas ganan a los lados: la franja de esquina es la cuarta parte
    // del alto, que es lo bastante generosa para acertarle sin apuntar y lo
    // bastante corta para que el centro del lateral siga dando media pantalla.
    let esquina = area.size.h / 4;
    let cerca_arriba = y <= area.loc.y + esquina;
    let cerca_abajo = y >= area.loc.y + area.size.h - esquina;

    match (izquierda, derecha, arriba) {
        // El borde de arriba maximiza, pero solo en su tramo central: las
        // esquinas de arriba son cuartos, no maximizar.
        (false, false, true) => Some(Zona::Maxima),
        (true, _, _) if cerca_arriba => Some(Zona::SupIzq),
        (true, _, _) if cerca_abajo => Some(Zona::InfIzq),
        (true, _, _) => Some(Zona::Izquierda),
        (_, true, _) if cerca_arriba => Some(Zona::SupDer),
        (_, true, _) if cerca_abajo => Some(Zona::InfDer),
        (_, true, _) => Some(Zona::Derecha),
        _ => None,
    }
}

/// Dónde colocar una ventana de tamaño `tam` dentro del área útil.
///
/// Centrada, y desplazada en cascada mientras su esquina caiga sobre la de una
/// ventana ya colocada. Se compara la esquina y no el solape entero porque dos
/// terminales del mismo tamaño se tapan por completo: lo que molesta no es que
/// se solapen, es que no se distingan.
pub fn hueco(
    tam: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
    ocupadas: &[Point<i32, Logical>],
) -> Point<i32, Logical> {
    let centrada = Point::<i32, Logical>::from((
        area.loc.x + (area.size.w - tam.w).max(0) / 2,
        area.loc.y + (area.size.h - tam.h).max(0) / 2,
    ));

    let mut sitio = centrada;
    // El tope existe para que una ventana no acabe fuera de la pantalla tras
    // abrir veinte terminales: pasado ese punto se vuelve al centro y se aceptan
    // las dos superpuestas, que es menos malo que una ventana inalcanzable.
    for _ in 0..8 {
        if !ocupadas.contains(&sitio) {
            return sitio;
        }
        sitio.x += CASCADA;
        sitio.y += CASCADA;
        if sitio.x + tam.w > area.loc.x + area.size.w || sitio.y + tam.h > area.loc.y + area.size.h
        {
            return centrada;
        }
    }
    centrada
}

/// Centra un diálogo sobre su ventana, sin salirse del área útil.
///
/// Si no cabe, manda la esquina de arriba a la izquierda: es donde suelen estar
/// el título y los campos, y un diálogo con la cabecera bajo el panel no se
/// puede ni mover.
pub(crate) fn encima_de(
    tam: Size<i32, Logical>,
    sobre: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let x = sobre.loc.x + (sobre.size.w - tam.w) / 2;
    let y = sobre.loc.y + (sobre.size.h - tam.h) / 2;
    (
        x.min(area.loc.x + area.size.w - tam.w).max(area.loc.x),
        y.min(area.loc.y + area.size.h - tam.h).max(area.loc.y),
    )
        .into()
}

/// De quién depende una ventana que no es la principal de su programa.
pub enum Duena {
    /// Diálogo con padre declarado (`xdg_toplevel.parent`, `WM_TRANSIENT_FOR`)
    /// o de tipo diálogo en X11. Se abre encima de ella.
    Padre(Window),
    /// Otra ventana del mismo proceso. Se coloca como cualquier ventana, pero
    /// es del mismo programa.
    Proceso(Window),
}

impl Duena {
    pub fn ventana(&self) -> &Window {
        match self {
            Duena::Padre(w) | Duena::Proceso(w) => w,
        }
    }
}

/// La ventana principal de la que cuelga esta, o `None` si la principal es ella.
///
/// Sin esto cada diálogo era una ventana más: salía del dock con el genio, se
/// ponía en el primer hueco libre lejos de su ventana y, si su `app_id` o su
/// `WM_CLASS` no coincidían, tenía icono propio en el dock. VirtualBox lo hacía
/// con cada diálogo de configuración.
///
/// Solo se agrupan ventanas del **mismo** proceso, no de procesos hijos: todo lo
/// que se lanza desde una terminal es hijo suyo, y acabaría colgando de ella.
pub fn duena(state: &BookosComp, window: &Window) -> Option<Duena> {
    if let Some(mut padre) = padre(state, window) {
        // Un diálogo abierto desde otro diálogo cuelga de la principal. El tope
        // es por si un cliente se declara padre de sí mismo en círculo.
        for _ in 0..8 {
            match self::padre(state, &padre) {
                Some(abuelo) if abuelo != *window => padre = abuelo,
                _ => break,
            }
        }
        return Some(Duena::Padre(padre));
    }
    let pid = pid_de(state, window)?;
    // La más antigua del proceso es la principal: un programa abre primero su
    // ventana y luego sus diálogos, no al revés. Solo cuentan las nacidas
    // antes; si no, la principal pasaría a depender de su propio diálogo.
    let mia = estado(window).nacida.get();
    let principal = state
        .space
        .elements()
        .chain(state.minimizadas.iter().map(|(w, _)| w))
        .chain(state.escritorios.apartadas())
        .filter(|w| *w != window && colocada(w) && padre(state, w).is_none())
        .filter(|w| mia.is_none_or(|m| estado(w).nacida.get() < Some(m)))
        .filter(|w| pid_de(state, w) == Some(pid))
        .min_by_key(|w| estado(w).nacida.get())?
        .clone();
    use smithay::xwayland::xwm::WmWindowType as T;
    let es_dialogo = window
        .x11_surface()
        .is_some_and(|x| matches!(x.window_type(), Some(T::Dialog | T::Utility)));
    Some(if es_dialogo {
        Duena::Padre(principal)
    } else {
        Duena::Proceso(principal)
    })
}

/// El `app_id` con el que la ventana cuenta en el dock: el de su principal.
pub fn app_id_de_grupo(state: &BookosComp, window: &Window) -> Option<String> {
    duena(state, window)
        .and_then(|d| crate::handlers::app_id(d.ventana()))
        .or_else(|| crate::handlers::app_id(window))
}

fn padre(state: &BookosComp, window: &Window) -> Option<Window> {
    let todas = || {
        state
            .space
            .elements()
            .chain(state.minimizadas.iter().map(|(w, _)| w))
            .chain(state.escritorios.apartadas())
    };
    if let Some(x11) = window.x11_surface() {
        let id = x11.is_transient_for()?;
        return todas()
            .find(|w| w.x11_surface().is_some_and(|s| s.window_id() == id))
            .cloned();
    }
    let superficie = window.toplevel()?.parent()?;
    todas()
        .find(|w| w.wl_surface().is_some_and(|s| *s == superficie))
        .cloned()
}

/// El proceso que dibuja la ventana.
///
/// En X11 sale de `_NET_WM_PID` y no de las credenciales del cliente Wayland:
/// todas las ventanas X11 llegan por la misma conexión, la de Xwayland, y con su
/// pid todos los programas X11 serían uno.
fn pid_de(state: &BookosComp, window: &Window) -> Option<u32> {
    use smithay::reexports::wayland_server::Resource as _;
    if let Some(x11) = window.x11_surface() {
        return x11.pid();
    }
    let credenciales = window
        .wl_surface()?
        .client()?
        .get_credentials(&state.display_handle)
        .ok()?;
    u32::try_from(credenciales.pid).ok()
}

/// Lo que el compositor sabe hacer con sus ventanas.
///
/// Va en un `impl` aparte —y no en `handlers.rs`— porque nada de esto responde
/// a un mensaje de Wayland: son las operaciones del escritorio, y los handlers
/// del protocolo solo las invocan.
impl BookosComp {
    /// Lo que pide el usuario: alterna, lo recuerda para la aplicación y lo
    /// dice en pantalla. Sin el aviso no había forma de saber si el atajo había
    /// hecho algo —una ventana que ya estaba delante no cambia nada a la vista—
    /// y sin recordarlo había que volver a pedirlo cada vez que se abría.
    pub fn alternar_encima(&mut self, window: &Window) {
        let encima = !siempre_encima(window);
        self.marcar_encima(window, encima);
        let raiz = crate::multitarea::raiz(self, window);
        let app_id = crate::handlers::app_id(&raiz);
        if let Some(app_id) = &app_id
            && let Err(err) = bookos_shell::marcar_app_encima(app_id, encima)
        {
            tracing::warn!(app_id, "no se pudo guardar «siempre encima»: {err}");
        }
        // El nombre de la aplicación y no el título: se recuerda para todas
        // sus ventanas, y «Bienvenido — KWrite siempre encima» decía otra cosa.
        let nombre = app_id
            .as_deref()
            .and_then(bookos_shell::nombre_de_app)
            .or_else(|| crate::handlers::titulo(&raiz))
            .unwrap_or_else(|| "La ventana".into());
        if let Some(shell) = self.shell.as_mut() {
            shell.mostrar_osd(
                "fijar",
                None,
                Some(if encima {
                    format!("{nombre} siempre encima")
                } else {
                    format!("{nombre} ya no está encima")
                }),
            );
        }
    }

    fn marcar_encima(&mut self, window: &Window, encima: bool) {
        for window in crate::multitarea::familia(self, window) {
            estado(&window).encima.set(encima);
            // Entre las ventanas normales (30) y las capas del escritorio (40).
            window.override_z_index(if encima { 35 } else { 30 });
            self.space.raise_element(&window, false);
        }
        self.needs_redraw = true;
    }

    /// Traslada coordenadas lógicas y conserva maximizado/fullscreen y el
    /// rectángulo flotante al que volver. El destino puede tener otra escala.
    pub fn mover_a_monitor(&mut self, window: &Window, nombre: &str) {
        let familia = crate::multitarea::familia(self, window);
        for w in familia {
            self.mover_a_monitor_una(&w, nombre);
        }
        crate::multitarea::ajustar_monitor(self, window, nombre);
    }

    fn mover_a_monitor_una(&mut self, window: &Window, nombre: &str) {
        if self.minimizando.iter().any(|(w, _)| w == window) {
            return;
        }
        crate::escritorios::terminar_todos(self);
        let Some(destino) = self
            .space
            .outputs()
            .find(|o| o.name() == nombre)
            .and_then(|o| self.space.output_geometry(o))
        else {
            return;
        };
        let Some(loc) = self.space.element_location(window) else {
            return;
        };
        let origen = crate::escritorios::salida_de(self, window)
            .and_then(|n| self.space.outputs().find(|o| o.name() == n))
            .and_then(|o| self.space.output_geometry(o))
            .unwrap_or(destino);
        if origen == destino {
            return;
        }
        let puntero = self.pointer_location;
        // Los helpers de área útil usan la salida bajo el puntero. Se cambia
        // solo durante la operación, sin emitir movimiento al cliente.
        self.pointer_location = (destino.loc.x as f64 + 1.0, destino.loc.y as f64 + 1.0).into();
        let area = crate::decoracion::area_util(self, window);
        let posicion = confinar(
            loc + (destino.loc - origen.loc),
            window.geometry().size,
            area,
        );
        if let Some(previa) = estado(window).restaurar.get() {
            estado(window).restaurar.set(Some(Rectangle::new(
                confinar(previa.loc + (destino.loc - origen.loc), previa.size, area),
                previa.size,
            )));
        }
        self.space.map_element(window.clone(), posicion, false);
        if completa(window) {
            // Reaplicar fullscreen al nuevo output aunque ya estuviera activo.
            estado(window).completa.set(false);
            self.pantalla_completa(window, true);
        } else if let Some(zona) = zona_de(window) {
            self.encajar(window, zona);
        } else if let Some(x11) = window.x11_surface() {
            let _ = x11.configure(Rectangle::new(posicion, window.geometry().size));
        }
        estado(window).desde.set(None);
        self.pointer_location = puntero;
        self.enfocar(window);
        self.revisar_barras();
    }
    /// El hueco que le queda a las ventanas: la pantalla menos el panel.
    ///
    /// El dock no se descuenta a propósito: flota por encima, como en macOS.
    /// Una ventana puede quedar por debajo de él, y con el foco y la elevación
    /// funcionando eso no es un problema — basta pulsarla para sacarla.
    /// El tamaño lógico de la pantalla. Lo usan el bloqueo y las emergentes a
    /// pantalla completa.
    pub fn pantalla_logica(&self) -> (f32, f32) {
        let r = self.pantalla();
        (r.size.w as f32, r.size.h as f32)
    }

    pub fn work_area(&self) -> Rectangle<i32, Logical> {
        let pantalla = self.pantalla();
        // Un panel que esquiva ventanas **no reserva sitio**: si lo reservara
        // no habría nada que esquivar, porque ninguna ventana llegaría hasta
        // él. Es lo que hace KDE con el mismo modo.
        // El shell vive en la pantalla principal, cuyo origen se normaliza a
        // (0,0). Las secundarias aprovechan toda su altura: reservar allí el
        // panel dejaría una franja vacía que nunca se dibuja.
        //
        // **El dock reserva igual que el panel.** Antes solo lo hacía el panel,
        // y una ventana maximizada llegaba hasta el borde de abajo con el dock
        // encima tapándole sus últimos píxeles: medido a 1280×800, el área
        // acababa en y=800 y el dock empezaba en y=712.
        let (arriba, abajo) = if pantalla.loc == (0, 0).into() {
            {
                self.shell
                    .as_ref()
                    .map(|s| {
                        (
                            s.reserva(crate::shell::Barra::Panel).round() as i32,
                            s.reserva(crate::shell::Barra::Dock).round() as i32,
                        )
                    })
                    .unwrap_or((0, 0))
            }
        } else {
            (0, 0)
        };
        Rectangle::new(
            (pantalla.loc.x, pantalla.loc.y + arriba).into(),
            (
                pantalla.size.w.max(1),
                (pantalla.size.h - arriba - abajo).max(1),
            )
                .into(),
        )
    }

    /// Coloca una ventana recién mapeada en cuanto se sabe su tamaño.
    ///
    /// No se puede hacer en `new_toplevel`: ahí el cliente todavía no ha
    /// adjuntado buffer y `geometry()` mide 0×0, así que centrarla la dejaría en
    /// la esquina inferior derecha. El primer commit con contenido es el primer
    /// momento en que hay un tamaño real que centrar.
    pub fn colocar_si_es_nueva(&mut self, window: &Window) {
        if colocada(window) {
            return;
        }
        let tam = window.geometry().size;
        if tam.w <= 0 || tam.h <= 0 {
            return;
        }
        // Menús y tooltips X11 se quedan donde el cliente los puso: él calcula
        // su sitio en coordenadas de X y no se le puede decir que se movió
        // (Smithay rechaza `configure` en override-redirect). Colocados como una
        // ventana más, el menú «Archivo» de VirtualBox salía en mitad de la
        // pantalla mientras X lo creía bajo la barra de menús.
        if window
            .x11_surface()
            .is_some_and(|x| x.is_override_redirect())
        {
            nace(window);
            return;
        }
        let duena = duena(self, window);
        // En el kiosko la ventana principal tapa la pantalla desde su primer
        // fotograma, sin la lámpara que la traería desde un dock que no hay.
        if self.modo.kiosko() && duena.is_none() {
            estado(window).completa.set(true);
            self.space
                .map_element(window.clone(), self.pantalla().loc, true);
            nace(window);
            self.enfocar(window);
            return;
        }
        let area = crate::decoracion::area_util(self, window);
        let sitio = match &duena {
            Some(Duena::Padre(padre)) => match self.space.element_geometry(padre) {
                Some(sobre) => encima_de(tam, sobre, area),
                None => hueco(tam, area, &self.esquinas_ocupadas(window)),
            },
            Some(Duena::Proceso(_)) | None => hueco(tam, area, &self.esquinas_ocupadas(window)),
        };
        tracing::debug!(
            ?sitio,
            ?tam,
            secundaria = duena.is_some(),
            "ventana colocada"
        );
        // Un cliente X11 tiene que enterarse de dónde lo hemos puesto: sus
        // menús y diálogos se posicionan en coordenadas absolutas de la
        // pantalla X, y si cree seguir en el (0,0) los abre en la esquina.
        if let Some(x11) = window.x11_surface() {
            let _ = x11.configure(Rectangle::new(sitio, tam));
        }
        self.space.map_element(window.clone(), sitio, true);
        nace(window);
        // Un diálogo de una ventana que está encima va con ella: debajo quedaría
        // tapado por su propia dueña. Y una aplicación marcada como siempre
        // encima lo está desde su primera ventana.
        let encima = match &duena {
            Some(Duena::Padre(padre)) => siempre_encima(padre),
            _ => crate::handlers::app_id(window)
                .is_some_and(|id| bookos_shell::apps_encima().contains(&id)),
        };
        if encima {
            self.marcar_encima(window, true);
        }
        // Abrir recorre la misma lámpara que minimizar, en sentido inverso.
        // Se arranca después de mapear porque el renderer necesita una ventana
        // viva para congelar su primer buffer. Si la aplicación aún no tiene
        // icono, `destino_dock` usa el borde inferior bajo ella como origen
        // estable y evita que aparezca desde una esquina arbitraria.
        //
        // Las secundarias no: el programa ya está abierto, y un diálogo que sale
        // disparado del dock parece otra aplicación. Les basta el zoom de
        // entrada de `nace`.
        if duena.is_none() {
            let origen = self
                .space
                .element_geometry(window)
                .unwrap_or(Rectangle::new(sitio, tam));
            let app_id = crate::handlers::app_id(window).unwrap_or_default();
            let destino = destino_dock(self, &app_id, origen);
            encoger(window, origen, destino, false);
        }
        self.revisar_barras();
        // Una ventana que aparece se lleva el foco: es lo que espera cualquiera
        // al abrir un programa, y sin esto habría que pulsarla para escribir.
        self.enfocar(window);
    }

    fn esquinas_ocupadas(&self, window: &Window) -> Vec<Point<i32, Logical>> {
        self.space
            .elements()
            .filter(|w| *w != window && colocada(w))
            .filter_map(|w| self.space.element_location(w))
            .collect()
    }

    /// Vuelve a colocar una ventana que crece justo después de abrirse y ya no
    /// cabe donde se puso.
    ///
    /// Algunos clientes (LibreOffice con gtk3) mapean con un tamaño provisional
    /// y a los pocos commits pasan al definitivo. `colocar_si_es_nueva` centró
    /// el provisional, así que la esquina se quedaba donde estaba y el resto se
    /// salía por la derecha y por abajo. Los dos primeros segundos son la ventana de
    /// tiempo: pasados, un cambio de tamaño es del usuario o de la propia aplicación y se
    /// respeta donde esté.
    pub fn recolocar_si_creció(&mut self, window: &Window) {
        const VENTANA_DE_CRECIMIENTO: Duration = Duration::from_secs(2);
        let Some(nacida) = estado(window).nacida.get() else {
            return;
        };
        // Las override-redirect, por lo mismo que en `colocar_si_es_nueva`: un
        // menú largo que asoma bajo el área útil es cosa del cliente.
        if nacida.elapsed() > VENTANA_DE_CRECIMIENTO
            || maximizada(window)
            || crate::multitarea::agrupada(self, window)
            || completa(window)
            || window
                .x11_surface()
                .is_some_and(|x| x.is_override_redirect())
        {
            return;
        }
        let Some(loc) = self.space.element_location(window) else {
            return;
        };
        let tam = window.geometry().size;
        let area = crate::decoracion::area_util(self, window);
        if area.contains_rect(Rectangle::new(loc, tam)) {
            return;
        }
        let sitio = hueco(tam, area, &self.esquinas_ocupadas(window));
        if sitio != loc {
            // Sin esto el cliente X11 seguiría creyéndose en el sitio viejo y
            // abriría sus menús desplazados: ver `colocar_si_es_nueva`.
            if let Some(x11) = window.x11_surface() {
                let _ = x11.configure(Rectangle::new(sitio, tam));
            }
            self.space.map_element(window.clone(), sitio, false);
        }
    }

    /// Da el foco de teclado a una ventana, la eleva y actualiza el estado
    /// `Activated` de todas.
    ///
    /// Lo de `Activated` no es cosmético: es cómo el cliente sabe si pintar su
    /// barra de título viva o apagada. Sin actualizarlo, todas las ventanas se
    /// dibujan como si tuvieran el foco a la vez.
    pub fn enfocar(&mut self, window: &Window) {
        let destino = crate::multitarea::preparar_foco(self, window);
        let window = &destino;
        self.space.raise_element(window, true);
        // El sello de uso reciente se pone aquí y no al abrir la ventana: lo que
        // importa para Alt+Tab es a qué has ido, no qué has lanzado.
        estado(window).enfocada.set(Some(Instant::now()));

        // El servidor X lleva su propio orden de apilado, y no se entera de que
        // el `Space` ha subido nada. Sin decírselo, dos ventanas X11 se tapan
        // entre ellas según el orden que tuviera X, no el que se ve.
        if let (Some(x11), Some(xwm)) = (window.x11_surface(), self.xwm.as_mut()) {
            let _ = xwm.raise_window(x11);
        }

        let todas: Vec<Window> = self.space.elements().cloned().collect();
        for otra in &todas {
            // `set_activated` devuelve si el estado cambió de verdad; solo
            // entonces hay que mandar configure. Mandarlo siempre es un ida y
            // vuelta con cada cliente en cada clic.
            if otra.set_activated(otra == window)
                && let Some(toplevel) = otra.toplevel()
            {
                toplevel.send_pending_configure();
            }
        }

        let surface = window.wl_surface().map(|s| s.into_owned());
        if let Some(kbd) = self.seat.get_keyboard() {
            kbd.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.needs_redraw = true;
    }

    /// Quita el teclado a todas las ventanas: lo pide pulsar el escritorio.
    ///
    /// Además de soltar el foco, las desactiva. Sin eso la de antes seguía con
    /// la barra de título encendida y el cursor de texto parpadeando, como si
    /// lo que se tecleara fuera a ir a ella.
    pub fn desenfocar(&mut self) {
        let todas: Vec<Window> = self.space.elements().cloned().collect();
        for ventana in &todas {
            if ventana.set_activated(false)
                && let Some(toplevel) = ventana.toplevel()
            {
                toplevel.send_pending_configure();
            }
        }
        if let Some(kbd) = self.seat.get_keyboard() {
            kbd.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.needs_redraw = true;
    }

    /// La ventana con el foco de teclado, si la hay.
    pub fn ventana_con_foco(&self) -> Option<Window> {
        let surface = self.seat.get_keyboard()?.current_focus()?;
        self.window_for_surface(&surface)
    }

    /// La ventana de un programa, la de más arriba si tiene varias.
    ///
    /// Se recorre al revés porque `elements()` va de abajo arriba: con tres
    /// terminales abiertas, pulsar el icono del dock tiene que traer la que ya
    /// estaba delante, no la del fondo.
    pub fn ventana_de_app(&self, app_id: &str) -> Option<Window> {
        self.space
            .elements()
            .rev()
            .find(|w| {
                crate::handlers::app_id(w)
                    .is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
            })
            .cloned()
    }

    /// Todas las ventanas de una aplicación. Es lo que necesita el "Cerrar"
    /// del menú del dock: cerrar la aplicación, no una de sus ventanas.
    pub fn ventanas_de_app(&self, app_id: &str) -> Vec<Window> {
        self.space
            .elements()
            .filter(|w| {
                crate::handlers::app_id(w)
                    .is_some_and(|suyo| bookos_shell::mismo_programa(app_id, &suyo))
            })
            .cloned()
            .collect()
    }

    /// La ventana de un toplevel concreto.
    pub fn window_de_toplevel(&self, toplevel: &ToplevelSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel() == Some(toplevel))
            .cloned()
    }

    /// Arranca un arrastre pedido por el propio cliente.
    ///
    /// A diferencia de Meta+ratón, aquí la ventana ya se sabe —la manda el
    /// cliente— y el punto de partida es donde esté el cursor: el clic que lo
    /// desencadenó ya ocurrió.
    pub fn arrastrar_toplevel(&mut self, toplevel: &ToplevelSurface, modo: Modo) {
        if let Some(window) = self.window_de_toplevel(toplevel) {
            self.arrastrar_ventana(&window, modo, true);
        }
    }

    /// El mismo arrastre, con la ventana ya resuelta. Lo usan tanto xdg-shell
    /// como X11, que llegan por caminos distintos al mismo sitio.
    ///
    /// `del_cliente` dice quién lo pidió, y decide si al soltar el botón el
    /// evento llega al cliente: si lo pidió él, ya vio la pulsación y necesita
    /// ver también la suelta. Arrastrando de **nuestra** barra de título el
    /// clic no ha existido para él.
    pub fn arrastrar_ventana(&mut self, window: &Window, modo: Modo, del_cliente: bool) {
        if modo != Modo::Mover && crate::multitarea::empezar_redimension(self, window, del_cliente)
        {
            return;
        }

        let Some(loc) = self.space.element_location(window) else {
            return;
        };
        if modo == Modo::Mover {
            self.soltar_para_arrastrar(window, loc, self.pointer_location);
        }
        let loc = self.space.element_location(window).unwrap_or(loc);
        let geo = Rectangle::new(loc, window.geometry().size);
        self.arrastre = Some(Arrastre {
            window: window.clone(),
            modo,
            inicio: self.pointer_location,
            geo,
            zona: None,
            previa_desde: None,
            previa_nacida: None,
            del_cliente,
        });
    }

    /// Arrastrar una ventana encajada —o la mitad de un grupo lado a lado— la
    /// suelta, como en KWin: recupera su tamaño flotante y se queda colgando
    /// del cursor. Sin esto arrastrarías media pantalla entera, que además no
    /// cabe en ningún sitio y vuelve a encajarse al primer roce con un borde.
    fn soltar_para_arrastrar(
        &mut self,
        window: &Window,
        loc: Point<i32, Logical>,
        punto: Point<f64, Logical>,
    ) {
        let antes = window.geometry().size.w;
        let agrupada = crate::multitarea::sacar(self, window);
        if !agrupada && !encajada(window) {
            return;
        }
        // El tamaño nuevo se toma de lo guardado, no de la geometría: el
        // cliente aún no ha contestado al configure y seguiría midiendo media
        // pantalla. Sin nada guardado —grupo hecho desde el menú— la ventana
        // vuelve a su rectángulo de antes del grupo, que ya pone `separar`.
        let nuevo = estado(window)
            .restaurar
            .get()
            .map_or(window.geometry().size, |r| r.size);
        self.desencajar(window);
        // El agarre se recoloca proporcionalmente: si tenías el cursor en
        // mitad del ancho de la ventana encajada, sigue en mitad del ancho de
        // la ventana pequeña. Si no, la ventana da un brinco lateral.
        if loc.x < punto.x as i32 && nuevo.w > 0 && antes > 0 {
            let f = (punto.x - loc.x as f64) / antes as f64;
            let x = punto.x - f * nuevo.w as f64;
            // La y de antes, no la restaurada: si no, el cursor queda fuera
            // de la barra que estás agarrando.
            self.space
                .map_element(window.clone(), (x.round() as i32, loc.y), false);
        }
    }

    /// Empieza a mover o redimensionar la ventana bajo el cursor.
    ///
    /// Devuelve `false` si no había ninguna: el llamante sigue con el reenvío
    /// normal del clic.
    pub fn empezar_arrastre(&mut self, modo: Modo) -> bool {
        let punto = self.pointer_location;
        let Some((window, loc)) = self.ventana_bajo(punto) else {
            return false;
        };
        // Redimensionar una ventana maximizada no tiene sentido: el cliente ya
        // ha aceptado el tamaño que le impusimos y volvería a pedirlo.
        if maximizada(&window) && modo != Modo::Mover {
            return false;
        }
        // Arrastrar una ventana encajada la suelta de su zona, como en KWin:
        // recupera su tamaño flotante y se queda colgando del cursor. Sin esto
        // arrastrarías media pantalla entera, que además no cabe en ningún
        // sitio y vuelve a encajarse al primer roce con un borde.
        if modo == Modo::Mover {
            self.soltar_para_arrastrar(&window, loc, punto);
        }
        let loc = self.space.element_location(&window).unwrap_or(loc);
        let geo = Rectangle::new(loc, window.geometry().size);
        // El cuadrante se decide **al empezar**: si se recalculara en cada
        // movimiento, cruzar la mitad de la ventana cambiaría el borde del que
        // se tira a mitad del arrastre.
        let modo = match modo {
            Modo::Mover => Modo::Mover,
            Modo::Redimensionar { .. } => Modo::por_cuadrante(punto, geo),
        };
        self.enfocar(&window);
        if modo != Modo::Mover && crate::multitarea::empezar_redimension(self, &window, false) {
            return true;
        }
        self.arrastre = Some(Arrastre {
            window,
            modo,
            inicio: punto,
            geo,
            zona: None,
            previa_desde: None,
            previa_nacida: None,
            del_cliente: false,
        });
        true
    }

    /// Lleva la ventana arrastrada a donde diga el puntero. `true` si había
    /// arrastre, en cuyo caso el evento no debe llegar al cliente.
    pub fn seguir_arrastre(&mut self) -> bool {
        let Some(arrastre) = self.arrastre.clone() else {
            return false;
        };
        let area = crate::decoracion::area_util(self, &arrastre.window);
        let destino = arrastre.geometria(self.pointer_location, area);
        if arrastre.modo == Modo::Mover
            && let Some(loc) = self.space.element_location(&arrastre.window)
            && crate::multitarea::arrastrar(self, &arrastre.window, destino.loc - loc)
        {
            self.needs_redraw = true;
            return true;
        }
        if arrastre.modo != Modo::Mover
            && crate::multitarea::redimensionar(self, &arrastre.window, destino)
        {
            return true;
        }
        // Sin animación: durante un arrastre la ventana tiene que ir pegada al
        // cursor. Interpolar aquí se siente como arrastrar algo con retardo.
        if arrastre.modo == Modo::Mover
            && let Some(loc) = self.space.element_location(&arrastre.window)
        {
            crate::multitarea::acompanar(self, &arrastre.window, destino.loc - loc);
        }
        self.space
            .map_element(arrastre.window.clone(), destino.loc, false);
        // Mientras arrastras, las barras se apartan o vuelven según dónde esté
        // la ventana: es el momento en que más se nota que esquivan.
        self.revisar_barras();
        if arrastre.modo != Modo::Mover {
            if let Some(toplevel) = arrastre.window.toplevel() {
                toplevel.with_pending_state(|state| state.size = Some(destino.size));
                toplevel.send_pending_configure();
            }
            if let Some(x11) = arrastre.window.x11_surface() {
                let _ = x11.configure(destino);
            }
        }
        if arrastre.modo == Modo::Mover {
            // La previa se dibuja sobre el área del escritorio: es dónde va a
            // quedar la ventana **con** su barra, y descontarla dos veces
            // dejaría el rectángulo por debajo de la ventana que anuncia.
            self.actualizar_previa(zona_en(self.pointer_location, self.work_area()));
        }
        self.needs_redraw = true;
        true
    }

    /// Apunta en qué zona encajaría la ventana ahora, para dibujar la previa.
    ///
    /// El deslizamiento de un rectángulo a otro es lo que hace legible el
    /// encaje: sin él, pasar de "media izquierda" a "cuarto superior" es un
    /// salto y hay que mirar dos veces para entender qué va a ocurrir.
    fn actualizar_previa(&mut self, zona: Option<Zona>) {
        let area = self.work_area();
        let Some(arrastre) = self.arrastre.as_mut() else {
            return;
        };
        if arrastre.zona == zona {
            return;
        }
        arrastre.previa_desde = match (arrastre.zona, zona) {
            // De una zona a otra: se desliza desde donde estaba.
            (Some(anterior), Some(_)) => Some((anterior.rect(area), Instant::now())),
            // Apareciendo: no hay de dónde venir, entra con el fundido.
            (None, Some(_)) => None,
            // Saliendo del borde: la previa desaparece, no hay nada que animar.
            (_, None) => None,
        };
        arrastre.previa_nacida = match (arrastre.zona, zona) {
            (None, Some(_)) => Some(Instant::now()),
            (_, None) => None,
            _ => arrastre.previa_nacida,
        };
        arrastre.zona = zona;
    }

    /// Suelta la ventana arrastrada, encajándola si el cursor estaba en un
    /// borde.
    ///
    /// Devuelve si el evento de soltar debe **tragarse**. Cuando el arrastre lo
    /// pidió el propio cliente, no: él vio la pulsación y tiene que ver también
    /// el soltar.
    pub fn soltar_arrastre(&mut self) -> bool {
        let Some(arrastre) = self.arrastre.take() else {
            return false;
        };
        let tragar = !arrastre.del_cliente;
        if let Some(zona) = arrastre.zona {
            // A dónde volver es la geometría de **antes** del arrastre, no la
            // de ahora: para encajar has tenido que llevar la ventana contra el
            // borde, y ahí está medio fuera de la pantalla. Guardar esa dejaba
            // la ventana en x = -322 al desencajarla, medida en el autotest.
            guardar_restaurar(&arrastre.window, arrastre.geo);
            self.encajar(&arrastre.window, zona);
        }
        self.needs_redraw = true;
        tragar
    }

    /// Pone la ventana en una zona de la pantalla, animando el recorrido.
    pub fn encajar(&mut self, window: &Window, zona: Zona) {
        crate::multitarea::separar(self, window);
        salir_de_pantalla_completa(window);
        // El área es la de esta ventana, no la del escritorio: una decorada
        // maximizada tiene que dejar arriba el hueco de su propia barra, o la
        // barra acaba debajo del panel.
        let destino = zona.rect(crate::decoracion::area_util(self, window));
        let actual = self
            .space
            .element_location(window)
            .map(|loc| Rectangle::new(loc, window.geometry().size));

        // Se guarda a dónde volver, pero **solo si no hay nada guardado ya**:
        // pasar de media pantalla a un cuarto tiene que seguir recordando el
        // tamaño flotante original, no la mitad en la que estaba. Quien viene
        // de un arrastre lo deja puesto antes de llamar aquí.
        if let Some(actual) = actual {
            if estado(window).restaurar.get().is_none() {
                guardar_restaurar(window, actual);
            }
            mover_desde(window, geometria_visual(window, actual));
        }
        estado(window).zona.set(Some(zona));

        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                // `Maximized` solo en la zona que de verdad lo es: los clientes
                // lo usan para quitarse las esquinas redondeadas y la sombra, y
                // una ventana a media pantalla sí las quiere.
                if zona == Zona::Maxima {
                    state.states.set(xdg_toplevel::State::Maximized);
                } else {
                    state.states.unset(xdg_toplevel::State::Maximized);
                }
                state.size = Some(destino.size);
            });
            toplevel.send_pending_configure();
        }
        // X11 no tiene configure diferido ni estados pendientes: se le manda el
        // rectángulo entero —posición incluida— y el cliente obedece. Y sí hay
        // que darle la posición, aunque el `Space` la lleve aparte: un cliente
        // X11 pregunta por su propia geometría y sale mal colocado si nunca se
        // la contamos.
        if let Some(x11) = window.x11_surface() {
            let _ = x11.set_maximized(zona == Zona::Maxima);
            let _ = x11.configure(destino);
        }
        self.space.map_element(window.clone(), destino.loc, true);
        crate::multitarea::encajada(self, window, zona);
        self.needs_redraw = true;
        self.revisar_barras();
    }

    /// Devuelve la ventana a su tamaño flotante, si estaba encajada.
    pub fn desencajar(&mut self, window: &Window) {
        let Some(previa) = tomar_restaurar(window) else {
            return;
        };
        salir_de_pantalla_completa(window);
        // Se vuelve a confinar al soltar: la posición guardada puede haber
        // dejado de ser válida —la pantalla cambió de tamaño, o el panel
        // creció— y devolver la ventana a un sitio inalcanzable es peor que no
        // devolverla del todo.
        let previa = Rectangle::new(
            confinar(
                previa.loc,
                previa.size,
                crate::decoracion::area_util(self, window),
            ),
            previa.size,
        );
        estado(window).zona.set(None);
        if let Some(actual) = self.space.element_location(window) {
            let actual = Rectangle::new(actual, window.geometry().size);
            mover_desde(window, geometria_visual(window, actual));
        }
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = Some(previa.size);
            });
            toplevel.send_pending_configure();
        }
        if let Some(x11) = window.x11_surface() {
            let _ = x11.set_maximized(false);
            let _ = x11.configure(previa);
        }
        self.space.map_element(window.clone(), previa.loc, true);
        self.needs_redraw = true;
        self.revisar_barras();
    }

    /// Alterna entre ocupar el área útil y volver al tamaño de antes.
    ///
    /// Sobre una ventana a pantalla completa, sale de ella: es lo que espera
    /// cualquiera al pulsar el atajo de tamaño, y además es la única salida a
    /// mano si el cliente que la puso ahí deja de responder.
    pub fn alternar_maximizada(&mut self, window: &Window) {
        if completa(window) {
            self.pantalla_completa(window, false);
        } else if maximizada(window) {
            self.desencajar(window);
        } else {
            self.encajar(window, Zona::Maxima);
        }
    }

    /// Pone —o saca— una ventana a pantalla completa.
    ///
    /// A diferencia de maximizar, aquí se ocupa la pantalla **entera** y el
    /// shell deja de dibujarse: un vídeo o un juego con el panel encima no
    /// está a pantalla completa, está maximizado.
    pub fn pantalla_completa(&mut self, window: &Window, activar: bool) {
        if activar {
            crate::multitarea::separar(self, window);
        }
        if completa(window) == activar {
            return;
        }
        let pantalla = self.pantalla();
        let actual = self
            .space
            .element_location(window)
            .map(|loc| Rectangle::new(loc, window.geometry().size));

        let destino = if activar {
            // A dónde volver: solo se guarda si no había nada guardado ya, para
            // no perder el tamaño flotante de una ventana que estaba encajada
            // cuando el cliente pidió pantalla completa.
            if let Some(actual) = actual
                && estado(window).restaurar.get().is_none()
            {
                guardar_restaurar(window, actual);
            }
            pantalla
        } else {
            // Sin nada guardado se vuelve al área útil: perder el sitio de
            // antes es malo, pero dejar la ventana tapando el panel es peor.
            let area = crate::decoracion::area_util(self, window);
            let previa = tomar_restaurar(window).unwrap_or_else(|| Zona::Maxima.rect(area));
            Rectangle::new(confinar(previa.loc, previa.size, area), previa.size)
        };

        estado(window).completa.set(activar);
        // Sin animación: un cambio a pantalla completa que se desliza se ve
        // como un tirón, y encima el cliente está repintando su contenido
        // entero justo en ese momento.
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                if activar {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                }
                state.size = Some(destino.size);
            });
            toplevel.send_pending_configure();
        }
        if let Some(x11) = window.x11_surface() {
            let _ = x11.set_fullscreen(activar);
            let _ = x11.configure(destino);
        }
        self.space.map_element(window.clone(), destino.loc, true);
        self.needs_redraw = true;
        self.revisar_barras();
    }

    /// La pantalla entera, en lógicos: lo mismo que `work_area` pero sin
    /// descontar el panel ni el dock.
    pub(crate) fn pantalla(&self) -> Rectangle<i32, Logical> {
        // Las acciones se aplican a la pantalla bajo el puntero. Esto hace que
        // maximizar, pantalla completa y encajar funcionen en salidas situadas
        // a cualquier lado del origen, no siempre en el primer monitor.
        let punto = self.pointer_location;
        self.space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .find(|r| {
                punto.x >= r.loc.x as f64
                    && punto.y >= r.loc.y as f64
                    && punto.x < (r.loc.x + r.size.w) as f64
                    && punto.y < (r.loc.y + r.size.h) as f64
            })
            .or_else(|| {
                self.space
                    .outputs()
                    .find_map(|o| self.space.output_geometry(o))
            })
            .unwrap_or_else(|| Rectangle::from_size((1920, 1080).into()))
    }

    /// Vuelve a colocar las ventanas encajadas contra el área útil de ahora.
    ///
    /// Hace falta cuando el área cambia de tamaño, que hoy solo pasa al
    /// alternar la visibilidad del panel: una ventana maximizada con el panel
    /// fijo se queda 32 px corta cuando el panel deja de reservar sitio.
    pub fn recolocar_encajadas(&mut self) {
        crate::multitarea::ajustar_areas(self);
        let encajadas: Vec<(Window, Zona)> = self
            .space
            .elements()
            .filter_map(|w| zona_de(w).map(|z| (w.clone(), z)))
            .collect();
        for (window, zona) in encajadas {
            self.encajar(&window, zona);
        }
    }

    /// Baja la ventana lo justo para que su barra de título quepa.
    ///
    /// Hace falta porque el modo de decoración llega **después** de que la
    /// ventana esté colocada: el cliente crea su `xdg_toplevel`, lo mapeamos, y
    /// solo entonces pide (o acepta) que la decoremos. Sin esto, la barra de la
    /// primera ventana de cada sesión aparecía metida bajo el panel.
    pub fn recolocar_por_barra(&mut self, window: &Window) {
        if !colocada(window) || encajada(window) || completa(window) {
            return;
        }
        let Some(loc) = self.space.element_location(window) else {
            return;
        };
        let area = crate::decoracion::area_util(self, window);
        let destino = confinar(loc, window.geometry().size, area);
        if destino == loc {
            return;
        }
        mover_desde(window, Rectangle::new(loc, window.geometry().size));
        if let Some(x11) = window.x11_surface() {
            let _ = x11.configure(Rectangle::new(destino, window.geometry().size));
        }
        self.space.map_element(window.clone(), destino, false);
        self.revisar_barras();
    }

    /// Mira si alguna ventana llega a donde están las barras y se lo dice al
    /// shell, para que se aparten o vuelvan.
    ///
    /// Se llama al mover, mapear o cerrar ventanas, no por frame: recorrer el
    /// espacio sesenta veces por segundo para comprobar un solape que casi
    /// nunca cambia es justo lo que este compositor evita.
    pub fn revisar_barras(&mut self) {
        let Some(zonas) = self.shell.as_ref().map(|s| s.zonas_barras()) else {
            return;
        };
        let geometrias: Vec<Rectangle<i32, Logical>> = self
            .space
            .elements()
            // `toplevel_destroyed` llama aquí con la ventana muerta aún en el
            // `Space` (lo vacía `space.refresh()` al final de la vuelta). Si
            // estaba maximizada seguía «tapando» las barras y en modo esquivar
            // no volvían. Pasa con los clientes que destruyen el toplevel sin
            // desmapear antes, como GTK/Tauri.
            // Las que se están apartando ya se van: esperar a que salgan para
            // traer el dock sumaba el viaje entero (260 ms) a su propia
            // animación de entrada.
            .filter(|w| {
                w.alive()
                    && colocada(w)
                    && !self.escritorios.se_va(w)
                    && crate::multitarea::visible(self, w)
            })
            .filter_map(|w| {
                let cuerpo = Rectangle::new(self.space.element_location(w)?, w.geometry().size);
                // La barra de título que dibujamos nosotros va **encima** de la
                // geometría del cliente. Sin contarla, una ventana maximizada
                // empezaba en y=32 —justo el alto del panel— y nunca lo
                // tocaba: el panel en modo esquivar volvía al instante.
                Some(match crate::decoracion::barra_rect(self, w) {
                    Some(barra) => cuerpo.merge(barra),
                    None => cuerpo,
                })
            })
            .collect();
        let mut repintar = false;
        for (cual, zona) in zonas {
            let estorba = geometrias.iter().any(|g| g.overlaps(zona));
            if let Some(shell) = self.shell.as_mut() {
                repintar |= shell.estorbada(cual, estorba);
            }
        }
        if repintar {
            self.needs_redraw = true;
        }
    }

    /// ¿Hay alguna ventana a pantalla completa tapándolo todo?
    ///
    /// Lo pregunta la escena para no dibujar el shell encima.
    pub fn hay_pantalla_completa(&self) -> bool {
        self.space.elements().any(completa)
    }

    /// ¿Hay una ventana a pantalla completa sobre esta salida concreta?
    /// Una aplicación fullscreen en el proyector no debe esconder el panel del
    /// portátil ni alterar la composición de los demás monitores.
    pub fn hay_pantalla_completa_en(&self, output: &Output) -> bool {
        let Some(salida) = self.space.output_geometry(output) else {
            return false;
        };
        self.space.elements().filter(|w| completa(w)).any(|window| {
            self.space
                .element_location(window)
                .is_some_and(|loc| Rectangle::new(loc, window.geometry().size).overlaps(salida))
        })
    }
}

/// En qué zona está encajada, si lo está.
pub fn soltar_encaje(window: &Window) {
    estado(window).zona.set(None);
    estado(window).desde.set(None);
    estado(window).encogido.set(None);
}

pub fn zona_de(window: &Window) -> Option<Zona> {
    estado(window).zona.get()
}

/// ¿Está esta ventana encajada en alguna zona?
pub fn siempre_encima(window: &Window) -> bool {
    estado(window).encima.get()
}

pub fn encajada(window: &Window) -> bool {
    estado(window).zona.get().is_some()
}

/// ¿Está maximizada, o sea encajada ocupando el área entera?
pub fn maximizada(window: &Window) -> bool {
    estado(window).zona.get() == Some(Zona::Maxima)
}

/// ¿Está a pantalla completa?
pub fn completa(window: &Window) -> bool {
    estado(window).completa.get()
}

fn salir_de_pantalla_completa(window: &Window) {
    if !completa(window) {
        return;
    }
    // Encajar y restaurar envían su propio tamaño. Pasar por
    // `pantalla_completa(false)` consumiría antes la geometría guardada;
    // conservar la marca, en cambio, oculta ambas barras en este escritorio.
    estado(window).completa.set(false);
    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
        });
    }
    if let Some(x11) = window.x11_surface() {
        let _ = x11.set_fullscreen(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un área útil como la de esta máquina: pantalla menos el panel de 32.
    fn area() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 32).into(), (1646, 997).into())
    }

    #[test]
    fn las_cuatro_esquinas_reparten_la_pantalla_sin_huecos() {
        // Es la prueba de que se pueden poner cuatro ventanas a la vez: los
        // cuatro cuartos tienen que cubrir el área entera sin solaparse y sin
        // dejar una raya de fondo entre ellos, que es lo que pasa al partir un
        // ancho impar con dos divisiones.
        let a = area();
        let cuartos = [Zona::SupIzq, Zona::SupDer, Zona::InfIzq, Zona::InfDer].map(|z| z.rect(a));
        let suma: i32 = cuartos.iter().map(|r| r.size.w * r.size.h).sum();
        assert_eq!(suma, a.size.w * a.size.h);
        // Y los bordes casan: el derecho del izquierdo es el izquierdo del
        // derecho.
        assert_eq!(
            cuartos[0].loc.x + cuartos[0].size.w,
            cuartos[1].loc.x,
            "queda un hueco vertical entre los cuartos"
        );
        assert_eq!(
            cuartos[0].loc.y + cuartos[0].size.h,
            cuartos[2].loc.y,
            "queda un hueco horizontal entre los cuartos"
        );
    }

    #[test]
    fn el_dialogo_se_centra_en_su_ventana_sin_salirse_del_area() {
        let a = area();
        let tam = Size::from((400, 300));
        let padre = Rectangle::new((200, 100).into(), (800, 600).into());
        assert_eq!(encima_de(tam, padre, a), Point::from((400, 250)));
        // Con la ventana pegada a la esquina, el diálogo se queda dentro en vez
        // de meterse bajo el panel.
        let arriba = Rectangle::new((-100, 0).into(), (300, 200).into());
        assert_eq!(encima_de(tam, arriba, a), Point::from((0, 32)));
        // Más grande que el área: manda la esquina de arriba a la izquierda.
        let enorme = Size::from((3000, 2000));
        assert_eq!(encima_de(enorme, padre, a), Point::from((0, 32)));
    }

    #[test]
    fn ninguna_zona_invade_el_panel() {
        // El área útil ya empieza bajo el panel, pero es la clase de cosa que
        // se rompe sola al tocar el reparto.
        let a = area();
        for zona in [
            Zona::Maxima,
            Zona::Izquierda,
            Zona::Derecha,
            Zona::SupIzq,
            Zona::SupDer,
            Zona::InfIzq,
            Zona::InfDer,
        ] {
            let r = zona.rect(a);
            assert!(r.loc.y >= a.loc.y, "{zona:?} se mete bajo el panel");
            assert!(r.loc.x >= a.loc.x, "{zona:?} se sale por la izquierda");
            assert!(r.loc.x + r.size.w <= a.loc.x + a.size.w);
            assert!(r.loc.y + r.size.h <= a.loc.y + a.size.h);
        }
    }

    #[test]
    fn el_borde_de_arriba_maximiza_pero_sus_esquinas_no() {
        let a = area();
        assert_eq!(zona_en((800.0, 34.0).into(), a), Some(Zona::Maxima));
        // La esquina gana al borde superior: si no, no habría forma de pedir un
        // cuarto con el ratón.
        assert_eq!(zona_en((2.0, 34.0).into(), a), Some(Zona::SupIzq));
        assert_eq!(zona_en((1644.0, 34.0).into(), a), Some(Zona::SupDer));
    }

    #[test]
    fn el_centro_de_los_lados_da_media_pantalla() {
        let a = area();
        let medio = (a.loc.y + a.size.h / 2) as f64;
        assert_eq!(zona_en((1.0, medio).into(), a), Some(Zona::Izquierda));
        assert_eq!(zona_en((1645.0, medio).into(), a), Some(Zona::Derecha));
        // Y el centro de la pantalla no encaja nada.
        assert_eq!(zona_en((800.0, medio).into(), a), None);
    }

    #[test]
    fn el_borde_de_abajo_no_encaja_nada() {
        // Ahí vive el dock: encajar al acercarse haría imposible arrastrar una
        // ventana cerca de él sin que saltara.
        let a = area();
        assert_eq!(zona_en((800.0, 1028.0).into(), a), None);
    }

    #[test]
    fn las_flechas_llegan_a_un_cuarto_en_dos_pasos() {
        use Direccion::*;
        // El camino de KDE y de Windows: media pantalla y luego partirla.
        let z = empujar(None, Izquierda);
        assert_eq!(z, Some(Zona::Izquierda));
        assert_eq!(empujar(z, Arriba), Some(Zona::SupIzq));
        // Y deshacer devuelve por donde vino.
        assert_eq!(empujar(Some(Zona::SupIzq), Abajo), Some(Zona::Izquierda));
        assert_eq!(empujar(Some(Zona::Izquierda), Derecha), None);
    }

    #[test]
    fn arriba_maximiza_y_abajo_restaura() {
        use Direccion::*;
        assert_eq!(empujar(None, Arriba), Some(Zona::Maxima));
        assert_eq!(empujar(Some(Zona::Maxima), Abajo), None);
    }

    #[test]
    fn no_se_puede_meter_la_ventana_bajo_el_panel() {
        // El fallo que se vio en pantalla: al arrastrar hacia arriba, la barra
        // de título se escondía tras el panel y la ventana ya no se podía
        // agarrar de ningún sitio.
        let a = area();
        let tam = Size::from((600, 400));
        assert_eq!(confinar((300, -400).into(), tam, a).y, a.loc.y);
        assert_eq!(confinar((300, 0).into(), tam, a).y, a.loc.y);
        // Estando ya dentro no se toca.
        assert_eq!(confinar((300, 500).into(), tam, a), (300, 500).into());
    }

    /// El fallo que se vio en pantalla: tirando del borde de **arriba** la
    /// ventana seguía subiendo y se metía bajo el panel, que se dibuja siempre
    /// encima. Arrastrando de un borde se conseguía justo lo que `confinar`
    /// impide al mover.
    #[test]
    fn redimensionar_por_arriba_no_se_mete_bajo_el_panel() {
        let a = area();
        let geo = Rectangle::new((100, 200).into(), (600, 400).into());
        let modo = Modo::Redimensionar {
            izquierda: false,
            arriba: true,
        };
        // Se tira del borde superior 300 px hacia arriba: 200 - 300 = -100, muy
        // por encima del panel.
        let r = geometria_de(modo, (400.0, 200.0).into(), geo, (400.0, -100.0).into(), a);
        assert_eq!(r.loc.y, a.loc.y);
        // Y el borde de abajo, que es el que no se toca, se queda donde estaba.
        assert_eq!(r.loc.y + r.size.h, geo.loc.y + geo.size.h);
    }

    /// Tirando hacia abajo no hay techo que valga: la ventana solo se encoge.
    #[test]
    fn redimensionar_por_arriba_hacia_abajo_no_toca_el_techo() {
        let a = area();
        let geo = Rectangle::new((100, 200).into(), (600, 400).into());
        let modo = Modo::Redimensionar {
            izquierda: false,
            arriba: true,
        };
        let r = geometria_de(modo, (400.0, 200.0).into(), geo, (400.0, 300.0).into(), a);
        assert_eq!(r.loc.y, 300);
        assert_eq!(r.size.h, 300);
    }

    /// El borde de abajo no tiene nada que ver con el panel y no debe tocarse.
    #[test]
    fn redimensionar_por_abajo_se_queda_igual() {
        let a = area();
        let geo = Rectangle::new((100, 200).into(), (600, 400).into());
        let modo = Modo::Redimensionar {
            izquierda: false,
            arriba: false,
        };
        let r = geometria_de(modo, (400.0, 600.0).into(), geo, (400.0, 700.0).into(), a);
        assert_eq!(r.loc, geo.loc);
        assert_eq!(r.size.h, 500);
    }

    #[test]
    fn una_ventana_apartada_sigue_siendo_alcanzable() {
        // Por los lados y por abajo sí se la deja salir —es cómodo para
        // apartarla— pero siempre queda un trozo con el que devolverla.
        let a = area();
        let tam = Size::from((600, 400));
        let izq = confinar((-5000, 500).into(), tam, a);
        assert!(izq.x + tam.w >= a.loc.x + VISIBLE);
        let der = confinar((5000, 500).into(), tam, a);
        assert!(der.x <= a.loc.x + a.size.w - VISIBLE);
        let abajo = confinar((300, 5000).into(), tam, a);
        assert!(abajo.y <= a.loc.y + a.size.h - VISIBLE);
    }

    #[test]
    fn una_ventana_sola_queda_centrada() {
        let area = Rectangle::new((0, 30).into(), (1000, 770).into());
        let sitio = hueco((400, 300).into(), area, &[]);
        assert_eq!(sitio, (300, 265).into());
    }

    #[test]
    fn la_segunda_ventana_se_aparta_de_la_primera() {
        let area = Rectangle::new((0, 30).into(), (1000, 770).into());
        let primera = hueco((400, 300).into(), area, &[]);
        let segunda = hueco((400, 300).into(), area, &[primera]);
        assert_ne!(primera, segunda);
        assert_eq!(segunda, (primera.x + CASCADA, primera.y + CASCADA).into());
    }

    #[test]
    fn la_cascada_no_saca_la_ventana_de_la_pantalla() {
        // Área justa: cabe la ventana centrada, pero un solo paso de cascada ya
        // se saldría. Tiene que volver al centro en vez de irse fuera.
        let area = Rectangle::new((0, 0).into(), (420, 320).into());
        let centrada = hueco((400, 300).into(), area, &[]);
        let segunda = hueco((400, 300).into(), area, &[centrada]);
        assert_eq!(segunda, centrada);
    }

    #[test]
    fn la_entrada_termina_en_uno_exacto() {
        // De esto depende que una ventana quieta se dibuje **sin** envolver:
        // `escena` compara `zoom == 1.0`, y si la curva aterrizara en
        // 0,9999999997 la ventana se quedaría pasando por
        // `RescaleRenderElement` el resto de su vida. Comprobado también con el
        // log de `BOOKOS_PERFIL`: los fotogramas de entrada dejan de emitirse
        // cuando acaba la animación.
        let zoom = tema::C_MUELLE.eval(1.0);
        assert_eq!(ZOOM_INICIAL + (1.0 - ZOOM_INICIAL) * zoom as f64, 1.0);
    }

    #[test]
    fn maximizar_interpela_las_cuatro_aristas_con_la_misma_bezier() {
        let origen = Rectangle::new((220, 180).into(), (800, 600).into());
        let destino = Rectangle::new((0, 32).into(), (1920, 1048).into());

        assert_eq!(interpolar_rect(origen, destino, 0.0), origen);
        assert_eq!(interpolar_rect(origen, destino, 1.0), destino);

        let s = tema::C_VENTANA.eval(0.5) as f64;
        let medio = interpolar_rect(origen, destino, s);
        // Esta curva es ease-out: a mitad de tiempo ya ha recorrido más de la
        // mitad, pero aún no ha saltado al destino.
        assert!(s > 0.5 && s < 1.0);
        assert!(medio.loc.x < origen.loc.x / 2);
        assert!(medio.size.w > (origen.size.w + destino.size.w) / 2);
        assert!(medio.size.w < destino.size.w);
        assert!(medio.size.h < destino.size.h);
    }

    #[test]
    fn restaurar_es_la_misma_interpolacion_con_los_extremos_invertidos() {
        let flotante = Rectangle::new((220, 180).into(), (800, 600).into());
        let maxima = Rectangle::new((0, 32).into(), (1920, 1048).into());
        for i in 0..=20 {
            let s = i as f64 / 20.0;
            let ida = interpolar_rect(flotante, maxima, s);
            let vuelta = interpolar_rect(maxima, flotante, 1.0 - s);
            assert_eq!(ida, vuelta, "maximizar y restaurar divergen en {s}");
        }
    }

    #[test]
    fn abrir_recorre_magic_lamp_exactamente_al_reves() {
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let ida = avance_encogido(t, true);
            let vuelta = avance_encogido(1.0 - t, false);
            assert!(
                (ida - vuelta).abs() < 1e-5,
                "los recorridos divergen en t={t}"
            );
        }
        assert_eq!(avance_encogido(0.0, true), 0.0);
        assert_eq!(avance_encogido(1.0, true), 1.0);
        assert_eq!(avance_encogido(0.0, false), 1.0);
        assert_eq!(avance_encogido(1.0, false), 0.0);
    }

    #[test]
    fn la_ventana_se_pasa_de_su_tamano_antes_de_asentarse() {
        // El muelle del sistema de diseño llega a 1,4 en el eje Y, así que la
        // ventana crece hasta un 3 % por encima de su tamaño. Sin ese pico, la
        // aparición se lee como que se despliega en vez de aparecer.
        let pico = (0..=100)
            .map(|i| tema::C_MUELLE.eval(i as f32 / 100.0))
            .fold(0.0f32, f32::max);
        let maximo = ZOOM_INICIAL + (1.0 - ZOOM_INICIAL) * pico as f64;
        assert!(maximo > 1.0 && maximo < 1.05, "pico del muelle: {maximo}");
    }
}
