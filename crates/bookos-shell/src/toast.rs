//! El aviso que asoma arriba a la derecha cuando llega una notificación.
//!
//! Es la otra mitad de las notificaciones: la tarjeta del panel guarda lo que
//! ha pasado, y esto es lo que se ve **mientras** pasa. Sin él, `notify-send`
//! parece no hacer nada hasta que abres la campana.
//!
//! Comparte forma con [`crate::osd`] —una superficie temporal que entra, se
//! queda y se va sola, sin repintarse mientras está quieta— pero no comparte
//! código: el aviso de volumen es una cápsula centrada abajo con una barra, y
//! este es una tarjeta arriba a la derecha con tres renglones. Lo único que de
//! verdad tienen igual son las curvas, y esas ya están en [`crate::tema`].

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, stack, text};

use crate::notificaciones::{Forma, Notificacion};
use crate::tema;
use crate::view::PanelElement;

/// Ancho de la tarjeta. Más estrecha que las del panel: es un aviso de paso, no
/// algo que se lea entero.
const ANCHO: f32 = 360.0;
const MARGEN: f32 = 12.0;
/// Lado del icono de la aplicación.
const ICONO: f32 = 36.0;
/// Aire entre la cabecera, el cuerpo y los botones.
const RESPIRO: f32 = 8.0;
/// Alto de una línea del cuerpo, a 13 px, y de la barra de progreso con su aire.
const LINEA_CUERPO: f32 = 17.0;
/// La app a 12 px y el resumen a 14, con el interlineado de 1,3 de iced.
// añadido en la 0.2.0
const ALTO_TITULOS: f32 = 36.0;
/// Alto de la línea de la app (y de la hora) y del resumen, y el 1 px entre ambas.
// añadido en la 0.2.2
const LINEA_APP: f32 = 17.0;
const LINEA_RESUMEN: f32 = 18.0;
/// Lo que se reserva a la hora, a 11,5 px, junto al nombre de la app.
const ANCHO_HORA: f32 = 72.0;
/// Separación entre el contenido y los botones desplegados.
const SEPARACION_BOTONES: f32 = 10.0;
/// El cuerpo va en la columna del resumen, a la derecha del icono. Puede ser
/// más ancho que los títulos porque empieza por debajo de la ✕.
// añadido en la 0.2.0
const ANCHO_CUERPO: f32 = ANCHO - MARGEN * 2.0 - ICONO - 12.0;
const ALTO_PROGRESO: f32 = 4.0 + RESPIRO;
/// Hueco para que el desenfoque y su desplazamiento quepan dentro del buffer.
pub const MARGEN_SOMBRA: f32 = 24.0;
/// Separación desde el borde derecho y desde el panel.
pub const MARGEN_LATERAL: f32 = 12.0;
pub const MARGEN_SUPERIOR: f32 = 8.0;
/// Cuántos avisos caben a la vez. Con más, la pila baja hasta media pantalla y
/// tapa lo que se está haciendo; los que no caben siguen en la tarjeta de la
/// campana.
pub const MAXIMO_A_LA_VISTA: usize = 3;
const ALTO_BOTON: f32 = 30.0;
/// La confirmación: el sello de 24 px y, si lleva lo enviado, dos líneas.
// añadido en la 0.2.2
const ALTO_HECHO: f32 = 36.0;
/// Lo que se queda a la vista la confirmación; más si hay un texto que leer.
// añadido en la 0.2.2
const QUIETO_HECHO: Duration = Duration::from_millis(1200);
const QUIETO_HECHO_ENVIADO: Duration = Duration::from_millis(1600);
const HUECO_BOTONES: f32 = 6.0;
/// Los aros de los Buds: el aro, 5 de aire y el porcentaje, más el aire que
/// los separa de la cabecera.
// añadido en la 0.2.2
const ARO: f32 = 52.0;
const ALTO_AROS: f32 = MARGEN + ARO + 5.0 + 17.0;
/// La barra de batería baja con su aire.
// añadido en la 0.2.2
const ALTO_BARRA_BATERIA: f32 = RESPIRO + 16.0;
/// Cuánto asoma por debajo la tarjeta de cada nivel de la pila. Se ve la
/// franja y no el contenido: las de atrás son tarjetas vacías, que es lo que
/// las hace leer como «hay más» y no como un aviso partido en dos.
pub const ASOMO: f32 = 10.0;
/// Cuánto más estrecha es cada nivel que el de delante.
const ENCOGE_POR_NIVEL: f32 = 0.05;
/// El círculo de la ✕ y su distancia a la esquina de la tarjeta.
const CIERRE: f32 = 22.0;
const CIERRE_BORDE: f32 = 10.0;

/// Lo que se queda a la vista una normal, y una crítica.
///
/// Cinco segundos es lo que tarda en leerse un renglón y medio sin prisa; el
/// doble para las críticas, que son las que no conviene perderse. Una
/// aplicación puede pedir otra cosa con `expire_timeout`, y se le respeta
/// dentro de estos límites: hay programas que piden treinta segundos, y un
/// aviso que no se va en medio minuto deja de ser un aviso.
const QUIETO: Duration = Duration::from_secs(5);
const QUIETO_CRITICA: Duration = Duration::from_secs(10);
const MINIMO: Duration = Duration::from_secs(2);
const MAXIMO: Duration = Duration::from_secs(15);
/// Lo que tarda en entrar y en irse. Los mismos tokens que el resto.
const ENTRADA: Duration = tema::D_TARJETA;
pub const SALIDA: Duration = tema::D_POPOVER;
const RECORRIDO: f32 = 48.0;

pub struct Toast {
    notificacion: Notificacion,
    /// El cuerpo ya partido en una o dos líneas. Se parte una vez porque el
    /// alto de la tarjeta depende de cuántas salen y se pregunta cada
    /// fotograma; reservar siempre dos dejaba un renglón vacío debajo de los
    /// avisos cortos, que son casi todos.
    cuerpo: String,
    /// Cuánto se queda quieto antes de empezar a irse.
    quieto: Duration,
    desde: Instant,
    /// Alfa, escala y posición al despedirlo. Conserva el punto de entrada si
    /// se pulsa antes de aterrizar, en vez de saltar a una tarjeta ya asentada.
    partida: Option<(f32, f32, f32)>,
    /// Cuántos avisos tiene delante: 0 es el que se lee y se pulsa. Lo pone
    /// [`crate::Shell`] al reordenar la pila.
    profundidad: usize,
    /// Alto impuesto a los de atrás: el de la cabeza. Con el suyo propio una
    /// tarjeta de atrás más alta que la de delante asomaría por arriba.
    fijo: Option<f32>,
    /// Cuánto se ven los botones: 0 recogidos, 1 a la vista. Se despliegan
    /// con el puntero encima, como en macOS: así el aviso en reposo es solo
    /// lo que dice, y las acciones aparecen cuando vas a usarlas.
    // añadido en la 0.2.2
    desplegado: tema::Transicion,
    /// Qué botón (o la ✕, [`CIERRE_IDX`]) tiene el puntero encima.
    // añadido en la 0.2.2
    realce: tema::Realce,
    /// Cuánto se ve la ✕ (y se va la hora). Va aparte de `desplegado`: en el
    /// diseño la ✕ sale al instante y los botones esperan 150 ms.
    // añadido en la 0.2.2
    cierre: tema::Transicion,
    /// Cuándo se despliegan los botones tras señalar el aviso. Pasar el
    /// puntero de camino a otra cosa no debe hacer saltar la tarjeta.
    // añadido en la 0.2.2
    desplegar_a: Option<Instant>,
    /// Lo que lleva arrastrado a mano, en lógicos y tal cual lo movió el
    /// puntero; la goma (hacia la izquierda y hacia abajo) se aplica al leerlo.
    // añadido en la 0.2.2
    arrastre: Option<(f32, f32)>,
    /// Lo escrito en el campo de respuesta; `None` si no se está respondiendo.
    /// Sustituye a los botones, como en el diseño.
    // añadido en la 0.2.2
    respuesta: Option<String>,
    /// Lo que se hizo desde el aviso: el texto de la confirmación y, si fue
    /// una respuesta, lo que se mandó. Sustituye al contenido durante lo que
    /// tarda en leerse, como en el diseño: sin esto el aviso desaparecía de
    /// golpe y no se sabía si la acción había salido.
    // añadido en la 0.2.2
    hecho: Option<(String, Option<String>)>,
}

/// El campo de respuesta y su botón de enviar, que son circulares.
// añadido en la 0.2.2
const ALTO_CAMPO: f32 = 32.0;
const ENVIAR_IDX: usize = usize::MAX - 1;

/// Qué hizo una tecla en el campo de respuesta.
// añadido en la 0.2.2
#[derive(Debug, PartialEq, Eq)]
pub enum TeclaAviso {
    Consumida,
    /// Intro con algo escrito: el texto, recortado.
    Enviar(String),
}

/// Lo que hay que arrastrar a la derecha para quitarlo, y hacia arriba para
/// guardarlo en el centro. Los valores del diseño.
// añadido en la 0.2.2
pub const UMBRAL_QUITAR: f32 = 140.0;
pub const UMBRAL_GUARDAR: f32 = -50.0;

/// Lo que espera el aviso señalado antes de enseñar sus botones.
// añadido en la 0.2.2
const RETRASO_BOTONES: Duration = Duration::from_millis(150);

/// Índice del realce para la ✕; los botones usan su posición.
const CIERRE_IDX: usize = usize::MAX;

/// Lo que queda a la vista un aviso después de que el puntero lo deje: lo
/// justo para volver a él si se fue sin querer.
// añadido en la 0.2.2
const TRAS_DEJARLO: Duration = Duration::from_secs(2);

/// Qué quiso hacer un arrastre al soltarlo.
// añadido en la 0.2.2
#[derive(Debug, PartialEq, Eq)]
pub enum Soltado {
    /// A la derecha: quitarlo también de la lista.
    Quitar,
    /// Arriba: que se vaya pero se quede en el centro de notificaciones.
    Guardar,
}

/// Lo que se ha pulsado dentro de un aviso.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulsado {
    /// El botón de una acción, con su clave.
    Boton(String),
    /// La ✕: quitarlo también de la lista.
    Cerrar,
    /// «Responder»: no se manda nada todavía, el aviso abre su campo.
    Responder,
    /// La flecha del campo, con lo escrito.
    Enviar(String),
}

impl Toast {
    /// Lo que avisa, para anunciarlo al lector de pantalla.
    // añadido en la 0.2.2
    pub fn notificacion(&self) -> &Notificacion {
        &self.notificacion
    }

    /// Vuelve a resolver el icono del aviso si procede del tema de iconos.
    pub fn recargar_icono(&mut self) {
        self.notificacion.recargar_icono();
    }

    /// `caducidad` es el `expire_timeout` de D-Bus: negativo «lo que decida el
    /// servidor», cero «que no se vaya sola» y positivo, milisegundos.
    // modified 0.1.1 -- parte el cuerpo una vez para el alto
    pub fn new(notificacion: Notificacion, caducidad: i32) -> Self {
        let por_defecto = if notificacion.critica {
            QUIETO_CRITICA
        } else {
            QUIETO
        };
        let quieto = match caducidad {
            // El «que no se vaya» se atiende como el máximo y no como para
            // siempre: un aviso clavado en la esquina que solo se quita
            // pulsándolo es lo que hace que la gente desactive las
            // notificaciones. Lo que pasó sigue en la tarjeta del panel.
            0 => MAXIMO,
            ms if ms > 0 => Duration::from_millis(ms as u64).clamp(MINIMO, MAXIMO),
            _ => por_defecto,
        };
        // modificado en la 0.2.0 -- se parte al ancho de la columna del resumen
        let cuerpo = crate::escritorio::dos_lineas(&notificacion.cuerpo, ANCHO_CUERPO, 13.0);
        Self {
            notificacion,
            cuerpo,
            quieto,
            desde: Instant::now(),
            partida: None,
            profundidad: 0,
            fijo: None,
            desplegado: tema::Transicion::nueva(0.0, tema::M_SUAVE),
            realce: tema::Realce::nuevo(),
            cierre: tema::Transicion::nueva(0.0, tema::M_RAPIDO),
            desplegar_a: None,
            arrastre: None,
            respuesta: None,
            hecho: None,
        }
    }

    // 0.1.0
    pub fn profundidad(&self) -> usize {
        self.profundidad
    }

    // 0.1.0
    pub fn id(&self) -> u32 {
        self.notificacion.id
    }

    // 0.1.1
    pub fn app(&self) -> &str {
        &self.notificacion.app
    }

    // 0.1.1
    pub fn critica(&self) -> bool {
        self.notificacion.critica
    }

    /// Cuánto queda para que desaparezca del todo. `None` si ya se fue.
    // 0.1.0
    pub fn queda(&self) -> Option<Duration> {
        (self.quieto + SALIDA).checked_sub(self.desde.elapsed())
    }

    /// Lo empuja a irse ya: es lo que hace pulsarlo.
    // 0.1.0
    // modified 0.1.1 -- sale desde donde estaba si se descarta a mitad de la entrada
    pub fn descartar(&mut self) {
        if self.saliendo() {
            return;
        }
        self.partida = Some((
            self.alfa(),
            self.escala() / self.encogido(),
            self.desplazamiento_horizontal(),
        ));
        // Se le deja la salida por delante en vez de quitarlo de golpe, para
        // que se vaya con la misma animación que si hubiera caducado.
        self.desde = Instant::now() - self.quieto;
    }

    // 0.1.0
    // modified 0.1.1 -- la salida parte del alfa que tenía
    pub fn alfa(&self) -> f32 {
        let t = self.desde.elapsed();
        if t < self.quieto {
            return tema::C_ENTRADA.eval(tema::fraccion(t, ENTRADA)) * self.palidez_arrastre();
        }
        self.partida.map_or(1.0, |p| p.0) * (1.0 - self.avance_salida())
    }

    /// Se aclara al arrastrarlo a la derecha, hasta el 30 %.
    // añadido en la 0.2.2
    fn palidez_arrastre(&self) -> f32 {
        match self.arrastre {
            Some((dx, dy)) if dx > 0.0 && dx.abs() >= dy.abs() => (1.0 - dx / 360.0).max(0.3),
            _ => 1.0,
        }
    }

    /// El puntero arrastra el aviso: `dx`, `dy` desde donde se pulsó.
    // añadido en la 0.2.2
    pub fn arrastrar(&mut self, dx: f32, dy: f32) {
        self.arrastre = Some((dx, dy));
    }

    /// Suelta lo arrastrado y devuelve qué quería el gesto, con `vx`, `vy`
    /// (px/s) para proyectar el final como el diseño: posición + velocidad × 200 ms.
    // añadido en la 0.2.2
    pub fn soltar_arrastre(&mut self, vx: f32, vy: f32) -> Option<Soltado> {
        let (dx, dy) = self.arrastre.take()?;
        let (fx, fy) = (dx + vx * 0.2, dy + vy * 0.2);
        if dx.abs() > dy.abs() && fx > UMBRAL_QUITAR {
            // Se va desde donde está, no desde la posición de reposo.
            self.arrastre = Some((dx, dy));
            self.descartar();
            self.arrastre = None;
            Some(Soltado::Quitar)
        } else if dy.abs() > dx.abs() && fy < UMBRAL_GUARDAR {
            Some(Soltado::Guardar)
        } else {
            None
        }
    }

    // 0.1.1
    fn avance_salida(&self) -> f32 {
        let t = self.desde.elapsed().saturating_sub(self.quieto);
        1.0 - tema::C_ENTRADA.eval(1.0 - tema::fraccion(t, SALIDA))
    }

    // 0.1.1
    pub fn desplazamiento_horizontal(&self) -> f32 {
        // Seguir a la mano no es una animación: también con efectos reducidos.
        if let Some((dx, _)) = self.arrastre.filter(|(dx, dy)| dx.abs() >= dy.abs()) {
            return if dx > 0.0 { dx } else { dx * 0.25 };
        }
        if tema::efectos_reducidos() {
            return 0.0;
        }
        if self.saliendo() {
            let origen = self.partida.map_or(0.0, |p| p.2);
            // Arrastrado más allá del recorrido normal sigue hacia donde iba.
            let destino = if origen > RECORRIDO {
                origen + RECORRIDO
            } else {
                RECORRIDO
            };
            return origen + (destino - origen) * self.avance_salida();
        }
        RECORRIDO * (1.0 - tema::C_ENTRADA.eval(tema::fraccion(self.desde.elapsed(), ENTRADA)))
    }

    /// ¿Ha empezado ya a irse?
    // 0.1.0
    pub fn saliendo(&self) -> bool {
        self.desde.elapsed() >= self.quieto
    }

    /// Pone el sitio en la pila. `true` si con eso hay que volver a pintarlo.
    // 0.1.0
    pub fn colocar(&mut self, profundidad: usize, fijo: Option<f32>) -> bool {
        let cambia = self.profundidad != profundidad || self.fijo != fijo;
        self.profundidad = profundidad;
        self.fijo = fijo;
        cambia
    }

    /// El alto que tendría delante, que es el que heredan los de atrás.
    // modified 0.1.1 -- reserva solo las líneas de cuerpo que hay
    // modificado en la 0.2.0 -- el cuerpo cuenta dentro de la columna de títulos, junto al icono
    pub fn alto_propio(&self) -> f32 {
        let n = &self.notificacion;
        let lineas = self.cuerpo.lines().count() as f32;
        if self.hecho.is_some() {
            return MARGEN * 2.0 + ICONO.max(LINEA_APP + 1.0 + ALTO_HECHO);
        }
        let mut alto = MARGEN * 2.0 + ICONO.max(ALTO_TITULOS + LINEA_CUERPO * lineas);
        if n.progreso.is_some() || n.progreso_indeterminado {
            alto += ALTO_PROGRESO;
        }
        alto += match n.forma {
            Forma::Auriculares(_) => ALTO_AROS,
            Forma::Bateria(_) => ALTO_BARRA_BATERIA,
            Forma::Normal => 0.0,
        };
        // modificado en la 0.2.2 -- los botones cuentan lo desplegados que estén
        alto + self.alto_botones() * self.desplegado.valor()
    }

    /// Lo que ocupan los botones desplegados del todo; 0 si no hay.
    // añadido en la 0.2.2
    fn alto_botones(&self) -> f32 {
        if self.hecho.is_some() {
            0.0
        } else if self.respuesta.is_some() {
            SEPARACION_BOTONES + ALTO_CAMPO
        } else if self.botones().is_empty() {
            0.0
        } else {
            SEPARACION_BOTONES + ALTO_BOTON
        }
    }

    /// El puntero entra en el aviso o sale de él. Encima se despliega y no
    /// caduca —no se va mientras lo estás leyendo o vas a pulsarlo—; al
    /// dejarlo se recoge y le quedan [`TRAS_DEJARLO`]. `true` si hay que
    /// repintar.
    // añadido en la 0.2.2
    pub fn señalar(&mut self, encima: bool) -> bool {
        // Respondiendo se queda como está: ni se recoge ni caduca mientras se escribe.
        if self.saliendo() || self.respuesta.is_some() || self.hecho.is_some() {
            return false;
        }
        let ahora = self.desde.elapsed();
        self.quieto = if encima {
            ahora + MAXIMO
        } else {
            (ahora + TRAS_DEJARLO).min(self.quieto)
        };
        let realce = !encima && self.realce.señalar(None);
        let cierre = self.cierre.ir_a(if encima { 1.0 } else { 0.0 });
        if encima {
            // Solo programa la espera si aún no se ha desplegado.
            if self.desplegado.destino() < 1.0 && self.desplegar_a.is_none() {
                self.desplegar_a = Some(Instant::now() + RETRASO_BOTONES);
            }
            return cierre || realce;
        }
        self.desplegar_a = None;
        self.desplegado.ir_a(0.0) || cierre || realce
    }

    /// El puntero se mueve dentro del aviso (coordenadas de la tarjeta):
    /// resalta el botón o la ✕ que tiene debajo. `true` si hay que repintar.
    // añadido en la 0.2.2
    pub fn señalar_en(&mut self, x: f32, y: f32) -> bool {
        let objetivo = if self.saliendo() {
            None
        } else {
            self.objetivo_en(x, y)
        };
        self.realce.señalar(objetivo)
    }

    /// Abre el campo de respuesta en el sitio de los botones. No caduca
    /// mientras se escribe.
    // añadido en la 0.2.2
    pub fn empezar_respuesta(&mut self) {
        self.respuesta = Some(String::new());
        self.quieto = self.desde.elapsed() + MAXIMO;
        self.desplegar_a = None;
        self.desplegado.ir_a(1.0);
    }

    /// Pasa a la confirmación de un botón, con su etiqueta como texto.
    // añadido en la 0.2.2
    pub fn confirmar_boton(&mut self, clave: &str) {
        let etiqueta = self
            .botones()
            .iter()
            .find(|a| a.clave == clave)
            .map_or_else(|| clave.to_string(), |a| a.etiqueta.clone());
        // «Desconectar» como confirmación no dice qué pasó; el diseño pone lo ocurrido.
        let texto = if clave == "desconectar"
            && matches!(self.notificacion.forma, Forma::Auriculares(_))
        {
            crate::emergente::t("Auriculares desconectados", "Headphones disconnected").to_string()
        } else {
            etiqueta
        };
        self.confirmar(texto, None);
    }

    /// Pasa a la confirmación de una respuesta, con lo que se mandó debajo.
    // añadido en la 0.2.2
    pub fn confirmar_respuesta(&mut self, texto: &str) {
        self.confirmar(
            crate::emergente::t("Respuesta enviada", "Reply sent").into(),
            Some(texto.to_string()),
        );
    }

    /// El aviso se queda lo justo para leer la confirmación y se va con su
    /// salida de siempre: no se llama a `descartar`, que lo echaría ya.
    fn confirmar(&mut self, texto: String, enviado: Option<String>) {
        if self.saliendo() {
            return;
        }
        self.quieto = self.desde.elapsed()
            + if enviado.is_some() {
                QUIETO_HECHO_ENVIADO
            } else {
                QUIETO_HECHO
            };
        self.hecho = Some((texto, enviado));
        self.respuesta = None;
        self.desplegar_a = None;
        self.desplegado.fijar(0.0);
    }

    /// ¿Está esperando que se escriba?
    // añadido en la 0.2.2
    pub fn respondiendo(&self) -> bool {
        self.respuesta.is_some()
    }

    /// Una tecla para el campo. `None` si no es suya y debe seguir su camino.
    // añadido en la 0.2.2
    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Option<TeclaAviso> {
        use crate::TeclaPulsada as T;
        let texto = self.respuesta.as_mut()?;
        match tecla {
            // Esc borra lo escrito antes de cerrar el campo: un mensaje se
            // contesta a medias muchas veces.
            T::Escape if !texto.is_empty() => texto.clear(),
            T::Escape => self.respuesta = None,
            T::BorrarTodo => texto.clear(),
            T::Caracter(c) if !c.is_control() => texto.push(c),
            T::Pegar(s) => {
                for c in s.chars() {
                    if matches!(c, '\r' | '\n' | '\t') {
                        texto.push(' ');
                    } else if !c.is_control() {
                        texto.push(c);
                    }
                }
            }
            T::Retroceso => {
                texto.pop();
            }
            // Un mensaje en blanco no se manda: es un dedo que resbala.
            T::Intro if texto.trim().is_empty() => {}
            T::Intro => return Some(TeclaAviso::Enviar(texto.trim().to_string())),
            _ => return None,
        }
        Some(TeclaAviso::Consumida)
    }

    /// Despliega los botones cuando se cumple la espera. Lo llama el bucle en
    /// cada vuelta, con `&mut`, porque la vista solo tiene `&self`.
    // añadido en la 0.2.2
    pub fn avanzar(&mut self) {
        if self.desplegar_a.is_some_and(|t| Instant::now() >= t) {
            self.desplegar_a = None;
            self.desplegado.ir_a(1.0);
        }
    }

    /// Desplegado del todo sin animar, para los tests de geometría.
    #[cfg(test)]
    fn desplegar_ya(&mut self) {
        self.desplegado.fijar(1.0);
    }

    /// El alto cambia mientras se despliega: hay que repintar cada fotograma.
    // añadido en la 0.2.2
    pub fn redibujando(&self) -> bool {
        self.desplegado.animando()
            || self.cierre.animando()
            || self.desplegar_a.is_some()
            || self.realce.animando()
    }

    /// Cuánto más abajo asoma que el de delante, en lógicos. El zoom del
    /// compositor crece desde el centro, así que además de bajar `ASOMO` por
    /// nivel hay que compensar lo que la tarjeta encogida sube su base.
    // 0.1.0
    pub fn desplazamiento(&self) -> f32 {
        let nivel = self.profundidad as f32;
        let a_mano = match self.arrastre {
            Some((dx, dy)) if dy.abs() > dx.abs() => {
                if dy < 0.0 {
                    dy
                } else {
                    dy * 0.25
                }
            }
            _ => 0.0,
        };
        nivel * ASOMO + self.alto_tarjeta() * (1.0 - self.encogido()) / 2.0 + a_mano
    }

    // 0.1.0
    fn encogido(&self) -> f32 {
        1.0 - ENCOGE_POR_NIVEL * self.profundidad as f32
    }

    /// Alto de la tarjeta, sin el margen de la sombra.
    // 0.1.0
    // modificado en la 0.2.2 -- el alto dibujado, no el del buffer, que reserva los botones
    pub fn alto_tarjeta(&self) -> f32 {
        self.alto()
    }

    /// Entra creciendo un poco, como los popovers.
    // 0.1.0
    // modified 0.1.1 -- la salida parte de la escala que tenía; sin zoom con efectos reducidos
    pub fn escala(&self) -> f32 {
        if tema::efectos_reducidos() {
            return self.encogido();
        }
        if self.saliendo() {
            let origen = self.partida.map_or(1.0, |p| p.1);
            return (origen + (0.98 - origen) * self.avance_salida()) * self.encogido();
        }
        let t = self.desde.elapsed();
        let entrada = if t >= ENTRADA {
            1.0
        } else {
            0.94 + 0.06 * tema::C_MUELLE_POPOVER.eval(tema::avance(t, ENTRADA))
        };
        entrada * self.encogido()
    }

    /// El temporizador despierta al empezar la salida; desde ahí hacen falta
    /// fotogramas hasta acabarla aunque no haya entrada del usuario.
    // 0.1.0
    // modified 0.1.1 -- cuenta también la salida
    pub fn animando(&self) -> bool {
        let t = self.desde.elapsed();
        t < ENTRADA || (t >= self.quieto && t < self.quieto + SALIDA) || self.redibujando()
    }

    /// Las acciones que van en botón. `default` no es un botón: según la
    /// especificación es lo que pasa al pulsar el aviso, y las apps la mandan
    /// con etiqueta vacía o «Activate», que se pintaba como un botón más.
    // 0.1.0
    fn botones(&self) -> Vec<&crate::notificaciones::Accion> {
        self.notificacion
            .acciones
            .iter()
            .filter(|a| a.clave != "default")
            .collect()
    }

    /// Dónde está cada botón, en x, relativo a la tarjeta. Lo usan el dibujo y
    /// el clic, que antes calculaban cada uno a su manera: los botones se
    /// dibujaban al ancho de su texto y el clic repartía la tarjeta a partes
    /// iguales, así que con «Responder» y «Marcar como leído» pulsar el
    /// segundo ejecutaba el primero.
    // 0.1.0
    fn huecos_botones(n: usize) -> impl Iterator<Item = (f32, f32)> {
        let ancho = (ANCHO - 2.0 * MARGEN - HUECO_BOTONES * (n as f32 - 1.0)) / n as f32;
        (0..n).map(move |i| (MARGEN + i as f32 * (ancho + HUECO_BOTONES), ancho))
    }

    /// Si pulsar el cuerpo tiene algo que hacer: la acción `default`.
    // 0.1.0
    pub fn accion_por_defecto(&self) -> Option<String> {
        if self.hecho.is_some() {
            return None;
        }
        self.notificacion
            .acciones
            .iter()
            .find(|a| a.clave == "default")
            .map(|a| a.clave.clone())
    }

    /// Alto de la tarjeta según lo que lleva: la cabecera siempre, y el
    /// cuerpo, la barra y los botones solo si hay. Con alto fijo, un aviso sin
    /// cuerpo dejaba un hueco y uno con cuerpo y botones no cabía.
    // 0.1.0
    fn alto(&self) -> f32 {
        self.fijo.unwrap_or_else(|| self.alto_propio())
    }

    /// Dónde empiezan los botones, desde arriba de la tarjeta: pegados al
    /// margen inferior.
    // 0.1.0
    fn y_botones(&self) -> f32 {
        self.alto()
            - MARGEN
            - if self.respuesta.is_some() {
                ALTO_CAMPO
            } else {
                ALTO_BOTON
            }
    }

    // 0.1.0
    pub fn size(&self) -> (f32, f32) {
        // modificado en la 0.2.2 -- reserva los botones desplegados: así el buffer no cambia de tamaño al desplegarse
        let alto = self.fijo.unwrap_or_else(|| {
            self.alto_propio() + self.alto_botones() * (1.0 - self.desplegado.valor())
        });
        (ANCHO + MARGEN_SOMBRA * 2.0, alto + MARGEN_SOMBRA * 2.0)
    }

    /// Lo pulsado en un punto, en coordenadas relativas a la tarjeta (sin el
    /// margen reservado para la sombra). Solo el de delante tiene algo que
    /// pulsar: los de atrás no enseñan contenido.
    // 0.1.0
    pub fn accion_en(&self, x: f32, y: f32) -> Option<Pulsado> {
        match self.objetivo_en(x, y)? {
            CIERRE_IDX => Some(Pulsado::Cerrar),
            ENVIAR_IDX => self
                .respuesta
                .as_deref()
                .map(|t| Pulsado::Enviar(t.trim().to_string())),
            i => match self.botones()[i].clave.as_str() {
                "inline-reply" => Some(Pulsado::Responder),
                clave => Some(Pulsado::Boton(clave.to_string())),
            },
        }
    }

    /// Qué hay pulsable en el punto: la ✕ o la posición de un botón. Lo
    /// comparten el clic y el realce para que resalte justo lo que se pulsa.
    fn objetivo_en(&self, x: f32, y: f32) -> Option<usize> {
        if self.profundidad > 0 || self.hecho.is_some() {
            return None;
        }
        let x_cierre = ANCHO - CIERRE_BORDE - CIERRE;
        // La ✕ solo existe con el puntero encima (sustituye a la hora).
        if self.cierre.valor() > 0.5
            && (x_cierre..=x_cierre + CIERRE).contains(&x)
            && (CIERRE_BORDE..=CIERRE_BORDE + CIERRE).contains(&y)
        {
            return Some(CIERRE_IDX);
        }
        let n = self.botones().len();
        let y_botones = self.y_botones();
        // Respondiendo solo se pulsa la flecha, y solo con algo escrito.
        if let Some(texto) = &self.respuesta {
            let x_enviar = ANCHO - MARGEN - ALTO_CAMPO;
            return (!texto.trim().is_empty()
                && (x_enviar..=x_enviar + ALTO_CAMPO).contains(&x)
                && (y_botones..=y_botones + ALTO_CAMPO).contains(&y))
            .then_some(ENVIAR_IDX);
        }
        // Recogidos no se pulsan: un clic ahí es el cuerpo del aviso.
        if n == 0
            || self.desplegado.valor() < 0.9
            || !(y_botones..=y_botones + ALTO_BOTON).contains(&y)
        {
            return None;
        }
        Self::huecos_botones(n).position(|(x0, ancho)| (x0..=x0 + ancho).contains(&x))
    }

    // modified 0.1.1 -- tarjeta compacta: app en gris arriba y resumen en negrita
    pub fn view(&self) -> PanelElement<'_> {
        if self.profundidad > 0 {
            return self.vista_de_atras();
        }
        let n = &self.notificacion;
        // modificado en la 0.2.2 -- los Buds y la batería baja llevan su propio icono
        let dibujo: PanelElement<'_> = match (&n.forma, &n.icono) {
            (Forma::Auriculares(_), _) => baldosa(
                iced_widget::image(crate::actividad::FOTOS_BUDS[3].clone())
                    .width(Length::Fixed(ICONO - 4.0))
                    .height(Length::Fixed(ICONO - 4.0))
                    .into(),
                tema::aviso_relieve(),
            ),
            (Forma::Bateria(nivel), _) => baldosa(
                svg(&pila_svg(*nivel), 24.0),
                tema::mezclar(tema::card(), tema::amarillo(), 0.18),
            ),
            (Forma::Normal, Some(ic)) => crate::icono::ver(ic, ICONO, ICONO),
            (Forma::Normal, None) => Space::new().width(Length::Fixed(ICONO)).into(),
        };
        let ancho_texto = ANCHO - MARGEN * 2.0;
        // Sin pisar la ✕, que cuelga de la esquina de la cabecera.
        let ancho_titulos = ancho_texto - ICONO - 12.0 - CIERRE - 8.0;
        // Quién avisa, pequeño y en gris, y debajo el resumen en negrita: lo
        // que se lee de un vistazo es qué pasa, no qué app lo dice. En rojo si
        // es crítica: es lo único que la distingue de las demás sin meterle un
        // fondo de color que taparía su icono.
        // Como el diseño: la hora a la derecha de la app, que cede el sitio a
        // la ✕ al señalar el aviso (se funden con el mismo valor que despliega).
        let visible = self.cierre.valor();
        let mut titulos = column![
            row![
                text(crate::emergente::recortar_texto(
                    &n.app,
                    // Deja sitio a «hace 59 min», la hora más larga.
                    ANCHO_CUERPO - ANCHO_HORA,
                    12.0
                ))
                .size(12.0)
                .line_height(iced_core::text::LineHeight::Absolute(LINEA_APP.into()))
                .color(tema::aviso_texto2()),
                Space::new().width(Length::Fill),
                text(hace(n.llegada.elapsed()))
                    .size(11.5)
                    .line_height(iced_core::text::LineHeight::Absolute(LINEA_APP.into()))
                    .color(tema::mezclar(
                        tema::aviso_texto2(),
                        tema::aviso_fondo(),
                        visible
                    )),
            ]
            .width(Length::Fixed(ANCHO_CUERPO)),
            Space::new().height(Length::Fixed(1.0)),
        ];
        if let Some((texto, enviado)) = &self.hecho {
            titulos = titulos.push(Self::confirmacion(texto, enviado.as_deref()));
        } else {
            titulos = titulos.push(
                text(crate::emergente::recortar_texto(
                    &n.resumen,
                    ancho_titulos,
                    14.0,
                ))
                .size(14.0)
                .line_height(iced_core::text::LineHeight::Absolute(LINEA_RESUMEN.into()))
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(if n.critica && !matches!(n.forma, Forma::Bateria(_)) {
                    // La batería baja ya avisa con su pila ámbar: en rojo
                    // además gritaba dos veces lo mismo.
                    tema::rojo()
                } else {
                    tema::texto()
                }),
            );
        }
        // modificado en la 0.2.0 -- el cuerpo va bajo el resumen y no bajo el icono
        // Debajo del resumen y no de toda la cabecera: colgado del margen
        // empezaba bajo el icono, desalineado del título, y además sumaba un
        // respiro y un renglón de alto que el aviso no necesita.
        if self.hecho.is_none() && !self.cuerpo.is_empty() {
            // Hasta dos líneas y no una: en una sola, un aviso normal de
            // «Descarga completada» se quedaba en el nombre del archivo
            // cortado a la mitad.
            titulos = titulos.push(text(self.cuerpo.as_str()).size(13.0).color(tema::texto()));
        }
        let cabecera = row![
            dibujo,
            Space::new().width(Length::Fixed(12.0)),
            // Ancho fijo: sin él, iced parte el resumen en dos líneas y la
            // tarjeta se desborda por abajo. Los títulos ya vienen recortados
            // al hueco que deja la ✕; el cuerpo usa el ancho entero.
            container(titulos).width(Length::Fixed(ANCHO_CUERPO)),
        ]
        .align_y(Vertical::Top);
        let mut contenido = column![cabecera];
        // añadido en la 0.2.2 -- aros de los Buds y barra de batería baja
        // La confirmación no lleva aros, barra ni progreso: sustituye a todo.
        let forma = if self.hecho.is_some() {
            &Forma::Normal
        } else {
            &n.forma
        };
        match forma {
            Forma::Auriculares(piezas) => {
                let aros =
                    piezas
                        .iter()
                        .enumerate()
                        .fold(row![], |fila, (i, (etiqueta, valor))| {
                            fila.push(
                                container(aro(*valor, i.min(2), etiqueta))
                                    .width(Length::Fill)
                                    .center_x(Length::Fill),
                            )
                        });
                contenido = contenido
                    .push(Space::new().height(Length::Fixed(MARGEN)))
                    .push(aros.width(Length::Fixed(ancho_texto)));
            }
            Forma::Bateria(nivel) => {
                let lleno = (ancho_texto - ICONO - 12.0 - 44.0) * f32::from(*nivel) / 100.0;
                let barra = container(
                    container(Space::new())
                        .width(Length::Fixed(lleno.max(4.0)))
                        .height(Length::Fixed(4.0))
                        .style(|_| container::Style {
                            background: Some(tema::amarillo().into()),
                            border: Border {
                                radius: 2.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                .width(Length::Fixed(ancho_texto - ICONO - 12.0 - 44.0))
                .height(Length::Fixed(4.0))
                .style(|_| container::Style {
                    background: Some(tema::aviso_relieve().into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                contenido = contenido
                    .push(Space::new().height(Length::Fixed(RESPIRO)))
                    .push(
                        row![
                            Space::new().width(Length::Fixed(ICONO + 12.0)),
                            barra,
                            Space::new().width(Length::Fixed(8.0)),
                            text(format!("{nivel} %"))
                                .size(12.0)
                                .color(tema::aviso_texto2()),
                        ]
                        .height(Length::Fixed(16.0))
                        .align_y(Vertical::Center),
                    );
            }
            Forma::Normal => {}
        }
        let progreso = n.progreso.filter(|_| self.hecho.is_none());
        if let Some(valor) = progreso {
            let lleno = (ancho_texto * valor as f32 / 100.0).max(2.0);
            contenido = contenido
                .push(Space::new().height(Length::Fixed(RESPIRO)))
                .push(
                    container(
                        container(Space::new())
                            .width(Length::Fixed(lleno))
                            .height(Length::Fixed(4.0))
                            .style(|_| iced_widget::container::Style {
                                background: Some(tema::acento().into()),
                                ..Default::default()
                            }),
                    )
                    .width(Length::Fixed(ancho_texto))
                    .height(Length::Fixed(4.0))
                    .style(|_| container::Style {
                        background: Some(tema::aviso_relieve().into()),
                        border: Border {
                            radius: 2.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
        } else if n.progreso_indeterminado && self.hecho.is_none() {
            // Ocupa lo mismo que la barra: `alto()` reserva ese hueco.
            contenido = contenido
                .push(Space::new().height(Length::Fixed(RESPIRO - 4.0)))
                .push(text("En curso…").size(10.0).color(tema::aviso_texto2()));
        }
        let acciones = if self.hecho.is_some() {
            Vec::new()
        } else {
            self.botones()
        };
        // Recogido, el borde de arriba de los botones cae 4 px dentro de la
        // tarjeta (el margen de abajo es 12 y el respiro 8) y asomaba en la
        // esquina; hasta que hay sitio para ellos no se pintan.
        if let Some(texto) = &self.respuesta {
            // El campo ocupa el sitio de los botones y la flecha espera
            // apagada hasta que haya algo que mandar.
            let escrito: PanelElement<'_> = if texto.is_empty() {
                text("Responder…")
                    .size(13.0)
                    .color(tema::aviso_texto2())
                    .into()
            } else {
                text(crate::emergente::recortar_texto(
                    &format!("{texto}▏"),
                    ancho_texto - ALTO_CAMPO - HUECO_BOTONES - 28.0,
                    13.0,
                ))
                .size(13.0)
                .color(tema::texto())
                .into()
            };
            let campo = container(row![Space::new().width(Length::Fixed(14.0)), escrito])
                .width(Length::Fill)
                .height(Length::Fixed(ALTO_CAMPO))
                .center_y(Length::Fixed(ALTO_CAMPO))
                .style(|_| container::Style {
                    background: Some(tema::aviso_relieve().into()),
                    border: Border {
                        radius: (ALTO_CAMPO / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            let listo = !texto.trim().is_empty();
            let flecha = svg(
                &format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M5 12l7-7 7 7"/></svg>"#,
                    css(tema::sobre_acento())
                ),
                15.0,
            );
            let enviar = container(flecha)
                .width(Length::Fixed(ALTO_CAMPO))
                .height(Length::Fixed(ALTO_CAMPO))
                .center_x(Length::Fixed(ALTO_CAMPO))
                .center_y(Length::Fixed(ALTO_CAMPO))
                .style(move |_| container::Style {
                    // Sin translúcidos: «apagada» es el acento mezclado con el fondo.
                    background: Some(
                        if listo {
                            tema::acento()
                        } else {
                            tema::mezclar(tema::aviso_fondo(), tema::acento(), 0.35)
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: (ALTO_CAMPO / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            contenido = contenido.push(Space::new().height(Length::Fill)).push(
                row![
                    campo,
                    Space::new().width(Length::Fixed(HUECO_BOTONES)),
                    enviar
                ]
                .align_y(Vertical::Center),
            );
        } else if !acciones.is_empty() && self.desplegado.valor() > 0.15 {
            let huecos: Vec<_> = Self::huecos_botones(acciones.len()).collect();
            // Solo la primera acción es primaria; las demás van en el fondo de
            // hover, como el botón secundario del sistema (AI-DESIGN-SYSTEM §4).
            // Con todos en acento no había un botón principal que mirar.
            let botones = acciones.iter().zip(huecos).enumerate().fold(
                row![],
                |fila, (i, (accion, (_, ancho)))| {
                    // «Responder» es el principal si lo hay (como en el diseño);
                    // si no, la primera acción, que es la que manda la app.
                    // Los Buds no tienen principal: ajustar el sonido y
                    // desconectar pesan lo mismo.
                    let primario = if acciones.iter().any(|a| a.clave == "inline-reply") {
                        accion.clave == "inline-reply"
                    } else {
                        i == 0 && !matches!(self.notificacion.forma, Forma::Auriculares(_))
                    };
                    let encima = self.realce.intensidad(i);
                    fila.push(
                        container(
                            text(crate::emergente::recortar_texto(
                                &accion.etiqueta,
                                ancho - 12.0,
                                13.0,
                            ))
                            .size(13.0)
                            .font(Font {
                                weight: Weight::Semibold,
                                ..Font::DEFAULT
                            })
                            .color(if primario {
                                tema::sobre_acento()
                            } else {
                                tema::texto()
                            }),
                        )
                        .width(Length::Fixed(ancho))
                        .height(Length::Fixed(ALTO_BOTON))
                        .center_x(Length::Fixed(ancho))
                        .center_y(Length::Fixed(ALTO_BOTON))
                        .style(move |_| container::Style {
                            background: Some(
                                if primario {
                                    tema::acento()
                                } else {
                                    tema::mezclar(
                                        tema::aviso_relieve(),
                                        tema::aviso_boton_hover(),
                                        encima,
                                    )
                                }
                                .into(),
                            ),
                            border: Border {
                                radius: tema::R_BOTON_PEQUENO.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    )
                    .push(Space::new().width(Length::Fixed(HUECO_BOTONES)))
                },
            );
            // Se empuja hasta el fondo con un hueco que estira: así los
            // botones quedan a la altura que mira `accion_en` aunque el cuerpo
            // ocupe una línea o dos.
            contenido = contenido
                .push(Space::new().height(Length::Fill))
                .push(botones);
        }
        let alto = self.alto();
        // modificado en la 0.2.2 -- recortado al alto de ahora: los botones se descubren al desplegarse
        let completo = alto + self.alto_botones() * (1.0 - self.desplegado.valor());
        let cuerpo = crate::recorte::Recorte::new(
            container(contenido)
                .width(Length::Fixed(ANCHO))
                .height(Length::Fixed(completo))
                .padding(MARGEN as u16),
            ANCHO,
            alto,
            0.0,
        );
        let tarjeta = container(stack![cuerpo, Self::cierre(self.cierre.valor())])
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(alto))
            // modificado en la 0.2.2 -- teñido con el fondo de pantalla, radio 22 y filo de 1 px
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::aviso_fondo().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    width: 1.0,
                    color: tema::aviso_filo(),
                },
                // En oscuro la sombra negra se lee como un borde. El fondo
                // carbón ya separa el toast; en claro se conserva el nivel de
                // sombra que prescribe el HIG para superficies temporales.
                shadow: if tema::es_claro() {
                    iced_core::Shadow {
                        color: Color {
                            a: 0.15,
                            ..Color::BLACK
                        },
                        offset: iced_core::Vector::new(0.0, 4.0),
                        blur_radius: 20.0,
                    }
                } else {
                    iced_core::Shadow::default()
                },
                ..Default::default()
            });
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }

    /// El sello verde con el ✓, el texto de lo hecho y, debajo, lo enviado.
    // añadido en la 0.2.2
    fn confirmacion<'a>(texto: &'a str, enviado: Option<&'a str>) -> PanelElement<'a> {
        let sello = svg(
            &format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="12" fill="{verde}"/><path d="M7 12.5l3.2 3.2L17 8.8" stroke="{blanco}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
                verde = css(tema::verde()),
                blanco = css(tema::sobre_acento()),
            ),
            24.0,
        );
        let mut lineas = column![
            text(crate::emergente::recortar_texto(
                texto,
                ANCHO_CUERPO - 34.0,
                13.0
            ))
            .size(13.0)
            .font(Font {
                weight: Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(tema::texto())
        ];
        if let Some(enviado) = enviado {
            lineas = lineas.push(
                text(format!(
                    "«{}»",
                    crate::emergente::recortar_texto(enviado, ANCHO_CUERPO - 50.0, 12.0)
                ))
                .size(12.0)
                .color(tema::aviso_texto2()),
            );
        }
        row![sello, Space::new().width(Length::Fixed(10.0)), lineas]
            .height(Length::Fixed(ALTO_HECHO))
            .align_y(Vertical::Center)
            .into()
    }

    /// La ✕ de la esquina: gris y discreta, porque la roja es la de cerrar
    /// ventanas y esta solo quita un aviso.
    // 0.1.0
    fn cierre(visible: f32) -> PanelElement<'static> {
        // Sin translúcidos: el fundido es una mezcla opaca con el fondo del
        // aviso, y por eso en reposo no se pinta: su fondo opaco taparía la
        // hora, que ocupa el mismo sitio.
        if visible < 0.02 {
            return Space::new().into();
        }
        let fondo = tema::aviso_fondo();
        let aspa: PanelElement<'static> = match crate::icono::propio("cerrar") {
            Some(ic) => crate::icono::ver_teñido_propio(
                &ic,
                CIERRE * 0.55,
                tema::mezclar(fondo, tema::aviso_texto2(), visible),
            ),
            None => Space::new().into(),
        };
        container(
            container(aspa)
                .center_x(Length::Fixed(CIERRE))
                .center_y(Length::Fixed(CIERRE))
                .style(move |_| container::Style {
                    background: Some(tema::mezclar(fondo, tema::aviso_relieve(), visible).into()),
                    border: Border {
                        radius: tema::R_CHIP.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fill)
        .align_x(iced_core::alignment::Horizontal::Right)
        .padding(CIERRE_BORDE as u16)
        .into()
    }

    /// Un nivel de la pila que no es el de delante: la tarjeta sola, un punto
    /// más oscura cuanto más al fondo, sin contenido ni sombra.
    // 0.1.0
    fn vista_de_atras(&self) -> PanelElement<'_> {
        let tarjeta = container(Space::new())
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(self.alto()))
            .style(move |_| container::Style {
                // modificado en la 0.2.2 -- `--tinte-atras` del diseño, igual en todos los niveles
                background: Some(tema::aviso_fondo_atras().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    width: 1.0,
                    color: tema::aviso_filo(),
                },
                ..Default::default()
            });
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }
}

/// La edad del aviso como la enseña el diseño: «ahora» o «hace N min».
// añadido en la 0.2.2
fn hace(edad: Duration) -> String {
    match edad.as_secs() / 60 {
        0 => "ahora".into(),
        m => format!("hace {m} min"),
    }
}

/// El icono sobre una baldosa del tamaño de los de aplicación.
// añadido en la 0.2.2
fn baldosa(dentro: PanelElement<'_>, fondo: Color) -> PanelElement<'_> {
    container(dentro)
        .center_x(Length::Fixed(ICONO))
        .center_y(Length::Fixed(ICONO))
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: 9.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

// añadido en la 0.2.2
fn svg(fuente: &str, lado: f32) -> PanelElement<'static> {
    match crate::icono::desde_svg(fuente) {
        crate::icono::Icono::Svg(handle) => iced_widget::svg(handle)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .into(),
        _ => Space::new().into(),
    }
}

// añadido en la 0.2.2
fn css(c: Color) -> String {
    let [r, g, b, _] = c.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// La pila de batería baja con el nivel dentro, del ámbar de aviso.
// añadido en la 0.2.2
fn pila_svg(nivel: u8) -> String {
    let lleno = (12.0 * f32::from(nivel) / 100.0).max(2.2);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 26 26" fill="none"><rect x="3" y="8" width="18" height="10" rx="3" stroke="{a}" stroke-width="1.8"/><rect x="22.2" y="11" width="1.8" height="4" rx=".9" fill="{a}"/><rect x="5.2" y="10.2" width="{lleno:.1}" height="5.6" rx="1.1" fill="{a}"/></svg>"#,
        a = css(tema::amarillo())
    )
}

/// Un aro de los Buds: el surco, el arco de lo que queda (verde, ámbar por
/// debajo del 20 %) con la foto de la pieza dentro, y debajo el porcentaje y
/// su etiqueta.
// añadido en la 0.2.2
fn aro(valor: u8, foto: usize, etiqueta: &str) -> PanelElement<'static> {
    let largo = 2.0 * std::f32::consts::PI * 23.0;
    let color = if valor <= 20 {
        tema::amarillo()
    } else {
        tema::verde()
    };
    let dibujo = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 52 52" fill="none"><circle cx="26" cy="26" r="23" stroke="{surco}" stroke-width="4"/><circle cx="26" cy="26" r="23" stroke="{color}" stroke-width="4" stroke-linecap="round" stroke-dasharray="{lleno:.1} {largo:.1}" transform="rotate(-90 26 26)"/></svg>"#,
        surco = css(tema::aviso_relieve()),
        color = css(color),
        lleno = largo * f32::from(valor) / 100.0,
    );
    column![
        stack![
            svg(&dibujo, ARO),
            container(
                iced_widget::image(crate::actividad::FOTOS_BUDS[foto].clone())
                    .width(Length::Fixed(30.0))
                    .height(Length::Fixed(30.0))
            )
            .center_x(Length::Fixed(ARO))
            .center_y(Length::Fixed(ARO)),
        ],
        Space::new().height(Length::Fixed(5.0)),
        row![
            text(format!("{valor} %"))
                .size(12.5)
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tema::texto()),
            text(format!(" {etiqueta}"))
                .size(11.0)
                .color(tema::aviso_texto2()),
        ]
        .align_y(Vertical::Bottom),
    ]
    .align_x(iced_core::alignment::Horizontal::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boton(clave: &str) -> Pulsado {
        Pulsado::Boton(clave.into())
    }

    fn boton_accion(clave: &str, etiqueta: &str) -> crate::notificaciones::Accion {
        crate::notificaciones::Accion {
            clave: clave.into(),
            etiqueta: etiqueta.into(),
        }
    }

    fn notif(critica: bool) -> Notificacion {
        Notificacion::nueva(
            1,
            "Prueba".into(),
            "Resumen".into(),
            String::new(),
            "",
            critica,
        )
    }

    /// La caducidad que pide la aplicación se respeta, pero acotada: hay
    /// programas que piden medio minuto, y eso deja de ser un aviso.
    #[test]
    fn la_caducidad_pedida_se_acota() {
        assert_eq!(
            Toast::new(notif(false), 3000).quieto,
            Duration::from_secs(3)
        );
        assert_eq!(Toast::new(notif(false), 60_000).quieto, MAXIMO);
        assert_eq!(Toast::new(notif(false), 500).quieto, MINIMO);
        // «Lo que decida el servidor» y «que no se vaya sola».
        assert_eq!(Toast::new(notif(false), -1).quieto, QUIETO);
        assert_eq!(Toast::new(notif(true), -1).quieto, QUIETO_CRITICA);
        assert_eq!(Toast::new(notif(false), 0).quieto, MAXIMO);
    }

    /// Entra, se queda y se va; y pulsarlo lo manda a irse sin quitarlo de
    /// golpe, para que la salida se vea.
    #[test]
    fn entra_y_al_descartarlo_se_va_con_su_animacion() {
        let mut t = Toast::new(notif(false), -1);
        assert!(
            t.alfa() < 1.0 && t.escala() < 1.0,
            "tiene que estar entrando"
        );
        assert!(t.animando());
        std::thread::sleep(ENTRADA);
        assert_eq!(t.alfa(), 1.0);
        assert_eq!(t.escala(), 1.0);
        assert!(!t.animando());

        t.descartar();
        let queda = t.queda().expect("todavía se está yendo");
        assert!(
            queda <= SALIDA,
            "la salida tiene que quedar por delante: {queda:?}"
        );
        // Justo al descartarlo el alfa sigue entero —la salida empieza ahí—; lo
        // que importa es que a partir de ese momento baje.
        assert_eq!(t.alfa(), 1.0);
        std::thread::sleep(SALIDA / 2);
        assert!(t.alfa() < 1.0, "no se está yendo");
        assert!(t.animando(), "la salida también necesita fotogramas");
        assert!(t.desplazamiento_horizontal() > 0.0);
        assert!(t.escala() < 1.0);
    }

    #[test]
    fn la_caducidad_anima_la_salida_y_deja_de_pedir_fotogramas_al_acabar() {
        let mut t = Toast::new(notif(false), -1);
        t.desde = Instant::now() - ENTRADA;
        assert!(!t.animando(), "quieto no debe pedir fotogramas");
        t.desde = Instant::now() - t.quieto - SALIDA / 2;
        assert!(t.saliendo());
        assert!(t.animando());
        assert!(t.alfa() > 0.0 && t.alfa() < 1.0);
        t.desde = Instant::now() - t.quieto - SALIDA;
        assert!(!t.animando());
        assert!(t.queda().is_none());
        assert_eq!(t.alfa(), 0.0);
    }

    #[test]
    fn despedir_durante_la_entrada_conserva_la_posicion_y_no_reinicia_la_salida() {
        let mut t = Toast::new(notif(false), -1);
        t.desde = Instant::now() - ENTRADA / 4;
        let alfa = t.alfa();
        let x = t.desplazamiento_horizontal();
        t.descartar();
        assert!((t.alfa() - alfa).abs() < 0.02, "el alfa no debe saltar");
        assert!(
            (t.desplazamiento_horizontal() - x).abs() < 1.0,
            "la posición no debe saltar"
        );
        let desde = t.desde;
        t.descartar();
        assert_eq!(t.desde, desde, "descartar de nuevo no alarga la salida");
    }

    /// El caso de KDE Connect: `default` más dos botones de largo distinto.
    /// Cada punto del botón dibujado tiene que devolver **ese** botón.
    #[test]
    fn cada_boton_responde_donde_se_dibuja() {
        let accion = |clave: &str, etiqueta: &str| crate::notificaciones::Accion {
            clave: clave.into(),
            etiqueta: etiqueta.into(),
        };
        let mut t = Toast::new(
            Notificacion::nueva_con_datos(
                9,
                "KDE Connect".into(),
                "Mensaje".into(),
                "Hola".into(),
                "",
                false,
                vec![
                    accion("default", ""),
                    accion("responder", "Responder"),
                    accion("leido", "Marcar como leído"),
                ],
                None,
                false,
            ),
            -1,
        );
        assert_eq!(t.botones().len(), 2, "`default` no es un botón");
        assert_eq!(t.accion_por_defecto().as_deref(), Some("default"));
        // Recogidos, donde irían los botones es el cuerpo del aviso.
        assert_eq!(
            t.accion_en(MARGEN + 5.0, t.y_botones() + ALTO_BOTON / 2.0),
            None
        );
        t.desplegar_ya();
        let y = t.y_botones() + ALTO_BOTON / 2.0;
        for ((x0, ancho), clave) in Toast::huecos_botones(2).zip(["responder", "leido"]) {
            for x in [x0 + 1.0, x0 + ancho / 2.0, x0 + ancho - 1.0] {
                assert_eq!(t.accion_en(x, y), Some(boton(clave)), "x={x}");
            }
        }
        // La mitad de arriba del botón también es el botón.
        assert_eq!(
            t.accion_en(MARGEN + 5.0, t.y_botones() + 1.0),
            Some(boton("responder"))
        );
        // Por encima de los botones es el cuerpo, no una acción.
        assert_eq!(t.accion_en(MARGEN + 5.0, t.y_botones() - 10.0), None);
    }

    #[test]
    fn las_acciones_amplian_el_toast_y_devuelven_su_clave() {
        let mut t = Toast::new(
            Notificacion::nueva_con_datos(
                7,
                "Prueba".into(),
                "Resumen".into(),
                String::new(),
                "",
                false,
                vec![crate::notificaciones::Accion {
                    clave: "abrir".into(),
                    etiqueta: "Abrir".into(),
                }],
                Some(42),
                false,
            ),
            -1,
        );
        assert!(t.size().1 > MARGEN * 2.0 + ICONO + MARGEN_SOMBRA * 2.0);
        // El buffer ya reserva los botones: desplegarse no lo cambia de tamaño.
        let antes = t.size();
        t.desplegar_ya();
        assert_eq!(t.size(), antes);
        assert_eq!(
            t.accion_en(80.0, t.y_botones() + ALTO_BOTON / 2.0),
            Some(boton("abrir"))
        );
    }

    /// La ✕ quita el aviso, y solo en el de delante: los de atrás no enseñan
    /// nada que pulsar.
    #[test]
    fn la_aspa_cierra_solo_el_de_delante() {
        let mut t = Toast::new(notif(false), -1);
        let (x, y) = (
            ANCHO - CIERRE_BORDE - CIERRE / 2.0,
            CIERRE_BORDE + CIERRE / 2.0,
        );
        // Sin el puntero encima la ✕ no se ve, y por tanto no se pulsa.
        assert_eq!(t.accion_en(x, y), None);
        t.cierre.fijar(1.0);
        assert_eq!(t.accion_en(x, y), Some(Pulsado::Cerrar));
        t.colocar(1, Some(t.alto_propio()));
        assert_eq!(t.accion_en(x, y), None);
    }

    /// Señalar no despliega los botones al instante: esperan 150 ms para que
    /// pasar el puntero de camino a otra cosa no haga saltar la tarjeta.
    #[test]
    fn los_botones_esperan_antes_de_desplegarse() {
        let mut t = Toast::new(notif(false), -1);
        t.señalar(true);
        t.avanzar();
        assert_eq!(t.desplegado.destino(), 0.0, "antes de los 150 ms");
        assert!(t.animando(), "pide fotogramas mientras espera");
        t.desplegar_a = Some(Instant::now() - Duration::from_millis(1));
        t.avanzar();
        assert_eq!(t.desplegado.destino(), 1.0);
        // Salir antes de tiempo cancela la espera.
        let mut u = Toast::new(notif(false), -1);
        u.señalar(true);
        u.señalar(false);
        assert!(u.desplegar_a.is_none());
    }

    /// Arrastrado a la derecha pasado el umbral se quita; corto, vuelve; arriba
    /// se guarda. La velocidad cuenta: un lanzamiento corto pero rápido vale.
    #[test]
    fn el_arrastre_quita_guarda_o_vuelve() {
        let mut t = Toast::new(notif(false), -1);
        t.arrastrar(60.0, 5.0);
        assert_eq!(t.desplazamiento_horizontal(), 60.0);
        assert_eq!(t.soltar_arrastre(0.0, 0.0), None);
        t.arrastrar(60.0, 5.0);
        assert_eq!(t.soltar_arrastre(600.0, 0.0), Some(Soltado::Quitar));
        assert!(t.saliendo());
        let mut u = Toast::new(notif(false), -1);
        u.arrastrar(4.0, -80.0);
        assert_eq!(u.soltar_arrastre(0.0, 0.0), Some(Soltado::Guardar));
        let mut v = Toast::new(notif(false), -1);
        v.arrastrar(-100.0, 0.0);
        assert_eq!(
            v.desplazamiento_horizontal(),
            -25.0,
            "a la izquierda, con goma"
        );
    }

    /// «Responder» abre el campo en el aviso; lo escrito se manda con Intro o con
    /// la flecha, un mensaje en blanco no, y Esc borra antes de cerrar el campo.
    #[test]
    fn responder_abre_el_campo_y_manda_lo_escrito() {
        use crate::TeclaPulsada as T;
        let mut n = notif(false);
        n.acciones = vec![boton_accion("inline-reply", "Responder")];
        let mut t = Toast::new(n, -1);
        t.desplegar_ya();
        let (x, y) = (ANCHO / 2.0, t.y_botones() + 5.0);
        assert_eq!(t.accion_en(x, y), Some(Pulsado::Responder));
        assert_eq!(
            t.tecla(T::Caracter('a')),
            None,
            "sin campo la tecla no es suya"
        );
        t.empezar_respuesta();
        assert!(t.respondiendo());
        assert_eq!(
            t.tecla(T::Intro),
            Some(TeclaAviso::Consumida),
            "en blanco no se manda"
        );
        t.tecla(T::Caracter('h'));
        t.tecla(T::Caracter('i'));
        let x_enviar = ANCHO - MARGEN - ALTO_CAMPO / 2.0;
        let y_campo = t.y_botones() + ALTO_CAMPO / 2.0;
        assert_eq!(
            t.accion_en(x_enviar, y_campo),
            Some(Pulsado::Enviar("hi".into()))
        );
        assert_eq!(t.tecla(T::Escape), Some(TeclaAviso::Consumida));
        assert_eq!(
            t.accion_en(x_enviar, y_campo),
            None,
            "borrado no hay nada que enviar"
        );
        t.tecla(T::Caracter('o'));
        assert_eq!(t.tecla(T::Intro), Some(TeclaAviso::Enviar("o".into())));
        t.tecla(T::Escape);
        t.tecla(T::Escape);
        assert!(!t.respondiendo(), "Esc sobre el campo vacío lo cierra");
    }

    /// Cada nivel asoma `ASOMO` más por debajo que el anterior, aunque la
    /// tarjeta encogida suba su base: la base del nivel n queda a n·ASOMO de la
    /// de delante, que es lo que se ve.
    #[test]
    fn cada_nivel_asoma_por_debajo_del_anterior() {
        let mut t = Toast::new(notif(false), -1);
        std::thread::sleep(ENTRADA);
        let alto = t.alto_propio();
        for nivel in 1..=2 {
            t.colocar(nivel, Some(alto));
            let base = t.alto_tarjeta() * t.escala() / 2.0 + t.desplazamiento();
            let esperada = alto / 2.0 + nivel as f32 * ASOMO;
            assert!(
                (base - esperada).abs() < 0.01,
                "nivel {nivel}: {base} ≠ {esperada}"
            );
        }
    }

    #[test]
    fn respuesta_en_toast_admite_pegar_limpio() {
        use crate::TeclaPulsada as T;
        let mut t = Toast::new(notif(false), -1);
        t.empezar_respuesta();
        assert_eq!(
            t.tecla(T::Pegar("hola\n\x07mundo".into())),
            Some(TeclaAviso::Consumida)
        );
        assert_eq!(t.tecla(T::Intro), Some(TeclaAviso::Enviar("hola mundo".into())));
    }
}
