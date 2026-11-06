//! El sistema de widgets del panel.
//!
//! Un widget es un módulo de este crate compilado dentro del binario, no un
//! proceso aparte como los plasmoides de Plasma. Eso es lo que permite que el
//! panel esté pintado en el primer frame de la sesión: no hay que arrancar
//! nada, ni conectar con nada, ni esperar a que responda. El precio es que
//! añadir un widget exige recompilar, y se paga a gusto.
//!
//! # Cada widget responde por sí mismo
//!
//! Antes el panel comparaba un `PanelData` entero con `PartialEq` para decidir
//! si repintar. Funcionaba, pero significaba que un widget nuevo tenía que
//! meter su campo en esa estructura y que cambiar un dato repintaba el panel
//! completo. Aquí cada widget dice si **lo suyo** cambió ([`Widget::refrescar`]),
//! cuándo quiere despertar ([`Widget::proxima_alarma`]) y qué eventos del
//! kernel le importan ([`Widget::subsistemas`]). El panel solo agrega.
//!
//! Eso último no es cosmético: hoy cualquier evento de udev provocaba releer
//! las cuatro fuentes de sysfs. Con los subsistemas declarados, un cambio de
//! brillo ya no hace mirar el estado de la red.
//!
//! # Un widget que revienta no se lleva el panel
//!
//! El compositor envuelve al shell entero en `catch_unwind`, pero eso es todo o
//! nada: un panic en el reloj dejaba la sesión sin panel **y** sin dock, para
//! siempre. Aquí cada llamada a un widget va en su propio `catch_unwind`; el
//! que revienta se apaga y los demás siguen dibujándose.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use iced_core::Length;
use iced_widget::Space;

use crate::tema;
use crate::view::PanelElement;

pub trait Widget {
    /// Nombre estable: es la clave en la configuración y lo que sale en las
    /// trazas cuando el widget falla.
    // 0.1.0
    fn nombre(&self) -> &'static str;

    /// Relee su fuente. `true` si lo que enseña ha cambiado.
    // 0.1.0
    fn refrescar(&mut self) -> bool;

    /// Dentro de cuánto tiene algo nuevo que enseñar aunque no pase nada.
    /// `None` = solo reacciona a eventos.
    // 0.1.0
    fn proxima_alarma(&self) -> Option<Duration> {
        None
    }

    /// Subsistemas de udev cuyos eventos le conciernen (`power_supply`, `net`,
    /// `backlight`…).
    // 0.1.0
    fn subsistemas(&self) -> &'static [&'static str] {
        &[]
    }

    // 0.1.0
    fn ver(&self) -> PanelElement<'_>;

    /// Cuánto ocupa de ancho, en píxeles lógicos.
    ///
    /// Hace falta para saber **dónde se ha pulsado**: el layout lo calcula iced
    /// al pintar, y el hit-test ocurre antes de eso y desde el compositor, así
    /// que sin esto un clic en el panel solo puede distinguir zonas fijas —era
    /// el motivo de que el brillo, la red y la batería no respondieran.
    ///
    /// Es una estimación, no una medida: se comprueba contra el panel pintado
    /// en `tests/panel.rs`. Un widget que se quede corto pierde clics en su
    /// borde; uno que se pase se los roba al vecino, así que más vale ajustado.
    // 0.1.0
    fn ancho(&self) -> f32 {
        crate::tema::ICONO_PANEL
    }

    /// Qué pasa al pulsar el widget, con `x` relativa a su borde izquierdo.
    ///
    /// `None` —lo normal— significa «no respondo por mi cuenta», y entonces el
    /// panel abre la tarjeta que le corresponda. Lo implementa el indicador de
    /// escritorios, que no tiene tarjeta: cada punto lleva a un sitio distinto,
    /// así que la zona no basta y hace falta el punto exacto.
    // 0.1.0
    fn pulsar(&self, _x: f32) -> Option<crate::Accion> {
        None
    }

    /// En qué escritorio virtual está la sesión, y cuántos hay.
    ///
    /// Va en el trait —y no en un método del widget concreto con un downcast—
    /// porque es un dato que el shell **no puede leer**: lo gobierna el
    /// compositor, igual que las ventanas abiertas del dock. Casi ningún widget
    /// lo quiere, así que por defecto lo ignora y dice que no hay que repintar.
    // 0.1.0
    fn escritorios(&mut self, _activo: usize, _cuantos: usize) -> bool {
        false
    }

    /// Espera a la consulta que se lanzó al construirse, si la hubo.
    ///
    /// Los widgets que dependen de un programa externo —volumen con `wpctl`,
    /// bluetooth con `bluetoothctl`— lanzan su consulta en el constructor y la
    /// recogen aquí. Medido: cada una tarda unos 19 ms, y **esperarlas dentro
    /// del constructor las ponía en fila**: 38 ms antes del primer frame.
    /// Lanzándolas todas y esperando después, los dos procesos corren a la vez
    /// y se paga una sola vez.
    ///
    /// Se sigue esperando —y no se deja para el primer evento— a propósito: el
    /// panel tiene que salir con el icono correcto en el primer frame en vez de
    /// aparecer vacío y llenarse solo.
    // 0.1.0
    fn esperar_arranque(&mut self) {}

    /// Cuántas notificaciones hay sin leer.
    ///
    /// Va en el trait por lo mismo que [`Widget::escritorios`]: es un dato que
    /// el shell no puede leer por su cuenta —llega por D-Bus, y de eso se
    /// encarga el compositor— y solo le interesa a un widget.
    // 0.1.0
    fn notificaciones(&mut self, _cuantas: u32) -> bool {
        false
    }

    /// Si «No molestar» está puesto. Solo le importa a la campana.
    // 0.1.0
    fn no_molestar(&mut self, _activo: bool) -> bool {
        false
    }

    /// Si tiene una animación en marcha y necesita más frames.
    ///
    /// Cada widget mide su propio tiempo porque solo él sabe qué cambió: un
    /// icono que cruza dura 250 ms, el aviso de batería crítica casi dos
    /// segundos. El panel pide frames mientras alguno diga que sí y uno más
    /// después, para que el último que se pinte sea el del estado final.
    // 0.1.0
    fn animando(&self) -> bool {
        false
    }
}

/// El cambio de un icono del panel por otro.
///
/// El viejo encoge a 0,7 y se funde en la primera mitad del recorrido; el
/// nuevo crece desde 0,7 en el recorrido entero. Sustituye al destello de
/// fondo que había antes, que pintaba la ranura entera y se leía como un clic
/// que no había hecho nadie.
// modified 0.2.0 -- muelle crítico en vez de 250 ms con rebote: el cambio no lo trae un gesto
pub struct Cruce {
    previo: Option<(crate::icono::Icono, iced_core::Color)>,
    avance: crate::tema::Transicion,
}

impl Default for Cruce {
    // añadido en la 0.2.0
    fn default() -> Self {
        Self {
            previo: None,
            avance: crate::tema::Transicion::nueva(1.0, crate::tema::M_RAPIDO),
        }
    }
}

impl Cruce {
    /// Empieza el cruce desde `previo`, pintado en `color`. Sin icono previo
    /// —el primer frame— no hay nada de lo que cruzar.
    // 0.1.0
    pub fn empezar(&mut self, previo: Option<crate::icono::Icono>, color: iced_core::Color) {
        if crate::tema::efectos_reducidos() || previo.is_none() {
            *self = Self::default();
            return;
        }
        self.previo = previo.map(|icono| (icono, color));
        self.avance.fijar(0.0);
        self.avance.ir_a(1.0);
    }

    // 0.1.0
    pub fn animando(&self) -> bool {
        self.previo.is_some() && self.avance.animando()
    }

    // 0.1.0
    pub fn ver<'a>(
        &'a self,
        actual: &'a crate::icono::Icono,
        color: iced_core::Color,
    ) -> PanelElement<'a> {
        use crate::tema::ICONO_PANEL;
        let entra = self.avance.valor();
        let Some((previo, color_previo)) = self.previo.as_ref().filter(|_| self.animando()) else {
            return crate::icono::ver_teñido(actual, ICONO_PANEL, Some(color));
        };
        let sale = (entra * 2.0).min(1.0);
        let capa = |icono: &'a crate::icono::Icono, escala: f32, opacidad: f32, color| {
            iced_widget::container(crate::icono::ver_escalado(
                icono,
                ICONO_PANEL * escala,
                opacidad,
                color,
            ))
            .center_x(Length::Fixed(ICONO_PANEL))
            .center_y(Length::Fixed(ICONO_PANEL))
        };
        iced_widget::stack![
            capa(previo, 1.0 - 0.3 * sale, 1.0 - sale, *color_previo),
            capa(actual, 0.7 + 0.3 * entra, entra, color),
        ]
        .into()
    }
}

/// Lo que dura «Desconectado» junto al icono. Lo bastante para leerlo de
/// reojo; quedarse fijo sería ocupar el panel con algo que el icono ya dice.
const D_DESCONECTADO: Duration = Duration::from_secs(3);
const TEXTO_DESCONECTADO: &str = "Desconectado";

/// «Desconectado» al lado del icono durante unos segundos, al perder la
/// conexión. El icono tachado solo se distingue del normal mirándolo: la
/// palabra avisa de que **acaba** de caerse. Lo usan la red y el Bluetooth.
// modified 0.2.0 -- entra y sale abriendo su hueco: antes aparecía de golpe y empujaba a los vecinos
pub struct AvisoDesconexion {
    desde: Option<Instant>,
    visible: crate::tema::Transicion,
}

impl Default for AvisoDesconexion {
    // añadido en la 0.2.0
    fn default() -> Self {
        Self {
            desde: None,
            visible: crate::tema::Transicion::nueva(0.0, crate::tema::M_RAPIDO),
        }
    }
}

impl AvisoDesconexion {
    // 0.1.1
    pub fn empezar(&mut self) {
        self.desde = Some(Instant::now());
        self.visible.ir_a(1.0);
    }

    // añadido en la 0.2.0
    pub fn animando(&self) -> bool {
        self.visible.animando()
    }

    /// Lo quita si ya pasó su tiempo. `true` si lo ha quitado: el panel tiene
    /// que repintarse sin él.
    // 0.1.1
    pub fn caducar(&mut self) -> bool {
        let caducado = self
            .desde
            .take_if(|t| t.elapsed() >= D_DESCONECTADO)
            .is_some();
        if caducado {
            self.visible.ir_a(0.0);
        }
        caducado
    }

    /// Cuándo hay que despertar para quitarlo. Sin esto esperaba al tick del
    /// reloj, hasta un minuto.
    // 0.1.1
    pub fn alarma(&self) -> Option<Duration> {
        self.desde
            .map(|t| D_DESCONECTADO.saturating_sub(t.elapsed()))
    }

    /// Lo que añade al ancho del icono, para que la zona de clic lo cubra.
    /// Es el de ahora, no el final: las zonas se calculan con esto y tienen
    /// que cuadrar con lo dibujado también a mitad de la animación.
    // modified 0.2.0 -- crece y encoge con el aviso
    pub fn ancho(&self) -> f32 {
        let v = self.visible.valor().clamp(0.0, 1.0);
        if v < 0.01 {
            return 0.0;
        }
        (5.0 + ancho_de(TEXTO_DESCONECTADO, crate::tema::T_CUERPO)) * v
    }

    /// El icono, y a su derecha el texto mientras dura. Mismo hueco y cuerpo
    /// que la cifra de la batería.
    // 0.1.1
    // modified 0.2.0 -- recortado al ancho animado y fundido
    pub fn ver<'a>(&self, icono: PanelElement<'a>) -> PanelElement<'a> {
        let ancho = self.ancho();
        if ancho == 0.0 {
            return icono;
        }
        let mut color = crate::tema::TEXTO2;
        color.a *= self.visible.valor().clamp(0.0, 1.0);
        iced_widget::row![
            icono,
            iced_widget::container(
                iced_widget::row![
                    iced_widget::Space::new().width(Length::Fixed(5.0)),
                    iced_widget::text(TEXTO_DESCONECTADO)
                        .size(crate::tema::T_CUERPO)
                        .color(color)
                        .wrapping(iced_core::text::Wrapping::None),
                ]
                .align_y(iced_core::alignment::Vertical::Center),
            )
            .width(Length::Fixed(ancho))
            .clip(true)
        ]
        .align_y(iced_core::alignment::Vertical::Center)
        .into()
    }
}

/// Ancho aproximado de un texto del panel, en lógicos, contando caracteres.
///
/// Se conserva para las medidas que no tienen el texto a mano —el recorte de
/// nombres largos en las listas— pero **no** para las zonas del panel: ahí la
/// aproximación desalineaba los clics. Ver [`ancho_de`].
// 0.1.0
pub fn ancho_texto(caracteres: usize) -> f32 {
    caracteres as f32 * 7.3
}

/// Lo que mide de verdad un texto del panel, en lógicos.
///
/// Antes esto se estimaba a 7,3 px por carácter y las zonas de clic no
/// cuadraban con lo dibujado: medido sobre el panel pintado, «16/8/26 18:49»
/// ocupa 73 px reales y la cuenta daba 95. El error se acumulaba de derecha a
/// izquierda —cada zona se coloca a partir del ancho de la anterior—, y para
/// cuando llegaba a la batería el dibujo estaba 10 px fuera de su zona: de ahí
/// que a veces un clic no abriera nada.
///
/// Mide con el mismo `Paragraph` de `iced_graphics` que usa el renderer al
/// pintar, con la fuente y el cuerpo por defecto del shell, así que no es una
/// estimación paralela: es el mismo motor.
// 0.1.0
pub fn ancho_de(texto: &str, tamaño: f32) -> f32 {
    ancho_con_fuente(texto, tamaño, iced_core::Font::DEFAULT)
}

/// Lo mismo con otra fuente: un texto en seminegrita ocupa más, y medirlo con
/// la regular lo dejaría recortado por la derecha.
// 0.1.0
pub fn ancho_con_fuente(texto: &str, tamaño: f32, fuente: iced_core::Font) -> f32 {
    use iced_core::text::{Paragraph as _, Shaping, Wrapping};
    use iced_core::{Pixels, Size, alignment};

    let parrafo = iced_graphics::text::Paragraph::with_text(iced_core::Text {
        content: texto,
        // Sin límite: lo que se quiere es el ancho natural de una línea.
        bounds: Size::INFINITE,
        size: Pixels(tamaño),
        line_height: iced_core::text::LineHeight::default(),
        font: fuente,
        align_x: iced_core::text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        // La misma que usa `text()` de iced_widget por defecto.
        shaping: Shaping::Basic,
        wrapping: Wrapping::None,
    });
    parrafo.min_bounds().width
}

/// Qué widget tiene que reventar, si `BOOKOS_SHELL_PANIC_TEST` lo pide.
///
/// Se lee del entorno una sola vez: esto se consulta en cada refresco de cada
/// widget, y `getenv` en ese camino sería trabajo por nada.
// 0.1.0
fn panic_de_prueba() -> Option<&'static str> {
    static QUIEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    QUIEN
        .get_or_init(|| std::env::var("BOOKOS_SHELL_PANIC_TEST").ok())
        .as_deref()
}

/// Un widget y su estado de salud.
struct Ranura {
    widget: Box<dyn Widget>,
    /// Un widget que ha entrado en pánico no se vuelve a llamar. Reintentarlo
    /// repetiría el fallo en cada frame y llenaría el log.
    muerto: bool,
    /// Si en el último frame animaba. Mantiene vivo el bucle un frame más
    /// cuando el widget termina, para que ese frame pinte el estado final y no
    /// el penúltimo.
    animaba: bool,
}

impl Ranura {
    // 0.1.0
    fn new(widget: Box<dyn Widget>) -> Self {
        Self {
            widget,
            muerto: false,
            animaba: false,
        }
    }

    /// Ejecuta `f` sobre el widget, apagándolo si revienta.
    ///
    /// `AssertUnwindSafe` es correcto porque el único estado que puede quedar a
    /// medias es el del propio widget, y al marcarlo muerto no se vuelve a
    /// tocar.
    // 0.1.0
    fn guard<T>(&mut self, que: &str, f: impl FnOnce(&mut Box<dyn Widget>) -> T) -> Option<T> {
        if self.muerto {
            return None;
        }
        let nombre = self.widget.nombre();
        match catch_unwind(AssertUnwindSafe(|| {
            // Escotilla para comprobar que el aislamiento **por widget**
            // funciona de verdad. Sin esto, "un widget que revienta no se lleva
            // el panel" es una afirmación sin probar.
            if panic_de_prueba() == Some(nombre) {
                panic!("panic de prueba del widget {nombre}");
            }
            f(&mut self.widget)
        })) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::error!(
                    widget = self.widget.nombre(),
                    que,
                    "el widget ha entrado en pánico; se apaga y el panel sigue"
                );
                self.muerto = true;
                None
            }
        }
    }
}

/// Los widgets del panel, repartidos por zonas.
///
/// Solo hay dos: el centro, que es una capa aparte, y la derecha, que es una
/// fila. El centro está separado porque tiene que estar centrado en la
/// **pantalla** y no entre sus vecinos: metido en la fila, el reloj daba un
/// salto cada vez que la batería pasaba de "85%" a "85% 2:15".
pub struct Panel {
    centro: Option<Ranura>,
    derecha: Vec<Ranura>,
    /// El widget bajo el puntero, por su sitio en `derecha`. Con `Realce`
    /// el que se deja se apaga fundiéndose en vez de desaparecer.
    // modified 0.2.0 -- realce por índice que se funde al entrar y al salir
    señalado: crate::tema::Realce,
    /// El widget cuya tarjeta está abierta. También se funde: al cerrar la
    /// tarjeta el acento se apagaba en seco.
    // modified 0.2.0
    abierto: crate::tema::Realce,
}

impl Panel {
    // 0.1.0
    pub fn new(centro: Option<Box<dyn Widget>>, derecha: Vec<Box<dyn Widget>>) -> Self {
        let mut panel = Self {
            centro: centro.map(Ranura::new),
            derecha: derecha.into_iter().map(Ranura::new).collect(),
            señalado: crate::tema::Realce::nuevo(),
            abierto: crate::tema::Realce::nuevo(),
        };
        // Ya están todos construidos, o sea que las consultas que hayan lanzado
        // están corriendo a la vez: aquí se recogen. Ver
        // [`Widget::esperar_arranque`].
        for ranura in panel.ranuras_mut() {
            ranura.guard("arranque", |w| w.esperar_arranque());
        }
        panel
    }

    /// Relee todos los widgets. `true` si alguno cambió.
    ///
    /// Se refrescan **todos** aunque uno ya haya dicho que sí: saltarse el
    /// resto los dejaría con el dato viejo y el siguiente repintado enseñaría
    /// una mezcla de dos instantes.
    // 0.1.0
    pub fn refrescar(&mut self) -> bool {
        let mut cambio = false;
        for ranura in self.ranuras_mut() {
            if ranura.guard("refrescar", |w| w.refrescar()) == Some(true) {
                cambio = true;
            }
        }
        cambio
    }

    /// Conserva el último frame pendiente hasta pintarlo, incluso si el bucle
    /// estuvo parado más que la animación. Nunca consulta hardware por frame.
    // 0.1.0
    pub fn animando(&self) -> bool {
        self.ranuras()
            .any(|r| !r.muerto && (r.animaba || r.widget.animando()))
            || self.resalte_animando()
    }

    // 0.1.0
    pub fn avanzar(&mut self) {
        for r in self.ranuras_mut() {
            r.animaba = !r.muerto && r.widget.animando();
        }
    }

    // 0.1.0
    // modified 0.2.0
    fn resalte_animando(&self) -> bool {
        self.señalado.animando() || self.abierto.animando()
    }

    // añadido en la 0.2.0
    fn indice(&self, nombre: Option<&'static str>) -> Option<usize> {
        let nombre = nombre?;
        self.derecha
            .iter()
            .position(|r| r.widget.nombre() == nombre)
    }

    /// El widget que hay bajo el puntero, o `None` si no hay ninguno. `true`
    /// si hay que repintar.
    // 0.1.0
    pub fn señalar(&mut self, nombre: Option<&'static str>) -> bool {
        let i = self.indice(nombre);
        self.señalado.señalar(i)
    }

    /// El widget cuya tarjeta está abierta. `true` si hay que repintar.
    // 0.1.0
    pub fn abrir(&mut self, nombre: Option<&'static str>) -> bool {
        let i = self.indice(nombre);
        self.abierto.señalar(i)
    }

    /// Lo antes que alguno quiere despertar.
    // 0.1.0
    pub fn proxima_alarma(&self) -> Option<Duration> {
        self.ranuras()
            .filter(|r| !r.muerto)
            .filter_map(|r| r.widget.proxima_alarma())
            .min()
    }

    /// Todos los subsistemas que hay que vigilar, sin repetir.
    ///
    /// Con esto el filtro del netlink se pone una vez al arrancar y el kernel
    /// ya no nos despierta por lo que no se enseña; por eso no hace falta
    /// preguntar después qué widget quería cada evento.
    // 0.1.0
    pub fn subsistemas(&self) -> Vec<&'static str> {
        let mut todos: Vec<&'static str> = self
            .ranuras()
            .flat_map(|r| r.widget.subsistemas())
            .copied()
            .collect();
        todos.sort_unstable();
        todos.dedup();
        todos
    }

    /// ¿Está este widget en la capa central?
    /// Dónde cae cada widget de la derecha, en lógicos: `(nombre, x0, x1)`.
    ///
    /// Se calcula de derecha a izquierda porque esa fila está anclada al borde
    /// derecho: el primero de la lista es el que queda más a la izquierda, pero
    /// el que tiene la posición fija es el último.
    // 0.1.0
    pub fn zonas_derecha(
        &self,
        ancho_panel: f32,
        margen: f32,
        hueco: f32,
    ) -> Vec<(&'static str, f32, f32)> {
        let mut zonas = Vec::new();
        let mut x = ancho_panel - margen;
        // Un widget muerto no se dibuja, así que tampoco ocupa sitio ni puede
        // recibir clics: se salta entero, huecos incluidos.
        for ranura in self.derecha.iter().rev().filter(|r| !r.muerto) {
            let ancho = ranura.widget.ancho();
            // Un widget sin nada que enseñar —el bluetooth sin adaptador, la
            // batería en un sobremesa— no se dibuja, así que ni ocupa sitio ni
            // recibe clics: se salta con su hueco. Sin esto quedaba una zona
            // de ancho cero que se comía los clics del vecino.
            if ancho <= 0.0 {
                continue;
            }
            // Cada zona se lleva medio hueco por cada lado. El aire entre
            // widgets es de 12 px y no era de nadie: pulsar ahí no abría nada, y
            // apuntar a un icono de 18 px en una barra de 32 con el ratón en
            // movimiento falla más de lo que parece. Repartido, todo el lado
            // derecho del panel responde y ningún widget se roba el dibujo del
            // vecino.
            zonas.push((
                ranura.widget.nombre(),
                x - ancho - hueco / 2.0,
                x + hueco / 2.0,
            ));
            x -= ancho + hueco;
        }
        zonas
    }

    /// Lo que devuelve pulsar el widget `nombre` en la `x` relativa que se le
    /// da. `None` si ese widget no responde por su cuenta.
    // 0.1.0
    pub fn pulsar(&mut self, nombre: &str, x: f32) -> Option<crate::Accion> {
        let ranura = self
            .ranuras_mut()
            .find(|r| !r.muerto && r.widget.nombre() == nombre)?;
        ranura.guard("pulsar", |w| w.pulsar(x)).flatten()
    }

    /// Reparte el número de notificaciones sin leer. `true` si hay que
    /// repintar.
    // 0.1.0
    pub fn notificaciones(&mut self, cuantas: u32) -> bool {
        let mut cambio = false;
        for ranura in self.ranuras_mut() {
            if let Some(si) = ranura.guard("notificaciones", |w| w.notificaciones(cuantas)) {
                cambio |= si;
            }
        }
        cambio
    }

    /// Reparte si «No molestar» está puesto. `true` si hay que repintar.
    // 0.1.0
    pub fn no_molestar(&mut self, activo: bool) -> bool {
        let mut cambio = false;
        for ranura in self.ranuras_mut() {
            if let Some(si) = ranura.guard("no molestar", |w| w.no_molestar(activo)) {
                cambio |= si;
            }
        }
        cambio
    }

    /// Reparte el escritorio activo entre los widgets que lo quieran. `true` si
    /// alguno pide repintar.
    // 0.1.0
    pub fn escritorios(&mut self, activo: usize, cuantos: usize) -> bool {
        let mut cambio = false;
        for ranura in self.ranuras_mut() {
            if let Some(si) = ranura.guard("escritorios", |w| w.escritorios(activo, cuantos)) {
                cambio |= si;
            }
        }
        cambio
    }

    // 0.1.0
    pub fn centro(&self) -> Option<PanelElement<'_>> {
        self.centro.as_ref().and_then(|r| ver(r, 0.0))
    }

    /// Los que de verdad se dibujan, en orden.
    ///
    /// Los de ancho cero se quedan fuera —y no como un elemento vacío— porque
    /// la fila de [`crate::view::panel`] mete un hueco **entre cada dos
    /// elementos** que recibe. Un widget invisible (la red o el bluetooth sin
    /// adaptador, la batería en un sobremesa) colaba así su hueco de 16 px en
    /// el dibujo, mientras [`Panel::zonas_derecha`] lo saltaba entero. Las
    /// zonas quedaban corridas 16 px por cada invisible respecto de lo pintado,
    /// y como se calculan de derecha a izquierda el error caía sobre los de la
    /// izquierda: con red y bluetooth apagados, pulsar en el hueco vacío a la
    /// derecha del porcentaje abría la tarjeta de la batería.
    // 0.1.0
    pub fn derecha(&self) -> Vec<PanelElement<'_>> {
        self.derecha
            .iter()
            .enumerate()
            .filter(|(_, r)| r.widget.ancho() > 0.0)
            .filter_map(|(i, r)| {
                // El acento de la abierta tapa el velo del ratón según entra:
                // los dos a la vez se sumaban en un tono que no es ninguno.
                let abierto = self.abierto.intensidad(i);
                let hover = tema::hover();
                let acento = tema::acento();
                let a_hover = hover.a * self.señalado.intensidad(i) * (1.0 - abierto);
                let a_acento = 0.14 * abierto;
                let a = a_hover + a_acento;
                let resalte = if a <= 0.0 {
                    iced_core::Color::TRANSPARENT
                } else {
                    let mezcla = |h: f32, c: f32| (h * a_hover + c * a_acento) / a;
                    iced_core::Color {
                        r: mezcla(hover.r, acento.r),
                        g: mezcla(hover.g, acento.g),
                        b: mezcla(hover.b, acento.b),
                        a,
                    }
                };
                ver(r, crate::view::HUECO / 2.0).map(|elemento| {
                    iced_widget::container(elemento)
                        .center_y(Length::Fixed(ALTO_RESALTE))
                        .style(move |_| iced_widget::container::Style {
                            background: Some(resalte.into()),
                            border: iced_core::Border {
                                radius: RADIO_RESALTE.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into()
                })
            })
            .collect()
    }

    // 0.1.0
    fn ranuras(&self) -> impl Iterator<Item = &Ranura> {
        self.centro.iter().chain(self.derecha.iter())
    }

    // 0.1.0
    fn ranuras_mut(&mut self) -> impl Iterator<Item = &mut Ranura> {
        self.centro.iter_mut().chain(self.derecha.iter_mut())
    }
}

/// La vista de una ranura, o nada si está muerta.
///
/// Aquí no se puede usar `Ranura::guard` porque `ver` toma `&self` y devolver
/// el `Element` prestado desde dentro del `catch_unwind` obligaría a marcar el
/// fallo con mutabilidad interior. Un widget que dibuja mal es raro y ya
/// reventaría en `refrescar`; lo que sí se respeta es no dibujar al que ya
/// está apagado.
// 0.1.0
fn ver(ranura: &Ranura, margen: f32) -> Option<PanelElement<'_>> {
    if ranura.muerto {
        return None;
    }
    Some(
        iced_widget::container(ranura.widget.ver())
            .padding(iced_core::Padding::ZERO.left(margen).right(margen))
            .into(),
    )
}

/// Alto del resalte de un widget: 28 en un panel de 32 deja 2 px arriba y
/// abajo, y la píldora no toca los bordes.
const ALTO_RESALTE: f32 = 28.0;
/// El radio de «hover de iconos pequeños» del sistema de diseño.
const RADIO_RESALTE: f32 = 8.0;

/// Un hueco de ancho cero, para cuando no hay nada que enseñar.
// 0.1.0
pub(crate) fn vacio<'a>() -> PanelElement<'a> {
    Space::new().width(Length::Fixed(0.0)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bomba;
    impl Widget for Bomba {
        fn nombre(&self) -> &'static str {
            "bomba"
        }
        fn refrescar(&mut self) -> bool {
            panic!("bum")
        }
        fn ver(&self) -> PanelElement<'_> {
            vacio()
        }
    }

    /// Dice que cambió las `veces` primeras veces que le preguntan.
    struct Cambiante(u32);
    impl Widget for Cambiante {
        fn nombre(&self) -> &'static str {
            "cambiante"
        }
        fn refrescar(&mut self) -> bool {
            self.0 = self.0.saturating_sub(1);
            self.0 > 0
        }
        fn proxima_alarma(&self) -> Option<Duration> {
            Some(Duration::from_secs(7))
        }
        fn subsistemas(&self) -> &'static [&'static str] {
            &["net"]
        }
        fn ver(&self) -> PanelElement<'_> {
            vacio()
        }
    }

    /// El aislamiento por widget es la razón de ser de [`Ranura`]: si esto se
    /// rompe, un `unwrap` en cualquier widget vuelve a apagar el panel entero.
    #[test]
    fn el_widget_que_revienta_no_se_lleva_a_los_demas() {
        // El panic de la bomba es esperado; sin esto llena la salida del test
        // con un backtrace que parece un fallo de verdad.
        let anterior = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let mut panel = Panel::new(Some(Box::new(Bomba)), vec![Box::new(Cambiante(3))]);
        assert!(panel.refrescar(), "el widget sano sí ha cambiado");
        assert!(
            panel.centro().is_none(),
            "el widget muerto se sigue dibujando"
        );
        // Y no se le vuelve a llamar: si se le llamara, volvería a reventar y
        // este segundo refresco no llegaría a devolver nada.
        panel.refrescar();

        std::panic::set_hook(anterior);
    }

    #[test]
    fn el_muerto_no_cuenta_para_alarmas_ni_subsistemas() {
        let anterior = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let mut panel = Panel::new(Some(Box::new(Bomba)), vec![Box::new(Cambiante(3))]);
        panel.refrescar();
        assert_eq!(panel.proxima_alarma(), Some(Duration::from_secs(7)));
        assert_eq!(panel.subsistemas(), vec!["net"]);

        std::panic::set_hook(anterior);
    }

    /// Un widget de los que no se dibujan: la red o el bluetooth sin adaptador,
    /// la batería en un sobremesa.
    struct Invisible;
    impl Widget for Invisible {
        fn nombre(&self) -> &'static str {
            "invisible"
        }
        fn refrescar(&mut self) -> bool {
            false
        }
        fn ancho(&self) -> f32 {
            0.0
        }
        fn ver(&self) -> PanelElement<'_> {
            vacio()
        }
    }

    /// Lo que se dibuja y lo que se puede pulsar tienen que ser lo mismo.
    ///
    /// La fila del panel mete un hueco entre cada dos elementos que recibe, así
    /// que un invisible que llegara hasta ella colaría 16 px en el dibujo que
    /// `zonas_derecha` no cuenta. Como las zonas se calculan de derecha a
    /// izquierda, ese desfase lo pagan los widgets de la izquierda: con la red
    /// y el bluetooth apagados, pulsar el hueco vacío a la derecha del
    /// porcentaje de la batería abría la tarjeta de la batería.
    #[test]
    fn un_widget_invisible_no_deja_hueco_ni_corre_las_zonas() {
        let panel = Panel::new(
            None,
            vec![
                Box::new(Cambiante(1)),
                Box::new(Invisible),
                Box::new(Invisible),
                Box::new(Cambiante(1)),
            ],
        );
        assert_eq!(
            panel.derecha().len(),
            panel.zonas_derecha(1000.0, 12.0, 16.0).len(),
            "se dibujan más elementos de los que reciben zona: cada uno de más \
             mete su hueco y corre las zonas de sus vecinos de la izquierda"
        );
    }

    #[test]
    fn la_alarma_es_la_del_mas_impaciente() {
        let panel = Panel::new(None, vec![Box::new(Cambiante(1)), Box::new(Bomba)]);
        // La bomba no pide alarma; el cambiante pide 7 s.
        assert_eq!(panel.proxima_alarma(), Some(Duration::from_secs(7)));
    }

    #[test]
    fn el_ultimo_frame_de_la_animacion_no_deja_un_bucle_activo() {
        let mut panel = Panel::new(None, vec![Box::new(Cambiante(1))]);
        // El widget ya terminó, pero en el frame anterior animaba: queda uno.
        panel.derecha[0].animaba = true;
        assert!(panel.animando());
        panel.avanzar();
        assert!(!panel.animando());
        assert!(
            !panel.refrescar(),
            "no hay datos nuevos ni animación perpetua"
        );
    }
}
